//! Admin API authentication middleware.
//!
//! Protects `/admin/*` endpoints with an `x-admin-key` header check.
//! The expected key is read from the `AGENT_PROXY_ADMIN_KEY` environment
//! variable at startup. If not set, a random key is generated and logged.
//!
//! # Security
//!
//! The admin key is never passed via CLI arguments (which are visible in
//! `/proc/pid/cmdline` on Linux). Environment variables are per-process
//! and not readable by other users on standard systems.

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
/// Header name for the admin API key.
pub const ADMIN_KEY_HEADER: &str = "x-admin-key";

/// Environment variable for the admin API key.
pub const ADMIN_KEY_ENV: &str = "AGENT_PROXY_ADMIN_KEY";

/// Resolves the admin key at startup.
///
/// Priority:
/// 1. `AGENT_PROXY_ADMIN_KEY` environment variable
/// 2. Generate a random 32-byte hex key and log it
#[must_use]
pub fn resolve_admin_key() -> String {
    resolve_admin_key_with(|var| std::env::var(var).ok())
}

/// Internal: resolves the key with an injectable env lookup (for testing).
fn resolve_admin_key_with(env_lookup: impl Fn(&str) -> Option<String>) -> String {
    if let Some(key) = env_lookup(ADMIN_KEY_ENV) {
        let key = key.trim().to_string();
        if !key.is_empty() {
            return key;
        }
    }
    // Generate random key
    #[allow(clippy::format_collect)]
    let key: String = (0..32)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect();
    key
}

/// Axum middleware that validates the `x-admin-key` header.
#[allow(clippy::missing_docs_in_private_items)]
#[derive(Debug, Clone)]
pub struct AdminAuthLayer {
    expected_key: String,
}

impl AdminAuthLayer {
    /// Creates a new admin auth layer.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            expected_key: key.into(),
        }
    }
}

/// The actual middleware function registered with axum.
pub async fn admin_auth_middleware(
    axum::extract::State(layer): axum::extract::State<AdminAuthLayer>,
    req: Request,
    next: Next,
) -> Response {
    let provided = req
        .headers()
        .get(ADMIN_KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if provided == layer.expected_key {
        next.run(req).await
    } else {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": {
                    "code": "unauthorized",
                    "message": "invalid or missing x-admin-key header"
                }
            })),
        )
            .into_response()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_from_env() {
        let key = resolve_admin_key_with(|var| {
            if var == ADMIN_KEY_ENV {
                Some("test-secret-key".into())
            } else {
                None
            }
        });
        assert_eq!(key, "test-secret-key");
    }

    #[test]
    fn test_resolve_empty_env_generates_key() {
        let key = resolve_admin_key_with(|var| {
            if var == ADMIN_KEY_ENV {
                Some(String::new())
            } else {
                None
            }
        });
        assert_eq!(key.len(), 64, "empty env var should generate a key");
    }

    #[test]
    fn test_resolve_no_env_generates_key() {
        let key = resolve_admin_key_with(|_| None);
        assert_eq!(key.len(), 64, "no env var should generate a key");
    }
}
