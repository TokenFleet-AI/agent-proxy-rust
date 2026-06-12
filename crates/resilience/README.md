# agent-proxy-rust-resilience

Resilience middlewares: rate limiter, retry, circuit breaker.

## 功能

### Rate Limiter（令牌桶限流）

- 每个通道独立的令牌桶（`TokenBucket`）
- 令牌按 `max_requests / window` 速率补充，上限为 `max_requests`
- 每次请求消耗一个令牌；令牌耗尽时返回 `ProxyError::RateLimited`
- 配置：`RateLimitConfig { max_requests, window }`

### Retry（指数退避重试）

- 在 `on_response` 中检测可重试状态码（5xx、429）
- 指数退避延迟：`base_delay * 2^attempt`，上限 `max_delay`
- 默认配置：3 次重试、100ms 基础延迟、5s 最大延迟
- 在上下文中设置 `EXT_RETRYABLE` 和 `EXT_RETRY_DELAY` 标记，由代理引擎执行重试
- 配置：`RetryConfig { max_retries, base_delay, max_delay }`

### Circuit Breaker（熔断器）

- 三态熔断器：Closed → Open → Half-Open
- `failure_threshold` 次连续失败后熔断（默认 3 次）
- 熔断后 `cooldown` 期间直接拒绝请求（默认 60 秒）
- 冷却期后进入 Half-Open，允许一次探测请求
  - 探测成功 → Closed
  - 探测失败 → 重新 Open
- 配置：`CircuitBreakerConfig { failure_threshold, cooldown }`

## 关键类型

- `RateLimiterMiddleware` / `RateLimitConfig` / `TokenBucket` — 令牌桶限流
- `RetryMiddleware` / `RetryConfig` — 指数退避重试
- `CircuitBreakerMiddleware` / `CircuitBreakerConfig` — 三态熔断器
- `is_retryable()` — 判断状态码是否可重试（5xx 或 429）
- `backoff_delay()` — 计算第 N 次重试的退避延迟

## 使用示例

```rust
use std::time::Duration;
use agent_proxy_rust_resilience::{
    RateLimiterMiddleware, RateLimitConfig,
    RetryMiddleware, RetryConfig,
    CircuitBreakerMiddleware, CircuitBreakerConfig,
};

// 每个通道每分钟最多 60 次请求
let rate_limiter = RateLimiterMiddleware::new(
    RateLimitConfig {
        max_requests: 60,
        window: Duration::from_secs(60),
    }
);

// 最多重试 3 次，指数退避
let retry = RetryMiddleware::new(RetryConfig::default());

// 3 次失败后熔断，60 秒冷却
let breaker = CircuitBreakerMiddleware::new(CircuitBreakerConfig::default());
```

## 依赖

本 crate 依赖：
- `agent-proxy-rust-core` — `ProxyMiddleware` trait、`ProxyError`、`ChannelConfig`、扩展键

## 相关文档

- [限流设计](../../specs/0013-rate-limiting.md)
- [健康状态机设计](../../specs/0015-health-state-machine.md)

---

Part of the [agent-proxy-rust](https://github.com/TokenFleet-AI/agent-proxy-rust) workspace.
