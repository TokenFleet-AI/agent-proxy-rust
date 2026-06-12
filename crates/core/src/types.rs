//! Core domain types for the proxy engine.

use std::{any::Any, collections::HashMap, time::Instant};

use bytes::Bytes;
use http::{Method, header::HeaderMap};
/// Re-exported from [`llm_bridge_core::model::ApiFormat`] as the single source of truth.
pub use llm_bridge_core::model::ApiFormat;

/// Detects the [`ApiFormat`] from the request path.
///
/// Returns `None` for unrecognized paths.
#[must_use]
pub fn detect_api_format(path: &str) -> Option<ApiFormat> {
    if path.ends_with("/v1/messages") || path == "/v1/messages" {
        Some(ApiFormat::AnthropicMessages)
    } else if path.ends_with("/v1/chat/completions") || path == "/v1/chat/completions" {
        Some(ApiFormat::OpenaiChat)
    } else if path.ends_with("/v1/responses") || path == "/v1/responses" {
        Some(ApiFormat::OpenaiResponses)
    } else {
        None
    }
}

/// The AI agent client type detected from request headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentType {
    /// Anthropic Claude Code.
    Claude,
    /// `OpenAI` Codex CLI.
    Codex,
    /// Google Gemini CLI.
    Gemini,
    /// `OpenCode` agent.
    OpenCode,
    /// `OpenClaw` agent.
    OpenClaw,
    /// `Hermes` agent.
    Hermes,
    /// Unrecognized agent type.
    Unknown,
}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Claude => write!(f, "claude-code"),
            Self::Codex => write!(f, "codex"),
            Self::Gemini => write!(f, "gemini"),
            Self::OpenCode => write!(f, "opencode"),
            Self::OpenClaw => write!(f, "openclaw"),
            Self::Hermes => write!(f, "hermes"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// Detects the [`AgentType`] from request headers.
///
/// Priority: `x-agent-type` header > `user-agent` pattern > path-based heuristic.
#[must_use]
pub fn detect_agent_type(headers: &HeaderMap, path: &str) -> AgentType {
    // Check x-agent-type header first
    if let Some(agent) = headers.get("x-agent-type").and_then(|v| v.to_str().ok()) {
        return match agent.to_lowercase().as_str() {
            "claude" | "claude-code" => AgentType::Claude,
            "codex" => AgentType::Codex,
            "gemini" | "gemini-cli" => AgentType::Gemini,
            "opencode" => AgentType::OpenCode,
            "openclaw" => AgentType::OpenClaw,
            "hermes" => AgentType::Hermes,
            _ => AgentType::Unknown,
        };
    }

    // Check user-agent header
    if let Some(ua) = headers.get("user-agent").and_then(|v| v.to_str().ok()) {
        let ua_lower = ua.to_lowercase();
        if ua_lower.contains("claude-code") || ua_lower.contains("claude") {
            return AgentType::Claude;
        }
        if ua_lower.contains("codex") {
            return AgentType::Codex;
        }
        if ua_lower.contains("gemini-cli") || ua_lower.contains("gemini") {
            return AgentType::Gemini;
        }
        if ua_lower.contains("opencode") {
            return AgentType::OpenCode;
        }
        if ua_lower.contains("openclaw") {
            return AgentType::OpenClaw;
        }
        if ua_lower.contains("hermes") {
            return AgentType::Hermes;
        }
    }

    // Path-based heuristic
    if path.contains("/v1/messages") && headers.contains_key("anthropic-beta") {
        return AgentType::Claude;
    }
    if path.contains("/v1/responses") && headers.contains_key("openai-organization") {
        return AgentType::Codex;
    }
    if path.contains("/v1/chat/completions") && headers.contains_key("x-goog-api-key") {
        return AgentType::Gemini;
    }

    AgentType::Unknown
}

/// An incoming proxy request before forwarding to upstream.
#[derive(Debug, Clone)]
pub struct ProxyRequest {
    /// HTTP method.
    pub method: Method,
    /// Request path (e.g., `/v1/messages`).
    pub path: String,
    /// Request headers.
    pub headers: HeaderMap,
    /// Full request body bytes.
    pub body: Bytes,
}

impl ProxyRequest {
    /// Creates a new [`ProxyRequest`].
    #[must_use]
    pub fn new(method: Method, path: String, headers: HeaderMap, body: Bytes) -> Self {
        Self {
            method,
            path,
            headers,
            body,
        }
    }

    /// Checks whether the request body contains `"stream": true`.
    ///
    /// Used to determine whether the upstream response will be a stream.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        serde_json::from_slice::<serde_json::Value>(&self.body)
            .ok()
            .and_then(|v| v.get("stream").and_then(serde_json::Value::as_bool))
            .unwrap_or(false)
    }
}

/// The response received from upstream, before forwarding to the client.
#[derive(Debug, Clone)]
pub struct ProxyResponse {
    /// HTTP status code from upstream.
    pub status: http::StatusCode,
    /// Response headers from upstream.
    pub headers: HeaderMap,
    /// Response body bytes.
    pub body: Bytes,
    /// Whether this is a streaming response.
    pub is_streaming: bool,
}

impl ProxyResponse {
    /// Creates a new [`ProxyResponse`].
    #[must_use]
    pub fn new(
        status: http::StatusCode,
        headers: HeaderMap,
        body: Bytes,
        is_streaming: bool,
    ) -> Self {
        Self {
            status,
            headers,
            body,
            is_streaming,
        }
    }
}

/// Per-connection context passed through the middleware chain.
///
/// Middleware communicates via the `extensions` type-map. Use the constants
/// defined in [`crate::extensions`] for well-known keys.
#[derive(Debug)]
pub struct ConnectionContext {
    /// Monotonically increasing request ID.
    pub request_id: u64,
    /// Detected agent type.
    pub agent_type: AgentType,
    /// Agent role (set by auth layer from role mapping). `None` for standalone usage.
    pub agent_role: Option<String>,
    /// API format detected from the request path.
    pub detected_format: Option<ApiFormat>,
    /// Time when the request was received.
    pub started_at: Instant,
    /// Target protocol for this request (set by model-router middleware).
    pub target_protocol: Option<ApiFormat>,
    /// Extension type-map for inter-middleware communication.
    pub extensions: HashMap<String, Box<dyn Any + Send + Sync>>,

    // ── Billing / session correlation fields ──
    /// The session ID extracted from `X-Claude-Code-Session-Id` header.
    pub session_id: Option<String>,
    /// The project path extracted from `X-Claude-Code-Project-Path` header.
    pub project_path: Option<String>,
    /// The user name extracted from the tokenless report file (`userName` field).
    pub user_name: Option<String>,
    /// Accumulated tokens saved by tokenless hooks (from report file).
    pub tokenless_saved_tokens: u64,
    /// RTK rewrite-command savings extracted from tokenless report.
    pub tokenless_rtk_saved: u64,
    /// Response compression savings extracted from tokenless report.
    pub tokenless_response_saved: u64,
    /// Schema compression savings extracted from tokenless report.
    pub tokenless_schema_saved: u64,
    /// Raw breakdown from tokenless reports, stored as JSON for `CostRecord`.
    pub tokenless_breakdown_json: Option<String>,
}

impl ConnectionContext {
    /// Creates a new [`ConnectionContext`] with the given request ID and format.
    #[must_use]
    pub fn new(
        request_id: u64,
        agent_type: AgentType,
        agent_role: Option<String>,
        detected_format: Option<ApiFormat>,
    ) -> Self {
        Self {
            request_id,
            agent_type,
            agent_role,
            detected_format,
            started_at: Instant::now(),
            target_protocol: None,
            extensions: HashMap::new(),
            session_id: None,
            project_path: None,
            user_name: None,
            tokenless_saved_tokens: 0,
            tokenless_rtk_saved: 0,
            tokenless_response_saved: 0,
            tokenless_schema_saved: 0,
            tokenless_breakdown_json: None,
        }
    }

    /// Inserts a value into the extensions map, returning the previous value if any.
    pub fn insert<T: Send + Sync + 'static>(
        &mut self,
        key: &str,
        value: T,
    ) -> Option<Box<dyn Any + Send + Sync>> {
        self.extensions.insert(key.to_string(), Box::new(value))
    }

    /// Gets a reference to a value from the extensions map by key and type.
    ///
    /// Returns `None` if the key is not present or the type does not match.
    #[must_use]
    pub fn get<T: 'static>(&self, key: &str) -> Option<&T> {
        self.extensions.get(key).and_then(|v| v.downcast_ref::<T>())
    }
}

/// A simple channel configuration used for forwarding.
///
/// Set by the model-router middleware via `ctx.extensions`.
#[derive(Debug, Clone)]
pub struct ChannelConfig {
    /// The upstream base URL (from the matched protocol entry's `base_url`).
    pub url: String,
    /// The API key for the upstream channel.
    ///
    /// Wrapped in [`secrecy::SecretString`] to prevent accidental exposure
    /// in logs, debug output, or error messages.
    pub api_key: secrecy::SecretString,
    /// The protocol format the channel expects.
    pub protocol: ApiFormat,
    /// The channel name for cost tracking.
    pub name: String,
    /// Optional path rewrite. When `Some`, overrides the request path entirely.
    /// When `None`, the original (possibly bridge-rewritten) request path is used.
    pub rewrite_path: Option<String>,
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_api_format_from_path() {
        assert_eq!(
            detect_api_format("/v1/messages"),
            Some(ApiFormat::AnthropicMessages)
        );
        assert_eq!(
            detect_api_format("/v1/chat/completions"),
            Some(ApiFormat::OpenaiChat)
        );
        assert_eq!(
            detect_api_format("/v1/responses"),
            Some(ApiFormat::OpenaiResponses)
        );
        assert_eq!(detect_api_format("/health"), None);
        assert_eq!(detect_api_format("/unknown"), None);
    }

    #[test]
    fn test_detect_agent_type_claude() {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-type", "Claude".parse().unwrap());
        let result = detect_agent_type(&headers, "/v1/messages");
        assert_eq!(result, AgentType::Claude);
    }

    #[test]
    fn test_detect_agent_type_codex() {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-type", "Codex".parse().unwrap());
        let result = detect_agent_type(&headers, "/v1/responses");
        assert_eq!(result, AgentType::Codex);
    }

    #[test]
    fn test_detect_agent_type_gemini() {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-type", "Gemini".parse().unwrap());
        let result = detect_agent_type(&headers, "/v1/chat/completions");
        assert_eq!(result, AgentType::Gemini);
    }

    #[test]
    fn test_detect_agent_type_from_user_agent() {
        let mut headers = HeaderMap::new();
        headers.insert("user-agent", "Claude-Code/1.0".parse().unwrap());
        let result = detect_agent_type(&headers, "/v1/messages");
        assert_eq!(result, AgentType::Claude);
    }

    #[test]
    fn test_detect_agent_type_unknown() {
        let headers = HeaderMap::new();
        let result = detect_agent_type(&headers, "/unknown");
        assert_eq!(result, AgentType::Unknown);
    }

    #[test]
    fn test_proxy_request_is_streaming() {
        let req = ProxyRequest::new(
            Method::POST,
            "/v1/messages".into(),
            HeaderMap::new(),
            Bytes::from(r#"{"model":"claude-sonnet","stream":true}"#),
        );
        assert!(req.is_streaming());

        let req2 = ProxyRequest::new(
            Method::POST,
            "/v1/messages".into(),
            HeaderMap::new(),
            Bytes::from(r#"{"model":"claude-sonnet","stream":false}"#),
        );
        assert!(!req2.is_streaming());

        let req3 = ProxyRequest::new(
            Method::POST,
            "/v1/messages".into(),
            HeaderMap::new(),
            Bytes::from(r#"{"model":"claude-sonnet"}"#),
        );
        assert!(!req3.is_streaming());
    }

    #[test]
    fn test_connection_context_extensions() {
        let mut ctx = ConnectionContext::new(1, AgentType::Unknown, None, None);
        ctx.insert("test_key", 42u64);
        assert_eq!(ctx.get::<u64>("test_key"), Some(&42u64));
        assert_eq!(ctx.get::<String>("test_key"), None);
        assert_eq!(ctx.get::<u64>("nonexistent"), None);
    }
}
