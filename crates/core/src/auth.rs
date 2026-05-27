//! Authentication middleware for the proxy.
//!
//! Supports two modes:
//! - **Simple mode**: a single `proxy_api_key` or `proxy_token`.
//! - **Role mapping mode**: multiple keys, each mapped to an agent role.
//!
//! The role (if any) is injected into the request extensions as [`AgentRole`]
//! before the request reaches the handler.

use std::collections::HashMap;

use axum::{extract::State, middleware::Next, response::Response};
use http::{Request, StatusCode};
use secrecy::ExposeSecret;

use crate::config::AuthKeyEntry;

/// Wrapper type stored in request extensions to carry the authenticated role.
#[derive(Debug, Clone)]
pub struct AgentRole(pub String);

/// Auth configuration extracted from [`ProxyConfig`](crate::config::ProxyConfig).
///
/// Used as axum state for the auth middleware.
#[derive(Debug, Clone)]
pub struct AuthState {
    /// Optional simple auth key.
    pub proxy_api_key: Option<secrecy::SecretString>,
    /// Optional simple token auth.
    pub proxy_token: Option<secrecy::SecretString>,
    /// Role-based auth mapping from API keys to roles.
    pub proxy_auth_keys: HashMap<String, AuthKeyEntry>,
}

impl AuthState {
    /// Creates an [`AuthState`] from a [`ProxyConfig`](crate::config::ProxyConfig).
    #[must_use]
    pub fn from_config(config: &crate::config::ProxyConfig) -> Self {
        Self {
            proxy_api_key: config.proxy_api_key.clone(),
            proxy_token: config.proxy_token.clone(),
            proxy_auth_keys: config.proxy_auth_keys.clone(),
        }
    }

    /// Returns `true` if any authentication mechanism is configured.
    #[must_use]
    pub fn has_auth(&self) -> bool {
        self.proxy_api_key.is_some()
            || self.proxy_token.is_some()
            || !self.proxy_auth_keys.is_empty()
    }
}

/// Axum middleware that authenticates every request.
///
/// On success, injects [`AgentRole`] into request extensions (for role mapping mode).
/// On failure, returns `401 Unauthorized`.
///
/// # Errors
///
/// Returns `StatusCode::UNAUTHORIZED` if authentication is required and the
/// request does not provide valid credentials.
pub async fn auth_middleware(
    State(auth_state): State<AuthState>,
    mut req: Request<axum::body::Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if !auth_state.has_auth() {
        return Ok(next.run(req).await);
    }

    // Role mapping mode: check against proxy_auth_keys
    if !auth_state.proxy_auth_keys.is_empty() {
        if let Some(entry) =
            extract_api_key(req.headers()).and_then(|k| auth_state.proxy_auth_keys.get(&k))
        {
            req.extensions_mut().insert(AgentRole(entry.role.clone()));
            return Ok(next.run(req).await);
        }
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Simple mode: check proxy_api_key via Authorization header
    if let Some(ref expected) = auth_state.proxy_api_key {
        let provided = req
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided == Some(expected.expose_secret()) {
            return Ok(next.run(req).await);
        }
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Simple mode: check proxy_token via X-Proxy-Token header
    if let Some(ref expected) = auth_state.proxy_token {
        let provided = req
            .headers()
            .get("x-proxy-token")
            .and_then(|v| v.to_str().ok());
        if provided == Some(expected.expose_secret()) {
            return Ok(next.run(req).await);
        }
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(req).await)
}

/// Extracts the API key from request headers.
///
/// Checks `x-api-key` first, then `Authorization: Bearer <key>`.
pub fn extract_api_key(headers: &http::HeaderMap) -> Option<String> {
    if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(key.to_string());
    }
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(std::string::ToString::to_string)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use http::HeaderMap;

    use super::*;

    #[test]
    fn test_extract_api_key_x_api_key() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-test-key".parse().unwrap());
        assert_eq!(extract_api_key(&headers), Some("sk-test-key".into()));
    }

    #[test]
    fn test_extract_api_key_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer sk-test-key".parse().unwrap());
        assert_eq!(extract_api_key(&headers), Some("sk-test-key".into()));
    }

    #[test]
    fn test_extract_api_key_x_api_key_priority() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "sk-x-api".parse().unwrap());
        headers.insert("authorization", "Bearer sk-bearer".parse().unwrap());
        assert_eq!(extract_api_key(&headers), Some("sk-x-api".into()));
    }

    #[test]
    fn test_extract_api_key_none() {
        let headers = HeaderMap::new();
        assert_eq!(extract_api_key(&headers), None);
    }

    #[test]
    fn test_extract_api_key_malformed_bearer() {
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Basic sk-test".parse().unwrap());
        assert_eq!(extract_api_key(&headers), None);
    }

    #[test]
    fn test_auth_state_has_auth_empty() {
        let state = AuthState {
            proxy_api_key: None,
            proxy_token: None,
            proxy_auth_keys: HashMap::new(),
        };
        assert!(!state.has_auth());
    }

    #[test]
    fn test_auth_state_has_auth_with_key() {
        let state = AuthState {
            proxy_api_key: Some(secrecy::SecretString::new("sk-test".into())),
            proxy_token: None,
            proxy_auth_keys: HashMap::new(),
        };
        assert!(state.has_auth());
    }
}
