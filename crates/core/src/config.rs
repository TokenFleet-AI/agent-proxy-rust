//! Proxy configuration types.

use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

use secrecy::SecretString;

/// Auth configuration for a single role-mapped key.
#[derive(Debug, Clone)]
pub struct AuthKeyEntry {
    /// The role assigned to this key (e.g., "architect", "coder").
    pub role: String,
}

/// Core proxy configuration.
#[derive(Debug, Clone)]
pub struct ProxyConfig {
    /// Address to listen on.
    pub listen: SocketAddr,
    /// Maximum request body size in bytes (default 16 MB).
    pub max_body_size: usize,
    /// Timeout for upstream HTTP requests.
    pub upstream_timeout: Duration,
    /// Timeout for establishing upstream TCP connections.
    pub upstream_connect_timeout: Duration,
    /// Optional simple auth key (check `Authorization: Bearer <key>`).
    pub proxy_api_key: Option<SecretString>,
    /// Optional simple token auth (check `X-Proxy-Token: <token>`).
    pub proxy_token: Option<SecretString>,
    /// Role-based auth: maps API keys to roles.
    pub proxy_auth_keys: HashMap<String, AuthKeyEntry>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            listen: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 8787),
            max_body_size: 16 * 1024 * 1024, // 16 MB
            upstream_timeout: Duration::from_secs(30),
            upstream_connect_timeout: Duration::from_secs(10),
            proxy_api_key: None,
            proxy_token: None,
            proxy_auth_keys: HashMap::new(),
        }
    }
}

impl ProxyConfig {
    /// Creates a new [`ProxyConfig`] with the given listen address.
    #[must_use]
    pub fn new(listen: SocketAddr) -> Self {
        Self {
            listen,
            ..Default::default()
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_values() {
        let config = ProxyConfig::default();
        assert_eq!(config.max_body_size, 16 * 1024 * 1024);
        assert_eq!(config.upstream_timeout, Duration::from_secs(30));
        assert_eq!(config.upstream_connect_timeout, Duration::from_secs(10));
        assert!(!config.has_auth());
    }

    #[test]
    fn test_has_auth_with_api_key() {
        let config = ProxyConfig {
            proxy_api_key: Some(SecretString::new("sk-test".into())),
            ..Default::default()
        };
        assert!(config.has_auth());
    }

    #[test]
    fn test_has_auth_with_token() {
        let config = ProxyConfig {
            proxy_token: Some(SecretString::new("token-test".into())),
            ..Default::default()
        };
        assert!(config.has_auth());
    }

    #[test]
    fn test_has_auth_with_role_mapping() {
        let config = ProxyConfig {
            proxy_auth_keys: HashMap::from([(
                "sk-test".into(),
                AuthKeyEntry {
                    role: "coder".into(),
                },
            )]),
            ..Default::default()
        };
        assert!(config.has_auth());
    }
}
