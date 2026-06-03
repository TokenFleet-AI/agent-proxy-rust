//! Resilience middlewares for agent-proxy-rust.
//!
//! Provides three middleware implementations for building a robust proxy:
//!
//! - [`RateLimiterMiddleware`] — Token bucket per-channel rate limiting
//! - [`RetryMiddleware`] — Exponential backoff retry on transient failures
//! - [`CircuitBreakerMiddleware`] — Three-state circuit breaker per channel

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

pub mod circuit_breaker;
pub mod rate_limiter;
pub mod retry;

pub use circuit_breaker::{CircuitBreakerConfig, CircuitBreakerMiddleware};
pub use rate_limiter::{RateLimitConfig, RateLimiterMiddleware};
pub use retry::{RetryConfig, RetryMiddleware};
