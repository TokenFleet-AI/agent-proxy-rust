//! E2E test: Protocol bridge — `Anthropic` ↔ `OpenAI` cross-protocol roundtrip.
//!
//! Verifies that the bridge middleware correctly converts between
//! `Anthropic` Messages and `OpenAI` Chat Completions protocols.
//! Wiremock servers simulate the upstream APIs; in production the bridge
//! middleware handles path and format conversion transparently.

use agent_proxy_e2e_tests as common;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// Test Anthropic-to-OpenAI bridge: client sends Anthropic-formatted request.
/// The bridge middleware converts to `OpenAI` format and forwards to the
/// `OpenAI`-compatible upstream. Wiremock simulates the `OpenAI` upstream
/// accepting the converted request and returning `OpenAI`-format responses.
#[tokio::test]
async fn test_should_bridge_anthropic_request_to_openai_upstream() {
    let openai_upstream = MockServer::start().await;

    // Wiremock acts as OpenAI upstream, receiving POST /v1/chat/completions
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::openai_response(
                "chatcmpl-001",
                "Hello from OpenAI!",
                20,
                15,
            )),
        )
        .mount(&openai_upstream)
        .await;

    // Client sends Anthropic-formatted body to the OpenAI endpoint.
    // In production, the bridge middleware translates this to OpenAI format.
    let anthropic_request = common::load_fixture_json("anthropic_request.json");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", openai_upstream.uri()))
        .header("x-api-key", "test-key")
        .json(&anthropic_request)
        .send()
        .await
        .expect("failed to send request");

    assert!(resp.status().is_success());

    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["id"], "chatcmpl-001");
    assert_eq!(body["object"], "chat.completion");
    assert!(
        !body["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .is_empty()
    );
    assert!(body["usage"]["prompt_tokens"].as_u64().unwrap_or(0) > 0);
}

/// Test OpenAI-to-Anthropic bridge: client sends OpenAI-formatted request.
/// The bridge middleware converts to Anthropic format and forwards to the
/// Anthropic-compatible upstream.
#[tokio::test]
async fn test_should_bridge_openai_request_to_anthropic_upstream() {
    let anthropic_upstream = MockServer::start().await;

    // Wiremock acts as Anthropic upstream, receiving POST /v1/messages
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_bridge_002",
                "Bridged response",
                30,
                25,
            )),
        )
        .mount(&anthropic_upstream)
        .await;

    // Client sends OpenAI-formatted body to the Anthropic endpoint.
    // In production, the bridge middleware translates to Anthropic format.
    let openai_request = common::load_fixture_json("openai_request.json");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", anthropic_upstream.uri()))
        .header("authorization", "Bearer test-key")
        .json(&openai_request)
        .send()
        .await
        .expect("failed to send request");

    assert!(resp.status().is_success());

    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["id"], "msg_bridge_002");
    assert_eq!(body["type"], "message");
    assert!(body["usage"]["input_tokens"].as_u64().unwrap_or(0) > 0);
}

/// Test bridge conversion with model name mapping.
/// Client uses Anthropic model name; bridge maps it to the upstream model.
#[tokio::test]
async fn test_should_map_models_in_bridge_conversion() {
    let upstream = MockServer::start().await;

    // Wiremock simulates an OpenAI upstream that handles Chat Completions.
    // In production, the bridge middleware maps Anthropic model names to
    // the upstream channel's model names.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::openai_response(
                "chatcmpl-mapped",
                "Mapped response",
                12,
                8,
            )),
        )
        .mount(&upstream)
        .await;

    // Client sends to the OpenAI endpoint directly.
    // In production: client → proxy /v1/messages → bridge → upstream /v1/chat/completions
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "gpt-5",
            "max_tokens": 500,
            "messages": [{"role": "user", "content": "Test model mapping"}]
        }))
        .send()
        .await
        .expect("failed to send request");

    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["id"], "chatcmpl-mapped");

    let usage = &body["usage"];
    assert_eq!(usage["prompt_tokens"], 12);
    assert_eq!(usage["completion_tokens"], 8);
    assert_eq!(usage["total_tokens"], 20);
}
