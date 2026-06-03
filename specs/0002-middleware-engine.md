# 0002 — ProxyMiddleware Trait & Server Engine

> **Phase 1**: Core trait + axum engine + single handle_proxy_request dispatch. `on_init`/`on_shutdown` lifecycle hooks.
> **Phase 2**: Additional middleware implementations (health probe, rate-limit, observability) as separate crates.

## Trait Design

```rust
#[async_trait]
pub trait ProxyMiddleware: Send + Sync {
    async fn on_request(
        &self,
        req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError>;

    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        ctx: &ConnectionContext,
    ) -> Result<(), ProxyError>;

    async fn on_connect(&self, ctx: &ConnectionContext) {}
    async fn on_disconnect(&self, ctx: &ConnectionContext) {}

    fn name(&self) -> &'static str;
}
```

## Core Types

```rust
struct ProxyRequest {
    headers: HeaderMap,
    method: Method,
    path: String,           // /v1/messages, /v1/chat/completions
    body: Bytes,
}

struct ProxyResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Bytes,
    is_streaming: bool,
}

struct ConnectionContext {
    request_id: u64,
    agent_type: AgentType,         // Claude / Codex / Gemini / Unknown
    agent_role: Option<String>,    // Ruflo swarm role (architect/coder/tester/...)
                                   // Set by auth layer from x-api-key → role_mapping.
                                   // None for standalone / non-Ruflo usage.
    detected_format: ApiFormat,    // from path
    started_at: Instant,
    target_protocol: Option<ApiFormat>,  // set by model-router middleware
    extensions: HashMap<String, Box<dyn Any + Send + Sync>>,
}

enum AgentType {
    Claude,
    Codex,
    Gemini,
    OpenCode,
    OpenClaw,
    Hermes,
    Unknown,
}
```

## Execution Order

```
on_connect:  [A, B, C]  →  registration order
on_request:  [A, B, C]  →  registration order
  → forward to upstream →
on_response: [C, B, A]  →  REVERSE order
on_disconnect: [C, B, A]  →  REVERSE order
```

Reverse on_response because the last middleware to touch the request should be the first to see the response (symmetry).

## Inter-Middleware Communication

Via `ConnectionContext.extensions`:

```rust
// CompressMiddleware writes:
ctx.extensions.insert("stats_record", CompressionStats { ... });

// CostMiddleware reads:
let record = ctx.extensions.get::<CompressionStats>("stats_record");

// ModelRouterMiddleware writes:
ctx.extensions.insert("selected_channel", channel);
ctx.extensions.insert("selected_mapping", mapping);
ctx.target_protocol = Some(channel.protocol);

// BridgeMiddleware reads:
if let Some(target) = ctx.target_protocol {
    // convert req body to target format
}
```

Key: `extensions` is typed via `Any`. Middlewares must agree on the key names and types. This is a convention, not enforced by the compiler.

To mitigate type-safety risk, all extension keys are defined as `&'static str` constants in the `core` crate:

```rust
// core/src/extensions.rs
pub const EXT_STATS_RECORD: &str = "stats_record";
pub const EXT_SELECTED_CHANNEL: &str = "selected_channel";
pub const EXT_SELECTED_MAPPING: &str = "selected_mapping";
```

Tests in each middleware crate verify that get/insert use matching types for each key.

## Server Engine

Built on axum:

```rust
pub struct AgentProxy {
    config: ProxyConfig,
    middlewares: Arc<Vec<Box<dyn ProxyMiddleware>>>,
}

impl AgentProxy {
    pub fn builder() -> AgentProxyBuilder;

    pub async fn serve(self) -> Result<JoinHandle<()>>;
}

pub struct AgentProxyBuilder {
    config: Option<ProxyConfig>,
    middlewares: Vec<Box<dyn ProxyMiddleware>>,
}

impl AgentProxyBuilder {
    pub fn config(mut self, config: ProxyConfig) -> Self;
    pub fn middleware<M: ProxyMiddleware + 'static>(mut self, m: M) -> Self;
    pub fn build(self) -> Result<AgentProxy>;
}
```

### Router

```rust
fn build_router(state: Arc<ProxyState>) -> Router {
    Router::new()
        .route("/v1/messages", post(handle_proxy_request))
        .route("/v1/chat/completions", post(handle_proxy_request))
        .route("/v1/responses", post(handle_proxy_request))
        .route("/health", get(handle_health))
        .layer(RequestBodyLimitLayer::new(16 * 1024 * 1024))
        .with_state(state)
}
```

Single `handle_proxy_request` dispatches based on path → detected_format.

### ProxyConfig

```rust
struct ProxyConfig {
    listen: SocketAddr,
    max_body_size: usize,                // default 16MB
    upstream_timeout: Duration,          // default 30s
    upstream_connect_timeout: Duration,  // default 10s
    proxy_api_key: Option<String>,       // optional auth for proxy itself
}
```

No hardcoded upstream URLs in core — that's the model-router middleware's job.

## Streaming Path

```
on_request: normal (compress schemas, route channel, convert protocol)
  ↓
forward to upstream with `Accept: text/event-stream`
  ↓
stream::unfold over upstream response bytes_stream:
  - accumulate pending bytes
  - extract complete SSE frames
  - for each frame:
    1. BridgeMiddleware: transform_stream(frame, source_format, &mut StreamState)
    2. Emit transformed SSE bytes downstream
  - on message_stop: parse usage, store in ctx for CostMiddleware
  ↓
CostMiddleware: log cost record from parsed usage
```

CompressMiddleware is **skipped** on streaming responses (body not available upfront). Schema compression on the request side still applies.

## AgentType Detection

| Agent | Detection Method |
|-------|-----------------|
| Claude | `user-agent` contains `Claude-Code` or path `/v1/messages` with `anthropic-beta` header |
| Codex | `user-agent` contains `Codex` or path `/v1/responses` with `openai` org header |
| Gemini | `user-agent` contains `Gemini-CLI` or path `/v1/chat/completions` with `x-goog-api-key` |
| OpenCode | `x-agent-type: opencode` header or `user-agent` contains `OpenCode` |
| OpenClaw | `x-agent-type: openclaw` header or `user-agent` contains `OpenClaw` |
| Hermes | `x-agent-type: hermes` header or `user-agent` contains `Hermes` |
| Unknown | None of the above matches |

Detection priority: `x-agent-type` header > `user-agent` pattern > path-based heuristic.

## ProxyError

```rust
#[derive(Debug, thiserror::Error)]
enum ProxyError {
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("protocol conversion error: {0}")]
    ProtocolConversion(String),
    #[error("channel selection failed: {0}")]
    ChannelSelection(String),
    #[error("compression error: {0}")]
    Compression(String),
    #[error("rate limited: {0}")]
    RateLimited(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("internal error: {0}")]
    Internal(String),
}
```

## Middleware Lifecycle Hooks

In addition to per-connection `on_connect`/`on_disconnect`, middleware may need global lifecycle:

```rust
#[async_trait]
pub trait ProxyMiddleware: Send + Sync {
    // ... existing methods ...

    /// Called once when the proxy starts. Use for opening DB pools, loading config, etc.
    async fn on_init(&self) -> Result<(), ProxyError> { Ok(()) }

    /// Called once when the proxy shuts down gracefully.
    async fn on_shutdown(&self) -> Result<(), ProxyError> { Ok(()) }
}
```

## Reference

- llm-bridge-rust http-proxy example: `crates/core/examples/http-proxy.rs`
- axum streaming pattern: `stream::unfold` with persistent state
