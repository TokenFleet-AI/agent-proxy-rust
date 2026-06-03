//! Token Bucket rate limiter middleware.
//!
//! Each channel gets its own token bucket. Requests consume tokens;
//! when a bucket is empty, the middleware returns an error so the
//! model-router can switch to the next channel.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use agent_proxy_rust_core::{
    ProxyError,
    middleware::ProxyMiddleware,
    types::{ConnectionContext, ProxyRequest, ProxyResponse},
};
use async_trait::async_trait;

/// Configuration for a single channel's rate limit.
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Maximum number of requests allowed per window.
    pub max_requests: u32,
    /// Time window for the rate limit.
    pub window: Duration,
}

/// Token Bucket rate limiter.
///
/// Tokens refill at a rate of `max_requests / window`, up to `max_requests`.
/// Each request consumes one token. When tokens are exhausted, the request
/// is rejected with [`ProxyError::RateLimited`].
#[derive(Debug)]
pub struct TokenBucket {
    /// Maximum tokens the bucket can hold.
    capacity: u32,
    /// Current token count.
    tokens: f64,
    /// Tokens added per second.
    refill_rate: f64,
    /// Last time tokens were refilled.
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates a new token bucket.
    #[must_use]
    pub fn new(max_requests: u32, window: Duration) -> Self {
        let capacity = max_requests;
        let refill_rate = f64::from(max_requests) / window.as_secs_f64();
        Self {
            capacity,
            tokens: f64::from(capacity),
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    /// Attempts to consume a token. Returns `true` if successful.
    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Refills tokens based on elapsed time.
    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        let added = elapsed * self.refill_rate;
        self.tokens = (self.tokens + added).min(f64::from(self.capacity));
        self.last_refill = now;
    }
}

/// Rate limiter middleware — one bucket per channel.
#[derive(Debug)]
pub struct RateLimiterMiddleware {
    buckets: Mutex<HashMap<String, TokenBucket>>,
    default_config: RateLimitConfig,
}

impl RateLimiterMiddleware {
    /// Creates a new rate limiter with a default config for all channels.
    #[must_use]
    pub fn new(default_config: RateLimitConfig) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            default_config,
        }
    }

    fn check_rate_limit(&self, channel_id: &str) -> Result<(), ProxyError> {
        let mut buckets = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bucket = buckets.entry(channel_id.to_string()).or_insert_with(|| {
            TokenBucket::new(self.default_config.max_requests, self.default_config.window)
        });

        if bucket.try_consume() {
            Ok(())
        } else {
            Err(ProxyError::RateLimited {
                retry_after: self.default_config.window,
            })
        }
    }
}

#[async_trait]
impl ProxyMiddleware for RateLimiterMiddleware {
    async fn on_request(
        &self,
        _req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError> {
        let channel_id = ctx
            .get::<agent_proxy_rust_core::types::ChannelConfig>(
                agent_proxy_rust_core::extensions::EXT_SELECTED_CHANNEL,
            )
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        if !channel_id.is_empty() {
            self.check_rate_limit(&channel_id)?;
        }
        Ok(())
    }

    async fn on_response(
        &self,
        _res: &mut ProxyResponse,
        _ctx: &ConnectionContext,
    ) -> Result<(), ProxyError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "rate-limiter"
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_token_bucket_initial_has_tokens() {
        let mut bucket = TokenBucket::new(10, Duration::from_secs(60));
        assert!(bucket.try_consume(), "new bucket should have tokens");
    }

    #[test]
    fn test_token_bucket_exhaustion() {
        let mut bucket = TokenBucket::new(3, Duration::from_secs(60));
        // Consume all 3 tokens
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        assert!(bucket.try_consume());
        // 4th attempt must fail
        assert!(!bucket.try_consume(), "bucket should be exhausted");
    }

    #[test]
    fn test_rate_limiter_rejects_when_limit_exceeded() {
        let limiter = RateLimiterMiddleware::new(RateLimitConfig {
            max_requests: 2,
            window: Duration::from_secs(60),
        });

        // First two requests should pass
        assert!(limiter.check_rate_limit("ch-1").is_ok());
        assert!(limiter.check_rate_limit("ch-1").is_ok());
        // Third must fail
        assert!(
            matches!(
                limiter.check_rate_limit("ch-1"),
                Err(ProxyError::RateLimited { .. })
            ),
            "third request must be rate-limited"
        );
    }

    #[test]
    fn test_different_channels_independent() {
        let limiter = RateLimiterMiddleware::new(RateLimitConfig {
            max_requests: 1,
            window: Duration::from_secs(60),
        });

        // Exhaust ch-1
        assert!(limiter.check_rate_limit("ch-1").is_ok());
        assert!(matches!(
            limiter.check_rate_limit("ch-1"),
            Err(ProxyError::RateLimited { .. })
        ));

        // ch-2 still has tokens
        assert!(limiter.check_rate_limit("ch-2").is_ok());
    }
}
