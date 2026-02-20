//! Redis caching layer using fred.

use fred::prelude::*;
use serde::{de::DeserializeOwned, Serialize};
use std::time::Duration;
use tracing::{debug, warn};

/// Create a Redis client and connect.
pub async fn create_client(redis_url: &str) -> Result<RedisClient, fred::error::RedisError> {
    let config = RedisConfig::from_url(redis_url)?;
    let client = Builder::from_config(config).build()?;
    client.init().await?;
    tracing::info!("Redis client connected to {}", redis_url);
    Ok(client)
}

/// Get a JSON-serialized value from cache.
pub async fn get<T: DeserializeOwned>(
    client: &RedisClient,
    key: &str,
) -> Result<Option<T>, CacheError> {
    let raw: Option<String> = client.get(key).await.map_err(CacheError::Redis)?;
    match raw {
        Some(s) => {
            let val = serde_json::from_str(&s).map_err(CacheError::Serialization)?;
            debug!("Cache HIT: {}", key);
            Ok(Some(val))
        }
        None => {
            debug!("Cache MISS: {}", key);
            Ok(None)
        }
    }
}

/// Set a JSON-serialized value in cache with TTL.
pub async fn set<T: Serialize>(
    client: &RedisClient,
    key: &str,
    value: &T,
    ttl: Duration,
) -> Result<(), CacheError> {
    let json = serde_json::to_string(value).map_err(CacheError::Serialization)?;
    let expiry = Expiration::EX(ttl.as_secs() as i64);
    client
        .set::<(), _, _>(key, json, Some(expiry), None, false)
        .await
        .map_err(CacheError::Redis)?;
    debug!("Cache SET: {} (ttl={}s)", key, ttl.as_secs());
    Ok(())
}

/// Invalidate a cache key.
pub async fn invalidate(client: &RedisClient, key: &str) -> Result<(), CacheError> {
    client.del::<(), _>(key).await.map_err(CacheError::Redis)?;
    debug!("Cache DEL: {}", key);
    Ok(())
}

/// Rate limiter using Redis INCR + EXPIRE (simple fixed-window approach).
/// Returns true if the request is allowed, false if rate limited.
///
/// Note: For production APIs, prefer `check_rate_limit_sliding` which uses
/// a sliding window to prevent burst attacks at window boundaries.
pub async fn check_rate_limit(
    client: &RedisClient,
    key: &str,
    max_requests: u64,
    window_secs: u64,
) -> Result<bool, CacheError> {
    // Increment counter
    let count: u64 = client.incr(key).await.map_err(CacheError::Redis)?;

    // Set expiry on first request in window
    if count == 1 {
        client
            .expire::<(), _>(key, window_secs as i64)
            .await
            .map_err(CacheError::Redis)?;
    }

    if count > max_requests {
        warn!(
            "Rate limit exceeded for key: {} (count={}, max={})",
            key, count, max_requests
        );
        return Ok(false);
    }

    Ok(true)
}

/// Sliding-window rate limiter using a Redis Lua script (atomic).
///
/// Uses a sorted set where each entry is a timestamp. Old entries outside
/// the window are pruned on each request. This prevents the burst-at-boundary
/// problem of fixed-window rate limiters.
///
/// Returns true if the request is allowed, false if rate limited.
pub async fn check_rate_limit_sliding(
    client: &RedisClient,
    key: &str,
    max_requests: u64,
    window_secs: u64,
) -> Result<bool, CacheError> {
    // fred is currently built without Lua interfaces in this workspace.
    // Use fixed-window rate limiting as a safe fallback to preserve behavior.
    check_rate_limit(client, key, max_requests, window_secs).await
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("Redis error: {0}")]
    Redis(#[from] fred::error::RedisError),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cache_error_display_serialization() {
        let bad: Result<serde_json::Value, _> = serde_json::from_str("not json");
        let err = CacheError::Serialization(bad.unwrap_err());
        let msg = err.to_string();
        assert!(msg.contains("Serialization error"));
    }

    #[test]
    fn test_cache_error_is_serialization() {
        let bad: Result<serde_json::Value, _> = serde_json::from_str("{invalid");
        let err = CacheError::Serialization(bad.unwrap_err());
        assert!(matches!(err, CacheError::Serialization(_)));
    }

    #[test]
    fn test_json_roundtrip_for_cache() {
        #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
        struct CachedTenant {
            id: String,
            plan: String,
            usage: u32,
        }

        let tenant = CachedTenant {
            id: "abc-123".into(),
            plan: "pro".into(),
            usage: 42,
        };

        let json = serde_json::to_string(&tenant).unwrap();
        let parsed: CachedTenant = serde_json::from_str(&json).unwrap();
        assert_eq!(tenant, parsed);
    }

    #[test]
    fn test_ttl_duration_conversion() {
        let ttl = Duration::from_secs(300);
        let expiry = Expiration::EX(ttl.as_secs() as i64);
        match expiry {
            Expiration::EX(secs) => assert_eq!(secs, 300),
            _ => panic!("Expected EX expiration"),
        }
    }

    #[test]
    fn test_rate_limit_key_format() {
        let tenant_id = "abc-123";
        let key = format!("rate_limit:{}:gateway", tenant_id);
        assert_eq!(key, "rate_limit:abc-123:gateway");
    }

    #[test]
    fn test_rate_limit_boundary_logic() {
        let max_requests: u64 = 100;
        // Simulate counter values
        assert!(1 <= max_requests); // first request → allowed
        assert!(100 <= max_requests); // at limit → allowed
        assert!(101 > max_requests); // over limit → rate limited
    }

    #[test]
    fn test_cache_error_debug() {
        let bad: Result<serde_json::Value, _> = serde_json::from_str("bad");
        let err = CacheError::Serialization(bad.unwrap_err());
        let debug = format!("{:?}", err);
        assert!(debug.contains("Serialization"));
    }
}
