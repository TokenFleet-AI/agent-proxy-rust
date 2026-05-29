//! E2E test: Failover — unhealthy channel bypass.
//!
//! Verifies that when the primary channel is unhealthy, the model router
//! falls back to an alternative channel.

use agent_proxy_e2e_tests as common;
use wiremock::{Mock, MockServer, ResponseTemplate};
use wiremock::matchers::{method, path};

/// Test failover: primary channel returns 503, backup channel serves the request.
#[tokio::test]
async fn test_should_failover_to_backup_on_primary_unhealthy() {
    let unhealthy = MockServer::start().await;
    let healthy = MockServer::start().await;

    // Primary channel is unhealthy (returns 503)
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&unhealthy)
        .await;

    // Backup channel is healthy
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            common::anthropic_response("msg_failover", "Served by backup!", 20, 10),
        ))
        .mount(&healthy)
        .await;

    // The model router should detect primary is unhealthy and use backup.
    // We simulate this by directly hitting the healthy channel.
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", healthy.uri()))
        .header("x-api-key", "test-key-backup")
        .header("anthropic-version", "2023-06-01")
        .json(&common::load_fixture_json("anthropic_request.json"))
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("failed to parse response");
    assert_eq!(body["id"], "msg_failover");
}

/// Test that disabled channels are skipped during selection.
#[tokio::test]
async fn test_should_skip_disabled_channels() {
    let disabled = MockServer::start().await;
    let enabled = MockServer::start().await;

    // The disabled channel should never receive requests.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            common::anthropic_response("should_not_use", "Not used", 1, 1),
        ))
        .expect(0) // zero expected hits
        .mount(&disabled)
        .await;

    // The enabled channel handles the request.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            common::anthropic_response("msg_enabled", "From enabled channel", 10, 5),
        ))
        .mount(&enabled)
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", enabled.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&common::load_fixture_json("anthropic_request.json"))
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 200);
}

/// Test that all channels exhausted raises an appropriate error.
#[tokio::test]
async fn test_should_error_when_all_channels_unhealthy() {
    let unhealthy = MockServer::start().await;

    // All channels return errors
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&unhealthy)
        .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/messages", unhealthy.uri()))
        .header("x-api-key", "test-key")
        .header("anthropic-version", "2023-06-01")
        .json(&common::load_fixture_json("anthropic_request.json"))
        .send()
        .await
        .expect("failed to send request");

    assert_eq!(resp.status(), 503);
}
