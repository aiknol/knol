//! Webhook event notification system.
//!
//! Allows external systems to subscribe to memory events (create, update, delete,
//! conflict detected, consolidation, etc.) and receive HTTP POST notifications.
//!
//! ## Features
//!
//! - Register webhook endpoints with event type filters
//! - Automatic retry with exponential backoff (3 attempts)
//! - HMAC-SHA256 signature verification
//! - Event deduplication via idempotency keys
//! - Async delivery (fire-and-forget with retry queue)
//!
//! This is a key enterprise differentiator — neither Mem0 nor Zep offer webhooks.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Events that can trigger webhooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEventType {
    /// New memory created.
    MemoryCreated,
    /// Memory updated.
    MemoryUpdated,
    /// Memory deleted or superseded.
    MemoryDeleted,
    /// Conflict detected between memories.
    ConflictDetected,
    /// Entity created in knowledge graph.
    EntityCreated,
    /// Edge created in knowledge graph.
    EdgeCreated,
    /// Memory consolidation completed (episodic → semantic).
    ConsolidationCompleted,
    /// Memory importance decayed below threshold.
    DecayThresholdReached,
    /// Extraction completed for an episode.
    ExtractionCompleted,
    /// All events (wildcard).
    All,
}

impl WebhookEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MemoryCreated => "memory.created",
            Self::MemoryUpdated => "memory.updated",
            Self::MemoryDeleted => "memory.deleted",
            Self::ConflictDetected => "memory.conflict",
            Self::EntityCreated => "graph.entity_created",
            Self::EdgeCreated => "graph.edge_created",
            Self::ConsolidationCompleted => "memory.consolidated",
            Self::DecayThresholdReached => "memory.decayed",
            Self::ExtractionCompleted => "extraction.completed",
            Self::All => "*",
        }
    }
}

impl std::fmt::Display for WebhookEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Webhook registration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRegistration {
    /// Unique ID of this webhook registration.
    pub id: Uuid,
    /// Tenant that owns this webhook.
    pub tenant_id: Uuid,
    /// URL to POST events to.
    pub url: String,
    /// Secret for HMAC-SHA256 signature.
    pub secret: Option<String>,
    /// Event types to subscribe to (empty = all).
    pub event_types: Vec<WebhookEventType>,
    /// Whether this webhook is active.
    pub active: bool,
    /// Description for admin panel.
    pub description: Option<String>,
    /// Created timestamp.
    pub created_at: DateTime<Utc>,
}

/// Webhook event payload sent to subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookEvent {
    /// Unique event ID (for idempotency).
    pub id: Uuid,
    /// Event type.
    pub event_type: WebhookEventType,
    /// Tenant ID.
    pub tenant_id: Uuid,
    /// Timestamp when the event occurred.
    pub timestamp: DateTime<Utc>,
    /// Event-specific data.
    pub data: serde_json::Value,
}

impl WebhookEvent {
    /// Create a new webhook event.
    pub fn new(event_type: WebhookEventType, tenant_id: Uuid, data: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            event_type,
            tenant_id,
            timestamp: Utc::now(),
            data,
        }
    }
}

/// Webhook delivery result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDelivery {
    pub webhook_id: Uuid,
    pub event_id: Uuid,
    pub status_code: Option<u16>,
    pub success: bool,
    pub attempt: u32,
    pub error: Option<String>,
    pub delivered_at: DateTime<Utc>,
}

/// Webhook delivery configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Enable webhook system.
    pub enabled: bool,
    /// Maximum retry attempts.
    pub max_retries: u32,
    /// Base delay between retries (exponential backoff).
    pub retry_delay_ms: u64,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Maximum concurrent webhook deliveries.
    pub max_concurrent: usize,
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_retries: 3,
            retry_delay_ms: 1000,
            timeout_secs: 10,
            max_concurrent: 10,
        }
    }
}

/// Compute HMAC-SHA256 signature for webhook payload verification.
pub fn compute_signature(payload: &[u8], secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can accept any key length");
    mac.update(payload);
    let result = mac.finalize();
    hex::encode(result.into_bytes())
}

/// Check whether an IP address is private/internal.
fn is_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || (v4.octets()[0] == 169 && v4.octets()[1] == 254) // link-local
                || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64) // CGN 100.64/10
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// Check whether a URL resolves to a private/internal IP address.
/// Protects against SSRF by resolving DNS and checking all resolved addresses.
/// Re-validates at delivery time to prevent DNS rebinding attacks.
fn is_private_url(url: &str) -> bool {
    let parsed = match url::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return true, // invalid URL = treat as private
    };
    let host = match parsed.host_str() {
        Some(h) => h,
        None => return true,
    };
    // Block known internal hostnames
    if host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host == "0.0.0.0"
        || host == "169.254.169.254"
        || host.ends_with(".internal")
        || host.ends_with(".local")
    {
        return true;
    }
    // Check if host is a literal IP
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return is_private_ip(ip);
    }
    // Resolve DNS and check ALL resolved addresses.
    // This prevents attackers from registering a hostname that resolves to
    // an internal IP (e.g., attacker.com → 127.0.0.1).
    let port = parsed.port().unwrap_or(443);
    match std::net::ToSocketAddrs::to_socket_addrs(&(host, port)) {
        Ok(addrs) => {
            for addr in addrs {
                if is_private_ip(addr.ip()) {
                    return true;
                }
            }
            false
        }
        // DNS resolution failure — treat as blocked to be safe
        Err(_) => true,
    }
}

/// Deliver a webhook event to a registered endpoint.
/// Returns delivery result.
pub async fn deliver_webhook(
    client: &reqwest::Client,
    registration: &WebhookRegistration,
    event: &WebhookEvent,
    config: &WebhookConfig,
) -> WebhookDelivery {
    // Re-validate URL at delivery time to prevent DNS rebinding attacks.
    // The URL was validated at registration time, but DNS may have changed.
    if is_private_url(&registration.url) {
        return WebhookDelivery {
            webhook_id: registration.id,
            event_id: event.id,
            status_code: None,
            success: false,
            attempt: 0,
            error: Some("Webhook URL resolves to a private/internal address".into()),
            delivered_at: Utc::now(),
        };
    }

    let payload = match serde_json::to_vec(event) {
        Ok(p) => p,
        Err(e) => {
            return WebhookDelivery {
                webhook_id: registration.id,
                event_id: event.id,
                status_code: None,
                success: false,
                attempt: 0,
                error: Some(format!("Serialization error: {}", e)),
                delivered_at: Utc::now(),
            };
        }
    };

    let mut last_error = None;

    for attempt in 0..=config.max_retries {
        if attempt > 0 {
            let delay = config.retry_delay_ms * 2u64.pow(attempt - 1);
            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        }

        let mut req = client
            .post(&registration.url)
            .header("Content-Type", "application/json")
            .header("X-Knol-Event", event.event_type.as_str())
            .header("X-Knol-Event-ID", event.id.to_string())
            .header("X-Knol-Timestamp", event.timestamp.to_rfc3339())
            .timeout(std::time::Duration::from_secs(config.timeout_secs));

        // Add HMAC signature if secret is configured
        if let Some(ref secret) = registration.secret {
            let sig = compute_signature(&payload, secret);
            req = req.header("X-Knol-Signature", format!("sha256={}", sig));
        }

        match req.body(payload.clone()).send().await {
            Ok(resp) => {
                let status = resp.status().as_u16();
                let success = resp.status().is_success();
                if success || resp.status().is_client_error() {
                    // 2xx: success — return immediately.
                    // 4xx: client error — retrying won't help, return immediately.
                    return WebhookDelivery {
                        webhook_id: registration.id,
                        event_id: event.id,
                        status_code: Some(status),
                        success,
                        attempt,
                        error: if success {
                            None
                        } else {
                            Some(format!("HTTP {}", status))
                        },
                        delivered_at: Utc::now(),
                    };
                }
                // 5xx: server error — treat as retryable (same as network error).
                last_error = Some(format!("HTTP {}", status));
            }
            Err(e) => {
                last_error = Some(e.to_string());
            }
        }
    }

    WebhookDelivery {
        webhook_id: registration.id,
        event_id: event.id,
        status_code: None,
        success: false,
        attempt: config.max_retries,
        error: last_error,
        delivered_at: Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_webhook_event_type_as_str() {
        assert_eq!(WebhookEventType::MemoryCreated.as_str(), "memory.created");
        assert_eq!(
            WebhookEventType::ConflictDetected.as_str(),
            "memory.conflict"
        );
        assert_eq!(WebhookEventType::All.as_str(), "*");
    }

    #[test]
    fn test_webhook_event_creation() {
        let event = WebhookEvent::new(
            WebhookEventType::MemoryCreated,
            Uuid::new_v4(),
            serde_json::json!({"memory_id": "abc"}),
        );
        assert_eq!(event.event_type, WebhookEventType::MemoryCreated);
        assert!(event.timestamp <= Utc::now());
    }

    #[test]
    fn test_webhook_event_serialization() {
        let event = WebhookEvent::new(
            WebhookEventType::EntityCreated,
            Uuid::new_v4(),
            serde_json::json!({"entity_name": "Google"}),
        );
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("entity_created"));
        assert!(json.contains("Google"));
    }

    #[test]
    fn test_webhook_config_defaults() {
        let config = WebhookConfig::default();
        assert!(config.enabled);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.timeout_secs, 10);
    }

    #[test]
    fn test_hmac_signature_deterministic() {
        let sig1 = compute_signature(b"hello", "secret");
        let sig2 = compute_signature(b"hello", "secret");
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_hmac_signature_differs_with_secret() {
        let sig1 = compute_signature(b"hello", "secret1");
        let sig2 = compute_signature(b"hello", "secret2");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_hmac_signature_differs_with_payload() {
        let sig1 = compute_signature(b"hello", "secret");
        let sig2 = compute_signature(b"world", "secret");
        assert_ne!(sig1, sig2);
    }

    #[test]
    fn test_webhook_registration_serialization() {
        let reg = WebhookRegistration {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4(),
            url: "https://example.com/webhook".into(),
            secret: Some("whsec_test123".into()),
            event_types: vec![
                WebhookEventType::MemoryCreated,
                WebhookEventType::ConflictDetected,
            ],
            active: true,
            description: Some("Test webhook".into()),
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&reg).unwrap();
        let back: WebhookRegistration = serde_json::from_str(&json).unwrap();
        assert_eq!(back.url, "https://example.com/webhook");
        assert_eq!(back.event_types.len(), 2);
    }
}
