//! Memory Graph Service
//!
//! NATS consumer that processes write events: extracts entities and relationships
//! using the LLM, then upserts them into the knowledge graph.
//!
//! ## Full write-path pipeline
//!
//! 1. **Content triage** — skip trivial messages (greetings, acks) without any LLM call
//! 2. **Redis cache** — if the same content+role+entities was recently extracted, reuse result
//! 3. **Entity pruning** — only include entities mentioned in content (capped at N)
//! 4. **Dynamic output tokens** — scale max_output_tokens by content length
//! 5. **Inline verification** — embed grounding fields in extraction prompt (no 2nd call)
//! 6. **Token usage logging** — persist per-call token stats for cost monitoring
//! 7. **Conflict detection** — detect contradictions/duplicates against existing memories
//! 8. **Embedding generation** — generate and store vector embeddings at write time
//! 9. **Webhook dispatch** — fire events to registered webhook subscribers

use async_nats::jetstream::consumer::PullConsumer;
use axum::Router;
use memory_common::{
    webhook::{
        deliver_webhook, WebhookConfig, WebhookEvent, WebhookEventType, WebhookRegistration,
    },
    GroundingConfig, MemoryWriteEvent, VerificationStatus,
};
use memory_llm::{
    cache_key, detect_conflicts, dynamic_output_tokens, get_cached, log_token_usage,
    prune_entity_context, set_cached, triage_content, ConflictAction, ConflictConfig,
    EmbeddingProvider, ExistingMemory, ExtractionOptions, LlmCacheConfig, TriageConfig,
    TriageDecision,
};
use sha2::Digest;
use std::{net::SocketAddr, sync::Arc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

struct AppState {
    db_pool: sqlx::PgPool,
    llm: Arc<dyn memory_llm::LlmProvider>,
    grounding: GroundingConfig,
    triage: TriageConfig,
    max_entity_context: usize,
    llm_cache: LlmCacheConfig,
    redis: Option<fred::prelude::RedisClient>,
    dynamic_output_tokens_enabled: bool,
    inline_verification: bool,
    // ── NEW: Write-path intelligence ──
    embedder: EmbeddingProvider,
    conflict_config: ConflictConfig,
    webhook_config: WebhookConfig,
    webhook_client: reqwest::Client,
    webhook_encryption_key: Option<[u8; 32]>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    info!("Starting Memory Graph Service...");

    memory_common::startup::validate_env("service-graph")?;

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://memory:memory_dev@localhost:5432/memory".into());
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());

    let db_pool = memory_db::create_pool(&database_url, 4).await?;

    let port: u16 = memory_common::db_config::load_u64(
        &db_pool,
        "services.graph_port",
        "GRAPH_SERVICE_PORT",
        8083,
    )
    .await as u16;
    let (_nats_client, nats_js) = memory_queue::connect(&nats_url).await?;
    memory_queue::ensure_stream(&nats_js).await?;

    // Build dynamic LLM provider — reloads config/keys from DB every 60s
    let llm: Arc<dyn memory_llm::LlmProvider> =
        memory_llm::DynamicLlmProvider::new(db_pool.clone())
            .await
            .expect("Failed to initialize LLM provider");
    info!("LLM provider: dynamic (auto-refreshes from admin DB)");

    // Load grounding config from admin DB
    let grounding = memory_llm::build_grounding_config_from_db(&db_pool).await;
    info!(
        "Grounding config: citations={}, verification={}",
        grounding.enable_citations, grounding.enable_verification
    );

    // Load triage config from admin DB
    let triage = memory_llm::build_triage_config_from_db(&db_pool).await;
    let max_entity_context = memory_common::db_config::load_u64(
        &db_pool,
        "llm.max_entity_context",
        "LLM_MAX_ENTITY_CONTEXT",
        20,
    )
    .await as usize;
    info!(
        "Triage config: enabled={}, min_words={}, light_threshold={}, max_entities={}",
        triage.enabled, triage.min_words, triage.light_threshold_words, max_entity_context
    );

    // ── Optimization configs ──
    let dynamic_output_tokens_enabled = memory_common::db_config::load_bool(
        &db_pool,
        "llm.dynamic_output_tokens",
        "LLM_DYNAMIC_OUTPUT_TOKENS",
        true,
    )
    .await;

    let inline_verification = memory_common::db_config::load_bool(
        &db_pool,
        "grounding.inline_verification",
        "GROUNDING_INLINE_VERIFICATION",
        true,
    )
    .await;

    let cache_enabled = memory_common::db_config::load_bool(
        &db_pool,
        "llm.cache_enabled",
        "LLM_CACHE_ENABLED",
        true,
    )
    .await;
    let cache_ttl = memory_common::db_config::load_u64(
        &db_pool,
        "llm.cache_ttl_secs",
        "LLM_CACHE_TTL_SECS",
        3600,
    )
    .await;

    let llm_cache = LlmCacheConfig {
        enabled: cache_enabled,
        ttl_secs: cache_ttl,
    };

    // Try to connect to Redis (optional — degrades gracefully if unavailable)
    let redis_url = memory_common::db_config::load_str(
        &db_pool,
        "services.redis_url",
        "REDIS_URL",
        "redis://localhost:6379",
    )
    .await;
    let redis = if cache_enabled {
        match memory_cache::create_client(&redis_url).await {
            Ok(client) => {
                info!("Redis connected for LLM cache (ttl={}s)", cache_ttl);
                Some(client)
            }
            Err(e) => {
                warn!("Redis connection failed (cache disabled): {}", e);
                None
            }
        }
    } else {
        info!("LLM cache disabled");
        None
    };

    // ── Embedding provider (write-time embedding generation) ──
    let embedder = EmbeddingProvider::from_db(&db_pool)
        .await
        .unwrap_or_else(|e| {
            warn!(
                "Failed to initialize embedding provider from DB: {}. Using local fallback.",
                e
            );
            EmbeddingProvider::new(memory_llm::EmbeddingConfig {
                provider: "local".into(),
                api_key: String::new(),
                model: "local-hash".into(),
                dimensions: 1024,
                api_url: None,
                cache_enabled: true,
                cache_max_entries: 10000,
            })
        });
    info!(
        "Embedding provider: {} ({}D) — write-time generation enabled",
        embedder.provider_name(),
        embedder.dimensions()
    );

    // ── Conflict detection config ──
    let conflict_config = memory_llm::build_conflict_config_from_db(&db_pool).await;
    info!(
        "Conflict detection: enabled={}, threshold={:.2}, resolution={:?}",
        conflict_config.enabled, conflict_config.similarity_threshold, conflict_config.resolution
    );

    // ── Webhook config ──
    let webhook_enabled =
        memory_common::db_config::load_bool(&db_pool, "webhooks.enabled", "WEBHOOKS_ENABLED", true)
            .await;
    let webhook_max_retries = memory_common::db_config::load_u64(
        &db_pool,
        "webhooks.max_retries",
        "WEBHOOKS_MAX_RETRIES",
        3,
    )
    .await as u32;
    let webhook_timeout = memory_common::db_config::load_u64(
        &db_pool,
        "webhooks.timeout_secs",
        "WEBHOOKS_TIMEOUT_SECS",
        10,
    )
    .await;
    let webhook_config = WebhookConfig {
        enabled: webhook_enabled,
        max_retries: webhook_max_retries,
        timeout_secs: webhook_timeout,
        ..Default::default()
    };
    let webhook_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(webhook_timeout))
        .build()
        .unwrap_or_default();
    let webhook_encryption_key = memory_common::webhook_crypto::load_encryption_key_from_env();
    if webhook_encryption_key.is_some() {
        info!("Webhook secret decryption: enabled");
    } else {
        info!("Webhook secret decryption: disabled (no encryption key set)");
    }
    info!(
        "Webhooks: enabled={}, max_retries={}",
        webhook_enabled, webhook_max_retries
    );

    info!(
        "Optimizations: dynamic_tokens={}, inline_verification={}, cache={}, embeddings=write-time, conflicts={}, webhooks={}",
        dynamic_output_tokens_enabled, inline_verification, cache_enabled,
        conflict_config.enabled, webhook_enabled
    );

    let worker_state = Arc::new(AppState {
        db_pool,
        llm,
        grounding,
        triage,
        max_entity_context,
        llm_cache,
        redis,
        dynamic_output_tokens_enabled,
        inline_verification,
        embedder,
        conflict_config,
        webhook_config,
        webhook_client,
        webhook_encryption_key,
    });

    // Create consumer for write events
    let consumer = memory_queue::create_consumer(
        &nats_js,
        memory_queue::STREAM_NAME,
        "graph-builder",
        memory_queue::SUBJECT_WRITE,
    )
    .await?;

    info!("Graph service consumer ready, processing events...");
    let worker_state_for_task = worker_state.clone();
    tokio::spawn(async move {
        if let Err(e) = process_messages(worker_state_for_task, consumer).await {
            error!("Graph consumer loop exited: {}", e);
        }
    });

    let app = Router::new().route(
        "/health",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({"status": "ok", "service": "memory-graph"}))
        }),
    );

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Graph service listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Graph service shut down gracefully");
    Ok(())
}

async fn process_messages(state: Arc<AppState>, consumer: PullConsumer) -> anyhow::Result<()> {
    use futures::StreamExt;

    loop {
        let mut messages = consumer.fetch().max_messages(10).messages().await?;

        while let Some(Ok(message)) = messages.next().await {
            let state = state.clone();
            let payload = message.payload.to_vec();

            // Process in a spawned task for concurrency
            tokio::spawn(async move {
                match process_single_event(&state, &payload).await {
                    Ok(()) => {
                        if let Err(e) = message.ack().await {
                            error!("Failed to ACK message: {}", e);
                        }
                    }
                    Err(e) => {
                        error!("Failed to process event: {}", e);
                        // NACK for retry (message will be redelivered)
                    }
                }
            });
        }

        // Small delay to avoid busy-loop when no messages
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn process_single_event(state: &AppState, payload: &[u8]) -> anyhow::Result<()> {
    let event: MemoryWriteEvent = memory_queue::deserialize_message(payload)?;
    info!(
        "Processing write event: episode={} tenant={}",
        event.episode_id, event.tenant_id
    );

    // ── 1. Content Triage: skip trivial content before LLM call ──
    let triage_decision = triage_content(&event.content, &state.triage);
    match &triage_decision {
        TriageDecision::Skip { reason } => {
            debug!("Triage: skipping episode {} ({})", event.episode_id, reason);
            return Ok(());
        }
        TriageDecision::Light => {
            debug!("Triage: light extraction for episode {}", event.episode_id);
        }
        TriageDecision::Full => {
            debug!("Triage: full extraction for episode {}", event.episode_id);
        }
    }

    // ── 2. Entity context pruning ──
    let existing = memory_graph::list_entities(&state.db_pool, event.tenant_id, None, 100)
        .await
        .unwrap_or_default();
    let all_entity_names: Vec<String> = existing.iter().map(|e| e.name.clone()).collect();
    let entity_names =
        prune_entity_context(&event.content, &all_entity_names, state.max_entity_context);
    debug!(
        "Entity context: {} relevant of {} total",
        entity_names.len(),
        all_entity_names.len()
    );

    // ── 3. Redis cache check ──
    let ck = cache_key(&event.content, &event.role, &entity_names);
    if let Some(ref redis) = state.redis {
        if state.llm_cache.enabled {
            if let Some(cached) = get_cached(redis, &ck).await {
                info!(
                    "LLM cache hit for episode {} ({} memories, {} entities)",
                    event.episode_id,
                    cached.memories.len(),
                    cached.entities.len()
                );
                // Log as cache hit
                log_token_usage(
                    &state.db_pool,
                    event.tenant_id,
                    state.llm.provider_name(),
                    state.llm.model_name(),
                    "extraction",
                    0,
                    0,
                    true,
                )
                .await;
                return store_extraction(state, &event, &cached).await;
            }
        }
    }

    // ── 4. Build extraction options (dynamic tokens + inline verification) ──
    let max_tokens = if state.dynamic_output_tokens_enabled {
        Some(dynamic_output_tokens(&event.content, true))
    } else {
        None
    };

    // Use inline verification if enabled AND grounding verification is on
    let use_inline = state.inline_verification && state.grounding.enable_verification;

    let options = ExtractionOptions {
        max_output_tokens: max_tokens,
        inline_verification: use_inline,
    };

    debug!(
        "Extraction options: max_tokens={:?}, inline_verification={}",
        max_tokens, use_inline
    );

    // ── 5. LLM extraction call ──
    let extraction = state
        .llm
        .extract_memories_with_options(&event.content, &event.role, &entity_names, &options)
        .await
        .map_err(|e| anyhow::anyhow!("LLM extraction failed: {}", e))?;

    info!(
        "Extracted {} memories, {} entities, {} relationships",
        extraction.memories.len(),
        extraction.entities.len(),
        extraction.relationships.len()
    );

    // ── 6. Log token usage ──
    {
        let usage = state.llm.get_token_usage().await;
        log_token_usage(
            &state.db_pool,
            event.tenant_id,
            state.llm.provider_name(),
            state.llm.model_name(),
            "extraction",
            usage.input_tokens,
            usage.output_tokens,
            false,
        )
        .await;
    }

    // ── 7. Cache the result for future identical requests ──
    if let Some(ref redis) = state.redis {
        if state.llm_cache.enabled {
            set_cached(redis, &ck, &extraction, state.llm_cache.ttl_secs).await;
        }
    }

    // ── 8. Separate verification pass (only if NOT using inline verification) ──
    let verifications =
        if state.grounding.enable_verification && !use_inline && !extraction.memories.is_empty() {
            match state
                .llm
                .verify_memories(&extraction.memories, &event.content)
                .await
            {
                Ok(v) => {
                    info!("Verification complete: {} results", v.len());
                    // Log verification token usage
                    let usage = state.llm.get_token_usage().await;
                    log_token_usage(
                        &state.db_pool,
                        event.tenant_id,
                        state.llm.provider_name(),
                        state.llm.model_name(),
                        "verification",
                        usage.input_tokens,
                        usage.output_tokens,
                        false,
                    )
                    .await;
                    v
                }
                Err(e) => {
                    warn!("Verification failed (proceeding without): {}", e);
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

    // ── 9. Conflict detection against existing memories ──
    let conflicts = if state.conflict_config.enabled && !extraction.memories.is_empty() {
        let existing_memories =
            load_existing_memories(&state.db_pool, event.tenant_id, event.user_id).await;
        if existing_memories.is_empty() {
            Vec::new()
        } else {
            let c = detect_conflicts(
                &extraction.memories,
                &existing_memories,
                &entity_names,
                &state.conflict_config,
            );
            if !c.is_empty() {
                info!(
                    "Detected {} conflicts for episode {}",
                    c.len(),
                    event.episode_id
                );
            }
            c
        }
    } else {
        Vec::new()
    };

    store_extraction_with_verifications(state, &event, &extraction, &verifications, &conflicts)
        .await
}

/// Load existing active memories for a tenant+user for conflict checking.
async fn load_existing_memories(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    user_id: Option<Uuid>,
) -> Vec<ExistingMemory> {
    // Load the most recent active memories for this user (capped to limit cost)
    let rows = if let Some(uid) = user_id {
        sqlx::query_as::<_, ExistingMemoryRow>(
            r#"
            SELECT m.id, m.content, m.kind, m.confidence, m.importance, m.tags, m.created_at,
                   COALESCE(array_agg(DISTINCT e.name) FILTER (WHERE e.name IS NOT NULL), '{}') as entity_names
            FROM memories m
            LEFT JOIN memory_citations mc ON mc.memory_id = m.id
            LEFT JOIN entities e ON e.tenant_id = m.tenant_id
                AND m.content ILIKE '%' || e.name || '%'
            WHERE m.tenant_id = $1 AND m.user_id = $2
                AND m.status = 'active'
            GROUP BY m.id
            ORDER BY m.created_at DESC
            LIMIT 200
            "#,
        )
        .bind(tenant_id)
        .bind(uid)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, ExistingMemoryRow>(
            r#"
            SELECT m.id, m.content, m.kind, m.confidence, m.importance, m.tags, m.created_at,
                   COALESCE(array_agg(DISTINCT e.name) FILTER (WHERE e.name IS NOT NULL), '{}') as entity_names
            FROM memories m
            LEFT JOIN entities e ON e.tenant_id = m.tenant_id
                AND m.content ILIKE '%' || e.name || '%'
            WHERE m.tenant_id = $1 AND m.user_id IS NULL
                AND m.status = 'active'
            GROUP BY m.id
            ORDER BY m.created_at DESC
            LIMIT 200
            "#,
        )
        .bind(tenant_id)
        .fetch_all(pool)
        .await
    };

    match rows {
        Ok(rows) => rows
            .into_iter()
            .map(|r| ExistingMemory {
                id: r.id,
                content: r.content,
                kind: r.kind,
                confidence: r.confidence,
                importance: r.importance,
                tags: r.tags,
                entity_names: r.entity_names,
                created_at: r.created_at,
            })
            .collect(),
        Err(e) => {
            warn!(
                "Failed to load existing memories for conflict detection: {}",
                e
            );
            Vec::new()
        }
    }
}

#[derive(sqlx::FromRow)]
struct ExistingMemoryRow {
    id: Uuid,
    content: String,
    kind: String,
    confidence: f32,
    importance: f32,
    tags: Vec<String>,
    entity_names: Vec<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Store extraction results (used for cache hits with no separate verification).
async fn store_extraction(
    state: &AppState,
    event: &MemoryWriteEvent,
    extraction: &memory_common::ExtractionResult,
) -> anyhow::Result<()> {
    store_extraction_with_verifications(state, event, extraction, &[], &[]).await
}

/// Store extraction results with optional verification data and conflict handling.
async fn store_extraction_with_verifications(
    state: &AppState,
    event: &MemoryWriteEvent,
    extraction: &memory_common::ExtractionResult,
    verifications: &[memory_common::MemoryVerification],
    conflicts: &[memory_llm::ConflictDetection],
) -> anyhow::Result<()> {
    // Build a lookup: memory_index → (status, score)
    let verification_map: std::collections::HashMap<usize, (&VerificationStatus, f32)> =
        verifications
            .iter()
            .map(|v| (v.memory_index, (&v.status, v.score)))
            .collect();

    // Build a lookup: new_content → conflict action (use first conflict per content)
    let conflict_map: std::collections::HashMap<&str, &memory_llm::ConflictDetection> = conflicts
        .iter()
        .map(|c| (c.new_content.as_str(), c))
        .collect();

    // Track created resources for webhook events
    let mut created_memory_ids: Vec<Uuid> = Vec::new();
    let mut created_entity_ids: Vec<(Uuid, String)> = Vec::new();
    let mut created_edge_count: usize = 0;
    let mut skipped_count: usize = 0;
    let mut superseded_ids: Vec<Uuid> = Vec::new();

    // Begin transaction
    let mut tx = memory_db::begin_tenant_tx(&state.db_pool, event.tenant_id).await?;

    // ── Handle conflicts: supersede/skip before inserting new ──
    for conflict in conflicts {
        match conflict.recommended_action {
            ConflictAction::Supersede => {
                // Mark the old memory as superseded
                let _ = sqlx::query(
                    "UPDATE memories SET status = 'superseded', updated_at = now() WHERE id = $1",
                )
                .bind(conflict.existing_memory_id)
                .execute(&mut *tx)
                .await;
                superseded_ids.push(conflict.existing_memory_id);
                debug!(
                    "Superseded memory {} ({})",
                    conflict.existing_memory_id,
                    conflict_type_str(&conflict.conflict_type)
                );
            }
            ConflictAction::SkipNew => {
                skipped_count += 1;
                debug!(
                    "Skipping duplicate new memory (existing={})",
                    conflict.existing_memory_id
                );
            }
            ConflictAction::Review => {
                debug!(
                    "Flagging conflict for review (existing={})",
                    conflict.existing_memory_id
                );
                // Still insert the new memory but mark for review via metadata
            }
            ConflictAction::Merge => {
                // For now, treat merge as supersede (insert new version)
                let _ = sqlx::query(
                    "UPDATE memories SET status = 'superseded', updated_at = now() WHERE id = $1",
                )
                .bind(conflict.existing_memory_id)
                .execute(&mut *tx)
                .await;
                superseded_ids.push(conflict.existing_memory_id);
            }
        }
    }

    // Store extracted memories (with grounding data)
    for (idx, mem) in extraction.memories.iter().enumerate() {
        // Check if this memory was flagged to be skipped
        if let Some(conflict) = conflict_map.get(mem.content.as_str()) {
            if matches!(conflict.recommended_action, ConflictAction::SkipNew) {
                continue;
            }
        }

        let (v_status, v_score) = verification_map
            .get(&idx)
            .map(|(s, sc)| (s.as_str(), Some(*sc)))
            .unwrap_or(("unverified", None));

        let memory_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO memories (tenant_id, user_id, scope, kind, content, confidence, importance,
                                  source_episode_id, created_by, tags,
                                  source_quote, source_offset_start, source_offset_end,
                                  verification_status, verification_score)
            VALUES ($1, $2, 'user', $3, $4, $5, $6, $7, 'system', $8,
                    $9, $10, $11, $12, $13)
            RETURNING id
            "#,
        )
        .bind(event.tenant_id)
        .bind(event.user_id)
        .bind(&mem.kind)
        .bind(&mem.content)
        .bind(mem.confidence)
        .bind(mem.importance)
        .bind(event.episode_id)
        .bind(&mem.tags)
        .bind(mem.source_quote.as_deref())
        .bind(mem.source_offset_start.map(|v| v as i32))
        .bind(mem.source_offset_end.map(|v| v as i32))
        .bind(v_status)
        .bind(v_score)
        .fetch_one(&mut *tx)
        .await;

        match memory_id {
            Ok(mid) => {
                created_memory_ids.push(mid);

                // Insert citation record if we have a source quote
                if let Some(ref quote) = mem.source_quote {
                    let _ = sqlx::query(
                        r#"
                        INSERT INTO memory_citations (memory_id, episode_id, source_quote, offset_start, offset_end)
                        VALUES ($1, $2, $3, $4, $5)
                        "#,
                    )
                    .bind(mid)
                    .bind(event.episode_id)
                    .bind(quote)
                    .bind(mem.source_offset_start.map(|v| v as i32))
                    .bind(mem.source_offset_end.map(|v| v as i32))
                    .execute(&mut *tx)
                    .await;
                }

                // ── Generate and store embedding at write time ──
                match state.embedder.embed(&mem.content).await {
                    Ok(embedding) => {
                        let mut hasher = sha2::Sha256::new();
                        hasher.update(mem.content.as_bytes());
                        let content_hash = format!("{:x}", hasher.finalize());
                        if let Err(e) = memory_vector::store_vector(
                            &state.db_pool,
                            mid,
                            event.tenant_id,
                            event.user_id,
                            "user",
                            &mem.kind,
                            chrono::Utc::now(),
                            None,
                            &embedding,
                            &content_hash,
                        )
                        .await
                        {
                            warn!("Failed to store embedding for memory {}: {}", mid, e);
                        } else {
                            debug!("Stored {}D embedding for memory {}", embedding.len(), mid);
                        }
                    }
                    Err(e) => {
                        warn!("Embedding generation failed for memory {}: {}", mid, e);
                    }
                }
            }
            Err(e) => warn!("Failed to insert memory: {}", e),
        }
    }

    // Upsert entities
    let mut entity_id_map: std::collections::HashMap<String, Uuid> =
        std::collections::HashMap::new();
    for ent in &extraction.entities {
        match sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO entities (tenant_id, name, entity_type, summary, attributes)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (tenant_id, name, entity_type) WHERE status = 'active'
            DO UPDATE SET
                summary = COALESCE(EXCLUDED.summary, entities.summary),
                attributes = entities.attributes || EXCLUDED.attributes,
                updated_at = now()
            RETURNING id
            "#,
        )
        .bind(event.tenant_id)
        .bind(&ent.name)
        .bind(&ent.entity_type)
        .bind(ent.summary.as_deref())
        .bind(ent.attributes.as_ref().unwrap_or(&serde_json::json!({})))
        .fetch_one(&mut *tx)
        .await
        {
            Ok(id) => {
                created_entity_ids.push((id, ent.name.clone()));
                entity_id_map.insert(ent.name.clone(), id);
            }
            Err(e) => warn!("Failed to upsert entity '{}': {}", ent.name, e),
        }
    }

    // Upsert edges
    for rel in &extraction.relationships {
        let source_id = entity_id_map.get(&rel.source_entity);
        let target_id = entity_id_map.get(&rel.target_entity);

        if let (Some(&src), Some(&tgt)) = (source_id, target_id) {
            let result = sqlx::query(
                r#"
                INSERT INTO edges (tenant_id, source_entity_id, target_entity_id, rel_type, properties, weight, source_episode_id)
                VALUES ($1, $2, $3, $4, $5, $6, $7)
                "#,
            )
            .bind(event.tenant_id)
            .bind(src)
            .bind(tgt)
            .bind(&rel.rel_type)
            .bind(rel.properties.as_ref().unwrap_or(&serde_json::json!({})))
            .bind(rel.weight.unwrap_or(1.0))
            .bind(event.episode_id)
            .execute(&mut *tx)
            .await;

            if result.is_ok() {
                created_edge_count += 1;
            }
        } else {
            warn!(
                "Skipping edge: entity not found ({} -> {})",
                rel.source_entity, rel.target_entity
            );
        }
    }

    tx.commit().await?;
    info!(
        "Graph update complete for episode {}: {} memories, {} entities, {} edges, {} skipped, {} superseded",
        event.episode_id,
        created_memory_ids.len(),
        created_entity_ids.len(),
        created_edge_count,
        skipped_count,
        superseded_ids.len()
    );

    // ── Webhook dispatch (async, non-blocking) ──
    if state.webhook_config.enabled {
        let db_pool = state.db_pool.clone();
        let wh_client = state.webhook_client.clone();
        let wh_config = state.webhook_config.clone();
        let wh_encryption_key = state.webhook_encryption_key;
        let summary = WebhookDispatchSummary {
            tenant_id: event.tenant_id,
            episode_id: event.episode_id,
            memory_ids: created_memory_ids.clone(),
            entity_ids: created_entity_ids.clone(),
            edge_count: created_edge_count,
            superseded_ids: superseded_ids.clone(),
            conflict_count: conflicts.len(),
        };

        tokio::spawn(async move {
            dispatch_webhooks(
                &db_pool,
                &wh_client,
                &wh_config,
                &wh_encryption_key,
                summary,
            )
            .await;
        });
    }

    Ok(())
}

/// Summary of resources created during extraction for webhook fan-out.
struct WebhookDispatchSummary {
    tenant_id: Uuid,
    episode_id: Uuid,
    memory_ids: Vec<Uuid>,
    entity_ids: Vec<(Uuid, String)>,
    edge_count: usize,
    superseded_ids: Vec<Uuid>,
    conflict_count: usize,
}

/// Dispatch webhook events for all resources created in this extraction.
async fn dispatch_webhooks(
    db_pool: &sqlx::PgPool,
    wh_client: &reqwest::Client,
    wh_config: &WebhookConfig,
    wh_encryption_key: &Option<[u8; 32]>,
    summary: WebhookDispatchSummary,
) {
    let WebhookDispatchSummary {
        tenant_id,
        episode_id,
        memory_ids,
        entity_ids,
        edge_count,
        superseded_ids,
        conflict_count,
    } = summary;

    // Load registered webhooks for this tenant
    let webhooks = match load_tenant_webhooks(db_pool, tenant_id, wh_encryption_key).await {
        Ok(w) => w,
        Err(e) => {
            debug!("No webhooks registered for tenant {}: {}", tenant_id, e);
            return;
        }
    };

    if webhooks.is_empty() {
        return;
    }

    // Fire memory.created events
    for mid in &memory_ids {
        let event = WebhookEvent::new(
            WebhookEventType::MemoryCreated,
            tenant_id,
            serde_json::json!({
                "memory_id": mid.to_string(),
                "episode_id": episode_id.to_string(),
            }),
        );
        fire_to_subscribers(db_pool, wh_client, &webhooks, &event, wh_config).await;
    }

    // Fire entity.created events
    for (eid, name) in &entity_ids {
        let event = WebhookEvent::new(
            WebhookEventType::EntityCreated,
            tenant_id,
            serde_json::json!({
                "entity_id": eid.to_string(),
                "name": name,
            }),
        );
        fire_to_subscribers(db_pool, wh_client, &webhooks, &event, wh_config).await;
    }

    // Fire edge.created summary (one event for all edges)
    if edge_count > 0 {
        let event = WebhookEvent::new(
            WebhookEventType::EdgeCreated,
            tenant_id,
            serde_json::json!({
                "episode_id": episode_id.to_string(),
                "edge_count": edge_count,
            }),
        );
        fire_to_subscribers(db_pool, wh_client, &webhooks, &event, wh_config).await;
    }

    // Fire conflict.detected events
    if conflict_count > 0 {
        let event = WebhookEvent::new(
            WebhookEventType::ConflictDetected,
            tenant_id,
            serde_json::json!({
                "episode_id": episode_id.to_string(),
                "conflict_count": conflict_count,
                "superseded_memory_ids": superseded_ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
            }),
        );
        fire_to_subscribers(db_pool, wh_client, &webhooks, &event, wh_config).await;
    }

    // Fire extraction.completed summary
    let event = WebhookEvent::new(
        WebhookEventType::ExtractionCompleted,
        tenant_id,
        serde_json::json!({
            "episode_id": episode_id.to_string(),
            "memories_created": memory_ids.len(),
            "entities_upserted": entity_ids.len(),
            "edges_created": edge_count,
            "conflicts_detected": conflict_count,
        }),
    );
    fire_to_subscribers(db_pool, wh_client, &webhooks, &event, wh_config).await;
}

/// Fire a webhook event to all subscribers that match the event type.
/// Persists delivery results to the webhook_deliveries table for audit.
async fn fire_to_subscribers(
    db_pool: &sqlx::PgPool,
    client: &reqwest::Client,
    webhooks: &[WebhookRegistration],
    event: &WebhookEvent,
    config: &WebhookConfig,
) {
    for wh in webhooks {
        if !wh.active {
            continue;
        }
        // Check if this webhook subscribes to this event type
        let subscribed = wh.event_types.is_empty()
            || wh.event_types.contains(&WebhookEventType::All)
            || wh.event_types.contains(&event.event_type);

        if subscribed {
            let delivery = deliver_webhook(client, wh, event, config).await;
            if delivery.success {
                debug!("Webhook delivered: {} → {}", event.event_type, wh.url);
            } else {
                warn!(
                    "Webhook delivery failed: {} → {} ({})",
                    event.event_type,
                    wh.url,
                    delivery.error.as_deref().unwrap_or("unknown")
                );
            }

            // Persist delivery result for audit trail (best-effort, don't fail on DB error)
            if let Err(e) = sqlx::query(
                r#"INSERT INTO webhook_deliveries (webhook_id, event_id, event_type, status_code, success, attempt, error)
                   VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
            )
            .bind(delivery.webhook_id)
            .bind(delivery.event_id)
            .bind(event.event_type.as_str())
            .bind(delivery.status_code.map(|s| s as i16))
            .bind(delivery.success)
            .bind(delivery.attempt as i32)
            .bind(&delivery.error)
            .execute(db_pool)
            .await
            {
                warn!("Failed to persist webhook delivery record: {}", e);
            }
        }
    }
}

/// Load active webhook registrations for a tenant from the database.
async fn load_tenant_webhooks(
    pool: &sqlx::PgPool,
    tenant_id: Uuid,
    wh_encryption_key: &Option<[u8; 32]>,
) -> anyhow::Result<Vec<WebhookRegistration>> {
    // Try to load from webhooks table; if table doesn't exist yet, return empty
    let rows = sqlx::query_as::<_, WebhookRow>(
        r#"
        SELECT id, tenant_id, url, secret, event_types, active, description, created_at
        FROM webhooks
        WHERE tenant_id = $1 AND active = true
        "#,
    )
    .bind(tenant_id)
    .fetch_all(pool)
    .await;

    match rows {
        Ok(rows) => Ok(rows
            .into_iter()
            .map(|r| {
                let event_types: Vec<WebhookEventType> = r
                    .event_types
                    .iter()
                    .filter_map(|s| serde_json::from_str(&format!("\"{}\"", s)).ok())
                    .collect();

                // Decrypt secret if it was stored encrypted (enc: prefix)
                let decrypted_secret = r.secret.as_deref().and_then(|s| {
                    if let Some(key) = &wh_encryption_key {
                        memory_common::webhook_crypto::decrypt_secret_opt(Some(s), key)
                    } else if s.starts_with(memory_common::webhook_crypto::ENCRYPTED_PREFIX) {
                        tracing::warn!(
                            "Webhook secret is encrypted but no decryption key is available"
                        );
                        None
                    } else {
                        Some(s.to_string())
                    }
                });

                WebhookRegistration {
                    id: r.id,
                    tenant_id: r.tenant_id,
                    url: r.url,
                    secret: decrypted_secret,
                    event_types,
                    active: r.active,
                    description: r.description,
                    created_at: r.created_at,
                }
            })
            .collect()),
        Err(e) => {
            // Gracefully handle missing webhooks table (not yet migrated)
            debug!("Could not load webhooks (table may not exist): {}", e);
            Ok(Vec::new())
        }
    }
}

#[derive(sqlx::FromRow)]
struct WebhookRow {
    id: Uuid,
    tenant_id: Uuid,
    url: String,
    secret: Option<String>,
    event_types: Vec<String>,
    active: bool,
    description: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
}

/// Wait for SIGTERM or Ctrl+C for graceful shutdown.
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

/// Helper to convert conflict type to string (can't impl on foreign type).
fn conflict_type_str(ct: &memory_llm::ConflictType) -> &'static str {
    match ct {
        memory_llm::ConflictType::Contradiction => "contradiction",
        memory_llm::ConflictType::Duplicate => "duplicate",
        memory_llm::ConflictType::Refinement => "refinement",
    }
}
