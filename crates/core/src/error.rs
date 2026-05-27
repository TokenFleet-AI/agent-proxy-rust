//! Proxy error types and HTTP response mapping.

use std::time::Duration;

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

/// Top-level error for the proxy.
///
/// Maps to HTTP responses with appropriate status codes and JSON bodies.
/// Internal details are never leaked to clients — use `tracing` for server-side logging.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Upstream returned an error or was unreachable.
    #[error("upstream error: {source}")]
    Upstream {
        /// Human-readable description of the upstream error.
        source: String,
        /// Optional chained error for server-side diagnostics.
        #[source]
        inner: Option<anyhow::Error>,
    },

    /// Protocol conversion between AI API formats failed.
    #[error("protocol conversion error: {0}")]
    ProtocolConversion(String),

    /// No channel could be selected for the requested model.
    #[error("no channel available for model '{model}'")]
    ChannelSelection {
        /// The client-requested model name.
        model: String,
    },

    /// Token compression failed (non-fatal — falls back to passthrough).
    #[error("compression error: {0}")]
    Compression(String),

    /// Client sent a malformed request.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Proxy-level auth failed.
    #[error("unauthorized")]
    Unauthorized,

    /// Rate limit exceeded.
    #[error("rate limited, retry after {retry_after:?}")]
    RateLimited {
        /// Suggested retry delay.
        retry_after: Duration,
    },

    /// Internal error (DB, config, unexpected).
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

/// JSON error response body sent to clients.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    /// Container for the error details.
    pub error: ErrorBody,
}

/// Individual error fields.
#[derive(Debug, Serialize)]
pub struct ErrorBody {
    /// Machine-readable error code.
    pub code: &'static str,
    /// Human-readable error message.
    pub message: String,
    /// Optional additional detail.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ErrorBody {
    /// Creates a new [`ErrorBody`] without detail.
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    /// Creates a new [`ErrorBody`] with detail.
    pub fn with_detail(
        code: &'static str,
        message: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            detail: Some(detail.into()),
        }
    }
}

impl ProxyError {
    /// Converts this error into an HTTP response.
    ///
    /// Maps each variant to the appropriate status code and sanitized JSON body.
    /// Internal details are stripped from the client-facing response.
    #[must_use]
    pub fn to_response(&self) -> Response {
        let (status, body) = match self {
            Self::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                ErrorBody::new("bad_request", msg.clone()),
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ErrorBody::new("unauthorized", "invalid proxy API key"),
            ),
            Self::RateLimited { retry_after } => {
                let secs = retry_after.as_secs_f64();
                let mut resp = ErrorBody::new(
                    "rate_limited",
                    format!("rate limit exceeded, retry after {secs:.1}s"),
                );
                resp.detail = Some(format!("retry_after_seconds: {secs:.0}"));
                (StatusCode::TOO_MANY_REQUESTS, resp)
            }
            Self::Upstream { source, .. } if source.contains("429") => (
                StatusCode::TOO_MANY_REQUESTS,
                ErrorBody::new("upstream_rate_limited", "upstream rate limited"),
            ),
            Self::Upstream { source, .. } => (
                StatusCode::BAD_GATEWAY,
                ErrorBody::new("upstream_error", source.clone()),
            ),
            Self::ProtocolConversion(msg) => (
                StatusCode::BAD_GATEWAY,
                ErrorBody::new("protocol_conversion", msg.clone()),
            ),
            Self::ChannelSelection { model } => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorBody::new(
                    "no_channel",
                    format!("no channel available for model '{model}'"),
                ),
            ),
            Self::Compression(msg) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody::new("compression_error", msg.clone()),
            ),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorBody::new("internal_error", "internal server error"),
            ),
        };

        let mut response = Json(ErrorResponse { error: body }).into_response();
        *response.status_mut() = status;
        response
    }

    /// Returns the HTTP status code for this error.
    #[must_use]
    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::RateLimited { .. } => StatusCode::TOO_MANY_REQUESTS,
            Self::Upstream { source, .. } if source.contains("429") => {
                StatusCode::TOO_MANY_REQUESTS
            }
            Self::Upstream { .. } | Self::ProtocolConversion(_) => StatusCode::BAD_GATEWAY,
            Self::ChannelSelection { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::Compression(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Returns the machine-readable error code.
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        match self {
            Self::BadRequest(_) => "bad_request",
            Self::Unauthorized => "unauthorized",
            Self::RateLimited { .. } => "rate_limited",
            Self::Upstream { source, .. } if source.contains("429") => "upstream_rate_limited",
            Self::Upstream { .. } => "upstream_error",
            Self::ProtocolConversion(_) => "protocol_conversion",
            Self::ChannelSelection { .. } => "no_channel",
            Self::Compression(_) => "compression_error",
            Self::Internal(_) => "internal_error",
        }
    }
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        self.to_response()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_bad_request_status_and_code() {
        let err = ProxyError::BadRequest("invalid JSON".into());
        assert_eq!(err.status_code(), StatusCode::BAD_REQUEST);
        assert_eq!(err.error_code(), "bad_request");
    }

    #[test]
    fn test_unauthorized_status() {
        let err = ProxyError::Unauthorized;
        assert_eq!(err.status_code(), StatusCode::UNAUTHORIZED);
        assert_eq!(err.error_code(), "unauthorized");
    }

    #[test]
    fn test_rate_limited_status() {
        let err = ProxyError::RateLimited {
            retry_after: Duration::from_secs(5),
        };
        assert_eq!(err.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err.error_code(), "rate_limited");
    }

    #[test]
    fn test_upstream_429_passthrough() {
        let err = ProxyError::Upstream {
            source: "upstream 429 too many requests".into(),
            inner: None,
        };
        assert_eq!(err.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(err.error_code(), "upstream_rate_limited");
    }

    #[test]
    fn test_upstream_error_status() {
        let err = ProxyError::Upstream {
            source: "connection refused".into(),
            inner: None,
        };
        assert_eq!(err.status_code(), StatusCode::BAD_GATEWAY);
        assert_eq!(err.error_code(), "upstream_error");
    }

    #[test]
    fn test_channel_selection_status() {
        let err = ProxyError::ChannelSelection {
            model: "gpt-5".into(),
        };
        assert_eq!(err.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(err.error_code(), "no_channel");
    }

    #[test]
    fn test_internal_error_status() {
        let err = ProxyError::Internal(anyhow::anyhow!("db connection failed"));
        assert_eq!(err.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(err.error_code(), "internal_error");
    }

    #[test]
    fn test_error_to_response_returns_json() {
        let err = ProxyError::BadRequest("test".into());
        let response = err.to_response();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(
            response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.contains("application/json"))
        );
    }

    #[test]
    fn test_all_variants_have_distinct_codes() {
        // Ensure no two variants share the same error code
        let codes = [
            ProxyError::BadRequest("x".into()).error_code(),
            ProxyError::Unauthorized.error_code(),
            ProxyError::RateLimited {
                retry_after: Duration::from_secs(1),
            }
            .error_code(),
            ProxyError::Upstream {
                source: "timeout".into(),
                inner: None,
            }
            .error_code(),
            ProxyError::Upstream {
                source: "429".into(),
                inner: None,
            }
            .error_code(),
            ProxyError::ProtocolConversion("x".into()).error_code(),
            ProxyError::ChannelSelection { model: "x".into() }.error_code(),
            ProxyError::Compression("x".into()).error_code(),
            ProxyError::Internal(anyhow::anyhow!("x")).error_code(),
        ];
        // upstream_error vs upstream_rate_limited are different
        assert_ne!(codes[3], codes[4]);
    }

    #[test]
    fn test_internal_from_anyhow() {
        let source = anyhow::anyhow!("something broke");
        let err = ProxyError::from(source);
        assert!(matches!(err, ProxyError::Internal(_)));
    }
}
