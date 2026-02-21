//! Application state for the gateway service.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use fred::prelude::*;
use memory_common::db_config;
use sqlx::PgPool;
use std::collections::HashMap;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// In-memory auth failure tracker entry. Used as fallback when Redis is unavailable
/// to prevent brute-force attacks even without a cache backend.
#[derive(Clone)]
pub struct AuthFailEntry {
    pub count: u64,
    pub window_start: std::time::Instant,
}

pub struct AppState {
    pub db_pool: PgPool,
    pub redis_client: Option<RedisClient>,
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
    /// If true, allow requests through when Redis is down (skip rate limiting).
    pub skip_rate_limit_on_redis_failure: bool,
    /// Configurable graph traversal limits (loaded from system_config).
    pub graph_max_traversal_depth: u32,
    pub graph_max_traversal_results: i64,
    pub graph_max_path_depth: u32,
    /// Configurable rate limit tiers: plan -> (max_ops, window_secs).
    pub rate_limit_tiers: HashMap<String, (u64, u64)>,
    /// Configurable body size limit in bytes.
    pub max_body_size: usize,
    /// Configurable max webhooks per tenant.
    pub max_webhooks_per_tenant: i64,
    /// Internal service shared secret for verifying internal service calls.
    pub internal_service_secret: Option<String>,
    /// Optional bearer token for the /metrics endpoint to prevent information disclosure.
    pub metrics_token: Option<String>,
    /// In-memory auth failure tracker, used as fallback when Redis is unavailable.
    pub auth_fail_tracker: Mutex<HashMap<String, AuthFailEntry>>,
    /// Set of trusted proxy IPs that are allowed to set X-Forwarded-For.
    /// Only connections from these IPs will have their XFF header trusted.
    pub trusted_proxies: std::collections::HashSet<String>,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgresql://memory:memory_dev@localhost:5432/memory".into());
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:6379".into());

        let db_pool = memory_db::create_pool(&database_url, 8).await?;
        memory_db::run_migrations(&db_pool).await?;
        info!("OSS migrations applied/verified");

        // Try to connect to Redis (optional — degrades gracefully if unavailable)
        let redis_client = match memory_cache::create_client(&redis_url).await {
            Ok(client) => {
                info!("Redis connected for rate limiting and caching");
                Some(client)
            }
            Err(e) => {
                warn!(
                    "Redis connection failed — rate limiting and auth failure tracking disabled: {}",
                    e
                );
                None
            }
        };
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

        // Resilience config
        let skip_rate_limit_on_redis_failure = db_config::load_bool(
            &db_pool,
            "resilience.skip_rate_limit_on_redis_failure",
            "RESILIENCE_SKIP_RATE_LIMIT_ON_REDIS_FAILURE",
            false,
        )
        .await;

        // Graph traversal limits
        let graph_max_traversal_depth = db_config::load_u64(
            &db_pool,
            "graph.max_traversal_depth",
            "GRAPH_MAX_TRAVERSAL_DEPTH",
            10,
        )
        .await as u32;
        let graph_max_traversal_results = db_config::load_u64(
            &db_pool,
            "graph.max_traversal_results",
            "GRAPH_MAX_TRAVERSAL_RESULTS",
            1000,
        )
        .await as i64;
        let graph_max_path_depth =
            db_config::load_u64(&db_pool, "graph.max_path_depth", "GRAPH_MAX_PATH_DEPTH", 10).await
                as u32;

        // Rate limit tiers (configurable from DB)
        let mut rate_limit_tiers = HashMap::new();
        rate_limit_tiers.insert(
            "free".to_string(),
            (
                db_config::load_u64(&db_pool, "gateway.rate_limit_free", "", 10).await,
                60,
            ),
        );
        rate_limit_tiers.insert(
            "developer".to_string(),
            (
                db_config::load_u64(&db_pool, "gateway.rate_limit_developer", "", 100).await,
                60,
            ),
        );
        rate_limit_tiers.insert(
            "pro".to_string(),
            (
                db_config::load_u64(&db_pool, "gateway.rate_limit_pro", "", 500).await,
                60,
            ),
        );
        rate_limit_tiers.insert(
            "team".to_string(),
            (
                db_config::load_u64(&db_pool, "gateway.rate_limit_team", "", 2000).await,
                60,
            ),
        );
        rate_limit_tiers.insert(
            "enterprise".to_string(),
            (
                db_config::load_u64(&db_pool, "gateway.rate_limit_enterprise", "", 10000).await,
                60,
            ),
        );

        // Body size limit
        let max_body_size = db_config::load_u64(
            &db_pool,
            "gateway.max_body_size_bytes",
            "GATEWAY_MAX_BODY_SIZE",
            10 * 1024 * 1024, // 10 MB default
        )
        .await as usize;

        // Max webhooks per tenant
        let max_webhooks_per_tenant = db_config::load_u64(
            &db_pool,
            "gateway.max_webhooks_per_tenant",
            "GATEWAY_MAX_WEBHOOKS_PER_TENANT",
            50,
        )
        .await as i64;

        // Internal service shared secret
        let internal_service_secret = std::env::var("INTERNAL_SERVICE_SECRET").ok();

        // Optional metrics endpoint token
        let metrics_token = std::env::var("METRICS_TOKEN").ok();

        // Trusted proxy IPs for X-Forwarded-For header validation.
        // Default: loopback + Docker bridge, configurable via TRUSTED_PROXIES env (comma-separated).
        let trusted_proxies: std::collections::HashSet<String> = std::env::var("TRUSTED_PROXIES")
            .unwrap_or_else(|_| "127.0.0.1,::1,172.17.0.1".to_string())
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

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
            skip_rate_limit_on_redis_failure,
            graph_max_traversal_depth,
            graph_max_traversal_results,
            graph_max_path_depth,
            rate_limit_tiers,
            max_body_size,
            max_webhooks_per_tenant,
            internal_service_secret,
            metrics_token,
            auth_fail_tracker: Mutex::new(HashMap::new()),
            trusted_proxies,
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
