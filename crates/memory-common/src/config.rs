//! Configuration types loaded from environment variables.
//!
//! Custom Debug impls redact secrets (API keys, passwords in URLs, JWT secrets)
//! to prevent accidental leakage in log output.

use serde::Deserialize;

/// Redact a connection URL by hiding the password component.
/// "postgresql://user:secret@host/db" → "postgresql://user:***@host/db"
fn redact_url(url: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(url) {
        if parsed.password().is_some() {
            let _ = parsed.set_password(Some("***"));
        }
        parsed.to_string()
    } else {
        "[invalid-url]".to_string()
    }
}

/// Redact a secret string, showing only the first 4 chars.
/// "sk-abc123xyz" → "sk-a..."
fn redact_secret(s: &str) -> String {
    if s.len() <= 4 {
        "***".to_string()
    } else {
        format!("{}...", &s[..4])
    }
}

#[derive(Clone, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_url")]
    pub database_url: String,
    #[serde(default = "default_max_connections")]
    pub max_connections: u32,
}

impl std::fmt::Debug for DatabaseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatabaseConfig")
            .field("database_url", &redact_url(&self.database_url))
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

fn default_db_url() -> String {
    "postgresql://memory:memory_dev@localhost:5432/memory".to_string()
}

fn default_max_connections() -> u32 {
    10
}

#[derive(Clone, Deserialize)]
pub struct RedisConfig {
    #[serde(default = "default_redis_url")]
    pub redis_url: String,
}

impl std::fmt::Debug for RedisConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RedisConfig")
            .field("redis_url", &redact_url(&self.redis_url))
            .finish()
    }
}

fn default_redis_url() -> String {
    "redis://localhost:6379".to_string()
}

#[derive(Clone, Deserialize)]
pub struct NatsConfig {
    #[serde(default = "default_nats_url")]
    pub nats_url: String,
}

impl std::fmt::Debug for NatsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsConfig")
            .field("nats_url", &redact_url(&self.nats_url))
            .finish()
    }
}

fn default_nats_url() -> String {
    "nats://localhost:4222".to_string()
}

#[derive(Clone, Deserialize)]
pub struct LlmConfig {
    pub anthropic_api_key: String,
    #[serde(default = "default_extraction_model")]
    pub extraction_model: String,
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    #[serde(default = "default_embedding_dim")]
    pub embedding_dim: usize,
}

impl std::fmt::Debug for LlmConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmConfig")
            .field("anthropic_api_key", &redact_secret(&self.anthropic_api_key))
            .field("extraction_model", &self.extraction_model)
            .field("embedding_model", &self.embedding_model)
            .field("embedding_dim", &self.embedding_dim)
            .finish()
    }
}

fn default_extraction_model() -> String {
    "claude-haiku-4-5-20251001".to_string()
}

fn default_embedding_model() -> String {
    "voyage-3-lite".to_string()
}

fn default_embedding_dim() -> usize {
    1024
}

#[derive(Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_gateway_port")]
    pub port: u16,
    pub jwt_secret: Option<String>,
}

impl std::fmt::Debug for GatewayConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayConfig")
            .field("port", &self.port)
            .field(
                "jwt_secret",
                &self.jwt_secret.as_ref().map(|s| redact_secret(s)),
            )
            .finish()
    }
}

fn default_gateway_port() -> u16 {
    8080
}

#[derive(Clone, Deserialize)]
pub struct ServiceConfig {
    pub database: DatabaseConfig,
    pub redis: RedisConfig,
    pub nats: NatsConfig,
    pub llm: LlmConfig,
    pub gateway: GatewayConfig,
}

impl std::fmt::Debug for ServiceConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceConfig")
            .field("database", &self.database)
            .field("redis", &self.redis)
            .field("nats", &self.nats)
            .field("llm", &self.llm)
            .field("gateway", &self.gateway)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_config_defaults() {
        let config = DatabaseConfig {
            database_url: default_db_url(),
            max_connections: default_max_connections(),
        };
        assert!(config.database_url.contains("memory"));
        assert_eq!(config.max_connections, 10);
    }

    #[test]
    fn test_redis_config_defaults() {
        let config = RedisConfig {
            redis_url: default_redis_url(),
        };
        assert!(config.redis_url.contains("6379"));
    }

    #[test]
    fn test_nats_config_defaults() {
        let config = NatsConfig {
            nats_url: default_nats_url(),
        };
        assert!(config.nats_url.contains("4222"));
    }

    #[test]
    fn test_llm_config_defaults() {
        let config = LlmConfig {
            anthropic_api_key: "test-key".to_string(),
            extraction_model: default_extraction_model(),
            embedding_model: default_embedding_model(),
            embedding_dim: default_embedding_dim(),
        };
        assert!(config.extraction_model.contains("claude"));
        assert_eq!(config.embedding_dim, 1024);
    }

    #[test]
    fn test_gateway_config_defaults() {
        let config = GatewayConfig {
            port: default_gateway_port(),
            jwt_secret: None,
        };
        assert_eq!(config.port, 8080);
    }

    #[test]
    fn test_debug_redacts_api_key() {
        let config = LlmConfig {
            anthropic_api_key: "sk-ant-secret-key-12345".to_string(),
            extraction_model: "claude-haiku-4-5-20251001".to_string(),
            embedding_model: "voyage-3-lite".to_string(),
            embedding_dim: 1024,
        };
        let debug_output = format!("{:?}", config);
        assert!(!debug_output.contains("sk-ant-secret-key-12345"));
        assert!(debug_output.contains("sk-a..."));
    }

    #[test]
    fn test_debug_redacts_jwt_secret() {
        let config = GatewayConfig {
            port: 8080,
            jwt_secret: Some("super-secret-jwt-value".to_string()),
        };
        let debug_output = format!("{:?}", config);
        assert!(!debug_output.contains("super-secret-jwt-value"));
        assert!(debug_output.contains("supe..."));
    }

    #[test]
    fn test_debug_redacts_database_password() {
        let config = DatabaseConfig {
            database_url: "postgresql://user:my_password@host:5432/db".to_string(),
            max_connections: 10,
        };
        let debug_output = format!("{:?}", config);
        assert!(!debug_output.contains("my_password"));
        assert!(debug_output.contains("***"));
    }

    #[test]
    fn test_debug_url_without_password() {
        let config = RedisConfig {
            redis_url: "redis://localhost:6379".to_string(),
        };
        let debug_output = format!("{:?}", config);
        // Should show URL normally when no password present
        assert!(debug_output.contains("localhost"));
    }
}
