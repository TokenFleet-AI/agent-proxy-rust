//! Protocol bridge middleware for agent-proxy-rust.
//!
//! Translates between Anthropic Messages, `OpenAI` Chat Completions,
//! and `OpenAI` Responses API formats using [`llm_bridge_core`].
//!
//! # Middleware position
//!
//! Bridge must be registered **after** the model-router middleware so that
//! `ctx.target_protocol` is already set.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

use std::collections::HashMap;

use agent_proxy_rust_core::{
    error::ProxyError,
    extensions::EXT_BRIDGE_REVERSE,
    middleware::ProxyMiddleware,
    types::{ApiFormat, ConnectionContext, ProxyRequest, ProxyResponse},
};
use async_trait::async_trait;
use http::{HeaderMap, HeaderName, HeaderValue};
use llm_bridge_core::model::{StreamState, TransformError, TransformRequest, TransformResponse};
use tracing::debug;

// ---------------------------------------------------------------------------
// Conversion direction
// ---------------------------------------------------------------------------

/// Direction of protocol conversion between client and upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversionDirection {
    /// Client sends Anthropic, upstream expects `OpenAI` Chat.
    AnthropicToOpenai,
    /// Client sends Anthropic, upstream expects `OpenAI` Responses.
    AnthropicToResponses,
    /// Client sends `OpenAI` Chat, upstream expects Anthropic.
    OpenaiToAnthropic,
    /// Client sends `OpenAI` Responses, upstream expects Anthropic.
    ResponsesToAnthropic,
    /// No conversion needed — same protocol on both sides.
    Passthrough,
}

impl ConversionDirection {
    /// Determines the conversion direction from client and upstream protocols.
    fn resolve(client: ApiFormat, upstream: ApiFormat) -> Self {
        match (client, upstream) {
            (a, b) if a == b => Self::Passthrough,
            (ApiFormat::AnthropicMessages, ApiFormat::OpenaiChat) => Self::AnthropicToOpenai,
            (ApiFormat::AnthropicMessages, ApiFormat::OpenaiResponses) => {
                Self::AnthropicToResponses
            }
            (ApiFormat::OpenaiChat, ApiFormat::AnthropicMessages) => Self::OpenaiToAnthropic,
            (ApiFormat::OpenaiResponses, ApiFormat::AnthropicMessages) => {
                Self::ResponsesToAnthropic
            }
            // Future protocol pairs fall through to passthrough.
            _ => Self::Passthrough,
        }
    }
}

// ---------------------------------------------------------------------------
// Middleware
// ---------------------------------------------------------------------------

/// Protocol bridge middleware implementing [`ProxyMiddleware`].
///
/// Converts request/response bodies between LLM API protocols using
/// [`llm_bridge_core`]. Must be registered **after** the model-router
/// middleware so that `ctx.detected_format` and `ctx.target_protocol`
/// are already set.
#[derive(Debug, Default)]
pub struct BridgeMiddleware;

impl BridgeMiddleware {
    /// Creates a new [`BridgeMiddleware`].
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Converts a buffered SSE streaming response body between protocols.
    ///
    /// Since `handle_streaming_response` already buffers the entire SSE body,
    /// we pass it to the llm-bridge-core batch SSE transform functions which
    /// parse all frames in one pass and produce converted SSE bytes.
    /// The response body remains SSE after conversion — `is_streaming` stays `true`.
    fn on_response_streaming(
        res: &mut ProxyResponse,
        upstream_format: ApiFormat,
        client_format: ApiFormat,
    ) -> Result<(), ProxyError> {
        let mut state = StreamState::default();

        let output: Vec<u8> = match client_format {
            ApiFormat::AnthropicMessages => {
                llm_bridge_core::stream::transform_stream_to_anthropic_sse(
                    &res.body,
                    upstream_format,
                    &mut state,
                )
            }
            ApiFormat::OpenaiChat => llm_bridge_core::stream::transform_stream_to_openai_sse(
                &res.body,
                upstream_format,
                &mut state,
            ),
            ApiFormat::OpenaiResponses => {
                llm_bridge_core::stream::transform_stream_to_openai_responses_sse(
                    &res.body,
                    upstream_format,
                    &mut state,
                )
            }
            _ => {
                // Future ApiFormat variants — pass through unchanged.
                // This is safe because new variants added in llm-bridge-core
                // won't have corresponding SSE transforms yet.
                return Ok(());
            }
        }
        .map_err(|e| ProxyError::Internal(anyhow::anyhow!("{e}")))?;

        res.body = output.into();
        // is_streaming remains true — body is still SSE

        Ok(())
    }
}

#[async_trait]
impl ProxyMiddleware for BridgeMiddleware {
    async fn on_request(
        &self,
        req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError> {
        // ── Input validation ──────────────────────────────────────────
        validate_input(req)?;

        let client = ctx
            .detected_format
            .ok_or_else(|| ProxyError::BadRequest("client protocol not detected".into()))?;
        let upstream = ctx.target_protocol.ok_or_else(|| {
            ProxyError::BadRequest("upstream protocol not set by model-router".into())
        })?;

        let direction = ConversionDirection::resolve(client, upstream);
        if direction == ConversionDirection::Passthrough {
            debug!(?client, "bridge: same protocol, passthrough");
            return Ok(());
        }

        debug!(?client, ?upstream, ?direction, "bridge: converting request");

        let bridge_req = proxy_request_to_transform_request(req);

        let bridge_resp = convert_request(&bridge_req, direction)?;

        apply_transform_response_to_request(req, &bridge_resp)?;

        // Mark that reverse conversion is needed for the response.
        ctx.insert(EXT_BRIDGE_REVERSE, true);

        Ok(())
    }

    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        ctx: &ConnectionContext,
    ) -> Result<(), ProxyError> {
        let needs_reverse = ctx
            .get::<bool>(EXT_BRIDGE_REVERSE)
            .copied()
            .unwrap_or(false);
        if !needs_reverse {
            return Ok(());
        }

        let client = ctx.detected_format.ok_or_else(|| {
            ProxyError::Internal(anyhow::anyhow!("client protocol missing in response phase"))
        })?;
        let upstream = ctx.target_protocol.ok_or_else(|| {
            ProxyError::Internal(anyhow::anyhow!(
                "upstream protocol missing in response phase"
            ))
        })?;

        // Reverse: upstream → client
        let reverse_direction = ConversionDirection::resolve(upstream, client);
        if reverse_direction == ConversionDirection::Passthrough {
            return Ok(());
        }

        // Skip conversion for upstream error responses (passthrough to client)
        if is_upstream_error_body(&res.body) {
            debug!("bridge: upstream error response, skipping conversion");
            return Ok(());
        }

        // Streaming SSE responses use batch SSE conversion instead of JSON parsing
        if res.is_streaming {
            debug!(?client, ?upstream, "bridge: converting streaming response");
            return Self::on_response_streaming(res, upstream, client);
        }

        debug!(
            ?client,
            ?upstream,
            ?reverse_direction,
            "bridge: converting response"
        );

        let bridge_req = TransformRequest {
            headers: HashMap::new(),
            path: String::new(),
            body: res.body.clone(),
        };

        let bridge_resp = convert_response(&bridge_req, reverse_direction)?;
        res.body = bridge_resp.body;

        Ok(())
    }

    fn name(&self) -> &'static str {
        "bridge"
    }
}

// ---------------------------------------------------------------------------
// Helpers: input validation
// ---------------------------------------------------------------------------

/// Returns `true` if the response body is an upstream error (e.g. `{"error": {...}}`).
///
/// Upstream error responses should be passed through to the client without
/// protocol conversion, since they don't contain the expected response schema.
fn is_upstream_error_body(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").cloned())
        .is_some()
}

/// Validates the incoming request after basic checks have passed.
///
/// Performs JSON depth validation to prevent stack overflow from
/// deeply nested payloads. Content-type and empty-body checks are
/// handled earlier by [`validate_proxy_request`] in the server layer.
fn validate_input(req: &ProxyRequest) -> Result<(), ProxyError> {
    let value: serde_json::Value = serde_json::from_slice(&req.body)
        .map_err(|e| ProxyError::BadRequest(format!("invalid JSON: {e}")))?;
    llm_bridge_core::model::validate_json_depth(&value)
        .map_err(|e| ProxyError::BadRequest(e.to_string()))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers: request conversion
// ---------------------------------------------------------------------------

/// Converts an agent-proxy [`ProxyRequest`] to an llm-bridge-core [`TransformRequest`].
fn proxy_request_to_transform_request(req: &ProxyRequest) -> TransformRequest {
    let headers: HashMap<String, String> = req
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_lowercase(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();

    TransformRequest {
        headers,
        path: req.path.clone(),
        body: req.body.clone(),
    }
}

/// Applies an llm-bridge-core [`TransformResponse`] back to an agent-proxy [`ProxyRequest`].
fn apply_transform_response_to_request(
    req: &mut ProxyRequest,
    bridge_resp: &TransformResponse,
) -> Result<(), ProxyError> {
    req.path.clone_from(&bridge_resp.path);
    req.body.clone_from(&bridge_resp.body);

    let mut h = HeaderMap::new();
    for (k, v) in &bridge_resp.headers {
        let name =
            HeaderName::from_bytes(k.as_bytes()).map_err(|e| ProxyError::Internal(e.into()))?;
        let value = HeaderValue::from_str(v).map_err(|e| ProxyError::Internal(e.into()))?;
        h.insert(name, value);
    }
    req.headers = h;

    Ok(())
}

/// Calls the appropriate llm-bridge-core request transform based on direction.
fn convert_request(
    bridge_req: &TransformRequest,
    direction: ConversionDirection,
) -> Result<TransformResponse, ProxyError> {
    match direction {
        ConversionDirection::AnthropicToOpenai => {
            llm_bridge_core::transform::anthropic_to_openai(bridge_req)
        }
        ConversionDirection::AnthropicToResponses => {
            llm_bridge_core::transform::anthropic_to_openai_responses(bridge_req)
        }
        ConversionDirection::OpenaiToAnthropic => {
            llm_bridge_core::transform::openai_to_anthropic(bridge_req)
        }
        ConversionDirection::ResponsesToAnthropic => {
            llm_bridge_core::transform::responses_to_anthropic(bridge_req)
        }
        ConversionDirection::Passthrough => unreachable!("passthrough handled earlier"),
    }
    .map_err(|e: TransformError| ProxyError::BadRequest(e.sanitized_message()))
}

/// Calls the appropriate llm-bridge-core response transform based on direction.
fn convert_response(
    bridge_req: &TransformRequest,
    direction: ConversionDirection,
) -> Result<TransformResponse, ProxyError> {
    match direction {
        ConversionDirection::OpenaiToAnthropic => {
            llm_bridge_core::transform::openai_response_to_anthropic_message(bridge_req)
        }
        ConversionDirection::AnthropicToOpenai => {
            llm_bridge_core::transform::anthropic_response_to_openai_response(bridge_req)
        }
        ConversionDirection::AnthropicToResponses => {
            llm_bridge_core::transform::anthropic_response_to_responses_response(bridge_req)
        }
        ConversionDirection::ResponsesToAnthropic => {
            // `OpenAI` Responses response → Anthropic uses the same reverse transform
            llm_bridge_core::transform::openai_response_to_anthropic_message(bridge_req)
        }
        ConversionDirection::Passthrough => unreachable!("passthrough handled earlier"),
    }
    .map_err(|e: TransformError| ProxyError::Internal(e.into()))
}

// ---------------------------------------------------------------------------
// Streaming SSE adapter
// ---------------------------------------------------------------------------

/// Wraps an upstream byte stream into a protocol-converted byte stream.
///
/// Uses [`tokio::task::spawn_blocking`] internally because llm-bridge-core's
/// stream functions are synchronous (`&[u8] → Vec<u8>`). The blocking thread
/// calls the CPU-bound transform and sends results through a channel, keeping
/// the async runtime free for I/O.
///
/// # Type Parameters
///
/// * `S` — An async stream of `Result<Bytes, E>` from the upstream HTTP client.
/// * `E` — The upstream stream's error type.
pub fn bridge_stream<S, E>(
    upstream: S,
    source_format: ApiFormat,
    target_format: ApiFormat,
) -> impl futures::Stream<Item = Result<bytes::Bytes, ProxyError>> + Send
where
    S: futures::Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    use futures::StreamExt;
    use llm_bridge_core::model::StreamState;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, ProxyError>>(32);

    tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(async move {
            tokio::pin!(upstream);
            let mut state = StreamState::default();

            while let Some(chunk_result) = upstream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(ProxyError::Internal(e.into()))).await;
                        return;
                    }
                };

                if chunk.is_empty() {
                    continue;
                }

                // Dispatch to the appropriate synchronous transform.
                let output = match target_format {
                    ApiFormat::AnthropicMessages => {
                        llm_bridge_core::stream::transform_stream_to_anthropic_sse(
                            &chunk,
                            source_format,
                            &mut state,
                        )
                    }
                    ApiFormat::OpenaiChat => {
                        llm_bridge_core::stream::transform_stream_to_openai_sse(
                            &chunk,
                            source_format,
                            &mut state,
                        )
                    }
                    ApiFormat::OpenaiResponses => {
                        llm_bridge_core::stream::transform_stream_to_openai_responses_sse(
                            &chunk,
                            source_format,
                            &mut state,
                        )
                    }
                    _ => {
                        // Unknown format → passthrough
                        Ok(chunk.to_vec())
                    }
                };

                let output = match output {
                    Ok(o) => o,
                    Err(e) => {
                        let _ = tx.send(Err(ProxyError::Internal(e.into()))).await;
                        return;
                    }
                };

                if !output.is_empty() && tx.send(Ok(bytes::Bytes::from(output))).await.is_err() {
                    break; // client disconnected
                }
            }
        });
    });

    tokio_stream::wrappers::ReceiverStream::new(rx)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use agent_proxy_rust_core::{
        error::ProxyError,
        extensions::EXT_BRIDGE_REVERSE,
        middleware::ProxyMiddleware,
        types::{AgentType, ApiFormat, ConnectionContext, ProxyRequest, ProxyResponse},
    };
    use bytes::Bytes;
    use http::{HeaderMap, Method, StatusCode};

    use super::*;
    use std::sync::LazyLock;

    // -------------------------------------------------------------------------
    // Test fixtures
    // -------------------------------------------------------------------------

    /// `OpenAI` Chat SSE streaming body simulating a `DeepSeek` response.
    ///
    /// Contains: role delta, `reasoning_content` deltas, content deltas, `finish_reason`,
    /// and usage in the final chunk.
    static OPENAI_CHAT_SSE_BODY: LazyLock<Vec<u8>> = LazyLock::new(|| {
        let lines = [
            r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1780572171,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"role":"assistant"}}]}"#,
            r"",
            r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1780572171,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"content":"Hello"}}]}"#,
            r"",
            r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1780572171,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{"content":"!"}}]}"#,
            r"",
            r#"data: {"id":"chatcmpl-123","object":"chat.completion.chunk","created":1780572171,"model":"deepseek-v4-pro","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}"#,
            r"",
            r"data: [DONE]",
            r"",
        ];
        lines.join("\n").into_bytes()
    });

    /// A minimal Anthropic Messages request body.
    const ANTHROPIC_BODY: &str = r#"{"model":"claude-sonnet","max_tokens":1024,"messages":[{"role":"user","content":"hello"}]}"#;

    /// A minimal `OpenAI` Chat Completions request body.
    const OPENAI_CHAT_BODY: &str =
        r#"{"model":"gpt-4","messages":[{"role":"user","content":"hello"}]}"#;

    fn make_ctx(
        detected_format: Option<ApiFormat>,
        target_protocol: Option<ApiFormat>,
    ) -> ConnectionContext {
        let mut ctx = ConnectionContext::new(1, AgentType::Unknown, None, detected_format);
        ctx.target_protocol = target_protocol;
        ctx
    }

    fn make_request(path: &str, body: &str) -> ProxyRequest {
        ProxyRequest::new(
            Method::POST,
            path.into(),
            HeaderMap::new(),
            Bytes::copy_from_slice(body.as_bytes()),
        )
    }

    fn make_request_with_header(
        path: &str,
        body: &str,
        header_name: &str,
        header_value: &str,
    ) -> ProxyRequest {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::HeaderName::from_bytes(header_name.as_bytes()).unwrap(),
            http::HeaderValue::from_str(header_value).unwrap(),
        );
        ProxyRequest::new(
            Method::POST,
            path.into(),
            headers,
            Bytes::copy_from_slice(body.as_bytes()),
        )
    }

    fn make_response(body: &str, is_streaming: bool) -> ProxyResponse {
        ProxyResponse::new(
            StatusCode::OK,
            HeaderMap::new(),
            Bytes::copy_from_slice(body.as_bytes()),
            is_streaming,
        )
    }

    /// Builds a deeply nested JSON value for depth validation tests.
    fn make_deep_json(depth: usize) -> serde_json::Value {
        let mut val = serde_json::json!("leaf");
        for _ in 0..depth {
            val = serde_json::json!({"nested": val});
        }
        val
    }

    // -------------------------------------------------------------------------
    // ConversionDirection tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_conversion_direction_same_protocol_is_passthrough() {
        assert_eq!(
            ConversionDirection::resolve(
                ApiFormat::AnthropicMessages,
                ApiFormat::AnthropicMessages
            ),
            ConversionDirection::Passthrough
        );
        assert_eq!(
            ConversionDirection::resolve(ApiFormat::OpenaiChat, ApiFormat::OpenaiChat),
            ConversionDirection::Passthrough
        );
        assert_eq!(
            ConversionDirection::resolve(ApiFormat::OpenaiResponses, ApiFormat::OpenaiResponses),
            ConversionDirection::Passthrough
        );
    }

    #[test]
    fn test_conversion_direction_anthropic_to_openai() {
        assert_eq!(
            ConversionDirection::resolve(ApiFormat::AnthropicMessages, ApiFormat::OpenaiChat),
            ConversionDirection::AnthropicToOpenai
        );
    }

    #[test]
    fn test_conversion_direction_openai_to_anthropic() {
        assert_eq!(
            ConversionDirection::resolve(ApiFormat::OpenaiChat, ApiFormat::AnthropicMessages),
            ConversionDirection::OpenaiToAnthropic
        );
    }

    // -------------------------------------------------------------------------
    // Passthrough tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_passthrough_anthropic_same_protocol() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/messages", ANTHROPIC_BODY);
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::AnthropicMessages),
        );

        let original_body = req.body.clone();
        let original_path = req.path.clone();

        mw.on_request(&mut req, &mut ctx).await.unwrap();

        assert_eq!(req.body, original_body);
        assert_eq!(req.path, original_path);
        assert!(
            !ctx.get::<bool>(EXT_BRIDGE_REVERSE)
                .copied()
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn test_passthrough_openai_same_protocol() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/chat/completions", OPENAI_CHAT_BODY);
        let mut ctx = make_ctx(Some(ApiFormat::OpenaiChat), Some(ApiFormat::OpenaiChat));

        let original_body = req.body.clone();
        mw.on_request(&mut req, &mut ctx).await.unwrap();
        assert_eq!(req.body, original_body);
    }

    // -------------------------------------------------------------------------
    // Request conversion tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_anthropic_to_openai_chat_request_converts_path_and_body() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/messages", ANTHROPIC_BODY);
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );

        mw.on_request(&mut req, &mut ctx).await.unwrap();

        // Path should be rewritten to `OpenAI` chat endpoint
        assert_eq!(req.path, "/v1/chat/completions");

        // Body should be valid JSON in `OpenAI` format
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert!(
            body.get("messages").is_some(),
            "`OpenAI` body should contain 'messages'"
        );

        // Reverse flag should be set
        assert!(
            ctx.get::<bool>(EXT_BRIDGE_REVERSE)
                .copied()
                .unwrap_or(false)
        );
    }

    #[tokio::test]
    async fn test_openai_to_anthropic_request_converts_path_and_body() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/chat/completions", OPENAI_CHAT_BODY);
        let mut ctx = make_ctx(
            Some(ApiFormat::OpenaiChat),
            Some(ApiFormat::AnthropicMessages),
        );

        mw.on_request(&mut req, &mut ctx).await.unwrap();

        // Path should be rewritten to Anthropic endpoint
        assert_eq!(req.path, "/v1/messages");

        // Body should be valid JSON in Anthropic format
        let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap();
        assert!(
            body.get("messages").is_some(),
            "Anthropic body should contain 'messages'"
        );

        // Reverse flag should be set
        assert!(
            ctx.get::<bool>(EXT_BRIDGE_REVERSE)
                .copied()
                .unwrap_or(false)
        );
    }

    // -------------------------------------------------------------------------
    // Error tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_missing_detected_format_returns_bad_request() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/messages", ANTHROPIC_BODY);
        let mut ctx = make_ctx(None, Some(ApiFormat::OpenaiChat));

        let result = mw.on_request(&mut req, &mut ctx).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProxyError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_missing_target_protocol_returns_bad_request() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/messages", ANTHROPIC_BODY);
        let mut ctx = make_ctx(Some(ApiFormat::AnthropicMessages), None);

        let result = mw.on_request(&mut req, &mut ctx).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ProxyError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_invalid_json_body_returns_error() {
        let mw = BridgeMiddleware::new();
        let mut req = make_request("/v1/messages", "not valid json {{{");
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );

        let result = mw.on_request(&mut req, &mut ctx).await;
        assert!(
            result.is_err(),
            "invalid JSON should cause conversion failure"
        );
    }

    // -------------------------------------------------------------------------
    // Response tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_response_passthrough_without_reverse_flag() {
        let mw = BridgeMiddleware::new();
        let response_body = r#"{"id":"chatcmpl-123","object":"chat.completion"}"#;
        let mut res = make_response(response_body, false);
        let ctx = make_ctx(Some(ApiFormat::OpenaiChat), Some(ApiFormat::OpenaiChat));

        let original_body = res.body.clone();
        mw.on_response(&mut res, &ctx).await.unwrap();
        assert_eq!(res.body, original_body);
    }

    #[tokio::test]
    async fn test_response_reverse_conversion_after_anthropic_to_openai_request() {
        let mw = BridgeMiddleware::new();

        // Simulate an `OpenAI` Chat response body
        let openai_response = r#"{"id":"chatcmpl-123","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"Hello!"},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
        let mut res = make_response(openai_response, false);

        // Simulate ctx state after an Anthropic→`OpenAI` request
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );
        ctx.insert(EXT_BRIDGE_REVERSE, true);

        mw.on_response(&mut res, &ctx).await.unwrap();

        // Response should now be Anthropic format
        let body: serde_json::Value = serde_json::from_slice(&res.body).unwrap();
        assert!(
            body.get("id").is_some(),
            "Anthropic response should have 'id'"
        );
    }

    // -------------------------------------------------------------------------
    // Streaming response tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_on_response_streaming_converts_openai_sse_to_anthropic_sse() {
        let mw = BridgeMiddleware::new();

        let body_bytes = Bytes::from(OPENAI_CHAT_SSE_BODY.clone());
        let mut res = ProxyResponse::new(
            StatusCode::OK,
            HeaderMap::new(),
            body_bytes,
            true, // is_streaming
        );

        // Simulate ctx state after an Anthropic → OpenAI Chat request
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );
        ctx.insert(EXT_BRIDGE_REVERSE, true);

        let original_body = res.body.clone();
        let result = mw.on_response(&mut res, &ctx).await;

        assert!(
            result.is_ok(),
            "streaming SSE conversion should succeed, got: {result:?}"
        );

        // Body must have changed (converted)
        assert_ne!(res.body, original_body, "SSE body should be converted");

        // is_streaming should remain true
        assert!(res.is_streaming, "response should still be streaming");

        // Converted body should contain Anthropic SSE event markers
        let body_text = String::from_utf8_lossy(&res.body);
        assert!(
            body_text.contains("event: message_start"),
            "should contain message_start event, got: {body_text}"
        );
        assert!(
            body_text.contains("event: content_block_delta"),
            "should contain content_block_delta event, got: {body_text}"
        );
        assert!(
            body_text.contains("event: message_delta"),
            "should contain message_delta event, got: {body_text}"
        );
        assert!(
            body_text.contains("event: message_stop"),
            "should contain message_stop event, got: {body_text}"
        );
    }

    #[tokio::test]
    async fn test_on_response_streaming_skips_when_no_reverse_needed() {
        let mw = BridgeMiddleware::new();

        let body_bytes = Bytes::from(OPENAI_CHAT_SSE_BODY.clone());
        let mut res = ProxyResponse::new(StatusCode::OK, HeaderMap::new(), body_bytes, true);

        // Same protocol both sides → no reverse flag set
        let ctx = make_ctx(Some(ApiFormat::OpenaiChat), Some(ApiFormat::OpenaiChat));

        let original_body = res.body.clone();
        mw.on_response(&mut res, &ctx).await.unwrap();

        // Body should be unchanged (passthrough)
        assert_eq!(
            res.body, original_body,
            "SSE body should be unchanged for passthrough"
        );
    }

    // -------------------------------------------------------------------------
    // Middleware metadata
    // -------------------------------------------------------------------------

    #[test]
    fn test_middleware_name() {
        let mw = BridgeMiddleware::new();
        assert_eq!(mw.name(), "bridge");
    }

    #[test]
    fn test_middleware_debug() {
        let mw = BridgeMiddleware::new();
        let debug_str = format!("{mw:?}");
        assert!(debug_str.contains("BridgeMiddleware"));
    }

    // -------------------------------------------------------------------------
    // Input validation tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_accept_non_json_content_type_validation_moved_upstream() {
        // Content-type validation now happens in server.rs before middleware.
        // Bridge should accept any content-type since it's already validated upstream.
        let mw = BridgeMiddleware::new();
        let mut req =
            make_request_with_header("/v1/messages", ANTHROPIC_BODY, "content-type", "text/plain");
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );

        // Bridge no longer checks content-type — should succeed
        let result = mw.on_request(&mut req, &mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_reject_deeply_nested_json() {
        let mw = BridgeMiddleware::new();
        // Build JSON deeper than MAX_JSON_DEPTH (64)
        let deep = make_deep_json(70);
        let body_str = serde_json::to_string(&deep).unwrap();
        let mut req = make_request("/v1/messages", &body_str);
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );

        let result = mw.on_request(&mut req, &mut ctx).await;
        assert!(result.is_err(), "deeply nested JSON should be rejected");
    }

    // -------------------------------------------------------------------------
    // Protocol fingerprint validation tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_mismatched_protocol_body_converts_with_warning() {
        let mw = BridgeMiddleware::new();
        // Client sends to /v1/messages (Anthropic path) but body has OpenAI-only fields.
        // Fields like "model" and "messages" overlap, so the conversion may succeed
        // on common fields but the conversion result will reflect Anthropic format.
        let body_with_openai_fields = r#"{"model":"gpt-4","messages":[{"role":"user","content":"hello"}],"stream_options":{"include_usage":true}}"#;
        let mut req = make_request("/v1/messages", body_with_openai_fields);
        let mut ctx = make_ctx(
            Some(ApiFormat::AnthropicMessages),
            Some(ApiFormat::OpenaiChat),
        );

        // Conversion should still work — overlapping fields get mapped,
        // unknown fields (stream_options) may be dropped by the transform.
        let result = mw.on_request(&mut req, &mut ctx).await;
        // Note: llm-bridge-core handles unknown fields gracefully;
        // strict protocol fingerprint validation is a future enhancement.
        assert!(
            result.is_ok(),
            "conversion with overlapping fields should succeed"
        );
    }
}
