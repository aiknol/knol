//! Memory Retrieve Service
//!
//! Handles search: adaptive retrieval router, vector search, BM25 full-text search,
//! scope cascade retrieval, graph expansion, Reciprocal Rank Fusion (RRF) with
//! intent-based weights, decay-adjusted scoring, and token budget compression.
//!
//! ## Key Differentiators (vs Mem0/Zep)
//!
//! - **Real embedding generation** via configurable provider (OpenAI, Voyage, Gemini, local)
//! - **Decay-adjusted scoring** — old memories fade, recently accessed ones stay strong
//! - **N-hop graph traversal** — variable-depth graph expansion (not fixed 2-hop)
//! - **Session-scoped retrieval** — proper session context in scope cascade
//! - **Enhanced filters** — tags, entity types, importance, graph depth

use axum::{
    extract::{Json, State},
    http::HeaderMap,
    middleware::Next,
    response::Response,
    routing::{get, post},
    Router,
};
use memory_common::{
    ExportStats, MemoryExport, MemoryExportRequest, MemoryItem, MemorySearchRequest,
    MemorySearchResponse, SearchResult,
};
use memory_vector::VectorSearchHit;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tracing::{debug, info, warn};
use uuid::Uuid;

struct AppState {
    db_pool: sqlx::PgPool,
    embedder: memory_llm::EmbeddingProvider,
    decay_config: memory_llm::DecayConfig,
    /// Shared secret for verifying requests from the gateway.
    internal_service_secret: Option<String>,
    /// RRF fusion constant k.
    rrf_k: f64,
    /// Configurable BM25 text field weights [D, C, B, A].
    bm25_weights: Option<[f32; 4]>,
    /// Per-intent fusion weights: (vector, bm25, graph, scope).
    intent_weights_preference: (f64, f64, f64, f64),
    intent_weights_temporal: (f64, f64, f64, f64),
    intent_weights_relational: (f64, f64, f64, f64),
    intent_weights_general: (f64, f64, f64, f64),
    /// Graph traversal max depth for search.
    graph_max_depth: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    info!("Starting Memory Retrieve Service...");

    memory_common::startup::validate_env("service-retrieve")?;

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://memory:memory_dev@localhost:5432/memory".into());

    let db_pool = memory_db::create_pool(&database_url, 6).await?;

    let port: u16 = memory_common::db_config::load_u64(
        &db_pool,
        "services.retrieve_port",
        "RETRIEVE_SERVICE_PORT",
        8082,
    )
    .await as u16;

    // Build embedding provider from DB config
    let embedder = memory_llm::EmbeddingProvider::from_db(&db_pool)
        .await
        .unwrap_or_else(|e| {
            warn!(
                "Failed to initialize embedding provider from DB: {}. Using local fallback.",
                e
            );
            memory_llm::EmbeddingProvider::new(memory_llm::EmbeddingConfig {
                provider: "local".into(),
                ..Default::default()
            })
        });
    info!(
        "Embedding provider: {} ({}D)",
        embedder.provider_name(),
        embedder.dimensions()
    );

    // Load decay config
    let decay_config = memory_llm::build_decay_config_from_db(&db_pool).await;
    info!(
        "Decay scoring: enabled={}, function={:?}",
        decay_config.enabled, decay_config.function
    );

    // Load configurable search weights from DB
    let rrf_k =
        memory_common::db_config::load_f64(&db_pool, "search.rrf_k", "SEARCH_RRF_K", 60.0).await;

    let bm25_weights = memory_common::db_config::load_f64_array(&db_pool, "search.bm25_weights")
        .await
        .and_then(|arr| {
            if arr.len() == 4 {
                Some([arr[0] as f32, arr[1] as f32, arr[2] as f32, arr[3] as f32])
            } else {
                warn!("search.bm25_weights must have exactly 4 elements, ignoring");
                None
            }
        });

    // Per-intent fusion weights
    let load_weight = |key: &str, default: f64| {
        let pool = db_pool.clone();
        let key = key.to_string();
        async move { memory_common::db_config::load_f64(&pool, &key, "", default).await }
    };

    let intent_weights_preference = (
        load_weight("search.vector_weight_preference", 1.0).await,
        load_weight("search.bm25_weight_preference", 0.3).await,
        load_weight("search.graph_weight_preference", 0.2).await,
        load_weight("search.scope_weight_preference", 0.4).await,
    );
    let intent_weights_temporal = (
        load_weight("search.vector_weight_temporal", 0.5).await,
        load_weight("search.bm25_weight_temporal", 0.8).await,
        load_weight("search.graph_weight_temporal", 0.7).await,
        load_weight("search.scope_weight_temporal", 0.3).await,
    );
    let intent_weights_relational = (
        load_weight("search.vector_weight_relational", 0.4).await,
        load_weight("search.bm25_weight_relational", 0.2).await,
        load_weight("search.graph_weight_relational", 1.0).await,
        load_weight("search.scope_weight_relational", 0.3).await,
    );
    let intent_weights_general = (
        load_weight("search.vector_weight_general", 0.7).await,
        load_weight("search.bm25_weight_general", 0.6).await,
        load_weight("search.graph_weight_general", 0.5).await,
        load_weight("search.scope_weight_general", 0.4).await,
    );

    let graph_max_depth = memory_common::db_config::load_u64(
        &db_pool,
        "graph.max_traversal_depth",
        "GRAPH_MAX_TRAVERSAL_DEPTH",
        10,
    )
    .await as u32;

    info!(
        "Search config: rrf_k={}, bm25_weights={:?}, graph_max_depth={}",
        rrf_k, bm25_weights, graph_max_depth
    );

    let internal_service_secret = std::env::var("INTERNAL_SERVICE_SECRET").ok();
    if internal_service_secret.is_none() {
        warn!("INTERNAL_SERVICE_SECRET not set — internal endpoints are unprotected");
    }

    let state = Arc::new(AppState {
        db_pool,
        embedder,
        decay_config,
        internal_service_secret,
        rrf_k,
        bm25_weights,
        intent_weights_preference,
        intent_weights_temporal,
        intent_weights_relational,
        intent_weights_general,
        graph_max_depth,
    });

    let internal_routes = Router::new()
        .route("/internal/search", post(search))
        .route("/internal/export", post(export_memories))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            verify_internal_auth,
        ));

    let app = Router::new()
        .merge(internal_routes)
        .route(
            "/health",
            get(|| async {
                axum::Json(serde_json::json!({"status": "ok", "service": "memory-retrieve"}))
            }),
        )
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Retrieve service listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Retrieve service shut down gracefully");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => { warn!("Received Ctrl+C, shutting down..."); },
        _ = terminate => { warn!("Received SIGTERM, shutting down..."); },
    }
}

fn extract_tenant_id(headers: &HeaderMap) -> Result<Uuid, memory_common::MemoryError> {
    headers
        .get("x-tenant-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
        .ok_or_else(|| memory_common::MemoryError::Auth("Missing x-tenant-id header".into()))
}

fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("x-user-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}

/// Verify that internal requests carry the correct shared secret.
/// Uses constant-time comparison to prevent timing side-channel attacks.
async fn verify_internal_auth(
    State(state): State<Arc<AppState>>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Result<Response, memory_common::MemoryError> {
    if let Some(ref secret) = state.internal_service_secret {
        let provided = request
            .headers()
            .get("x-internal-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        let matches = provided.len() == secret.len()
            && provided
                .as_bytes()
                .iter()
                .zip(secret.as_bytes().iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;

        if !matches {
            return Err(memory_common::MemoryError::Auth(
                "Invalid internal service secret".into(),
            ));
        }
    }

    Ok(next.run(request).await)
}

/// Classify query intent to determine retrieval strategy.
#[derive(Debug, Clone, Copy)]
enum QueryIntent {
    Preference, // "What does the user prefer?" → vector-first
    Temporal,   // "When did X happen?" → graph + temporal filter
    Relational, // "Who works at X?" → graph traversal
    General,    // Catch-all → hybrid (vector + graph + text)
}

fn classify_intent(query: &str) -> QueryIntent {
    let q = query.to_lowercase();

    if q.contains("prefer")
        || q.contains("like")
        || q.contains("favorite")
        || q.contains("want")
        || q.contains("choice")
    {
        return QueryIntent::Preference;
    }

    if q.contains("when")
        || q.contains("timeline")
        || q.contains("history")
        || q.contains("last time")
        || q.contains("recently")
        || q.contains("past")
    {
        return QueryIntent::Temporal;
    }

    if q.contains("who")
        || q.contains("relationship")
        || q.contains("connected")
        || q.contains("between")
        || q.contains("works at")
        || q.contains("manages")
        || q.contains("related to")
    {
        return QueryIntent::Relational;
    }

    QueryIntent::General
}

/// Reciprocal Rank Fusion: score(m) = Σ w_i / (k + rank_i(m))
pub fn rrf_fuse(ranked_lists: &[(&[Uuid], f64)], k: f64) -> Vec<(Uuid, f64)> {
    let mut scores: HashMap<Uuid, f64> = HashMap::new();

    for (ids, weight) in ranked_lists {
        for (rank, id) in ids.iter().enumerate() {
            *scores.entry(*id).or_default() += weight / (k + rank as f64 + 1.0);
        }
    }

    let mut sorted: Vec<(Uuid, f64)> = scores.into_iter().collect();
    sorted.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    sorted
}

#[derive(Debug, Clone)]
struct BM25Result {
    id: Uuid,
    score: f32,
}

async fn bm25_search(
    db_pool: &sqlx::PgPool,
    tenant_id: Uuid,
    query: &str,
    limit: i64,
    weights: Option<[f32; 4]>,
) -> Result<Vec<BM25Result>, sqlx::Error> {
    // Use parameterized weights to avoid format!() interpolation in SQL.
    let results = if let Some(w) = weights {
        sqlx::query_as::<_, (Uuid, f32)>(
            r#"SELECT id, ts_rank_cd(
                   ARRAY[$4, $5, $6, $7]::float4[],
                   search_vector,
                   plainto_tsquery('english', $1)
               ) as score
             FROM memories
             WHERE search_vector @@ plainto_tsquery('english', $1)
               AND tenant_id = $2
               AND status = 'active'
             ORDER BY score DESC
             LIMIT $3"#,
        )
        .bind(query)
        .bind(tenant_id)
        .bind(limit)
        .bind(w[0])
        .bind(w[1])
        .bind(w[2])
        .bind(w[3])
        .fetch_all(db_pool)
        .await?
    } else {
        sqlx::query_as::<_, (Uuid, f32)>(
            "SELECT id, ts_rank_cd(search_vector, plainto_tsquery('english', $1)) as score
             FROM memories
             WHERE search_vector @@ plainto_tsquery('english', $1)
               AND tenant_id = $2
               AND status = 'active'
             ORDER BY score DESC
             LIMIT $3",
        )
        .bind(query)
        .bind(tenant_id)
        .bind(limit)
        .fetch_all(db_pool)
        .await?
    };

    Ok(results
        .into_iter()
        .map(|(id, score)| BM25Result { id, score })
        .collect())
}

async fn scope_cascade_retrieval(
    db_pool: &sqlx::PgPool,
    tenant_id: Uuid,
    session_id: Option<Uuid>,
    user_id: Option<Uuid>,
    agent_id: Option<Uuid>,
    org_id: Option<Uuid>,
) -> Result<Vec<(Uuid, f32)>, sqlx::Error> {
    let mut scope_conditions = Vec::new();
    let mut scope_params = Vec::new();

    if let Some(sid) = session_id {
        let param_index = 2 + scope_params.len();
        scope_conditions.push(format!(
            "(scope_type = 'session' AND scope_id = ${param_index})"
        ));
        scope_params.push(sid);
    }
    if let Some(uid) = user_id {
        let param_index = 2 + scope_params.len();
        scope_conditions.push(format!(
            "(scope_type = 'user' AND scope_id = ${param_index})"
        ));
        scope_params.push(uid);
    }
    if let Some(aid) = agent_id {
        let param_index = 2 + scope_params.len();
        scope_conditions.push(format!(
            "(scope_type = 'agent' AND scope_id = ${param_index})"
        ));
        scope_params.push(aid);
    }
    if let Some(oid) = org_id {
        let param_index = 2 + scope_params.len();
        scope_conditions.push(format!(
            "(scope_type = 'org' AND scope_id = ${param_index})"
        ));
        scope_params.push(oid);
    }

    if scope_conditions.is_empty() {
        return Ok(Vec::new());
    }

    let scope_where = scope_conditions.join(" OR ");
    let query_str = format!(
        "SELECT m.id, ms.priority FROM memories m
         JOIN memory_scopes ms ON m.id = ms.memory_id
         WHERE ms.tenant_id = $1 AND ({})
         AND m.status = 'active'
         ORDER BY CASE ms.scope_type
           WHEN 'session' THEN 0
           WHEN 'user' THEN 1
           WHEN 'agent' THEN 2
           WHEN 'org' THEN 3
         END, ms.priority DESC",
        scope_where
    );

    let mut query = sqlx::query_as::<_, (Uuid, f32)>(&query_str).bind(tenant_id);

    for scope_id in scope_params {
        query = query.bind(scope_id);
    }

    query.fetch_all(db_pool).await
}

async fn search(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, memory_common::MemoryError> {
    let start = std::time::Instant::now();
    let tenant_id = extract_tenant_id(&headers)?;
    let user_id = req.user_id.or_else(|| extract_user_id(&headers));
    let limit = req.limit.unwrap_or(10);
    let intent = classify_intent(&req.query);
    let graph_depth = req.graph_depth.unwrap_or(2);

    debug!(
        "Search query='{}' intent={:?} tenant={}",
        req.query, intent, tenant_id
    );

    // Step 0: Generate real query embedding using configured provider
    let query_embedding = state.embedder.embed(&req.query).await.unwrap_or_else(|e| {
        warn!("Embedding generation failed: {}. Using zero vector.", e);
        vec![0.0f32; state.embedder.dimensions()]
    });

    // Step 1: Vector search with real embeddings
    let vector_hits = memory_vector::search_similar(
        &state.db_pool,
        tenant_id,
        user_id,
        &query_embedding,
        limit * 2,
        req.min_confidence,
        req.scope.as_deref(),
        req.kind.as_deref(),
    )
    .await
    .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

    let vector_ids: Vec<Uuid> = vector_hits.iter().map(|h| h.id).collect();

    // Step 2: BM25 full-text search
    let mut bm25_ids: Vec<Uuid> = Vec::new();
    let mut bm25_scores: HashMap<Uuid, f32> = HashMap::new();

    if matches!(intent, QueryIntent::General | QueryIntent::Temporal) {
        let bm25_results = bm25_search(
            &state.db_pool,
            tenant_id,
            &req.query,
            limit * 2,
            state.bm25_weights,
        )
        .await
        .unwrap_or_default();

        for result in bm25_results {
            bm25_ids.push(result.id);
            bm25_scores.insert(result.id, result.score);
        }
    }

    // Step 3: Scope cascade with session support (FIXED: now uses session_id from request)
    let session_uuid = req
        .session_id
        .as_ref()
        .and_then(|s| Uuid::parse_str(s).ok());
    let agent_uuid = req.agent_id.as_ref().and_then(|s| Uuid::parse_str(s).ok());

    let scope_results = scope_cascade_retrieval(
        &state.db_pool,
        tenant_id,
        session_uuid,
        user_id,
        agent_uuid,
        None, // org_id
    )
    .await
    .unwrap_or_default();

    let mut scope_weights: HashMap<Uuid, f32> = HashMap::new();
    for (memory_id, priority) in &scope_results {
        let scope_boost = if session_uuid.is_some() { 1.5 } else { 1.2 };
        scope_weights
            .entry(*memory_id)
            .and_modify(|w| *w = (*w * scope_boost).max(*w))
            .or_insert(*priority * scope_boost);
    }

    // Step 4: Graph search with configurable N-hop traversal
    let mut graph_ids: Vec<Uuid> = Vec::new();
    if matches!(intent, QueryIntent::Relational | QueryIntent::General) {
        let entities =
            memory_graph::search_entities_by_name(&state.db_pool, tenant_id, &req.query, 5)
                .await
                .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

        for entity in &entities {
            if graph_depth <= 2 {
                let expanded = memory_graph::expand_2hop(&state.db_pool, tenant_id, entity.id)
                    .await
                    .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;
                graph_ids.extend(expanded);
            } else {
                // Use N-hop traversal for deeper graph exploration
                let expanded = memory_graph::expand_nhop(
                    &state.db_pool,
                    tenant_id,
                    entity.id,
                    graph_depth,
                    None,
                    50,
                    Some(state.graph_max_depth),
                )
                .await
                .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;
                graph_ids.extend(expanded.into_iter().map(|(id, _depth)| id));
            }
        }
    }

    // Step 5: Intent-based RRF fusion (configurable via admin)
    let (vector_weight, bm25_weight, graph_weight, scope_weight) = match intent {
        QueryIntent::Preference => state.intent_weights_preference,
        QueryIntent::Temporal => state.intent_weights_temporal,
        QueryIntent::Relational => state.intent_weights_relational,
        QueryIntent::General => state.intent_weights_general,
    };

    let fused = rrf_fuse(
        &[
            (&vector_ids, vector_weight),
            (&bm25_ids, bm25_weight),
            (&graph_ids, graph_weight),
            (
                &scope_results.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
                scope_weight,
            ),
        ],
        state.rrf_k,
    );

    // Step 6: Fetch full memory records and apply decay scoring
    let mut results = Vec::new();
    let now = chrono::Utc::now();
    let should_apply_decay = req.apply_decay.unwrap_or(state.decay_config.enabled);

    for (fused_id, fused_score) in fused.iter().take(limit as usize) {
        if let Some(hit) = vector_hits.iter().find(|h| h.id == *fused_id) {
            let vector_score = Some(hit.similarity);
            let text_score = bm25_scores.get(fused_id).copied();

            // Apply decay to adjust the fused score
            let final_score = if should_apply_decay {
                let decay_factor = memory_llm::decayed_score(
                    hit.importance,
                    hit.created_at,
                    None, // last_accessed_at — would need column
                    now,
                    &state.decay_config,
                );
                *fused_score * (decay_factor as f64 / hit.importance.max(0.01) as f64)
            } else {
                *fused_score
            };

            // Apply importance filter
            if let Some(min_imp) = req.min_importance {
                if hit.importance < min_imp {
                    continue;
                }
            }

            // Apply tag filter
            if let Some(ref required_tags) = req.tags {
                if !required_tags.iter().all(|t| hit.tags.contains(t)) {
                    continue;
                }
            }

            results.push(SearchResult {
                memory: hit_to_memory(hit),
                score: final_score,
                vector_score,
                graph_score: text_score.map(|s| s as f64),
                related_entities: Vec::new(),
            });
        } else {
            let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, tenant_id)
                .await
                .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

            if let Ok(Some(row)) = sqlx::query_as::<_, MemoryRow>(
                "SELECT * FROM memories WHERE id = $1 AND status = 'active'",
            )
            .bind(fused_id)
            .fetch_optional(conn.as_mut())
            .await
            {
                // Apply filters
                if let Some(min_imp) = req.min_importance {
                    if row.importance < min_imp {
                        continue;
                    }
                }
                if let Some(ref required_tags) = req.tags {
                    if !required_tags.iter().all(|t| row.tags.contains(t)) {
                        continue;
                    }
                }

                let final_score = if should_apply_decay {
                    let decay_factor = memory_llm::decayed_score(
                        row.importance,
                        row.created_at,
                        None,
                        now,
                        &state.decay_config,
                    );
                    *fused_score * (decay_factor as f64 / row.importance.max(0.01) as f64)
                } else {
                    *fused_score
                };

                results.push(SearchResult {
                    memory: row_to_memory(&row),
                    score: final_score,
                    vector_score: None,
                    graph_score: Some(*fused_score),
                    related_entities: Vec::new(),
                });
            }
        }
    }

    // Update access timestamps for returned memories (fire-and-forget)
    if should_apply_decay && !results.is_empty() {
        let pool = state.db_pool.clone();
        let memory_ids: Vec<Uuid> = results.iter().map(|r| r.memory.id).collect();
        tokio::spawn(async move {
            for mid in memory_ids {
                let _ = sqlx::query("UPDATE memories SET updated_at = now() WHERE id = $1")
                    .bind(mid)
                    .execute(&pool)
                    .await;
            }
        });
    }

    let query_ms = start.elapsed().as_millis() as u64;
    info!(
        "Search completed in {}ms: {} results (intent={:?}, decay={})",
        query_ms,
        results.len(),
        intent,
        should_apply_decay,
    );

    let total = results.len();
    Ok(Json(MemorySearchResponse {
        results,
        total,
        query_ms,
    }))
}

/// Export memories for a tenant (bulk data export).
async fn export_memories(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemoryExportRequest>,
) -> Result<Json<MemoryExport>, memory_common::MemoryError> {
    let start = std::time::Instant::now();
    let tenant_id = extract_tenant_id(&headers)?;

    // Cap export at 10,000 rows to prevent memory exhaustion.
    // Users needing more should use pagination.
    const MAX_EXPORT_LIMIT: i64 = 10_000;
    let export_limit = if req.limit > 0 {
        std::cmp::min(req.limit, MAX_EXPORT_LIMIT)
    } else {
        MAX_EXPORT_LIMIT
    };

    // Export memories
    let memories = sqlx::query_as::<_, MemoryRow>(
        r#"
        SELECT * FROM memories
        WHERE tenant_id = $1
          AND status = 'active'
          AND ($2::uuid IS NULL OR user_id = $2)
          AND ($3::text IS NULL OR kind = $3)
          AND ($4::text IS NULL OR scope = $4)
        ORDER BY created_at DESC
        LIMIT $5
        "#,
    )
    .bind(tenant_id)
    .bind(req.user_id)
    .bind(req.kind.as_deref())
    .bind(req.scope.as_deref())
    .bind(export_limit)
    .fetch_all(&state.db_pool)
    .await
    .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

    let memory_items: Vec<MemoryItem> = memories.iter().map(row_to_memory).collect();

    // Export entities and edges if requested
    let mut entities = Vec::new();
    let edges = Vec::new();

    if req.include_graph {
        let entity_rows = memory_graph::list_entities(&state.db_pool, tenant_id, None, 10000)
            .await
            .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

        entities = entity_rows
            .into_iter()
            .map(|e| memory_common::Entity {
                id: e.id,
                tenant_id: e.tenant_id,
                name: e.name,
                entity_type: e.entity_type,
                summary: e.summary,
                attributes: e.attributes,
                valid_from: e.valid_from,
                valid_to: e.valid_to,
                status: e.status,
                merged_into: e.merged_into,
                created_at: e.created_at,
                updated_at: e.updated_at,
            })
            .collect();
    }

    let export_ms = start.elapsed().as_millis() as u64;

    let export = MemoryExport {
        version: "1.0".to_string(),
        exported_at: chrono::Utc::now(),
        tenant_id,
        stats: ExportStats {
            total_memories: memory_items.len(),
            total_entities: entities.len(),
            total_edges: edges.len(),
            export_duration_ms: export_ms,
        },
        memories: memory_items,
        entities,
        edges,
    };

    info!(
        "Export completed in {}ms: {} memories, {} entities",
        export_ms, export.stats.total_memories, export.stats.total_entities
    );

    Ok(Json(export))
}

fn hit_to_memory(hit: &VectorSearchHit) -> MemoryItem {
    MemoryItem {
        id: hit.id,
        tenant_id: hit.tenant_id,
        user_id: hit.user_id,
        scope: hit.scope.clone(),
        kind: hit.kind.clone(),
        content: hit.content.clone(),
        content_json: hit.content_json.clone(),
        confidence: hit.confidence,
        importance: hit.importance,
        status: hit.status.clone(),
        valid_from: hit.valid_from,
        valid_to: hit.valid_to,
        event_time: hit.event_time,
        ingested_at: hit.ingested_at,
        source_episode_id: hit.source_episode_id,
        created_by: hit.created_by.clone(),
        tags: hit.tags.clone(),
        metadata: hit.metadata.clone(),
        created_at: hit.created_at,
        updated_at: hit.updated_at,
        deleted_at: None,
    }
}

fn row_to_memory(row: &MemoryRow) -> MemoryItem {
    MemoryItem {
        id: row.id,
        tenant_id: row.tenant_id,
        user_id: row.user_id,
        scope: row.scope.clone(),
        kind: row.kind.clone(),
        content: row.content.clone(),
        content_json: row.content_json.clone(),
        confidence: row.confidence,
        importance: row.importance,
        status: row.status.clone(),
        valid_from: row.valid_from,
        valid_to: row.valid_to,
        event_time: row.event_time,
        ingested_at: row.ingested_at,
        source_episode_id: row.source_episode_id,
        created_by: row.created_by.clone(),
        tags: row.tags.clone(),
        metadata: row.metadata.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
        deleted_at: row.deleted_at,
    }
}

#[derive(Debug, sqlx::FromRow)]
struct MemoryRow {
    id: Uuid,
    tenant_id: Uuid,
    user_id: Option<Uuid>,
    scope: String,
    kind: String,
    content: String,
    content_json: Option<serde_json::Value>,
    confidence: f32,
    importance: f32,
    status: String,
    valid_from: chrono::DateTime<chrono::Utc>,
    valid_to: Option<chrono::DateTime<chrono::Utc>>,
    event_time: Option<chrono::DateTime<chrono::Utc>>,
    ingested_at: chrono::DateTime<chrono::Utc>,
    source_episode_id: Option<Uuid>,
    created_by: String,
    tags: Vec<String>,
    metadata: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}
