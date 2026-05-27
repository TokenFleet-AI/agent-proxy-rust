# 0008 — Security

> **Phase 1**: OS file permissions (chmod 600) + `secrecy::SecretString` for api_key in memory. SSRF validation, input validation.
> **Phase 2**: AES-256-GCM encryption at rest, Argon2id master key derivation, mandatory PROXY_SECRET.

## Overview

Security boundaries in `agent-proxy-rust`:

1. **Client ↔ Proxy**: authentication, rate limiting, input validation
2. **Proxy ↔ Upstream**: API key management, TLS, SSRF prevention
3. **Proxy internal**: secret storage, memory safety, error sanitization

## API Key Encryption at Rest

Channel API keys stored in SQLite must be encrypted. The encryption scheme:

```rust
use secrecy::{ExposeSecret, SecretString};
use aes_gcm::{Aes256Gcm, Nonce, Key};
use rand::rngs::OsRng;

/// Encrypts an API key before writing to SQLite.
fn encrypt_api_key(key: &SecretString, master_key: &Key<Aes256Gcm>) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new(master_key);
    let nonce = Nonce::from_slice(&OsRng.next_u128().to_le_bytes()[..12]);
    let ciphertext = cipher.encrypt(nonce, key.expose_secret().as_bytes())
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    // Prepend nonce to ciphertext for later decryption
    let mut output = nonce.to_vec();
    output.extend(ciphertext);
    Ok(output)
}

/// Decrypts an API key read from SQLite.
fn decrypt_api_key(encrypted: &[u8], master_key: &Key<Aes256Gcm>) -> Result<SecretString> {
    let cipher = Aes256Gcm::new(master_key);
    let (nonce_bytes, ciphertext) = encrypted.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("decryption failed"))?;
    Ok(SecretString::new(String::from_utf8(plaintext)?))
}
```

### Master Key

The master key is derived from a proxy secret:

1. `PROXY_SECRET` environment variable (required at startup).
2. Derived via Argon2id: `Key = Argon2id::derive(PROXY_SECRET, salt="agent-proxy-rust-v1")`.
3. If `PROXY_SECRET` is not set, the proxy refuses to start.

In memory, the master key and all decrypted API keys are wrapped in `secrecy::Secret<>` types. Their `Debug` output is redacted.

## Proxy Authentication

The proxy itself can require authentication via `proxy_api_key` or a role-based key mapping:

### Simple Mode (single key)

```rust
struct ProxyConfig {
    /// If set, clients must provide this key in `Authorization: Bearer <key>`.
    proxy_api_key: Option<SecretString>,
    /// If set, clients must provide this header: `X-Proxy-Token: <token>`.
    proxy_token: Option<SecretString>,
}
```

### Role Mapping Mode (Ruflo Swarm)

For Ruflo swarm deployments, each role gets its own proxy key. The key serves dual purpose: auth + role identification (see `0004-cost-tracking.md §Role Detection`).

```yaml
# config.yaml
proxy_auth:
  keys:
    sk-proxy-architect: { role: architect }
    sk-proxy-coder:     { role: coder }
    sk-proxy-tester:    { role: tester }
    sk-proxy-reviewer:  { role: reviewer }
```

The auth layer:

1. Reads `x-api-key` (Anthropic) or `Authorization: Bearer` (OpenAI) from the request.
2. Looks up the key in `proxy_auth.keys`.
3. If found → extracts `role` → injects into `ConnectionContext.agent_role`.
4. If not found → returns `401 Unauthorized` (when auth is enabled).
5. Replaces the client-facing key with the real upstream channel API key before forwarding.

No custom headers required — every AI agent client already sends its API key. The proxy reuses this existing header, then swaps to the real channel key for upstream.

Authentication check happens in a Tower layer before any middleware runs:

```rust
async fn auth_layer(
    req: Request,
    next: Next,
    config: &ProxyConfig,
) -> Result<Response, StatusCode> {
    if let Some(expected) = &config.proxy_api_key {
        let provided = req.headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided != Some(expected.expose_secret()) {
            return Err(StatusCode::UNAUTHORIZED);
        }
    }
    Ok(next.run(req).await)
}
```

## TLS

The proxy should support TLS for production deployments:

```rust
struct ProxyConfig {
    /// Path to TLS certificate (PEM).
    tls_cert: Option<PathBuf>,
    /// Path to TLS private key (PEM).
    tls_key: Option<PathBuf>,
}
```

When both are set, the axum server uses `axum_server::tls_rustls::TlsAcceptor`. Use `rustls` with the `aws-lc-rs` backend.

For upstream connections, reqwest already enforces TLS for HTTPS URLs. `rustls` is used as the TLS backend (consistent with the project cryptography policy in CLAUDE.md).

## Input Validation

Every value crossing the proxy boundary is validated at the trust boundary:

### Request Body

- Max body size: 16 MB (`RequestBodyLimitLayer`).
- JSON parse failure → `400 Bad Request` (never attempt to process invalid JSON).
- Model name in body must match `^[a-zA-Z0-9._-]{1,128}$` (reject before channel lookup).
- Message count capped at 1000 messages per request.

### Headers

- Header value byte length: max 8 KB per header.
- Allowed headers (see 0001 §Header Forwarding Policy).
- Unknown headers are dropped — not forwarded to upstream.

### URL / Path

- Only `/v1/messages`, `/v1/chat/completions`, `/v1/responses`, `/health`, `/metrics` are routed.
- Unknown paths → `404 Not Found`.
- Query parameters are stripped before forwarding.

## SSRF Prevention

The channel URL must be validated to prevent SSRF attacks:

```rust
fn validate_channel_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url)?;

    // Only HTTPS (production) or localhost (development)
    if parsed.scheme() != "https" && !is_localhost_dev(&parsed) {
        return Err(anyhow::anyhow!("channel URL must use https"));
    }

    // Resolve host to IP
    let addrs: Vec<SocketAddr> = format!("{}:{}", parsed.host_str().unwrap(), parsed.port().unwrap_or(443))
        .to_socket_addrs()?
        .collect();

    for addr in &addrs {
        let ip = addr.ip();
        if ip.is_loopback() || ip.is_private() || ip.is_unspecified() || ip.is_multicast() {
            return Err(anyhow::anyhow!("channel URL resolves to banned address: {}", ip));
        }
    }

    Ok(parsed)
}

fn is_localhost_dev(url: &Url) -> bool {
    url.host_str() == Some("localhost") && cfg!(debug_assertions)
}
```

This validation runs when a channel is created or updated — not on every request.

## Rate Limiting

Per-client rate limiting using a token bucket:

```rust
struct RateLimitConfig {
    /// Maximum requests per second per client (by IP or proxy token).
    requests_per_second: u32,  // default: 50
    /// Burst size.
    burst_size: u32,  // default: 100
}
```

Implemented as a Tower layer using `governor` or a simple token-bucket. Rate limit state is per-proxy-instance (not shared) — sufficient for single-machine deployments.

429 responses from the proxy itself are distinct from upstream 429s. The proxy's 429s include a `Retry-After` header.

## Error Sanitization

Error responses returned to the client must NOT leak internal details:

- Upstream error messages → mapped to generic categories (see `ProxyError` in 0002).
- Internal stack traces → never included in responses (`tracing::error!` logs them server-side).
- API keys → redacted from all logs, errors, and debug output via `secrecy::SecretString`.
- Channel URLs → logged at `debug!` level only (API key stripped from URL in logs).

## Memory Safety

- `#![forbid(unsafe_code)]` in all crates.
- `secrecy` crate for all secrets in memory.
- Zeroize secret buffers on drop: use `zeroize::ZeroizeOnDrop` for any custom secret-bearing types.
