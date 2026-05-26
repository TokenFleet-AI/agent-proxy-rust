# 0013 — Rate Limiting (Cloud Extension)

> **Status**: Optional extension — NOT part of the core local proxy.
> The core project targets single-user desktop use. Rate limiting is only relevant for cloud/multi-tenant deployments.
> Implementation should be a separate crate (`agent-proxy-rate-limit`) or standalone sidecar, not bundled in the default binary.

## Overview

Rate limiting operates at three levels: client (IP), project, and channel. Each level has independent token buckets. When any bucket is exhausted, the request is rejected with 429.

## Architecture

```
Request
  │
  ├── 1. Client Rate Limit (by IP)
  │      └── Exceeded? → 429 + Retry-After
  │
  ├── 2. Project Rate Limit (by project_path)
  │      └── Exceeded? → 429 + Retry-After
  │
  ├── 3. Channel Rate Limit (by channel + model)
  │      └── Exceeded? → 429 + Retry-After (or select different channel)
  │
  ▼
  Continue to middleware chain
```

## Client Rate Limit

Per-IP token bucket. Prevents a single client from overwhelming the proxy:

```rust
struct ClientRateLimiter {
    /// Token bucket per client IP.
    buckets: DashMap<IpAddr, TokenBucket>,
    config: ClientRateLimitConfig,
}

struct ClientRateLimitConfig {
    /// Requests per second allowed per client.
    requests_per_second: u32,  // default: 50
    /// Burst capacity.
    burst_size: u32,  // default: 100
}
```

Client identity is determined by (in priority order):
1. `X-Forwarded-For` header (if trusted proxy is configured)
2. Direct connection IP

## Project Rate Limit

Per-project token bucket. Prevents any single project from consuming disproportionate resources:

```rust
struct ProjectRateLimiter {
    buckets: LruCache<String, TokenBucket>,  // project_path → bucket
    config: ProjectRateLimitConfig,
}

struct ProjectRateLimitConfig {
    /// Token refill rate per second per project.
    tokens_per_second: u32,  // default: 500
    /// Burst capacity.
    burst_size: u32,  // default: 2000
}
```

Project identity comes from the project detection chain (see 0004 §Project Detection).

The project limiter is scoped to token count rather than request count — different models have vastly different token consumption per request.

## Channel Rate Limit

Per-channel + per-model rate limit. Prevents exhausting a single upstream API's rate limit:

```rust
struct ChannelRateLimiter {
    /// Key: (channel_id, model_name) → bucket
    buckets: DashMap<(String, String), TokenBucket>,
    config: ChannelRateLimitConfig,
}

struct ChannelRateLimitConfig {
    /// Per-channel RPM (requests per minute) — provider-specific.
    default_rpm: u32,  // default: 100
    /// Per-channel TPM (tokens per minute) — provider-specific.
    default_tpm: u32,  // default: 1_000_000
}
```

Channel rate limits are also defined per-channel in the configuration:

```yaml
rate_limit:
  client:
    requests_per_second: 50
    burst_size: 100
  project:
    tokens_per_second: 500
    burst_size: 2000
  channels:
    anthropic-official:
      rpm: 100
      tpm: 1000000
    openrouter:
      rpm: 200
      tpm: 2000000
```

## Token Bucket Implementation

Uses a simple token bucket algorithm:

```rust
struct TokenBucket {
    /// Current token count.
    tokens: AtomicU32,
    /// Maximum tokens (burst capacity).
    max_tokens: u32,
    /// Token refill rate per second.
    refill_rate: u32,
    /// Last refill timestamp (milliseconds since epoch).
    last_refill: AtomicU64,
}

impl TokenBucket {
    /// Try to consume `n` tokens. Returns true if allowed, false if exceeded.
    fn try_consume(&self, n: u32, now_ms: u64) -> bool {
        // Refill tokens based on elapsed time
        let elapsed = now_ms.saturating_sub(self.last_refill.load(Ordering::Relaxed));
        let refill = (elapsed as u64 * self.refill_rate as u64) / 1000;
        if refill > 0 {
            let new_tokens = self.tokens.load(Ordering::Relaxed)
                .saturating_add(refill as u32)
                .min(self.max_tokens);
            self.tokens.store(new_tokens, Ordering::Relaxed);
            self.last_refill.store(now_ms, Ordering::Relaxed);
        }

        let current = self.tokens.load(Ordering::Relaxed);
        if current >= n {
            self.tokens.store(current - n, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}
```

Uses `AtomicU32` and `AtomicU64` for lock-free operation. Precision is acceptable for rate limiting (not billing).

## 429 Response

When a rate limit is hit, the response includes:

```http
HTTP/1.1 429 Too Many Requests
Retry-After: 2
Content-Type: application/json

{
  "error": {
    "code": "rate_limited",
    "message": "project rate limit exceeded: 500 tokens/sec",
    "detail": {
      "limit_type": "project",
      "retry_after_seconds": 2,
      "current_tokens_per_second": 523,
      "limit_tokens_per_second": 500
    }
  }
}
```

The `detail` field helps the client (or developer) understand which limit was hit and how close they are.

## Priority / Fairness

When multiple projects share limited channel capacity, use weighted fair queuing to prevent any one project from starving others:

```rust
struct FairChannelQueue {
    /// Per-project token allocation fraction.
    /// Total should sum to 1.0. Unlisted projects share the remainder equally.
    allocations: HashMap<String, f64>,  // project_path → fraction
    default_fraction: f64,  // for unlisted projects
}
```

This is a P2 feature — initial implementation can use simple first-come-first-served with the hard limits above.

## Rate Limit Metrics

`GET /metrics` includes rate limit counters:

```prometheus
# HELP agent_proxy_rate_limited_total Total requests rate-limited
# TYPE agent_proxy_rate_limited_total counter
agent_proxy_rate_limited_total{type="client"} 12
agent_proxy_rate_limited_total{type="project"} 3
agent_proxy_rate_limited_total{type="channel"} 45

# HELP agent_proxy_rate_limit_bucket_level Current token bucket fill level (0-1)
# TYPE agent_proxy_rate_limit_bucket_level gauge
agent_proxy_rate_limit_bucket_level{type="client",client="192.168.1.1"} 0.73
agent_proxy_rate_limit_bucket_level{type="channel",channel="anthropic-official"} 0.15
```

## Backpressure Integration

When a channel rate limit is approaching exhaustion (bucket < 20%), the channel selection strategy can deprioritize it:

```rust
fn select_channel(model: &str, channels: &[Channel], limiter: &ChannelRateLimiter) -> Option<&Channel> {
    let candidates: Vec<_> = channels.iter()
        .filter(|c| c.is_healthy())
        .collect();

    // Remove channels with nearly-exhausted rate limits (unless only option)
    if candidates.len() > 1 {
        candidates.retain(|c| limiter.bucket_level(c) > 0.2);
    }

    weighted_random(&candidates)
}
```

This provides natural backpressure — heavily-used channels are temporarily excluded from selection, allowing other channels to absorb traffic.

## Configuration

Rate limiting can be disabled entirely for development:

```yaml
rate_limit:
  enabled: false  # Disable all rate limiting (dev only)
```

Or selectively:

```yaml
rate_limit:
  client:
    enabled: false   # Disable client-level only
  project:
    enabled: true
  channels:
    enabled: true
```
