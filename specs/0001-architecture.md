# 0001 — Architecture

## Overview

`agent-proxy-rust` is a composable middleware proxy for AI agent APIs. It sits between AI coding agents (Claude Code, Codex, Gemini CLI, etc.) and upstream API providers.

Every request passes through a pluggable middleware chain: compress → route → bridge → cost.

**Scope**: Local desktop proxy (Phase 1). Single user, single machine. Rate limiting, multi-tenancy, and cloud-scale features are reserved for Phase 2 extensions — designed but not implemented in the core binary.

## Phased Strategy

```
Phase 1 (local MVP)                     Phase 2 (cloud-ready)
────────────────────────              ────────────────────────
• Simple channel priority list        • Health check probe loop
• DELETE journal SQLite               • WAL mode + connection pool
• OS chmod 600 for api_key            • AES-256 encryption
• cargo install + clap CLI            • Docker + config layers
• Full 6-direction protocol bridge         • —
• 4 core providers builtin            • Community pricing fetch
• Single cost table (Phase 1 only)          • —
```

Design principle: middleware trait is the extension point. Phase 2 features are implemented as new middleware or feature-gated — never rewrite Phase 1 code.

## Crate Map

```
crates/
├── core/               Middleware trait + axum server engine + upstream forwarding
├── model-router/       Channel management + model name mapping + selection strategy
├── compress/           Token compression middleware (wraps tokenless-schema)
├── bridge/             Protocol translation middleware (wraps llm-bridge-core)
└── cost/               Per-project cost tracking + compression savings (SQLite)

apps/
└── cli/                Standalone CLI binary (cargo install agent-proxy)
```

### Dependency Direction

```
core ← model-router, compress, bridge, cost
          ↑                        ↑
     tokenless-schema         llm-bridge-core
          ↑ (runtime fetch)
     agent-proxy-pricing   ← provider & model pricing data (independent repo)
```

Core defines the `ProxyMiddleware` trait. Other crates implement it. Core has zero knowledge of tokenless, llm-bridge, or SQLite.

Provider and model pricing data lives in a separate community repository (`agent-proxy-pricing`), fetched at startup with a builtin fallback of 5 core providers. This keeps pricing updates decoupled from proxy releases.

## ProxyMiddleware Trait

```rust
#[async_trait]
pub trait ProxyMiddleware: Send + Sync {
    /// Before forwarding to upstream
    async fn on_request(
        &self,
        req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError>;

    /// After upstream responds
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

Execution order: `on_request` in registration order, `on_response` in reverse.

## Request Flow

```
1. Client sends POST /v1/messages {"model":"claude-sonnet","messages":[...]}

2. Core: build ConnectionContext
   - agent_type: detect from path/headers
   - detected_format: AnthropicMessages / OpenaiChat / OpenaiResponses

3. on_request chain (registration order):
   a. CompressMiddleware   → compress tool definitions via tokenless SchemaCompressor
   b. ModelRouterMiddleware → match client_name → select channel → replace model/base_url/api_key
   c. BridgeMiddleware      → if channel.protocol != detected_format, convert via llm-bridge-core

4. Core: forward to channel.url via reqwest

5. on_response chain (reverse order):
   c. BridgeMiddleware      → reverse protocol conversion (if needed)
   b. ModelRouterMiddleware → record channel outcome (health, latency)
   a. CompressMiddleware    → compress response body via tokenless ResponseCompressor

6. CostMiddleware (runs after response):
   - Read selected channel + pricing from ctx.extensions
   - Read StatsRecord (compression before/after) from ctx.extensions
   - Calculate actual_cost + compression savings
   - Write to SQLite

7. Return response to client
```

## Streaming

```
- on_request: same as non-streaming
- Forward: reqwest streaming response (bytes_stream)
- on_response: SSE frame-by-frame transform
  - BridgeMiddleware: transform_stream(frame, source_format, &mut StreamState) → target frames
  - CompressMiddleware: not applied to streaming (response content not known in advance)
- CostMiddleware: parse usage from message_delta / message_stop SSE events
- CompressMiddleware: skipped on streaming responses. compression_tokens_saved=0 for streaming requests.
```

## Channel Model

```
Channel
├── name: "DashScope"
├── url: "https://coding.dashscope.aliyuncs.com/apps/anthropic"
├── api_key: "sk-xxx"
├── protocol: AnthropicMessages  (决定是否调 bridge)
│
└── model_mappings: [
    {
      client_name: "claude-sonnet",       ← 客户端请求的模型名
      upstream_name: "claude-sonnet-4-7", ← 发给上游的模型名
      channel_kind: Metered | Subscription,
      pricing: PerToken | Subscription | Credits | CharBased,
      weight: 70,                          ← 同 priority 内加权随机
    },
    ...
]
```

### Channel Kind

| Kind | Description |
|------|-------------|
| `Subscription` | 包月，优先使用。monthly_price + quota + on_exhausted |
| `Metered` | 按量，加权随机选择。weight 决定流量分配比例 |

### Selection Strategy

```
Request model: "claude-sonnet"
        │
        ▼
Find all channels where model_mappings.client_name matches
        │
        ▼
┌─ Subscription channels with quota > 0 and healthy? ──▶ Use first
│
│ (none available or all exhausted)
│
┌─ Metered channels healthy? ──▶ Weighted random (weight)
│
│ (none healthy)
│
──▶ Last resort: any channel, health ignored
```

### Pricing Modes

| Mode | Fields | Use Case |
|------|--------|----------|
| `per_token` | input/output/cache_write/cache_read/thinking/image/audio per million | Anthropic, OpenAI |
| `subscription` | monthly_price, currency, quota, on_exhausted | Copilot, MiniMax Plan |
| `credits` | credits_per_mtok_input/output, credits_per_request | 积分制中转站 |
| `char_based` | price_per_million_chars, output_multiplier | 百度文心, 讯飞 |

## Protocol Translation

```
Client request format  vs  Channel.protocol  →  Action
─────────────────────────────────────────────────────────
AnthropicMessages        AnthropicMessages     Passthrough (no transform)
AnthropicMessages        OpenaiChat            anthropic_to_openai()
OpenaiChat               OpenaiChat            Passthrough
OpenaiChat               AnthropicMessages     openai_to_anthropic()
OpenaiResponses          AnthropicMessages     responses_to_anthropic()
```

Uses `llm-bridge-core` for all transforms (non-streaming + SSE streaming). Bridge is a separate crate that wraps it into a ProxyMiddleware impl.

## Cost Tracking

### Data Model

```sql
CREATE TABLE cost_records (
    id INTEGER PRIMARY KEY,
    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL,     -- from git remote
    agent_type TEXT NOT NULL,        -- claude / codex / gemini
    channel_name TEXT NOT NULL,
    model_name TEXT NOT NULL,        -- client model name
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER DEFAULT 0,
    cache_read_tokens INTEGER DEFAULT 0,
    thinking_tokens INTEGER DEFAULT 0,
    actual_cost REAL NOT NULL,       -- actual spend (0 for subscription)
    pre_compress_tokens INTEGER,     -- token count before tokenless compression
    post_compress_tokens INTEGER,    -- token count after tokenless compression
    compression_tokens_saved INTEGER, -- tokens saved by compression
    timestamp INTEGER NOT NULL
);

CREATE INDEX idx_cost_project ON cost_records(project_path, timestamp);
```

### Dashboard Aggregation

Per project × model × month:

- Total actual cost
- Total tokens (in/out/cache/thinking)
- Total compression savings (tokens)
- Per-channel breakdown

### Cost Calculation

```
PerToken:     actual_cost = (input_tokens/1M × input_price) + (output_tokens/1M × output_price) + ...
Subscription: actual_cost = 0 (monthly fee tracked separately)
Credits:      credits_used (not converted to $ unless exchange rate known)
CharBased:    (input_chars/1M × char_price) + (output_chars/1M × char_price × multiplier)
```

## Configuration

Channels come from two sources, same table:

```sql
CREATE TABLE channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    protocol TEXT NOT NULL,
    is_builtin BOOLEAN DEFAULT 0,
    enabled BOOLEAN DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE model_mappings (
    id TEXT PRIMARY KEY,
    channel_id TEXT REFERENCES channels(id),
    client_name TEXT NOT NULL,
    upstream_name TEXT NOT NULL,
    channel_kind TEXT NOT NULL,       -- "subscription" | "metered"
    pricing_json TEXT NOT NULL,
    weight INTEGER DEFAULT 100,
    enabled BOOLEAN DEFAULT 1
);
```

Builtin channels seeded on first run. Users can toggle, copy+edit, or create new.

## Upstream Forwarding

Core handles the HTTP forwarding layer. Uses reqwest (HTTP/1.1 only, some upstreams don't support H2).

### Header Forwarding Policy

- Forward: content-type, accept, user-agent (end-to-end metadata)
- Rebuild: host, content-length, authorization (framing/target/auth)
- Drop: transfer-encoding, connection, keep-alive, accept-encoding (hop-by-hop)
- Inject: Authorization: Bearer {channel.api_key}

### Failover

```
Connection failure (timeout / DNS / TCP) → retry on same channel? No.
  → Record channel as unhealthy after 3 consecutive failures
  → Next request: selection strategy skips unhealthy channels
  → Health check loop (every 60s) probes unhealthy channels → mark healthy on success
```

No mid-request retry for streaming connections — the stream cannot be replayed.

For non-streaming requests (`stream: false`), the proxy MAY retry on connection failure (timeout / DNS / TCP reset) up to 1 additional attempt on a different channel. Retries are NOT attempted on 4xx errors (client error) or on requests that have already been partially read from the upstream.

## Related Projects

- `tokenless-schema` — JSON schema & response compression (schema_compressor + response_compressor)
- `llm-bridge-core` — Anthropic ↔ OpenAI Chat ↔ OpenAI Responses protocol translation
- http-proxy example — reference implementation in llm-bridge-rust/crates/core/examples/
