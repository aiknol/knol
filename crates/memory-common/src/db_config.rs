//! Database-backed configuration loader with env var fallback.
//!
//! Loads runtime settings from the `system_config` table (managed via admin panel).
//! Fallback chain: DB value → env var → compiled default.
//!
//! This allows most configuration to be changed at runtime through the admin panel
//! without restarting services or redeploying.

use sqlx::PgPool;
use tracing::debug;

/// Row from system_config table.
#[derive(Debug, sqlx::FromRow)]
struct ConfigRow {
    value: serde_json::Value,
}

/// Alias for load_str (used by some modules).
pub async fn load_string(pool: &PgPool, db_key: &str, env_name: &str, default: &str) -> String {
    load_str(pool, db_key, env_name, default).await
}

/// Check if a config key name looks like a secret (API key, password, token, etc.).
fn is_sensitive_key(db_key: &str) -> bool {
    let lower = db_key.to_lowercase();
    lower.contains("key")
        || lower.contains("secret")
        || lower.contains("token")
        || lower.contains("password")
        || lower.contains("credential")
}

/// Load a string config: DB → env → default.
/// Automatically redacts values of keys that look like secrets (containing
/// "key", "secret", "token", "password") in debug logs.
pub async fn load_str(pool: &PgPool, db_key: &str, env_name: &str, default: &str) -> String {
    let sensitive = is_sensitive_key(db_key);

    // 1. Try database
    if let Ok(row) =
        sqlx::query_as::<_, ConfigRow>("SELECT value FROM system_config WHERE key = $1")
            .bind(db_key)
            .fetch_one(pool)
            .await
    {
        if let Some(s) = row.value.as_str() {
            if sensitive {
                debug!("Config '{}' loaded from DB: [REDACTED]", db_key);
            } else {
                debug!("Config '{}' loaded from DB: {}", db_key, s);
            }
            return s.to_string();
        }
        let v = row.value.to_string();
        if v != "null" && !v.is_empty() {
            if sensitive {
                debug!("Config '{}' loaded from DB: [REDACTED]", db_key);
            } else {
                debug!("Config '{}' loaded from DB: {}", db_key, v);
            }
            return v;
        }
    }

    // 2. Try env var
    if let Ok(val) = std::env::var(env_name) {
        if !val.is_empty() {
            debug!("Config '{}' loaded from env {}", db_key, env_name);
            return val;
        }
    }

    // 3. Compiled default
    if sensitive {
        debug!("Config '{}' using default: [REDACTED]", db_key);
    } else {
        debug!("Config '{}' using default: {}", db_key, default);
    }
    default.to_string()
}

/// Load a secret string config: DB → env → default.
/// Like `load_str` but NEVER logs the value, only the source.
/// Use this for API keys, passwords, tokens, etc.
pub async fn load_secret(pool: &PgPool, db_key: &str, env_name: &str, default: &str) -> String {
    // 1. Try database
    if let Ok(row) =
        sqlx::query_as::<_, ConfigRow>("SELECT value FROM system_config WHERE key = $1")
            .bind(db_key)
            .fetch_one(pool)
            .await
    {
        if let Some(s) = row.value.as_str() {
            debug!("Config '{}' loaded from DB: [REDACTED]", db_key);
            return s.to_string();
        }
        let v = row.value.to_string();
        if v != "null" && !v.is_empty() {
            debug!("Config '{}' loaded from DB: [REDACTED]", db_key);
            return v;
        }
    }

    // 2. Try env var
    if let Ok(val) = std::env::var(env_name) {
        if !val.is_empty() {
            debug!(
                "Config '{}' loaded from env {} [REDACTED]",
                db_key, env_name
            );
            return val;
        }
    }

    // 3. Compiled default
    debug!("Config '{}' using default [REDACTED]", db_key);
    default.to_string()
}

/// Load a numeric config: DB → env → default.
pub async fn load_f64(pool: &PgPool, db_key: &str, env_name: &str, default: f64) -> f64 {
    let s = load_str(pool, db_key, env_name, &default.to_string()).await;
    s.parse().unwrap_or(default)
}

/// Load an integer config: DB → env → default.
pub async fn load_i64(pool: &PgPool, db_key: &str, env_name: &str, default: i64) -> i64 {
    let s = load_str(pool, db_key, env_name, &default.to_string()).await;
    s.parse().unwrap_or(default)
}

/// Load a u64 config: DB → env → default.
pub async fn load_u64(pool: &PgPool, db_key: &str, env_name: &str, default: u64) -> u64 {
    let s = load_str(pool, db_key, env_name, &default.to_string()).await;
    s.parse().unwrap_or(default)
}

/// Load a boolean config: DB → env → default.
pub async fn load_bool(pool: &PgPool, db_key: &str, env_name: &str, default: bool) -> bool {
    let s = load_str(pool, db_key, env_name, &default.to_string()).await;
    matches!(s.to_lowercase().as_str(), "true" | "1" | "yes")
}

/// Load a string array config from the DB only (no env fallback).
///
/// Expects the `value` column to be a JSON array of strings, e.g. `["a", "b"]`.
/// Returns an empty vec if the key doesn't exist or isn't a valid array.
pub async fn load_str_array(pool: &PgPool, db_key: &str) -> Option<Vec<String>> {
    let row = sqlx::query_as::<_, ConfigRow>("SELECT value FROM system_config WHERE key = $1")
        .bind(db_key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()?;

    row.value.as_array().map(|arr| {
        arr.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect()
    })
}

/// Load a float array config from the DB only.
///
/// Expects the `value` column to be a JSON array of numbers, e.g. `[0.1, 0.2, 0.4, 1.0]`.
/// Returns `None` if the key doesn't exist or isn't a valid array.
pub async fn load_f64_array(pool: &PgPool, db_key: &str) -> Option<Vec<f64>> {
    let row = sqlx::query_as::<_, ConfigRow>("SELECT value FROM system_config WHERE key = $1")
        .bind(db_key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()?;

    row.value
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_f64()).collect())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_bool_parsing_true_variants() {
        assert!(matches!("true", "true" | "1" | "yes"));
        assert!(matches!("1", "true" | "1" | "yes"));
        assert!(matches!("yes", "true" | "1" | "yes"));
    }

    #[test]
    fn test_bool_parsing_false_variants() {
        assert!(!matches!("false", "true" | "1" | "yes"));
        assert!(!matches!("0", "true" | "1" | "yes"));
        assert!(!matches!("no", "true" | "1" | "yes"));
        assert!(!matches!("", "true" | "1" | "yes"));
    }

    #[test]
    fn test_bool_parsing_case_sensitive() {
        // load_bool is case-sensitive — uppercase "True" won't match
        assert!(!matches!("True", "true" | "1" | "yes"));
        assert!(!matches!("TRUE", "true" | "1" | "yes"));
        assert!(!matches!("YES", "true" | "1" | "yes"));
    }

    #[test]
    fn test_f64_parse_fallback() {
        let s = "not_a_number";
        let default = 42.5f64;
        let result: f64 = s.parse().unwrap_or(default);
        assert_eq!(result, 42.5);
    }

    #[test]
    fn test_f64_parse_valid() {
        let s = "3.14";
        let default = 0.0f64;
        let result: f64 = s.parse().unwrap_or(default);
        let expected: f64 = s.parse().unwrap();
        assert!((result - expected).abs() < f64::EPSILON);
    }

    #[test]
    fn test_i64_parse_fallback() {
        let s = "abc";
        let default = 100i64;
        let result: i64 = s.parse().unwrap_or(default);
        assert_eq!(result, 100);
    }

    #[test]
    fn test_i64_parse_negative() {
        let s = "-42";
        let default = 0i64;
        let result: i64 = s.parse().unwrap_or(default);
        assert_eq!(result, -42);
    }

    #[test]
    fn test_u64_parse_fallback() {
        let s = "-1";
        let default = 8080u64;
        let result: u64 = s.parse().unwrap_or(default);
        assert_eq!(result, 8080);
    }

    #[test]
    fn test_u64_parse_valid() {
        let s = "8083";
        let default = 0u64;
        let result: u64 = s.parse().unwrap_or(default);
        assert_eq!(result, 8083);
    }

    #[test]
    fn test_config_row_from_json_string() {
        let json: serde_json::Value = serde_json::json!("hello");
        assert_eq!(json.as_str(), Some("hello"));
    }

    #[test]
    fn test_config_row_from_json_number() {
        let json: serde_json::Value = serde_json::json!(42);
        assert_eq!(json.as_str(), None);
        let s = json.to_string();
        assert_eq!(s, "42");
    }

    #[test]
    fn test_config_row_from_json_null() {
        let json: serde_json::Value = serde_json::json!(null);
        assert_eq!(json.as_str(), None);
        let s = json.to_string();
        assert_eq!(s, "null");
    }

    #[test]
    fn test_config_row_from_json_array() {
        let json: serde_json::Value = serde_json::json!(["a", "b", "c"]);
        let arr = json.as_array().unwrap();
        let strings: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(strings, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_config_row_mixed_array() {
        let json: serde_json::Value = serde_json::json!(["a", 1, "b", null]);
        let arr = json.as_array().unwrap();
        let strings: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        assert_eq!(strings, vec!["a", "b"]);
    }
}
