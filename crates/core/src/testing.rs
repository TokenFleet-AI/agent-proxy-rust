//! Shared test helpers for the core crate and downstream middleware crates.

use bytes::Bytes;
use http::{HeaderMap, Method};

use crate::types::{AgentType, ApiFormat, ChannelConfig, ConnectionContext, ProxyRequest};

/// Creates a minimal test [`ProxyRequest`] for `/v1/messages`.
#[must_use]
pub fn test_proxy_request() -> ProxyRequest {
    ProxyRequest::new(
        Method::POST,
        "/v1/messages".into(),
        HeaderMap::new(),
        Bytes::from(
            r#"{"model":"claude-sonnet","max_tokens":1024,"messages":[{"role":"user","content":"hello"}]}"#,
        ),
    )
}

/// Creates a streaming test [`ProxyRequest`].
#[must_use]
pub fn test_proxy_request_streaming() -> ProxyRequest {
    ProxyRequest::new(
        Method::POST,
        "/v1/messages".into(),
        HeaderMap::new(),
        Bytes::from(
            r#"{"model":"claude-sonnet","max_tokens":1024,"messages":[{"role":"user","content":"hello"}],"stream":true}"#,
        ),
    )
}

/// Creates a test [`ProxyRequest`] with a custom body.
#[must_use]
pub fn test_proxy_request_with_body(body: impl Into<Bytes>) -> ProxyRequest {
    ProxyRequest::new(
        Method::POST,
        "/v1/messages".into(),
        HeaderMap::new(),
        body.into(),
    )
}

/// Creates a minimal test [`ConnectionContext`].
#[must_use]
pub fn test_connection_context() -> ConnectionContext {
    ConnectionContext::new(
        1,
        AgentType::Claude,
        None,
        Some(ApiFormat::AnthropicMessages),
    )
}

/// Creates a test [`ConnectionContext`] with a specific role.
#[must_use]
pub fn test_connection_context_with_role(role: impl Into<String>) -> ConnectionContext {
    ConnectionContext::new(
        1,
        AgentType::Claude,
        Some(role.into()),
        Some(ApiFormat::AnthropicMessages),
    )
}

/// Creates a test [`ChannelConfig`] pointing to a local upstream.
#[must_use]
pub fn test_channel_config(base_url: impl Into<String>) -> ChannelConfig {
    ChannelConfig {
        url: base_url.into(),
        api_key: secrecy::SecretString::from("sk-test-channel-key"),
        protocol: ApiFormat::AnthropicMessages,
        name: "test-channel".into(),
        rewrite_path: None,
    }
}

/// Creates a test [`ChannelConfig`] for `OpenAI` Chat.
#[must_use]
pub fn test_channel_config_openai(base_url: impl Into<String>) -> ChannelConfig {
    ChannelConfig {
        url: base_url.into(),
        api_key: secrecy::SecretString::from("sk-test-openai-key"),
        protocol: ApiFormat::OpenaiChat,
        name: "test-openai-channel".into(),
        rewrite_path: None,
    }
}
