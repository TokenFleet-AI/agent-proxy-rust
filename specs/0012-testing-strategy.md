# 0012 — Testing Strategy

## Overview

Testing layers from fast (unit) to slow (E2E). Every layer must pass in CI before merge.

## Test Layers

| Layer | Location | Framework | Speed | Coverage Target |
|-------|----------|-----------|-------|-----------------|
| Unit tests | `src/` `#[cfg(test)]` | `cargo test` | <1s | 80% line |
| Doc tests | `src/` doc comments | `cargo test` | <1s | Public API |
| Integration | `tests/` | `cargo test` + wiremock | <5s | Critical paths |
| E2E | `tests/e2e/` | Custom harness | <60s | Main flow |
| Property | `src/` `#[cfg(test)]` | `proptest` | <5s | Invariants |
| Fuzz | `fuzz/` | `cargo fuzz` | CI nightly | Parsing |

## Unit Tests

Standard pattern — test modules alongside source:

```rust
// crates/model-router/src/selection.rs
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_select_subscription_with_quota() {
        // ...
    }

    #[test]
    fn test_should_fallback_to_metered_when_quota_exhausted() {
        // ...
    }
}
```

Naming: `test_should_<expected_behavior>`. Error cases use `matches!`:

```rust
#[test]
fn test_should_reject_invalid_channel_url() {
    let result = validate_channel_url("http://10.0.0.1/api");
    assert!(matches!(result, Err(ref e) if e.to_string().contains("banned address")));
}
```

## Parameterized Tests (`rstest`)

For testing selection strategy with many channel combinations:

```rust
#[rstest]
#[case("claude-sonnet", true,  ChannelHealth::Healthy,   Some("anthropic-official"))]
#[case("claude-sonnet", false, ChannelHealth::Healthy,   Some("openrouter"))]
#[case("claude-sonnet", false, ChannelHealth::Unhealthy, None)]
fn test_should_select_channel(
    #[case] model: &str,
    #[case] subscription_available: bool,
    #[case] health: ChannelHealth,
    #[case] expected: Option<&str>,
) {
    let channels = build_test_channels(subscription_available, health);
    let result = select_channel(model, &channels);
    assert_eq!(result.map(|c| c.name.as_str()), expected);
}
```

## Property Tests (`proptest`)

For invariants that should hold for any input:

```rust
proptest! {
    #[test]
    fn test_weighted_random_never_panics(weights in prop::collection::vec(0u32..100, 0..20)) {
        let candidates: Vec<ModelMapping> = weights.iter().map(|w| {
            ModelMapping { weight: *w, ..Default::default() }
        }).collect();
        let candidates_refs: Vec<&ModelMapping> = candidates.iter().collect();
        let result = weighted_random(&candidates_refs);
        // Must never panic, even with all-zero weights
        if weights.iter().sum::<u32>() == 0 {
            assert!(result.is_none());
        } else {
            assert!(result.is_some());
        }
    }

    #[test]
    fn test_cost_calculation_never_negative(usage in arb_usage()) {
        let cost = calc_cost(&usage, &test_metered_pricing());
        assert!(cost >= 0.0);
    }
}
```

## Integration Tests

Use `wiremock` to simulate upstream APIs:

```rust
// tests/integration/test_proxy_flow.rs
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

#[tokio::test]
async fn test_should_proxy_anthropic_request() {
    // Start mock upstream
    let upstream = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_json(json!({
                "id": "msg_001",
                "content": [{"type": "text", "text": "Hello"}],
                "usage": {"input_tokens": 100, "output_tokens": 50}
            })))
        .mount(&upstream)
        .await;

    // Configure proxy with mock upstream as channel
    let proxy = AgentProxy::builder()
        .middleware(ModelRouterMiddleware::new(test_channel(upstream.uri())))
        .middleware(BridgeMiddleware::new())
        .build().unwrap();

    // Send request
    let client = reqwest::Client::new();
    let resp = client.post("http://127.0.0.1:8787/v1/messages")
        .json(&anthropic_request())
        .send().await.unwrap();

    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_should_retry_non_streaming_on_timeout() {
    // ...
}

#[tokio::test]
async fn test_should_map_upstream_429_to_client() {
    // ...
}
```

### Test Fixtures

Shared test data in `tests/fixtures/`:

```
tests/fixtures/
├── anthropic_request.json
├── anthropic_response.json
├── openai_request.json
├── openai_response.json
├── channels.yaml
├── streaming_sse.txt
└── large_tools_array.json
```

## E2E Tests

E2E tests start a real proxy instance and send actual HTTP requests:

```rust
// tests/e2e/flow.rs
#[tokio::test]
async fn test_should_compress_route_bridge_and_track_cost() {
    let upstream = MockServer::start().await;
    // ... mount mocks ...

    let proxy = start_proxy_instance(&config_with_channel(upstream.uri())).await;

    let client = reqwest::Client::new();
    let resp = client.post(format!("http://{}/v1/messages", proxy.addr()))
        .json(&request_with_tools())
        .send().await.unwrap();

    assert_eq!(resp.status(), 200);

    // Verify cost was recorded
    let db = open_cost_db(&proxy.data_dir());
    let records = db.query_cost_records().unwrap();
    assert!(!records.is_empty());
    assert!(records[0].compression_tokens_saved > 0);
}
```

## Fuzz Testing

Target parsers and protocol converters:

```rust
// fuzz/fuzz_targets/parse_request.rs
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Parsing must never panic
    let _ = serde_json::from_slice::<AnthropicRequest>(data);
});
```

```rust
// fuzz/fuzz_targets/bridge_convert.rs
fuzz_target!(|data: &[u8]| {
    // Bridge conversion must never panic on arbitrary input
    if let Ok(req) = serde_json::from_slice::<AnthropicRequest>(data) {
        let _ = bridge::anthropic_to_openai(&req);
    }
});
```

## CI Matrix

```yaml
# .github/workflows/test.yml
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
        rust: [stable]
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --workspace --all-features
      - run: cargo test --workspace --doc
      - run: cargo clippy --all-targets --all-features -- -D warnings -W clippy::pedantic
      - run: cargo fmt --check

  audit:
    runs-on: ubuntu-latest
    steps:
      - run: cargo audit
      - run: cargo deny check

  fuzz:
    runs-on: ubuntu-latest
    if: github.event_name == 'schedule'  # nightly only
    steps:
      - run: cargo fuzz run parse_request -- -max_total_time=300
      - run: cargo fuzz run bridge_convert -- -max_total_time=300
```

## Test Helpers

Shared test utilities in `crates/core/src/testing.rs` (gated behind `#[cfg(test)]` or a `testing` feature):

```rust
/// Build a minimal ProxyRequest for tests.
pub fn test_request(body: Value) -> ProxyRequest { ... }

/// Build test channels with controlled health states.
pub fn test_channels(specs: &[ChannelSpec]) -> Vec<Channel> { ... }
```
