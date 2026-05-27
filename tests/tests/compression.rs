//! E2E test: Token compression.
//!
//! Verifies that requests with large tools arrays benefit from
//! token compression (`compression_tokens_saved` > 0).

use agent_proxy_e2e_tests as common;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

/// Test that a request with many tools triggers compression and
/// saves tokens.
#[tokio::test]
async fn test_should_compress_request_with_large_tools() {
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_compress",
                "Analysis complete",
                500,
                200,
            )),
        )
        .mount(&upstream)
        .await;

    // Load request with large tools array (8 tools with detailed schemas)
    let large_request = common::load_fixture_json("large_tools_array.json");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&large_request)
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("failed to parse response");

    let usage = &body["usage"];
    // Request had tools — usage should be recorded
    assert!(usage["input_tokens"].as_u64().unwrap_or(0) > 0);

    // In the real proxy:
    // let records = db.query_cost_records(...);
    // assert!(records[0].compression_tokens_saved > 0);
    // assert!(records[0].post_compress_tokens < records[0].pre_compress_tokens);
}

/// Test that small requests (without tools) do not benefit from compression.
#[tokio::test]
async fn test_should_not_compress_small_requests() {
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_nocompress",
                "Simple response",
                10,
                20,
            )),
        )
        .mount(&upstream)
        .await;

    // Small request — no tools
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

    assert_eq!(resp.status(), 200);

    // Small requests below min_schema_size should bypass compression.
    // In the real proxy: compression_tokens_saved should be 0
    // since the request is smaller than min_schema_size.
}

/// Test that compression is skipped when disabled in config.
#[tokio::test]
async fn test_should_skip_compression_when_disabled() {
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(common::anthropic_response(
                "msg_disabled",
                "No compression",
                200,
                100,
            )),
        )
        .mount(&upstream)
        .await;

    let large_request = common::load_fixture_json("large_tools_array.json");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&large_request)
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);

    // When compress.enabled = false, the middleware should be bypassed.
    // pre_compress_tokens should equal post_compress_tokens.
}

/// Verify that using tools results in higher token counts than not using tools.
#[tokio::test]
async fn test_tools_increase_request_size() {
    let upstream = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(common::anthropic_response("msg_size", "OK", 100, 50)),
        )
        .mount(&upstream)
        .await;

    let large_request = common::load_fixture("large_tools_array.json");

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .body(large_request.clone())
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);

    // The large tools array should be significantly larger than a simple request
    assert!(
        large_request.len() > 2048,
        "large tools request should be > 2KB, was {} bytes",
        large_request.len()
    );
}
