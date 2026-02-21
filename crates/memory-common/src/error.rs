//! Error types for the memory platform.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum MemoryError {
    #[error("Database error: {0}")]
    Database(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Rate limit exceeded")]
    RateLimited,

    #[error("Plan limit exceeded: {0}")]
    PlanLimitExceeded(String),

    #[error("Queue error: {0}")]
    Queue(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("{service} service error: {message}")]
    DownstreamServiceError { service: String, message: String },
}

impl From<serde_json::Error> for MemoryError {
    fn from(e: serde_json::Error) -> Self {
        Self::Serialization(e.to_string())
    }
}

// Axum integration: convert MemoryError into HTTP responses
impl axum::response::IntoResponse for MemoryError {
    fn into_response(self) -> axum::response::Response {
        use axum::http::StatusCode;
        let (status, message) = match &self {
            Self::NotFound(msg) => (StatusCode::NOT_FOUND, msg.clone()),
            Self::Validation(msg) => (StatusCode::BAD_REQUEST, msg.clone()),
            Self::Auth(msg) => (StatusCode::UNAUTHORIZED, msg.clone()),
            Self::Forbidden(msg) => (StatusCode::FORBIDDEN, msg.clone()),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded".into()),
            Self::PlanLimitExceeded(msg) => (StatusCode::PAYMENT_REQUIRED, msg.clone()),
            Self::ServiceUnavailable(msg) => (StatusCode::SERVICE_UNAVAILABLE, msg.clone()),
            Self::DownstreamServiceError { service, message } => {
                // Log the real error with service details for debugging, but return
                // a generic message to clients to avoid leaking internal architecture.
                tracing::error!("Downstream {} service error: {}", service, message);
                (
                    StatusCode::BAD_GATEWAY,
                    "A downstream service error occurred".to_string(),
                )
            }
            _ => {
                // Log the real error for debugging, but return a generic message to clients
                // to prevent leaking internal details (DB schema, query fragments, etc.).
                tracing::error!("Internal error: {}", self);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "An internal error occurred".to_string(),
                )
            }
        };
        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = MemoryError::NotFound("Memory 123".into());
        assert_eq!(err.to_string(), "Not found: Memory 123");
    }

    #[test]
    fn test_error_from_serde() {
        let bad_json = "not json";
        let result: Result<serde_json::Value, _> = serde_json::from_str(bad_json);
        let err: MemoryError = result.unwrap_err().into();
        assert!(matches!(err, MemoryError::Serialization(_)));
    }

    #[test]
    fn test_error_into_response() {
        use axum::response::IntoResponse;
        let err = MemoryError::RateLimited;
        let response = err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::TOO_MANY_REQUESTS);
    }

    #[test]
    fn test_error_variants() {
        assert!(matches!(
            MemoryError::Database("db".into()),
            MemoryError::Database(_)
        ));
        assert!(matches!(
            MemoryError::Auth("auth".into()),
            MemoryError::Auth(_)
        ));
        assert!(matches!(
            MemoryError::Forbidden("forbidden".into()),
            MemoryError::Forbidden(_)
        ));
        assert!(matches!(
            MemoryError::Validation("bad".into()),
            MemoryError::Validation(_)
        ));
        assert!(matches!(MemoryError::RateLimited, MemoryError::RateLimited));
        assert!(matches!(
            MemoryError::PlanLimitExceeded("limit".into()),
            MemoryError::PlanLimitExceeded(_)
        ));
        assert!(matches!(
            MemoryError::Queue("q".into()),
            MemoryError::Queue(_)
        ));
        assert!(matches!(
            MemoryError::Llm("llm".into()),
            MemoryError::Llm(_)
        ));
        assert!(matches!(
            MemoryError::Cache("cache".into()),
            MemoryError::Cache(_)
        ));
        assert!(matches!(
            MemoryError::Internal("internal".into()),
            MemoryError::Internal(_)
        ));
        assert!(matches!(
            MemoryError::ServiceUnavailable("unavailable".into()),
            MemoryError::ServiceUnavailable(_)
        ));
        assert!(matches!(
            MemoryError::DownstreamServiceError {
                service: "write".into(),
                message: "timeout".into()
            },
            MemoryError::DownstreamServiceError { .. }
        ));
    }

    #[test]
    fn test_service_unavailable_response() {
        use axum::response::IntoResponse;
        let err = MemoryError::ServiceUnavailable("Redis down".into());
        let response = err.into_response();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn test_downstream_service_error_response() {
        use axum::response::IntoResponse;
        let err = MemoryError::DownstreamServiceError {
            service: "write".into(),
            message: "connection refused".into(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn test_downstream_service_error_display() {
        let err = MemoryError::DownstreamServiceError {
            service: "retrieve".into(),
            message: "timeout".into(),
        };
        assert_eq!(err.to_string(), "retrieve service error: timeout");
    }
}
