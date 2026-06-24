//! Exponential backoff retry middleware.
//!
//! Retries upstream requests on transient failures (5xx, 429, connection errors)
//! with increasing delays: 100ms → 200ms → 400ms → ... up to `max_retries`.

use std::time::Duration;

use agent_proxy_rust_core::{
    ProxyError,
    middleware::ProxyMiddleware,
    types::{ConnectionContext, ProxyRequest, ProxyResponse},
};
use async_trait::async_trait;
use http::StatusCode;

/// Configuration for the retry middleware.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts (not including the original request).
    pub max_retries: u32,
    /// Base delay for the first retry.
    pub base_delay: Duration,
    /// Maximum delay cap.
    pub max_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
        }
    }
}

/// Determines whether an error is retryable.
#[must_use]
pub fn is_retryable(status: StatusCode) -> bool {
    status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS
}

/// Computes the delay for the nth retry (0-indexed) using exponential backoff.
#[must_use]
pub fn backoff_delay(attempt: u32, config: &RetryConfig) -> Duration {
    let delay = config.base_delay * 2u32.pow(attempt);
    delay.min(config.max_delay)
}

/// Retry middleware — marks the context so the proxy engine can retry.
///
/// This middleware runs in `on_response` and checks whether the upstream
/// returned a retryable status code. If so, it sets a flag in the context
/// that the proxy engine uses to decide whether to resend the request.
#[derive(Debug, Default)]
pub struct RetryMiddleware {
    config: RetryConfig,
}

impl RetryMiddleware {
    /// Creates a new retry middleware.
    #[must_use]
    pub fn new(config: RetryConfig) -> Self {
        Self { config }
    }
}

/// Context key set by the retry middleware when the response is retryable.
pub const EXT_RETRYABLE: &str = "retryable";
/// Context key for the suggested retry delay.
pub const EXT_RETRY_DELAY: &str = "retry_delay_ms";

#[async_trait]
impl ProxyMiddleware for RetryMiddleware {
    async fn on_request(
        &self,
        _req: &mut ProxyRequest,
        _ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError> {
        Ok(())
    }

    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        ctx: &ConnectionContext,
    ) -> Result<(), ProxyError> {
        if is_retryable(res.status) {
            // Read current retry count from context
            let attempt = ctx.get::<u32>(EXT_RETRYABLE).copied().unwrap_or(0);

            if attempt < self.config.max_retries {
                let delay = backoff_delay(attempt, &self.config);
                // Return a RateLimited error to signal the proxy engine to retry
                return Err(ProxyError::RateLimited { retry_after: delay });
            }
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "retry"
    }
}

#[cfg(test)]
#[allow(
    unknown_lints,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::duration_suboptimal_units
)]
mod tests {
    use super::*;

    #[test]
    fn test_is_retryable_server_errors() {
        assert!(is_retryable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable(StatusCode::BAD_GATEWAY));
        assert!(is_retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(is_retryable(StatusCode::GATEWAY_TIMEOUT));
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
    }

    #[test]
    fn test_is_not_retryable_client_errors() {
        assert!(!is_retryable(StatusCode::BAD_REQUEST));
        assert!(!is_retryable(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable(StatusCode::FORBIDDEN));
        assert!(!is_retryable(StatusCode::NOT_FOUND));
    }

    #[test]
    fn test_is_not_retryable_success() {
        assert!(!is_retryable(StatusCode::OK));
        assert!(!is_retryable(StatusCode::CREATED));
    }

    #[test]
    fn test_backoff_delays() {
        let config = RetryConfig::default();
        assert_eq!(backoff_delay(0, &config), Duration::from_millis(100));
        assert_eq!(backoff_delay(1, &config), Duration::from_millis(200));
        assert_eq!(backoff_delay(2, &config), Duration::from_millis(400));
        assert_eq!(backoff_delay(3, &config), Duration::from_millis(800));
    }

    #[test]
    fn test_backoff_capped_at_max() {
        let config = RetryConfig {
            max_delay: Duration::from_secs(1),
            ..Default::default()
        };
        // 2^6 * 100ms = 6400ms, capped at 1000ms
        assert_eq!(backoff_delay(6, &config), Duration::from_millis(1000));
    }
}
