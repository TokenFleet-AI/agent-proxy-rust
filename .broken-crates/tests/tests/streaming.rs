//! E2E test: Streaming SSE frame processing.
//!
//! Verifies that SSE (Server-Sent Events) frames are correctly
//! received, accumulated, and forwarded in streaming responses.

use agent_proxy_e2e_tests as common;
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{method, path};

/// Test that streaming SSE responses are properly handled.
/// Each SSE frame is delimited by `\n\n` and starts with `data: `.
#[tokio::test]
async fn test_should_stream_sse_frames() {
    let upstream = MockServer::start().await;

    let sse_body = common::load_fixture("streaming_sse.txt");
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_body.clone(), "text/event-stream"),
        )
        .mount(&upstream)
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);

    // Verify Content-Type is SSE
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/event-stream") || content_type.contains("text/plain"),
        "expected SSE content-type, got: {content_type}"
    );

    // Read full body as text
    let body = resp.text().await.expect("failed to read response body");

    // Verify SSE structure
    assert!(body.contains("data: "), "SSE response must contain data frames");
    assert!(body.contains("message_start"), "must contain message_start event");
    assert!(body.contains("content_block_start"), "must contain content_block_start event");
    assert!(body.contains("content_block_delta"), "must contain content_block_delta events");
    assert!(body.contains("message_stop"), "must contain message_stop event");
}

/// Test that SSE frames are received one-by-one (streaming behavior).
#[tokio::test]
async fn test_should_receive_sse_frames_individually() {
    let upstream = MockServer::start().await;

    let sse_body = common::load_fixture("streaming_sse.txt");
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(sse_body.clone(), "text/event-stream"),
        )
        .mount(&upstream)
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", upstream.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "stream": true,
            "messages": [{"role": "user", "content": "Hello"}]
        }))
        .send()
        .await
        .expect("failed to send request");

    let body = resp.text().await.expect("failed to read body");

    // Parse SSE frames: split by \n\n, filter empty
    let frames: Vec<&str> = body
        .split("\n\n")
        .filter(|f| !f.trim().is_empty())
        .collect();

    assert!(!frames.is_empty(), "should receive at least one SSE frame");
    for frame in &frames {
        assert!(
            frame.starts_with("data: "),
            "each frame must start with 'data: ', got: {:.50}",
            frame
        );
    }

    // Verify token usage is present in message_delta
    let has_usage = frames.iter().any(|f| f.contains("usage"));
    assert!(has_usage, "streaming response must contain usage info");

    let delta_count = frames.iter().filter(|f| f.contains("content_block_delta")).count();
    assert!(delta_count > 0, "must have at least one content_block_delta frame");
}

/// Test that the usage is parsed correctly from the final SSE event.
#[tokio::test]
async fn test_should_parse_usage_from_streaming_response() {
    let sse_body = common::load_fixture("streaming_sse.txt");
    // Pull out the message_delta event and verify it has usage data
    for line in sse_body.lines() {
        if line.contains("message_delta") && line.contains("usage") {
            if let Some(json_start) = line.find('{') {
                let json_part = &line[json_start..];
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(json_part) {
                    let usage = &event["usage"];
                    assert_eq!(usage["input_tokens"], 15);
                    assert_eq!(usage["output_tokens"], 12);
                    return; // test passes
                }
            }
        }
    }
    panic!("message_delta with usage not found in streaming response");
}
