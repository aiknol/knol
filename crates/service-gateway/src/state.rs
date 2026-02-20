//! Application state for the gateway service.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use fred::prelude::*;
use memory_common::db_config;
use sqlx::PgPool;
use tracing::warn;

pub struct AppState {
    pub db_pool: PgPool,
    pub redis_client: RedisClient,
    pub http_client: reqwest::Client,
    pub port: u16,
    pub write_service_url: String,
    pub retrieve_service_url: String,
    pub admin_service_url: String,
    pub cors_origins: String,
    /// Optional AES-256-GCM key for encrypting webhook secrets at rest.
    /// Set via WEBHOOK_ENCRYPTION_KEY or ADMIN_ENCRYPTION_KEY env var.
    /// If None, webhook secrets are stored in plaintext (with a warning).
    pub webhook_encryption_key: Option<[u8; 32]>,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://memory:memory_dev@localhost:5432/memory".into());
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());

        let db_pool = memory_db::create_pool(&database_url, 8).await?;
        let redis_client = memory_cache::create_client(&redis_url).await?;
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(5))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(std::time::Duration::from_secs(300))
            .build()?;

        // Port: DB → env → default
        let port = db_config::load_u64(&db_pool, "services.gateway_port", "GATEWAY_PORT", 8080)
            .await as u16;

        // Service URLs: DB → env → default
        let write_service_url = db_config::load_str(
            &db_pool,
            "gateway.write_service_url",
            "WRITE_SERVICE_URL",
            "http://localhost:8081",
        )
        .await;
        let retrieve_service_url = db_config::load_str(
            &db_pool,
            "gateway.retrieve_service_url",
            "RETRIEVE_SERVICE_URL",
            "http://localhost:8082",
        )
        .await;
        let admin_service_url = db_config::load_str(
            &db_pool,
            "gateway.admin_service_url",
            "ADMIN_SERVICE_URL",
            "http://localhost:8084",
        )
        .await;
        let cors_origins = db_config::load_str(
            &db_pool,
            "gateway.cors_origins",
            "GATEWAY_CORS_ORIGINS",
            "http://localhost:3005,http://localhost:3006,http://localhost:8080",
        )
        .await;

        // Load optional webhook encryption key (try WEBHOOK_ENCRYPTION_KEY, fall back to ADMIN_ENCRYPTION_KEY)
        let webhook_encryption_key = Self::load_webhook_encryption_key();

        Ok(Self {
            db_pool,
            redis_client,
            http_client,
            port,
            write_service_url,
            retrieve_service_url,
            admin_service_url,
            cors_origins,
            webhook_encryption_key,
        })
    }

    /// Try to load a 32-byte AES key from env vars.
    /// Returns None (with warning) if not configured.
    fn load_webhook_encryption_key() -> Option<[u8; 32]> {
        let b64 = std::env::var("WEBHOOK_ENCRYPTION_KEY")
            .or_else(|_| std::env::var("ADMIN_ENCRYPTION_KEY"))
            .ok()?;

        let bytes = match B64.decode(&b64) {
            Ok(b) => b,
            Err(e) => {
                warn!("Webhook encryption key is not valid base64: {}. Webhook secrets will be stored in plaintext.", e);
                return None;
            }
        };

        if bytes.len() != 32 {
            warn!(
                "Webhook encryption key must be 32 bytes (got {}). Webhook secrets will be stored in plaintext.",
                bytes.len()
            );
            return None;
        }

        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        tracing::info!("Webhook secret encryption enabled (AES-256-GCM)");
        Some(key)
    }
}
