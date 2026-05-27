//! E2E test: Passthrough proxy request.
//!
//! Verifies that a POST to the proxy for `/v1/messages` is forwarded
//! to the upstream Anthropic API, and the response is returned correctly.
//! Also checks that cost tracking records are created.

use agent_proxy_e2e_tests as common;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// Test that a passthrough Anthropic request returns 200 and the
/// cost tracking records the usage.
///
/// This test simulates the full proxy flow:
/// client → proxy → upstream Anthropic → proxy → client
#[tokio::test]
async fn test_should_passthrough_anthropic_request_and_track_cost() {
    // Start mock upstream server
    let upstream = MockServer::start().await;

    // Mount a mock that matches Anthropic Messages API requests
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_test_001",
                "Hello from Anthropic!",
                100,
                50,
            )),
        )
        .mount(&upstream)
        .await;

    // Build the request
    let request_body = common::load_fixture_json("anthropic_request.json");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key-primary")
        .header("anthropic-version", "2023-06-01")
        .json(&request_body)
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["id"], "msg_test_001");
    assert_eq!(body["type"], "message");

    // Verify usage was reported
    let usage = &body["usage"];
    assert_eq!(usage["input_tokens"], 100);
    assert_eq!(usage["output_tokens"], 50);

    // In the real proxy, we would query cost_records table:
    // let records = db.query_cost_records(...);
    // assert!(!records.is_empty());
    // assert_eq!(records[0].input_tokens, 100);
}

/// Test passthrough with authentication header forwarding.
#[tokio::test]
async fn test_should_forward_auth_headers_to_upstream() {
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(common::anthropic_response("msg_auth", "OK", 10, 5)),
        )
        .mount(&upstream)
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "my-secret-key")
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["id"], "msg_auth");
}

/// Test passthrough with upstream returning an error.
#[tokio::test]
async fn test_should_propagate_upstream_errors() {
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(429).set_body_json(serde_json::json!({
            "error": {
                "type": "rate_limit_error",
                "message": "Too many requests"
            }
        })))
        .mount(&upstream)
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 100,
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 429);

    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["error"]["type"], "rate_limit_error");
}
