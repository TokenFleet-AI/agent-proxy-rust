# 0007 — Bridge Middleware

> **Phase 1**: Full 6-direction protocol conversion matrix. Streaming + non-streaming.

## Overview

The bridge crate is a **thin wrapper** around the existing `llm-bridge-core` library (`github.com/TokenFleet-AI/llm-bridge-rust/crates/core`). 

- `llm-bridge-core` does the actual protocol translation: `model` (types), `transform` (non-streaming), `stream` (SSE frame-by-frame).
- The bridge crate only implements `ProxyMiddleware` — reading from `ProxyRequest`/`ProxyResponse`, calling `llm-bridge-core`, and writing back.

No protocol conversion logic is reimplemented in this project.

## Architecture

```
Client (POST /v1/messages, Anthropic format)
        │
        ▼
BridgeMiddleware.on_request()
  ├── Read detected_format from ConnectionContext (from request path)
  ├── Read target_protocol from ConnectionContext (set by ModelRouter)
  ├── detected_format == target_protocol? → PASSTHROUGH (no transform)
  ├── detected_format != target_protocol? → CONVERT body
  └── Store conversion metadata in ctx.extensions
        │
        ▼
    ... forward to upstream ...
        │
        ▼
BridgeMiddleware.on_response()
  ├── Was request converted? → REVERSE transform
  └── Passthrough? → return as-is
```

## Protocol Translation Matrix

```
detected_format (from path)   vs   target_protocol (channel)   →   Action
─────────────────────────────────────────────────────────────────────────────
AnthropicMessages                  AnthropicMessages                 Passthrough
AnthropicMessages                  OpenaiChat                        anthropic_to_openai()
OpenaiChat                         OpenaiChat                        Passthrough
OpenaiChat                         AnthropicMessages                 openai_to_anthropic()
OpenaiResponses                    OpenaiResponses                   Passthrough
OpenaiResponses                    AnthropicMessages                 responses_to_anthropic()
```

Uses `llm-bridge-core` for all transforms. The bridge crate is a thin wrapper — it delegates parsing and conversion to `llm-bridge-core` and only handles the middleware integration (reading/writing `ProxyRequest`/`ProxyResponse` and `ConnectionContext`).

## Non-Streaming Flow

```rust
impl ProxyMiddleware for BridgeMiddleware {
    async fn on_request(&self, req: &mut ProxyRequest, ctx: &mut ConnectionContext) -> Result<(), ProxyError> {
        let source = ctx.detected_format;
        let target = match ctx.target_protocol {
            Some(t) => t,
            None => return Ok(()),  // No channel selected yet, passthrough
        };

        if source == target {
            return Ok(());
        }

        let converted = self.bridge.convert(&req.body, source, target)
            .map_err(|e| ProxyError::ProtocolConversion(e.to_string()))?;

        ctx.extensions.insert(EXT_BRIDGE_REVERSE, BridgeReverse {
            source: target,   // Swapped: response converts back
            target: source,
        });

        req.body = converted.into();
        Ok(())
    }

    async fn on_response(&self, res: &mut ProxyResponse, ctx: &ConnectionContext) -> Result<(), ProxyError> {
        let reverse = match ctx.extensions.get::<BridgeReverse>(EXT_BRIDGE_REVERSE) {
            Some(r) => r,
            None => return Ok(()),  // Request was passthrough
        };

        let converted = self.bridge.convert(&res.body, reverse.source, reverse.target)
            .map_err(|e| ProxyError::ProtocolConversion(e.to_string()))?;

        res.body = converted.into();
        Ok(())
    }
}
```

## Streaming Flow

Streaming conversion is frame-by-frame using `llm-bridge-core`'s streaming API:

```rust
struct StreamState {
    source_format: ApiFormat,
    target_format: ApiFormat,
    buffer: Vec<u8>,           // Accumulated partial SSE data
    bridge_state: BridgeTransformState,  // llm-bridge-core internal state
}
```

For each SSE frame received from upstream:

```
1. Accumulate bytes into buffer
2. Extract complete SSE frame (ends with \n\n)
3. bridge.transform_stream(frame, source_format, &mut bridge_state) → target_frame
4. Emit target_frame as SSE downstream
5. On message_stop / [DONE] event: finalize conversion state
```

```rust
fn transform_stream_frame(
    frame: &[u8],
    state: &mut StreamState,
    bridge: &Bridge,
) -> Result<Vec<Vec<u8>>> {
    // Returns 0..N output frames per input frame.
    // Some input frames may be buffered without output (incomplete tool call accumulation).
    // Some input frames may produce multiple output frames (tool_use → multiple chat messages).
    bridge.transform_stream(frame, state.source_format, &mut state.bridge_state)
}
```

## Tool Call Round-Trip

The hardest conversion is Anthropic tool_use ↔ OpenAI tool_calls. The bridge must:

1. **Request direction**: Convert tool definitions and track tool use blocks.
2. **Response direction**: Convert tool_use content blocks to assistant messages with `tool_calls`, then convert subsequent user messages with `tool` role back to tool_result blocks.

The bridge maintains a tool call ID map across the stream for this round-trip:

```rust
struct BridgeTransformState {
    tool_id_map: HashMap<String, String>,  // anthropic_tool_id → openai_tool_id
    pending_tool_use: Vec<AnthropicToolUse>,
    stream_phase: StreamPhase,
}

enum StreamPhase {
    Content,         // Normal message content
    ToolUse,         // Accumulating tool_use block
    ToolResult,      // Converting tool result back
}
```

## Error Handling

Protocol conversion failures fall into two categories:

1. **Fatal** — the request cannot be converted (unsupported feature, malformed input). Return `ProxyError::ProtocolConversion` — the client gets a 400-level error.
2. **Degraded** — some content is lost in translation (e.g., Anthropic thinking blocks have no OpenAI equivalent). Log a warning at `debug!` level and proceed with best-effort conversion.

```rust
#[derive(Debug)]
struct ConversionWarning {
    field: String,
    message: String,
    action: WarningAction,
}

enum WarningAction {
    Dropped,       // Content was removed
    Simplified,    // Content was simplified
    Truncated,     // Content was truncated
}
```

## Configuration

```rust
struct BridgeConfig {
    /// Maximum body size for conversion (prevents OOM on malicious payloads).
    max_conversion_body_size: usize,  // default: 1MB
    /// Whether to log conversion warnings.
    log_warnings: bool,  // default: true
}
```

Conversion is stateless per-request — the bridge holds no global state beyond the `llm-bridge-core` instance.
