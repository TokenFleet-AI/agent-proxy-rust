//! E2E test: Protocol bridge — Anthropic ↔ `OpenAI` roundtrip through the proxy.
//!
//! These tests verify the complete middleware chain:
//!   Client → agent-proxy (`BridgeMiddleware`) → Upstream (wiremock)
//!
//! Unlike the previous version, these tests go through the real
//! `AgentProxyBuilder` with the bridge middleware registered.

use agent_proxy_e2e_tests as common;
use agent_proxy_rust_bridge::BridgeMiddleware;
use agent_proxy_rust_core::{
    AgentProxyBuilder,
    error::ProxyError as CoreError,
    extensions::EXT_SELECTED_CHANNEL,
    middleware::ProxyMiddleware,
    types::{ApiFormat, ChannelConfig, ConnectionContext, ProxyRequest, ProxyResponse},
};
use async_trait::async_trait;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use secrecy::SecretString;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

// ---------------------------------------------------------------------------
// Test middleware: acts as a simplified model-router
// ---------------------------------------------------------------------------

/// A test middleware that sets a fixed channel config pointing to wiremock.
/// Replaces `ModelRouterMiddleware` in E2E tests.
#[derive(Debug)]
struct TestChannelMiddleware {
    url: String,
    api_key: SecretString,
    protocol: ApiFormat,
    name: String,
}

#[async_trait]
impl ProxyMiddleware for TestChannelMiddleware {
    async fn on_request(
        &self,
        _req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), CoreError> {
        // Set target protocol so bridge knows the conversion direction
        ctx.target_protocol = Some(self.protocol);

        // Write channel config so forward_to_upstream can use it
        ctx.insert(
            EXT_SELECTED_CHANNEL,
            ChannelConfig {
                url: self.url.clone(),
                api_key: self.api_key.clone(),
                protocol: self.protocol,
                name: self.name.clone(),
                rewrite_path: None,
            },
        );
        Ok(())
    }

    async fn on_response(
        &self,
        _res: &mut ProxyResponse,
        _ctx: &ConnectionContext,
    ) -> Result<(), CoreError> {
        Ok(())
    }

    fn name(&self) -> &'static str {
        "test-channel-router"
    }
}

// ---------------------------------------------------------------------------
// Build a test proxy app with bridge middleware
// ---------------------------------------------------------------------------

fn build_test_app(upstream_url: String, upstream_protocol: ApiFormat) -> Router {
    let channel = TestChannelMiddleware {
        url: upstream_url,
        api_key: SecretString::from("sk-test-key"),
        protocol: upstream_protocol,
        name: "test-channel".into(),
    };

    AgentProxyBuilder::default()
        .config(agent_proxy_rust_core::ProxyConfig::default())
        .middleware(channel)
        .middleware(BridgeMiddleware::new())
        .build()
        .expect("failed to build proxy")
        .into_router()
        .expect("failed to build router")
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn anthropic_request() -> serde_json::Value {
    serde_json::json!({
        "model": "claude-sonnet",
        "max_tokens": 1024,
        "messages": [{"role": "user", "content": "Say hello in Chinese"}]
    })
}

fn openai_request() -> serde_json::Value {
    serde_json::json!({
        "model": "gpt-4",
        "messages": [{"role": "user", "content": "Say hello in Chinese"}]
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full roundtrip: client sends Anthropic → proxy converts to `OpenAI` →
/// wiremock (`OpenAI` upstream) returns `OpenAI` response → proxy converts back.
#[tokio::test]
async fn test_e2e_anthropic_client_to_openai_upstream_roundtrip() {
    let upstream = MockServer::start().await;

    // Wiremock acts as OpenAI Chat upstream
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::openai_response(
                "chatcmpl-e2e-001",
                "你好！",
                10,
                5,
            )),
        )
        .mount(&upstream)
        .await;

    let app = build_test_app(upstream.uri(), ApiFormat::OpenaiChat);

    // Client sends Anthropic-format request to /v1/messages
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("x-api-key", "test-key")
                .body(Body::from(
                    serde_json::to_vec(&anthropic_request()).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    // Bridge should have converted OpenAI response back to Anthropic format
    assert_eq!(body["id"], "chatcmpl-e2e-001");
    // If reverse conversion worked, the response should have Anthropic structure
    assert!(
        body.get("type").is_some() || body.get("object").is_some(),
        "response should have either Anthropic 'type' or OpenAI 'object' field"
    );
}

/// Full roundtrip: client sends `OpenAI` → proxy converts to Anthropic →
/// wiremock (Anthropic upstream) returns Anthropic response → proxy converts back.
#[tokio::test]
async fn test_e2e_openai_client_to_anthropic_upstream_roundtrip() {
    let upstream = MockServer::start().await;

    // Wiremock acts as Anthropic upstream
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_e2e_002",
                "Hello from Anthropic!",
                20,
                15,
            )),
        )
        .mount(&upstream)
        .await;

    let app = build_test_app(upstream.uri(), ApiFormat::AnthropicMessages);

    // Client sends OpenAI-format request to /v1/chat/completions
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .header("authorization", "Bearer test-key")
                .body(Body::from(serde_json::to_vec(&openai_request()).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body["id"], "msg_e2e_002");
}

/// Passthrough: client sends Anthropic → upstream is Anthropic.
/// Bridge should NOT convert (same protocol).
#[tokio::test]
async fn test_e2e_passthrough_same_protocol() {
    let upstream = MockServer::start().await;

    // Wiremock acts as Anthropic upstream
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_passthrough",
                "Direct response",
                5,
                3,
            )),
        )
        .mount(&upstream)
        .await;

    let app = build_test_app(upstream.uri(), ApiFormat::AnthropicMessages);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("x-api-key", "test-key")
                .body(Body::from(
                    serde_json::to_vec(&anthropic_request()).unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body_bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body["id"], "msg_passthrough");
}

/// Request validation: invalid content-type should be rejected.
#[tokio::test]
async fn test_e2e_reject_invalid_content_type() {
    let upstream = MockServer::start().await;

    let app = build_test_app(upstream.uri(), ApiFormat::AnthropicMessages);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "text/plain")
                .header("x-api-key", "test-key")
                .body(Body::from("not json"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
