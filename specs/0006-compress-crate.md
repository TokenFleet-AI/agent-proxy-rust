# 0006 — Compress Middleware

## Overview

The compress crate wraps `tokenless-schema` into a `ProxyMiddleware` implementation. It transparently compresses tool definitions in outgoing requests and compresses response bodies on the return path — without the AI agent or upstream API ever knowing compression is happening.

## Architecture

```
Client Request (full tool defs, ~12000 tokens)
        │
        ▼
CompressMiddleware.on_request()
  ├── Parse request body
  ├── Extract tool definitions from messages
  ├── SchemaCompressor::compress(tools) → compact tool schemas (~4500 tokens)
  ├── Inject compression metadata into ctx.extensions (CompressionStats)
  └── Return modified request body
        │
        ▼
    ... rest of middleware chain + upstream ...
        │
        ▼
Upstream Response (~800 tokens)
        │
        ▼
CompressMiddleware.on_response()   [non-streaming only]
  ├── Parse response body
  ├── ResponseCompressor::compress(response) → shorthanded response (~600 tokens)
  ├── Update CompressionStats in ctx.extensions
  └── Return modified response body
        │
        ▼
Client receives compressed response (semantically identical, fewer tokens)
```

## Tokenless Integration

```rust
// compress crate depends on tokenless-schema
use tokenless_schema::{SchemaCompressor, ResponseCompressor, CompressionStats};

pub struct CompressMiddleware {
    schema_compressor: SchemaCompressor,
    response_compressor: ResponseCompressor,
}
```

## on_request — Schema Compression

The middleware extracts tool definitions from the request body based on detected format:

- **Anthropic Messages**: `body.messages[].content[].type == "tool_use"` and `body.tools[]`
- **OpenAI Chat**: `body.tools[]` (function definitions)
- **OpenAI Responses**: `body.tools[]`

After extraction:

```rust
fn compress_request(req: &mut ProxyRequest, compressor: &SchemaCompressor) -> Result<CompressionStats> {
    let mut stats = CompressionStats::default();

    // 1. Count tokens before compression
    stats.pre_compress_tokens = token_counter::count(&req.body);

    // 2. Find and compress tool schemas
    let mut json: Value = serde_json::from_slice(&req.body)?;
    if let Some(tools) = json.get_mut("tools") {
        for tool in tools.as_array_mut().iter_mut() {
            compressor.compress_schema(tool)?;
        }
    }

    // 3. Count tokens after compression
    let compressed_body = serde_json::to_vec(&json)?;
    stats.post_compress_tokens = token_counter::count(&compressed_body);
    stats.compression_tokens_saved = stats.pre_compress_tokens.saturating_sub(stats.post_compress_tokens);

    req.body = compressed_body.into();
    Ok(stats)
}
```

Decompression hints are embedded in the compressed JSON so the upstream can still process the tool calls (the compression is lossless — field renaming and structure flattening, not semantic loss).

## on_response — Response Compression

Non-streaming only. The `ResponseCompressor` applies shorthand replacements:

- Common field names → abbreviated keys
- Repeated structural patterns → references
- Verbose content blocks → concise equivalents

```rust
fn compress_response(res: &mut ProxyResponse, compressor: &ResponseCompressor) -> Result<CompressionStats> {
    let pre_tokens = token_counter::count(&res.body);
    let compressed = compressor.compress(&res.body)?;
    let post_tokens = token_counter::count(&compressed);
    res.body = compressed.into();
    Ok(CompressionStats {
        pre_compress_tokens: pre_tokens,
        post_compress_tokens: post_tokens,
        compression_tokens_saved: pre_tokens.saturating_sub(post_tokens),
    })
}
```

## Streaming Path

Streaming responses are NOT compressed:
- Response content arrives in SSE frames — not all content is available at once.
- Decompression context would need to span frames, adding latency.
- The main cost benefit is in tool-heavy requests (schema compression on `on_request`).

`compression_tokens_saved` is set to 0 for streaming requests in the response stats.

## CompressionStats

```rust
/// Written to ctx.extensions by CompressMiddleware, read by CostMiddleware.
pub struct CompressionStats {
    /// Total tokens before any compression (request + response).
    pub pre_compress_tokens: u64,
    /// Total tokens after compression (request + response).
    pub post_compress_tokens: u64,
    /// Tokens saved (= pre - post).
    pub compression_tokens_saved: u64,
}
```

## Token Counter

A lightweight token estimator (not a full tokenizer — accuracy is ~95% sufficient for cost tracking):

```rust
mod token_counter {
    /// Estimate token count for a byte slice.
    /// Uses heuristic: ~4 chars per token for English text,
    /// adjusted for JSON structure overhead.
    pub fn count(data: &[u8]) -> u64;
}
```

For more precise counting, this can be replaced with `tiktoken-rs` in the future.

## Configuration

```rust
struct CompressConfig {
    /// Enable schema compression on requests.
    enabled: bool,  // default: true
    /// Enable response compression (non-streaming only).
    compress_responses: bool,  // default: true
    /// Minimum tool definition size in bytes to trigger compression.
    /// Below this threshold, compression overhead isn't worth it.
    min_schema_size: usize,  // default: 512
}
```

## Error Handling

Compression failures must never block the request. If compression fails:

1. Log a warning with the error details.
2. Pass the uncompressed body through.
3. Set `CompressionStats` with `pre = post` and `saved = 0`.

```rust
fn compress_or_passthrough(req: &mut ProxyRequest, compressor: &SchemaCompressor) -> CompressionStats {
    match compress_request(req, compressor) {
        Ok(stats) => stats,
        Err(e) => {
            tracing::warn!(error = %e, "schema compression failed, passing through uncompressed");
            CompressionStats::default()
        }
    }
}
```
