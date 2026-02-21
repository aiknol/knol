//! Memory Write Service
//!
//! Handles ingestion: stores episodes, emits events to NATS for async extraction.
//! Implements the "fast ACK" pattern — writes return immediately, extraction happens async.

use axum::{
    extract::{Json, State},
    http::HeaderMap,
    routing::post,
    Router,
};
use chrono::Utc;
use memory_common::{MemoryWriteEvent, MemoryWriteRequest, MemoryWriteResponse};
use sha2::{Digest, Sha256};
use std::{net::SocketAddr, sync::Arc};
use tracing::{error, info, warn};
use uuid::Uuid;

struct AppState {
    db_pool: sqlx::PgPool,
    nats_js: async_nats::jetstream::Context,
    max_content_len: usize,
    /// Optional shared secret to verify internal service calls.
    internal_service_secret: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .json()
        .init();

    info!("Starting Memory Write Service...");

    memory_common::startup::validate_env("service-write")?;

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgresql://memory:memory_dev@localhost:5432/memory".into());
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());

    let db_pool = memory_db::create_pool(&database_url, 6).await?;
    let (_nats_client, nats_js) = memory_queue::connect(&nats_url).await?;
    memory_queue::ensure_stream(&nats_js).await?;

    let port: u16 = memory_common::db_config::load_u64(
        &db_pool,
        "services.write_port",
        "WRITE_SERVICE_PORT",
        8081,
    )
    .await as u16;

    let max_content_len = memory_common::db_config::load_u64(
        &db_pool,
        "guardrails.max_input_content_len",
        "MAX_INPUT_CONTENT_LEN",
        50000,
    )
    .await as usize;

    let internal_service_secret = std::env::var("INTERNAL_SERVICE_SECRET").ok();

    let state = Arc::new(AppState {
        db_pool,
        nats_js,
        max_content_len,
        internal_service_secret,
    });

    let app = Router::new()
        .route("/internal/ingest", post(ingest))
        .route("/internal/ingest/batch", post(ingest_batch))
        .route(
            "/health",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({"status": "ok", "service": "memory-write"}))
            }),
        )
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("Write service listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("Write service shut down gracefully");
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

/// Verify optional internal service secret, then extract tenant ID.
/// Uses constant-time comparison to prevent timing attacks.
fn verify_internal_auth(
    headers: &HeaderMap,
    expected_secret: &Option<String>,
) -> Result<(), memory_common::MemoryError> {
    if let Some(secret) = expected_secret {
        let provided = headers
            .get("x-internal-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Constant-time comparison to prevent timing attacks.
        // First check lengths, then compare byte-by-byte with constant-time XOR.
        let matches = provided.len() == secret.len()
            && provided
                .as_bytes()
                .iter()
                .zip(secret.as_bytes().iter())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if !matches {
            return Err(memory_common::MemoryError::Auth(
                "Invalid or missing internal service secret".into(),
            ));
        }
    }
    Ok(())
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

async fn ingest(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, memory_common::MemoryError> {
    verify_internal_auth(&headers, &state.internal_service_secret)?;
    let tenant_id = extract_tenant_id(&headers)?;
    let user_id = req.user_id.or_else(|| extract_user_id(&headers));
    let role = req.role.as_deref().unwrap_or("user");

    // Input validation
    if req.content.is_empty() {
        return Err(memory_common::MemoryError::Validation(
            "Content cannot be empty".into(),
        ));
    }
    if req.content.len() > state.max_content_len {
        return Err(memory_common::MemoryError::Validation(format!(
            "Content too long: {} bytes (max {})",
            req.content.len(),
            state.max_content_len
        )));
    }

    // Compute content hash for dedup
    let content_hash = {
        let mut hasher = Sha256::new();
        hasher.update(req.content.as_bytes());
        hex::encode(hasher.finalize())
    };

    // Store episode in DB
    let episode_id = Uuid::new_v4();
    let mut tx = memory_db::begin_tenant_tx(&state.db_pool, tenant_id)
        .await
        .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

    sqlx::query(
        r#"
        INSERT INTO episodes (id, tenant_id, user_id, session_id, agent_id, content, role, content_hash, metadata)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
        "#,
    )
    .bind(episode_id)
    .bind(tenant_id)
    .bind(user_id)
    .bind(req.session_id.as_deref())
    .bind(req.agent_id.as_deref())
    .bind(&req.content)
    .bind(role)
    .bind(&content_hash)
    .bind(req.metadata.as_ref().unwrap_or(&serde_json::json!({})))
    .execute(&mut *tx)
    .await
    .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

    // Emit event to NATS for async extraction
    let event = MemoryWriteEvent {
        episode_id,
        tenant_id,
        user_id,
        content: req.content,
        role: role.to_string(),
        session_id: req.session_id,
        agent_id: req.agent_id,
        metadata: req.metadata.unwrap_or(serde_json::json!({})),
        timestamp: Utc::now(),
    };

    if let Err(e) = memory_queue::publish(&state.nats_js, memory_queue::SUBJECT_WRITE, &event).await
    {
        error!("Failed to publish write event: {}", e);
        // Don't fail the request — episode is stored, extraction will be retried
    }

    info!("Ingested episode {} for tenant {}", episode_id, tenant_id);

    Ok(Json(MemoryWriteResponse {
        episode_id,
        status: "accepted".to_string(),
    }))
}

/// Maximum items allowed in a single batch request to prevent resource exhaustion.
const MAX_BATCH_SIZE: usize = 100;

async fn ingest_batch(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(requests): Json<Vec<MemoryWriteRequest>>,
) -> Result<Json<Vec<MemoryWriteResponse>>, memory_common::MemoryError> {
    verify_internal_auth(&headers, &state.internal_service_secret)?;
    let tenant_id = extract_tenant_id(&headers)?;

    // Enforce batch size limit to prevent resource exhaustion
    if requests.len() > MAX_BATCH_SIZE {
        return Err(memory_common::MemoryError::Validation(format!(
            "Batch too large: {} items (max {})",
            requests.len(),
            MAX_BATCH_SIZE
        )));
    }

    let mut responses = Vec::with_capacity(requests.len());

    // Validate all items first
    for (i, req) in requests.iter().enumerate() {
        if req.content.is_empty() {
            return Err(memory_common::MemoryError::Validation(format!(
                "Item {}: content cannot be empty",
                i
            )));
        }
        if req.content.len() > state.max_content_len {
            return Err(memory_common::MemoryError::Validation(format!(
                "Item {}: content too long: {} bytes (max {})",
                i,
                req.content.len(),
                state.max_content_len
            )));
        }
    }

    for req in requests {
        let user_id = req.user_id.or_else(|| extract_user_id(&headers));
        let role = req.role.as_deref().unwrap_or("user");
        let content_hash = {
            let mut hasher = Sha256::new();
            hasher.update(req.content.as_bytes());
            hex::encode(hasher.finalize())
        };

        let episode_id = Uuid::new_v4();
        let mut tx = memory_db::begin_tenant_tx(&state.db_pool, tenant_id)
            .await
            .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO episodes (id, tenant_id, user_id, session_id, agent_id, content, role, content_hash, metadata)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(episode_id)
        .bind(tenant_id)
        .bind(user_id)
        .bind(req.session_id.as_deref())
        .bind(req.agent_id.as_deref())
        .bind(&req.content)
        .bind(role)
        .bind(&content_hash)
        .bind(req.metadata.as_ref().unwrap_or(&serde_json::json!({})))
        .execute(&mut *tx)
        .await
        .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| memory_common::MemoryError::Database(e.to_string()))?;

        let event = MemoryWriteEvent {
            episode_id,
            tenant_id,
            user_id,
            content: req.content,
            role: role.to_string(),
            session_id: req.session_id,
            agent_id: req.agent_id,
            metadata: req.metadata.unwrap_or(serde_json::json!({})),
            timestamp: Utc::now(),
        };

        let _ = memory_queue::publish(&state.nats_js, memory_queue::SUBJECT_WRITE, &event).await;

        responses.push(MemoryWriteResponse {
            episode_id,
            status: "accepted".to_string(),
        });
    }

    Ok(Json(responses))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_extract_tenant_id_valid() {
        let mut headers = HeaderMap::new();
        let id = Uuid::new_v4();
        headers.insert(
            "x-tenant-id",
            HeaderValue::from_str(&id.to_string()).unwrap(),
        );
        let result = extract_tenant_id(&headers);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), id);
    }

    #[test]
    fn test_extract_tenant_id_missing() {
        let headers = HeaderMap::new();
        let result = extract_tenant_id(&headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_tenant_id_invalid_uuid() {
        let mut headers = HeaderMap::new();
        headers.insert("x-tenant-id", HeaderValue::from_static("not-a-uuid"));
        let result = extract_tenant_id(&headers);
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_user_id_valid() {
        let mut headers = HeaderMap::new();
        let id = Uuid::new_v4();
        headers.insert("x-user-id", HeaderValue::from_str(&id.to_string()).unwrap());
        let result = extract_user_id(&headers);
        assert_eq!(result, Some(id));
    }

    #[test]
    fn test_extract_user_id_missing() {
        let headers = HeaderMap::new();
        assert!(extract_user_id(&headers).is_none());
    }

    #[test]
    fn test_extract_user_id_invalid() {
        let mut headers = HeaderMap::new();
        headers.insert("x-user-id", HeaderValue::from_static("bad"));
        assert!(extract_user_id(&headers).is_none());
    }

    #[test]
    fn test_content_hash_deterministic() {
        let content = "Hello, world!";
        let hash1 = {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            hex::encode(hasher.finalize())
        };
        let hash2 = {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            hex::encode(hasher.finalize())
        };
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_different_content() {
        let hash1 = {
            let mut hasher = Sha256::new();
            hasher.update(b"content A");
            hex::encode(hasher.finalize())
        };
        let hash2 = {
            let mut hasher = Sha256::new();
            hasher.update(b"content B");
            hex::encode(hasher.finalize())
        };
        assert_ne!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_is_sha256() {
        let hash = {
            let mut hasher = Sha256::new();
            hasher.update(b"test");
            hex::encode(hasher.finalize())
        };
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_write_request_deserialization() {
        let json = r#"{
            "content": "User prefers dark mode",
            "role": "user",
            "session_id": "sess-123",
            "metadata": {"source": "chat"}
        }"#;
        let req: MemoryWriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "User prefers dark mode");
        assert_eq!(req.role, Some("user".to_string()));
        assert_eq!(req.session_id, Some("sess-123".to_string()));
    }

    #[test]
    fn test_write_request_minimal() {
        let json = r#"{"content": "hello"}"#;
        let req: MemoryWriteRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.content, "hello");
        assert!(req.role.is_none());
        assert!(req.user_id.is_none());
        assert!(req.session_id.is_none());
        assert!(req.agent_id.is_none());
        assert!(req.metadata.is_none());
    }

    #[test]
    fn test_write_response_serialization() {
        let resp = MemoryWriteResponse {
            episode_id: Uuid::new_v4(),
            status: "accepted".to_string(),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("accepted"));
        assert!(json.contains("episode_id"));
    }

    #[test]
    fn test_default_role() {
        let role: Option<String> = None;
        assert_eq!(role.as_deref().unwrap_or("user"), "user");
    }

    #[test]
    fn test_explicit_role() {
        let role = Some("assistant".to_string());
        assert_eq!(role.as_deref().unwrap_or("user"), "assistant");
    }
}
