//! NATS JetStream publisher and subscriber for async memory processing.

use async_nats::jetstream::{self, consumer::PullConsumer, stream::Stream, Context};
use serde::{de::DeserializeOwned, Serialize};
use tracing::info;

pub const STREAM_NAME: &str = "MEMORY";
pub const SUBJECT_WRITE: &str = "memory.write";
pub const SUBJECT_EXTRACT: &str = "memory.extract";

/// Redact password from a connection URL for safe logging.
/// Replaces `://user:password@` with `://user:***@`.
fn redact_url_password(url: &str) -> String {
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        if let Some(at_pos) = after_scheme.find('@') {
            let userinfo = &after_scheme[..at_pos];
            if let Some(colon_pos) = userinfo.find(':') {
                let user = &userinfo[..colon_pos];
                let host_part = &after_scheme[at_pos..];
                return format!("{}://{}:***{}", &url[..scheme_end], user, host_part);
            }
        }
    }
    url.to_string()
}

/// Parse optional userinfo (user:password) from a NATS URL.
/// Returns (clean_url_without_userinfo, Option<(user, password)>).
fn parse_nats_credentials(url: &str) -> (String, Option<(String, String)>) {
    if let Some(scheme_end) = url.find("://") {
        let after_scheme = &url[scheme_end + 3..];
        if let Some(at_pos) = after_scheme.find('@') {
            let userinfo = &after_scheme[..at_pos];
            let host_part = &after_scheme[at_pos + 1..];
            let clean_url = format!("{}://{}", &url[..scheme_end], host_part);
            if let Some(colon_pos) = userinfo.find(':') {
                let user = userinfo[..colon_pos].to_string();
                let pass = userinfo[colon_pos + 1..].to_string();
                return (clean_url, Some((user, pass)));
            }
        }
    }
    (url.to_string(), None)
}

/// Connect to NATS and return a JetStream context.
/// Supports credentials via:
///   1. Embedded in URL: `nats://user:pass@host:port`
///   2. Environment variables: `NATS_USER` and `NATS_PASSWORD`
pub async fn connect(nats_url: &str) -> Result<(async_nats::Client, Context), QueueError> {
    let (clean_url, url_creds) = parse_nats_credentials(nats_url);

    // Prefer credentials from URL, fall back to env vars.
    let creds = url_creds.or_else(|| {
        let user = std::env::var("NATS_USER").ok()?;
        let pass = std::env::var("NATS_PASSWORD").ok()?;
        Some((user, pass))
    });

    let client = if let Some((user, pass)) = creds {
        async_nats::ConnectOptions::with_user_and_password(user, pass)
            .connect(&clean_url)
            .await
            .map_err(QueueError::Connect)?
    } else {
        async_nats::connect(&clean_url)
            .await
            .map_err(QueueError::Connect)?
    };

    let jetstream = jetstream::new(client.clone());
    // Redact password from URL before logging to prevent credential leakage.
    let safe_url = redact_url_password(nats_url);
    info!("Connected to NATS at {}", safe_url);
    Ok((client, jetstream))
}

/// Ensure the MEMORY stream exists with the right subjects.
pub async fn ensure_stream(js: &Context) -> Result<Stream, QueueError> {
    let stream = js
        .get_or_create_stream(jetstream::stream::Config {
            name: STREAM_NAME.to_string(),
            subjects: vec![SUBJECT_WRITE.to_string(), SUBJECT_EXTRACT.to_string()],
            retention: jetstream::stream::RetentionPolicy::WorkQueue,
            max_age: std::time::Duration::from_secs(86400 * 7), // 7 days
            storage: jetstream::stream::StorageType::File,
            ..Default::default()
        })
        .await
        .map_err(QueueError::Stream)?;
    info!("JetStream stream '{}' ready", STREAM_NAME);
    Ok(stream)
}

/// Publish a typed message to a subject.
pub async fn publish<T: Serialize>(
    js: &Context,
    subject: &str,
    payload: &T,
) -> Result<(), QueueError> {
    let bytes = serde_json::to_vec(payload).map_err(QueueError::Serialization)?;
    js.publish(subject.to_string(), bytes.into())
        .await
        .map_err(QueueError::Publish)?
        .await
        .map_err(QueueError::Ack)?;
    Ok(())
}

/// Create a pull-based consumer for a subject.
pub async fn create_consumer(
    js: &Context,
    stream_name: &str,
    consumer_name: &str,
    filter_subject: &str,
) -> Result<PullConsumer, QueueError> {
    let stream = js
        .get_stream(stream_name)
        .await
        .map_err(|e| QueueError::GetStream(e.to_string()))?;
    let consumer = stream
        .get_or_create_consumer(
            consumer_name,
            jetstream::consumer::pull::Config {
                durable_name: Some(consumer_name.to_string()),
                filter_subject: filter_subject.to_string(),
                ack_policy: jetstream::consumer::AckPolicy::Explicit,
                max_deliver: 3,
                ack_wait: std::time::Duration::from_secs(60),
                ..Default::default()
            },
        )
        .await
        .map_err(QueueError::Consumer)?;
    info!(
        "Consumer '{}' ready on subject '{}'",
        consumer_name, filter_subject
    );
    Ok(consumer)
}

/// Deserialize a message payload into a typed struct.
pub fn deserialize_message<T: DeserializeOwned>(data: &[u8]) -> Result<T, QueueError> {
    serde_json::from_slice(data).map_err(QueueError::Serialization)
}

#[derive(Debug, thiserror::Error)]
pub enum QueueError {
    #[error("NATS connection error: {0}")]
    Connect(#[source] async_nats::ConnectError),
    #[error("Stream create error: {0}")]
    Stream(#[source] async_nats::jetstream::context::CreateStreamError),
    #[error("Stream get error: {0}")]
    GetStream(String),
    #[error("Consumer error: {0}")]
    Consumer(#[source] async_nats::jetstream::stream::ConsumerError),
    #[error("Publish error: {0}")]
    Publish(#[source] async_nats::jetstream::context::PublishError),
    #[error("Ack error: {0}")]
    Ack(#[source] async_nats::error::Error<async_nats::jetstream::context::PublishErrorKind>),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants() {
        assert_eq!(STREAM_NAME, "MEMORY");
        assert_eq!(SUBJECT_WRITE, "memory.write");
        assert_eq!(SUBJECT_EXTRACT, "memory.extract");
    }

    #[test]
    fn test_deserialize_valid_json() {
        let data =
            br#"{"content":"test","kind":"fact","confidence":0.9,"importance":0.7,"tags":[]}"#;
        let result: Result<memory_common::ExtractedMemory, _> = deserialize_message(data);
        assert!(result.is_ok());
        let mem = result.unwrap();
        assert_eq!(mem.content, "test");
        assert_eq!(mem.kind, "fact");
    }

    #[test]
    fn test_deserialize_invalid_json() {
        let data = b"not json";
        let result: Result<memory_common::ExtractedMemory, _> = deserialize_message(data);
        assert!(result.is_err());
    }

    #[test]
    fn test_queue_error_display() {
        let err = QueueError::GetStream("test error".to_string());
        assert!(err.to_string().contains("test error"));
    }
}
