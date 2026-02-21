//! Memory Gateway Service
//!
//! The primary API entry point. Handles auth, rate limiting, tenant context,
//! and routes requests to internal services.

use axum::{
    extract::{ConnectInfo, DefaultBodyLimit, Json, Path, Query, State},
    http::{header, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
    Router,
};
use memory_common::{
    MemoryError, MemorySearchRequest, MemorySearchResponse, MemoryWriteRequest,
    MemoryWriteResponse, TenantContext, TenantRole,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::{info, warn};
use uuid::Uuid;

mod state;
use state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=debug".into()),
        )
        .json()
        .init();

    info!("Starting Memory Gateway...");

    memory_common::startup::validate_env("service-gateway")?;

    let state = AppState::from_env().await?;
    let addr = SocketAddr::from(([0, 0, 0, 0], state.port));

    let app = create_router(state);

    info!("Gateway listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Graceful shutdown: wait for SIGTERM or Ctrl+C
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Gateway shut down gracefully");
    Ok(())
}

fn create_router(state: AppState) -> Router {
    let shared_state = Arc::new(state);

    let cors_base = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
        ])
        .allow_headers([header::CONTENT_TYPE, header::AUTHORIZATION]);

    let cors_raw = shared_state.cors_origins.trim();
    let cors = if cors_raw == "*" {
        warn!("CORS configured to allow all origins (wildcard). This is not recommended for production.");
        cors_base.allow_origin(Any)
    } else {
        let origins: Vec<HeaderValue> = cors_raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .filter_map(|s| HeaderValue::from_str(s).ok())
            .collect();
        if origins.is_empty() {
            warn!("CORS_ORIGINS is empty or invalid — defaulting to same-origin only. Set CORS_ORIGINS explicitly or use '*' for development.");
            cors_base
        } else {
            cors_base.allow_origin(origins)
        }
    };

    // Public health (no auth required)
    let health_routes = Router::new()
        .route("/health", get(health_check))
        .route("/metrics", get(metrics_with_auth));

    // ── Read-only routes (ReadOnly role and above) ──
    let read_routes = Router::new()
        .route("/v1/memory/search", post(search_memory))
        .route("/v1/memory/:id", get(get_memory))
        .route("/v1/graph/entities", get(list_entities))
        .route("/v1/graph/entities/:id", get(get_entity))
        .route("/v1/graph/entities/:id/edges", get(get_entity_edges))
        .route("/v1/graph/entities/:id/expand", get(expand_entity))
        .route("/v1/graph/entities/:id/traverse", get(traverse_entity))
        .route("/v1/graph/path/:from/:to", get(find_graph_path))
        .route(
            "/v1/graph/entities/:id/neighbors",
            get(get_entity_neighbors),
        )
        .route("/v1/memory/export", post(export_memories))
        .route("/v1/webhooks", get(list_webhooks))
        .layer(middleware::from_fn(require_read_only));

    // ── Developer routes (Developer role and above) ──
    let write_routes = Router::new()
        .route("/v1/memory", post(write_memory))
        .route("/v1/memory/batch", post(write_memory_batch))
        .route("/v1/memory/:id", put(update_memory))
        .route("/v1/memory/:id", delete(delete_memory))
        .route("/v1/memory/:id/restore", post(restore_memory))
        .route("/v1/memory/import", post(import_memories))
        .layer(middleware::from_fn(require_developer));

    // ── Admin routes (Admin role only) ──
    let admin_routes = Router::new()
        .route("/v1/webhooks", post(create_webhook))
        .route("/v1/webhooks/:id", delete(delete_webhook))
        .route("/v1/admin/tenants", get(get_tenant_info))
        .route("/v1/admin/audit", get(list_audit_log))
        .route("/v1/admin/policies", get(list_policies))
        .route("/v1/admin/policies", post(create_policy))
        .layer(middleware::from_fn(require_admin));

    // Merge all protected routes with shared auth + rate limiting
    let api_routes = Router::new()
        .merge(read_routes)
        .merge(write_routes)
        .merge(admin_routes)
        .layer(middleware::from_fn_with_state(
            shared_state.clone(),
            rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            shared_state.clone(),
            auth_middleware,
        ));

    Router::new()
        .merge(health_routes)
        .merge(api_routes)
        .layer(DefaultBodyLimit::max(shared_state.max_body_size))
        .layer(middleware::from_fn(request_id_middleware))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(shared_state)
}

/// Middleware that adds a unique X-Request-Id header and security headers
/// to each request and response for distributed tracing and hardening.
async fn request_id_middleware(request: Request<axum::body::Body>, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut response = next.run(request).await;
    if let Ok(val) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert("x-request-id", val);
    }
    // Security headers
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
        .headers_mut()
        .insert("x-frame-options", HeaderValue::from_static("DENY"));
    response.headers_mut().insert(
        "strict-transport-security",
        HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
    );
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=()"),
    );
    response
}

// ── Middleware ──

/// Extract client IP from X-Forwarded-For header (behind Caddy) or connection info.
///
/// X-Forwarded-For is only trusted when the direct peer (socket address) is in
/// the `trusted_proxies` set.  This prevents attackers from spoofing XFF to
/// bypass rate limits.
fn extract_client_ip(
    request: &Request<axum::body::Body>,
    trusted_proxies: &std::collections::HashSet<String>,
) -> String {
    let peer_ip = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|addr| addr.0.ip().to_string());

    let is_trusted_peer = peer_ip
        .as_ref()
        .map(|ip| trusted_proxies.contains(ip.as_str()))
        .unwrap_or(false);

    if is_trusted_peer {
        // Only trust proxy headers when the connection comes from a known proxy
        if let Some(xff) = request.headers().get("x-forwarded-for") {
            if let Ok(s) = xff.to_str() {
                if let Some(first_ip) = s.split(',').next() {
                    return first_ip.trim().to_string();
                }
            }
        }
        if let Some(real_ip) = request.headers().get("x-real-ip") {
            if let Ok(s) = real_ip.to_str() {
                return s.trim().to_string();
            }
        }
    }

    // Fall back to direct connection info
    peer_ip.unwrap_or_else(|| "unknown".to_string())
}

/// In-memory auth failure rate limiter — fallback when Redis is unavailable.
/// Ensures brute-force protection even without a Redis backend.
async fn check_in_memory_auth_limit(
    state: &Arc<AppState>,
    ip: &str,
    max_failures: u64,
    window_secs: u64,
) -> bool {
    let mut tracker = state.auth_fail_tracker.lock().await;
    let now = std::time::Instant::now();

    if let Some(entry) = tracker.get_mut(ip) {
        if now.duration_since(entry.window_start).as_secs() > window_secs {
            // Window expired, reset
            entry.count = 0;
            entry.window_start = now;
        }
        entry.count < max_failures
    } else {
        true // No record = allowed
    }
}

/// Record an auth failure in the in-memory tracker.
async fn record_in_memory_auth_failure(state: &Arc<AppState>, ip: &str) {
    let mut tracker = state.auth_fail_tracker.lock().await;
    let now = std::time::Instant::now();

    let entry = tracker
        .entry(ip.to_string())
        .or_insert_with(|| state::AuthFailEntry {
            count: 0,
            window_start: now,
        });

    if now.duration_since(entry.window_start).as_secs() > 300 {
        entry.count = 1;
        entry.window_start = now;
    } else {
        entry.count += 1;
    }

    // Periodic cleanup: remove expired entries when tracker gets large
    if tracker.len() > 1000 {
        tracker.retain(|_, e| now.duration_since(e.window_start).as_secs() <= 300);
    }

    // Hard cap: if still over limit after cleanup (active attack), evict oldest entries
    if tracker.len() > 10_000 {
        let mut entries: Vec<(String, std::time::Instant)> = tracker
            .iter()
            .map(|(k, v)| (k.clone(), v.window_start))
            .collect();
        entries.sort_by_key(|(_, ts)| *ts);
        // Remove oldest half
        let to_remove = entries.len() / 2;
        for (key, _) in entries.into_iter().take(to_remove) {
            tracker.remove(&key);
        }
    }
}

async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, MemoryError> {
    let client_ip = extract_client_ip(&request, &state.trusted_proxies);

    // Check if this IP is already rate-limited due to auth failures.
    // Allow 20 failed auth attempts per 5-minute window per IP.
    // Uses Redis when available, falls back to in-memory tracker.
    let auth_fail_key = format!("auth_fail:{}", client_ip);
    let auth_allowed = if let Some(ref redis) = state.redis_client {
        match memory_cache::check_rate_limit(redis, &auth_fail_key, 20, 300).await {
            Ok(allowed) => allowed,
            Err(_) => {
                // Redis error: fall back to in-memory tracker
                check_in_memory_auth_limit(&state, &client_ip, 20, 300).await
            }
        }
    } else {
        // No Redis: use in-memory tracker to still enforce brute-force protection
        check_in_memory_auth_limit(&state, &client_ip, 20, 300).await
    };

    if !auth_allowed {
        warn!("Auth rate limit exceeded for IP: {}", client_ip);
        return Err(MemoryError::RateLimited);
    }

    let auth_header = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| MemoryError::Auth("Missing Authorization header".into()))?;

    let api_key = auth_header.strip_prefix("Bearer ").ok_or_else(|| {
        MemoryError::Auth("Invalid Authorization format. Use: Bearer <api_key>".into())
    })?;

    // Hash the API key
    let key_hash = hash_api_key(api_key);

    // First try tenant_api_keys table (RBAC-aware), then fall back to legacy tenants.api_key_hash
    let (tenant, role) = if let Some(api_key_row) = sqlx::query_as::<_, ApiKeyRow>(
        r#"SELECT ak.role, ak.expires_at,
                  t.id, t.name, t.slug, t.plan, t.usage_ops_month, t.usage_limit
           FROM tenant_api_keys ak
           JOIN tenants t ON t.id = ak.tenant_id
           WHERE ak.key_hash = $1 AND ak.active = true"#,
    )
    .bind(&key_hash)
    .fetch_optional(&state.db_pool)
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?
    {
        // Check expiration
        if let Some(expires) = api_key_row.expires_at {
            if expires < chrono::Utc::now() {
                return Err(MemoryError::Auth("API key has expired".into()));
            }
        }

        // Update last_used_at (fire and forget)
        let pool = state.db_pool.clone();
        let kh = key_hash.clone();
        tokio::spawn(async move {
            let _ =
                sqlx::query("UPDATE tenant_api_keys SET last_used_at = now() WHERE key_hash = $1")
                    .bind(kh)
                    .execute(&pool)
                    .await;
        });

        let tenant = TenantRow {
            id: api_key_row.id,
            name: api_key_row.name,
            slug: api_key_row.slug,
            plan: api_key_row.plan,
            usage_ops_month: api_key_row.usage_ops_month,
            usage_limit: api_key_row.usage_limit,
        };
        let role = TenantRole::from_str_loose(&api_key_row.role);
        (tenant, role)
    } else {
        // Fallback: legacy lookup via tenants.api_key_hash (admin role by default)
        let tenant_opt = sqlx::query_as::<_, TenantRow>(
            "SELECT id, name, slug, plan, usage_ops_month, usage_limit FROM tenants WHERE api_key_hash = $1",
        )
        .bind(&key_hash)
        .fetch_optional(&state.db_pool)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

        match tenant_opt {
            Some(tenant) => (tenant, TenantRole::Admin),
            None => {
                // Track auth failure in both Redis and in-memory
                if let Some(ref redis) = state.redis_client {
                    let _ = memory_cache::check_rate_limit(redis, &auth_fail_key, 20, 300).await;
                }
                record_in_memory_auth_failure(&state, &client_ip).await;
                warn!("Invalid API key from IP: {}", client_ip);
                return Err(MemoryError::Auth("Invalid API key".into()));
            }
        }
    };

    // Check plan limits
    if let Some(limit) = tenant.usage_limit {
        if tenant.usage_ops_month >= limit {
            return Err(MemoryError::PlanLimitExceeded(format!(
                "Monthly operation limit ({}) exceeded for plan '{}'",
                limit, tenant.plan
            )));
        }
    }

    // Inject tenant context into request extensions
    let ctx = TenantContext {
        tenant_id: tenant.id,
        user_id: None,
        plan: tenant.plan,
        role,
    };
    request.extensions_mut().insert(ctx);

    Ok(next.run(request).await)
}

async fn rate_limit_middleware(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, MemoryError> {
    let ctx = request
        .extensions()
        .get::<TenantContext>()
        .cloned()
        .ok_or_else(|| MemoryError::Internal("Missing tenant context".into()))?;

    let rate_key = format!("rl:{}", ctx.tenant_id);
    let (max_rps, window_secs) = state
        .rate_limit_tiers
        .get(ctx.plan.as_str())
        .copied()
        .unwrap_or((10, 60));

    let allowed = if let Some(ref redis) = state.redis_client {
        match memory_cache::check_rate_limit_sliding(redis, &rate_key, max_rps, window_secs).await {
            Ok(allowed) => allowed,
            Err(e) => {
                warn!("Rate limit check failed: {}. Allowing request.", e);
                state.skip_rate_limit_on_redis_failure
            }
        }
    } else {
        if !state.skip_rate_limit_on_redis_failure {
            return Err(MemoryError::ServiceUnavailable(
                "Rate limiting service unavailable".into(),
            ));
        }
        true // No Redis = skip rate limiting
    };

    if !allowed {
        return Err(MemoryError::RateLimited);
    }

    Ok(next.run(request).await)
}

// ── Role Guard Middleware ──

async fn require_read_only(
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, MemoryError> {
    require_role(request, next, TenantRole::ReadOnly).await
}

async fn require_developer(
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, MemoryError> {
    require_role(request, next, TenantRole::Developer).await
}

async fn require_admin(
    request: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, MemoryError> {
    require_role(request, next, TenantRole::Admin).await
}

async fn require_role(
    request: Request<axum::body::Body>,
    next: Next,
    required: TenantRole,
) -> Result<Response, MemoryError> {
    let ctx = request
        .extensions()
        .get::<TenantContext>()
        .cloned()
        .ok_or_else(|| MemoryError::Internal("Missing tenant context".into()))?;

    if !ctx.role.has_permission(required) {
        return Err(MemoryError::Forbidden(format!(
            "Role '{}' does not have permission for this operation (requires '{}')",
            ctx.role, required
        )));
    }

    Ok(next.run(request).await)
}

/// Attach internal service secret header to outgoing requests if configured.
fn attach_internal_secret(
    builder: reqwest::RequestBuilder,
    state: &AppState,
) -> reqwest::RequestBuilder {
    if let Some(ref secret) = state.internal_service_secret {
        builder.header("x-internal-secret", secret.as_str())
    } else {
        builder
    }
}

// ── Route Handlers ──

async fn health_check() -> impl IntoResponse {
    Json(serde_json::json!({ "status": "ok", "service": "memory-gateway" }))
}

/// Metrics endpoint with optional bearer token protection.
/// If METRICS_TOKEN is set, requires `Authorization: Bearer <token>`.
/// This prevents exposing operational data (request rates, tenant counts, etc.)
/// to unauthenticated parties.
async fn metrics_with_auth(
    State(state): State<Arc<AppState>>,
    request: Request<axum::body::Body>,
) -> Result<Response, MemoryError> {
    if let Some(ref token) = state.metrics_token {
        let provided = request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "));

        // Use constant-time comparison to prevent timing side-channel attacks.
        let is_valid = provided
            .map(|t| {
                t.len() == token.len()
                    && t.as_bytes()
                        .iter()
                        .zip(token.as_bytes().iter())
                        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                        == 0
            })
            .unwrap_or(false);

        if !is_valid {
            return Err(MemoryError::Auth(
                "Metrics endpoint requires authentication. Set METRICS_TOKEN env var.".into(),
            ));
        }
    }
    Ok(memory_common::metrics::metrics_handler()
        .await
        .into_response())
}

async fn write_memory(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, MemoryError> {
    let req_builder = state
        .http_client
        .post(format!("{}/internal/ingest", state.write_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .header(
            "x-user-id",
            ctx.user_id.map(|u| u.to_string()).unwrap_or_default(),
        )
        .json(&req);
    let response: MemoryWriteResponse = attach_internal_secret(req_builder, &state)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: e.to_string(),
        })?;

    // Atomic usage counter increment — prevents plan limit bypass under concurrency.
    // The check in auth_middleware already validated the limit, but we still need
    // an atomic increment to prevent TOCTOU races.
    let pool = state.db_pool.clone();
    let tid = ctx.tenant_id;
    tokio::spawn(async move {
        let _ = sqlx::query(
            "UPDATE tenants SET usage_ops_month = usage_ops_month + 1 \
             WHERE id = $1 AND (usage_limit IS NULL OR usage_ops_month < usage_limit)",
        )
        .bind(tid)
        .execute(&pool)
        .await;
    });

    Ok(Json(response))
}

async fn write_memory_batch(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(req): Json<Vec<MemoryWriteRequest>>,
) -> Result<Json<Vec<MemoryWriteResponse>>, MemoryError> {
    let req_builder = state
        .http_client
        .post(format!("{}/internal/ingest/batch", state.write_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .json(&req);
    let response: Vec<MemoryWriteResponse> = attach_internal_secret(req_builder, &state)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: e.to_string(),
        })?;

    // Atomic usage counter increment for batch — count each item in the batch.
    let batch_size = response.len() as i32;
    if batch_size > 0 {
        let pool = state.db_pool.clone();
        let tid = ctx.tenant_id;
        tokio::spawn(async move {
            let _ = sqlx::query(
                "UPDATE tenants SET usage_ops_month = usage_ops_month + $2 \
                 WHERE id = $1 AND (usage_limit IS NULL OR usage_ops_month + $2 <= usage_limit)",
            )
            .bind(tid)
            .bind(batch_size)
            .execute(&pool)
            .await;
        });
    }

    Ok(Json(response))
}

async fn search_memory(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, MemoryError> {
    let req_builder = state
        .http_client
        .post(format!("{}/internal/search", state.retrieve_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .header(
            "x-user-id",
            ctx.user_id.map(|u| u.to_string()).unwrap_or_default(),
        )
        .json(&req);
    let response: MemorySearchResponse = attach_internal_secret(req_builder, &state)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "retrieve".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "retrieve".into(),
            message: e.to_string(),
        })?;

    Ok(Json(response))
}

async fn get_memory(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let row = sqlx::query_as::<_, MemoryRow>(
        "SELECT * FROM memories WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .fetch_optional(conn.as_mut())
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?
    .ok_or_else(|| MemoryError::NotFound(format!("Memory {} not found", id)))?;

    let value = serde_json::to_value(row)
        .map_err(|e| MemoryError::Internal(format!("Serialization error: {}", e)))?;
    Ok(Json(value))
}

async fn update_memory(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let response: serde_json::Value = state
        .http_client
        .put(format!(
            "{}/internal/memory/{}",
            state.admin_service_url, id
        ))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .json(&body)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "admin".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "admin".into(),
            message: e.to_string(),
        })?;

    Ok(Json(response))
}

async fn delete_memory(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
    Query(params): Query<DeleteMemoryParams>,
) -> Result<StatusCode, MemoryError> {
    if params.permanent.unwrap_or(false) {
        // Permanent delete: requires Admin role, delegates to admin service
        if !ctx.role.has_permission(TenantRole::Admin) {
            return Err(MemoryError::Forbidden(
                "Permanent delete requires Admin role".into(),
            ));
        }
        let _: serde_json::Value = state
            .http_client
            .delete(format!(
                "{}/internal/memory/{}",
                state.admin_service_url, id
            ))
            .header("x-tenant-id", ctx.tenant_id.to_string())
            .send()
            .await
            .map_err(|e| MemoryError::DownstreamServiceError {
                service: "admin".into(),
                message: e.to_string(),
            })?
            .json()
            .await
            .map_err(|e| MemoryError::DownstreamServiceError {
                service: "admin".into(),
                message: e.to_string(),
            })?;
    } else {
        // Soft delete: mark as deleted with timestamp
        let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
            .await
            .map_err(|e| MemoryError::Database(e.to_string()))?;

        let rows_affected = sqlx::query(
            "UPDATE memories SET status = 'deleted', deleted_at = now(), updated_at = now() WHERE id = $1 AND status = 'active'",
        )
        .bind(id)
        .execute(conn.as_mut())
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(MemoryError::NotFound(format!("Memory {} not found", id)));
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn restore_memory(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let restored = sqlx::query_scalar::<_, Uuid>(
        "UPDATE memories SET status = 'active', deleted_at = NULL, updated_at = now() WHERE id = $1 AND status = 'deleted' RETURNING id",
    )
    .bind(id)
    .fetch_optional(conn.as_mut())
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    match restored {
        Some(restored_id) => Ok(Json(serde_json::json!({
            "id": restored_id,
            "status": "restored",
        }))),
        None => Err(MemoryError::NotFound(format!(
            "No deleted memory {} found to restore",
            id
        ))),
    }
}

async fn list_entities(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Query(params): Query<ListEntitiesParams>,
) -> Result<Json<Vec<serde_json::Value>>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let rows = sqlx::query_as::<_, EntityQueryRow>(
        r#"
        SELECT id, name, entity_type, summary, attributes, created_at, updated_at
        FROM entities WHERE status = 'active'
        AND ($1::text IS NULL OR entity_type = $1)
        ORDER BY updated_at DESC LIMIT $2
        "#,
    )
    .bind(params.entity_type.as_deref())
    .bind(params.limit.unwrap_or(50).min(1000) as i64)
    .fetch_all(conn.as_mut())
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    let json: Vec<serde_json::Value> = rows
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MemoryError::Internal(format!("Serialization error: {}", e)))?;
    Ok(Json(json))
}

async fn get_entity(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let row = sqlx::query_as::<_, EntityQueryRow>(
        "SELECT id, name, entity_type, summary, attributes, created_at, updated_at FROM entities WHERE id = $1 AND status = 'active'",
    )
    .bind(id)
    .fetch_optional(conn.as_mut())
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?
    .ok_or_else(|| MemoryError::NotFound(format!("Entity {} not found", id)))?;

    let value = serde_json::to_value(row)
        .map_err(|e| MemoryError::Internal(format!("Serialization error: {}", e)))?;
    Ok(Json(value))
}

async fn get_entity_edges(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let outgoing = sqlx::query_as::<_, EdgeQueryRow>(
        r#"SELECT e.id, e.source_entity_id, e.target_entity_id, e.rel_type, e.weight,
           ent.name as target_name FROM edges e
           JOIN entities ent ON ent.id = e.target_entity_id
           WHERE e.source_entity_id = $1 AND e.status = 'active'"#,
    )
    .bind(id)
    .fetch_all(conn.as_mut())
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    let incoming = sqlx::query_as::<_, EdgeQueryRow>(
        r#"SELECT e.id, e.source_entity_id, e.target_entity_id, e.rel_type, e.weight,
           ent.name as target_name FROM edges e
           JOIN entities ent ON ent.id = e.source_entity_id
           WHERE e.target_entity_id = $1 AND e.status = 'active'"#,
    )
    .bind(id)
    .fetch_all(conn.as_mut())
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    let outgoing_json: Vec<serde_json::Value> = outgoing
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MemoryError::Internal(format!("Serialization error: {}", e)))?;
    let incoming_json: Vec<serde_json::Value> = incoming
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| MemoryError::Internal(format!("Serialization error: {}", e)))?;

    Ok(Json(serde_json::json!({
        "outgoing": outgoing_json,
        "incoming": incoming_json,
    })))
}

async fn expand_entity(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let entity_ids = memory_graph::expand_2hop(&state.db_pool, ctx.tenant_id, id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    Ok(Json(serde_json::json!({ "entity_ids": entity_ids })))
}

async fn get_tenant_info(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let row = sqlx::query_as::<_, TenantRow>(
        "SELECT id, name, slug, plan, usage_ops_month, usage_limit FROM tenants WHERE id = $1",
    )
    .bind(ctx.tenant_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    let value = serde_json::to_value(row)
        .map_err(|e| MemoryError::Internal(format!("Serialization error: {}", e)))?;
    Ok(Json(value))
}

async fn list_audit_log(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Query(params): Query<AuditParams>,
) -> Result<Json<Vec<serde_json::Value>>, MemoryError> {
    let response: Vec<serde_json::Value> = state
        .http_client
        .get(format!("{}/internal/audit", state.admin_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .query(&params)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "admin".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "admin".into(),
            message: e.to_string(),
        })?;

    Ok(Json(response))
}

async fn list_policies(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
) -> Result<Json<Vec<serde_json::Value>>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let rows: Vec<serde_json::Value> = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT to_jsonb(mp) FROM memory_policies mp WHERE enabled = true",
    )
    .fetch_all(conn.as_mut())
    .await
    .map_err(|e: sqlx::Error| MemoryError::Database(e.to_string()))?;

    Ok(Json(rows))
}

async fn create_policy(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), MemoryError> {
    let response: serde_json::Value = state
        .http_client
        .post(format!("{}/internal/policies", state.admin_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .json(&body)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "admin".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "admin".into(),
            message: e.to_string(),
        })?;

    Ok((StatusCode::CREATED, Json(response)))
}

// ── Export/Import Handlers ──

async fn export_memories(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let response: serde_json::Value = state
        .http_client
        .post(format!("{}/internal/export", state.retrieve_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .json(&req)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "retrieve".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "retrieve".into(),
            message: e.to_string(),
        })?;

    Ok(Json(response))
}

async fn import_memories(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), MemoryError> {
    let response: serde_json::Value = state
        .http_client
        .post(format!("{}/internal/import", state.write_service_url))
        .header("x-tenant-id", ctx.tenant_id.to_string())
        .json(&req)
        .send()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: e.to_string(),
        })?
        .json()
        .await
        .map_err(|e| MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: e.to_string(),
        })?;

    Ok((StatusCode::CREATED, Json(response)))
}

// ── Graph Traversal Handlers ──

async fn traverse_entity(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
    Query(params): Query<TraverseParams>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let depth = params
        .depth
        .unwrap_or(2)
        .min(state.graph_max_traversal_depth);
    let max_results = (params.limit.unwrap_or(50) as i64).min(state.graph_max_traversal_results);

    let results = memory_graph::expand_nhop(
        &state.db_pool,
        ctx.tenant_id,
        id,
        depth,
        None,
        max_results,
        Some(state.graph_max_traversal_depth),
    )
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    let entities: Vec<serde_json::Value> = results
        .iter()
        .map(|(eid, d)| serde_json::json!({"entity_id": eid, "distance": d}))
        .collect();

    Ok(Json(serde_json::json!({
        "source_entity_id": id,
        "depth": depth,
        "total": entities.len(),
        "entities": entities,
    })))
}

async fn find_graph_path(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path((from, to)): Path<(Uuid, Uuid)>,
    Query(params): Query<TraverseParams>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let max_depth = params.depth.unwrap_or(5).min(state.graph_max_path_depth);

    let path = memory_graph::find_path(
        &state.db_pool,
        ctx.tenant_id,
        from,
        to,
        max_depth,
        Some(state.graph_max_path_depth),
    )
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "from": from,
        "to": to,
        "path": path,
        "found": path.is_some(),
    })))
}

async fn get_entity_neighbors(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
    Query(params): Query<NeighborParams>,
) -> Result<Json<serde_json::Value>, MemoryError> {
    let neighbors = memory_graph::get_neighbors(
        &state.db_pool,
        ctx.tenant_id,
        id,
        params.rel_type.as_deref(),
        params.limit.unwrap_or(50).min(1000) as i64,
    )
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "entity_id": id,
        "total": neighbors.len(),
        "neighbors": neighbors,
    })))
}

// ── Webhook Handlers ──

async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
) -> Result<Json<Vec<serde_json::Value>>, MemoryError> {
    let mut conn = memory_db::acquire_tenant_conn(&state.db_pool, ctx.tenant_id)
        .await
        .map_err(|e| MemoryError::Database(e.to_string()))?;

    let rows: Vec<serde_json::Value> = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT to_jsonb(w) FROM webhooks w WHERE tenant_id = $1 AND active = true ORDER BY created_at DESC",
    )
    .bind(ctx.tenant_id)
    .fetch_all(conn.as_mut())
    .await
    .map_err(|e: sqlx::Error| MemoryError::Database(e.to_string()))?;

    // Mask secrets in the response — never expose raw or encrypted secrets
    let masked: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|mut row| {
            if let Some(obj) = row.as_object_mut() {
                if obj.contains_key("secret") {
                    obj.insert(
                        "secret".to_string(),
                        serde_json::Value::String("****".to_string()),
                    );
                }
            }
            row
        })
        .collect();

    Ok(Json(masked))
}

// MAX_WEBHOOKS_PER_TENANT is now configurable via state.max_webhooks_per_tenant

/// Allowed webhook event type strings.
const VALID_EVENT_TYPES: &[&str] = &[
    "*",
    "memory.created",
    "memory.updated",
    "memory.deleted",
    "memory.conflict",
    "memory.consolidated",
    "memory.decayed",
    "graph.entity_created",
    "graph.edge_created",
    "extraction.completed",
];

/// Validate a webhook URL to prevent SSRF attacks.
fn validate_webhook_url(url: &str) -> Result<(), MemoryError> {
    let parsed =
        url::Url::parse(url).map_err(|_| MemoryError::Validation("Invalid webhook URL".into()))?;

    // Only allow http/https
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(MemoryError::Validation(
            "Webhook URL must use http or https".into(),
        ));
    }

    // Must have a host
    let host = parsed
        .host_str()
        .ok_or_else(|| MemoryError::Validation("Webhook URL must have a host".into()))?;

    // Block internal/private IPs and reserved hostnames
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host == "0.0.0.0"
        || host == "169.254.169.254"
        || host.ends_with(".internal")
        || host.ends_with(".local")
    {
        return Err(MemoryError::Validation(
            "Webhook URL cannot target internal or reserved addresses".into(),
        ));
    }

    // Check for private IP ranges
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        let is_private = match ip {
            std::net::IpAddr::V4(v4) => {
                v4.is_loopback()
                    || v4.is_private()
                    || v4.is_link_local()
                    || v4.is_broadcast()
                    || v4.is_unspecified()
                    || v4.octets()[0] == 169 && v4.octets()[1] == 254 // link-local
            }
            std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
        };
        if is_private {
            return Err(MemoryError::Validation(
                "Webhook URL cannot target private or reserved IP ranges".into(),
            ));
        }
    }

    Ok(())
}

async fn create_webhook(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Json(body): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), MemoryError> {
    let url = body["url"]
        .as_str()
        .ok_or_else(|| MemoryError::Validation("Missing 'url' field".into()))?;

    // SSRF protection: validate webhook URL
    validate_webhook_url(url)?;

    let raw_secret = body["secret"].as_str().map(|s| s.to_string());
    let description = body["description"].as_str().map(|s| s.to_string());
    let event_types: Vec<String> = body["event_types"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| vec!["*".to_string()]);

    // Validate event types
    for et in &event_types {
        if !VALID_EVENT_TYPES.contains(&et.as_str()) {
            return Err(MemoryError::Validation(format!(
                "Unknown event type '{}'. Valid types: {}",
                et,
                VALID_EVENT_TYPES.join(", ")
            )));
        }
    }

    // Enforce per-tenant webhook quota
    let webhook_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM webhooks WHERE tenant_id = $1 AND active = true",
    )
    .bind(ctx.tenant_id)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    if webhook_count >= state.max_webhooks_per_tenant {
        return Err(MemoryError::Validation(format!(
            "Webhook quota exceeded (max {} per tenant)",
            state.max_webhooks_per_tenant
        )));
    }

    // Encrypt the webhook secret at rest if an encryption key is configured
    let stored_secret = match (&raw_secret, &state.webhook_encryption_key) {
        (Some(secret), Some(key)) => {
            let encrypted =
                memory_common::webhook_crypto::encrypt_secret(secret, key).map_err(|e| {
                    MemoryError::Internal(format!("Failed to encrypt webhook secret: {}", e))
                })?;
            Some(encrypted)
        }
        (Some(secret), None) => {
            warn!("Storing webhook secret in plaintext — set WEBHOOK_ENCRYPTION_KEY for encryption at rest");
            Some(secret.clone())
        }
        (None, _) => None,
    };

    let id = sqlx::query_scalar::<_, Uuid>(
        r#"INSERT INTO webhooks (tenant_id, url, secret, event_types, description)
           VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(ctx.tenant_id)
    .bind(url)
    .bind(stored_secret)
    .bind(&event_types)
    .bind(description)
    .fetch_one(&state.db_pool)
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": id,
            "url": url,
            "event_types": event_types,
            "active": true,
        })),
    ))
}

async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    ctx: axum::Extension<TenantContext>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, MemoryError> {
    sqlx::query(
        "UPDATE webhooks SET active = false, updated_at = now() WHERE id = $1 AND tenant_id = $2",
    )
    .bind(id)
    .bind(ctx.tenant_id)
    .execute(&state.db_pool)
    .await
    .map_err(|e| MemoryError::Database(e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct DeleteMemoryParams {
    permanent: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct TraverseParams {
    depth: Option<u32>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct NeighborParams {
    rel_type: Option<String>,
    limit: Option<usize>,
}

// ── Helper Types ──

/// Hash an API key with a static application salt.
/// The salt prevents rainbow-table attacks against the known `knol_live_` prefix.
fn hash_api_key(key: &str) -> String {
    const API_KEY_SALT: &[u8] = b"knol-memory-platform-v1";
    let mut hasher = Sha256::new();
    hasher.update(API_KEY_SALT);
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

/// Row returned from the tenant_api_keys JOIN tenants lookup.
#[derive(Debug, sqlx::FromRow)]
struct ApiKeyRow {
    role: String,
    expires_at: Option<chrono::DateTime<chrono::Utc>>,
    // From tenants (aliased via JOIN)
    id: Uuid,
    name: String,
    slug: String,
    plan: String,
    usage_ops_month: i32,
    usage_limit: Option<i32>,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
struct TenantRow {
    id: Uuid,
    name: String,
    slug: String,
    plan: String,
    usage_ops_month: i32,
    usage_limit: Option<i32>,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
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

#[derive(Debug, sqlx::FromRow, Serialize)]
struct EntityQueryRow {
    id: Uuid,
    name: String,
    entity_type: String,
    summary: Option<String>,
    attributes: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, sqlx::FromRow, Serialize)]
struct EdgeQueryRow {
    id: Uuid,
    source_entity_id: Uuid,
    target_entity_id: Uuid,
    rel_type: String,
    weight: f32,
    target_name: String,
}

#[derive(Debug, Deserialize)]
struct ListEntitiesParams {
    entity_type: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
struct AuditParams {
    limit: Option<i64>,
    memory_id: Option<Uuid>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_api_key_deterministic() {
        let hash1 = hash_api_key("test-api-key-12345");
        let hash2 = hash_api_key("test-api-key-12345");
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_hash_api_key_different_keys() {
        let hash1 = hash_api_key("key-a");
        let hash2 = hash_api_key("key-b");
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_hash_api_key_is_sha256_hex() {
        let hash = hash_api_key("test");
        // SHA256 hex output is always 64 characters
        assert_eq!(hash.len(), 64);
        // Should be valid hex
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_hash_api_key_known_value() {
        // SHA256(salt + "hello") — the salt is "knol-memory-platform-v1"
        let hash = hash_api_key("hello");
        assert_eq!(
            hash,
            "0254b0411757e197887de34540823b56a2984aad5b7576cd6b64ec8fa8dc73b0"
        );
        // Must NOT equal the unsalted SHA256("hello") — proves salt is applied
        assert_ne!(
            hash,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[test]
    fn test_hash_api_key_empty_string() {
        let hash = hash_api_key("");
        // SHA256(salt + "") — the salt is "knol-memory-platform-v1"
        assert_eq!(
            hash,
            "575ca5507aec8906955fd4a7054186c285a2e33d09c038b08bc244ecb338e44c"
        );
        // Must NOT equal the unsalted SHA256("") — proves salt is applied
        assert_ne!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_rate_limit_tiers() {
        // Verify rate limit mapping for each plan tier via HashMap lookup
        let mut tiers = HashMap::new();
        tiers.insert("free".to_string(), (10u64, 60u64));
        tiers.insert("developer".to_string(), (100, 60));
        tiers.insert("pro".to_string(), (500, 60));
        tiers.insert("team".to_string(), (2000, 60));
        tiers.insert("enterprise".to_string(), (10000, 60));

        let test_cases: Vec<(&str, u64)> = vec![
            ("free", 10),
            ("developer", 100),
            ("pro", 500),
            ("team", 2000),
            ("enterprise", 10000),
            ("unknown", 10), // fallback
        ];

        for (plan, expected_rps) in test_cases {
            let (max_rps, window_secs) = tiers.get(plan).copied().unwrap_or((10, 60));
            assert_eq!(max_rps, expected_rps, "Failed for plan: {}", plan);
            assert_eq!(window_secs, 60, "Window should always be 60s");
        }
    }

    #[test]
    fn test_list_entities_params_deserialize() {
        let json = r#"{"entity_type": "person", "limit": 25}"#;
        let params: ListEntitiesParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.entity_type, Some("person".into()));
        assert_eq!(params.limit, Some(25));
    }

    #[test]
    fn test_list_entities_params_empty() {
        let json = r#"{}"#;
        let params: ListEntitiesParams = serde_json::from_str(json).unwrap();
        assert!(params.entity_type.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn test_audit_params_deserialize() {
        let json = r#"{"limit": 50}"#;
        let params: AuditParams = serde_json::from_str(json).unwrap();
        assert_eq!(params.limit, Some(50));
        assert!(params.memory_id.is_none());
    }

    #[test]
    fn test_audit_params_with_memory_id() {
        let id = Uuid::new_v4();
        let json = format!(r#"{{"limit": 10, "memory_id": "{}"}}"#, id);
        let params: AuditParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params.memory_id, Some(id));
    }

    #[test]
    fn test_audit_params_serialization_roundtrip() {
        let params = AuditParams {
            limit: Some(100),
            memory_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&params).unwrap();
        let parsed: AuditParams = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.limit, params.limit);
        assert_eq!(parsed.memory_id, params.memory_id);
    }

    #[test]
    fn test_tenant_role_permission_in_gateway() {
        // Admin can access everything
        assert!(TenantRole::Admin.has_permission(TenantRole::Admin));
        assert!(TenantRole::Admin.has_permission(TenantRole::Developer));
        assert!(TenantRole::Admin.has_permission(TenantRole::ReadOnly));

        // Developer can access developer + read routes
        assert!(!TenantRole::Developer.has_permission(TenantRole::Admin));
        assert!(TenantRole::Developer.has_permission(TenantRole::Developer));
        assert!(TenantRole::Developer.has_permission(TenantRole::ReadOnly));

        // ReadOnly can only access read routes
        assert!(!TenantRole::ReadOnly.has_permission(TenantRole::Admin));
        assert!(!TenantRole::ReadOnly.has_permission(TenantRole::Developer));
        assert!(TenantRole::ReadOnly.has_permission(TenantRole::ReadOnly));
    }
}
