# 0011 — Error Handling

## Overview

Error handling strategy for the proxy: classify, map, log, and respond — without leaking internals to clients.

## Error Type Hierarchy

```rust
/// Top-level error for the proxy.
#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    /// Upstream returned an error or was unreachable.
    #[error("upstream error: {source}")]
    Upstream {
        source: String,
        #[source]
        inner: Option<anyhow::Error>,
    },

    /// Protocol conversion between AI API formats failed.
    #[error("protocol conversion error: {0}")]
    ProtocolConversion(String),

    /// No channel could be selected for the requested model.
    #[error("no channel available for model '{model}'")]
    ChannelSelection { model: String },

    /// Token compression failed (non-fatal — falls back to passthrough).
    #[error("compression error: {0}")]
    Compression(String),

    /// Client sent a malformed request.
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Proxy-level auth failed.
    #[error("unauthorized")]
    Unauthorized,

    /// Rate limit exceeded.
    #[error("rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Duration },

    /// Internal error (DB, config, unexpected).
    #[error("internal error")]
    Internal(#[source] anyhow::Error),
}
```

## Error Classification

| Source | Error Variant | HTTP Status |
|--------|--------------|-------------|
| Client bad JSON / bad model name | `BadRequest` | 400 |
| Proxy auth failed | `Unauthorized` | 401 |
| Rate limit exceeded | `RateLimited` | 429 |
| No channel for model | `ChannelSelection` | 503 |
| All channels unhealthy | `ChannelSelection` | 503 |
| Upstream 4xx (except 429) | `Upstream` | 502 |
| Upstream 429 | `Upstream` | 429 (pass through) |
| Upstream 5xx | `Upstream` | 502 |
| Upstream timeout / DNS | `Upstream` | 504 |
| Protocol conversion failure | `ProtocolConversion` | 502 |
| Compression failure | (non-fatal — passthrough) | — |
| DB error | `Internal` | 500 |
| Config error | startup panic | — (no server) |

## Response Format

Error responses use a consistent JSON body:

```json
{
  "error": {
    "code": "upstream_error",
    "message": "upstream returned error: timeout after 30s",
    "detail": null
  }
}
```

```rust
#[derive(Serialize)]
struct ErrorResponse {
    error: ErrorBody,
}

#[derive(Serialize)]
struct ErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl ProxyError {
    fn to_response(&self) -> (StatusCode, ErrorResponse) {
        match self {
            Self::BadRequest(msg) => (
                StatusCode::BAD_REQUEST,
                ErrorResponse::new("bad_request", msg.clone()),
            ),
            Self::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ErrorResponse::new("unauthorized", "invalid proxy API key"),
            ),
            Self::RateLimited { retry_after } => {
                let mut resp = ErrorResponse::new("rate_limited", "rate limit exceeded");
                // Headers set separately: Retry-After
                (StatusCode::TOO_MANY_REQUESTS, resp)
            }
            Self::Upstream { source, .. } if source.contains("429") => (
                StatusCode::TOO_MANY_REQUESTS,
                ErrorResponse::new("upstream_rate_limited", "upstream rate limited"),
            ),
            Self::Upstream { .. } => (
                StatusCode::BAD_GATEWAY,
                ErrorResponse::new("upstream_error", self.to_string()),
            ),
            Self::ProtocolConversion(msg) => (
                StatusCode::BAD_GATEWAY,
                ErrorResponse::new("protocol_conversion", msg.clone()),
            ),
            Self::ChannelSelection { .. } => (
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorResponse::new("no_channel", self.to_string()),
            ),
            Self::Compression(_) => unreachable!(), // Never surfaced to client
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorResponse::new("internal_error", "internal server error"),
            ),
        }
    }
}
```

## Error Code Reference

| `code` | Meaning | Client Action |
|--------|---------|--------------|
| `bad_request` | Malformed request | Fix the request |
| `unauthorized` | Invalid proxy auth | Check API key |
| `rate_limited` | Proxy-level rate limit | Retry with backoff |
| `upstream_rate_limited` | Upstream rate limited | Retry with backoff |
| `upstream_error` | Upstream returned error | Retry or escalate |
| `no_channel` | No channel available | Check model name / channel config |
| `protocol_conversion` | Bridge conversion failed | Report to operator |
| `internal_error` | Internal server error | Report to operator |

## Logging Errors

All errors are logged server-side with full context. The client only sees sanitized messages:

```rust
fn handle_error(err: ProxyError, ctx: &ConnectionContext) -> (StatusCode, ErrorResponse) {
    match &err {
        ProxyError::Internal(e) => {
            tracing::error!(
                request_id = ctx.request_id,
                error = %e,
                "internal error"
            );
        }
        ProxyError::Upstream { source, inner } => {
            tracing::warn!(
                request_id = ctx.request_id,
                channel = ctx.extensions.get::<String>(EXT_SELECTED_CHANNEL),
                error = %source,
                "upstream error"
            );
        }
        _ => {
            tracing::debug!(
                request_id = ctx.request_id,
                error = %err,
                "request error"
            );
        }
    }
    err.to_response()
}
```

## Upstream Error Passthrough

Some upstream errors should be passed directly to the client (transparent proxy behavior):

| Upstream Status | Proxy Behavior |
|----------------|---------------|
| 200 | Pass through body |
| 400 | Map to `bad_request` |
| 401 | Map to `upstream_error` (upstream auth misconfigured) |
| 429 | Map to `upstream_rate_limited` |
| 5xx | Map to `upstream_error` |
| Timeout | Map to `upstream_error` with "timeout after Ns" |

The proxy should NEVER pass raw upstream error bodies to the client — they may contain API key hints, internal hostnames, or other sensitive data. Instead, log the raw upstream body at debug level and return the sanitized error response.

## Compression Error Strategy

Compression is a best-effort optimization. Compression failures are logged and the request proceeds uncompressed:

```rust
fn compress_or_passthrough(req: &mut ProxyRequest, compressor: &Compressor) -> StatsRecord {
    match compressor.compress(req) {
        Ok(stats) => stats,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "compression failed, passing through uncompressed"
            );
            StatsRecord::default()  // pre=0, post=0, saved=0
        }
    }
}
```

Compression errors never become `ProxyError` — they are absorbed and the uncompressed request is forwarded.

## Panic Strategy

The proxy must never panic in request-handling code. Uses:

```rust
#![forbid(unsafe_code)]

// Catch panics in middleware chain (defense in depth)
let result = AssertUnwindSafe(async {
    run_middleware_chain(req, ctx, middlewares).await
}).catch_unwind().await;

match result {
    Ok(Ok(response)) => response,
    Ok(Err(e)) => error_response(e),
    Err(panic) => {
        tracing::error!("middleware panicked: {:?}", panic);
        internal_error_response()
    }
}
```

Startup panics (config validation, DB init) are acceptable — they prevent the proxy from starting in a broken state.

## Middleware Chain Abort

If any middleware returns `Err` in `on_request`, the chain is aborted:

```
on_request: [A, B, C]
              │
              ├── A returns Ok → continue to B
              ├── B returns Err → abort, skip C
              └── (never reached: C, forward to upstream, on_response)
```

The error is immediately returned to the client — no partial processing.
