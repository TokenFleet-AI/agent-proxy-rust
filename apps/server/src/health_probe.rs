#![allow(clippy::cast_possible_truncation)]

//! Real health probing for upstream channels.
//!
//! Sends a minimal API request to verify that a channel's upstream is
//! reachable and its API key is valid. All probes use the cheapest model
//! bound to the channel to minimize cost.

use std::time::Instant;

use agent_proxy_rust_storage::{Channel, ModelMapping, ProtocolEntry, Storage};
use secrecy::ExposeSecret;
use serde::Serialize;

/// Outcome of a health probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeResult {
    /// Upstream responded with 200 — channel is healthy.
    Healthy,
    /// Upstream returned 401/403 — API key is invalid or expired.
    InvalidKey,
    /// Upstream returned 429 — rate limited (key is valid but throttled).
    RateLimited,
    /// Network error or timeout — upstream is unreachable.
    Unreachable,
    /// No models are bound to this channel — cannot probe.
    NoModels,
    /// No protocols configured — cannot determine endpoint.
    NoProtocols,
    /// Unexpected HTTP status.
    Unknown,
}

impl ProbeResult {
    /// Returns `true` when the probe indicates the channel can serve traffic.
    #[must_use]
    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy | Self::RateLimited)
    }

    /// Returns the string representation used for database `health_status`.
    #[allow(dead_code)]
    #[must_use]
    pub fn as_health_status(self) -> &'static str {
        match self {
            Self::Healthy | Self::RateLimited => "Healthy",
            Self::InvalidKey | Self::Unreachable | Self::Unknown => "Degraded",
            Self::NoModels | Self::NoProtocols => "Unavailable",
        }
    }
}

/// Full probe response returned to the admin API caller.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeResponse {
    /// The probe outcome.
    pub result: ProbeResult,
    /// The model name used for the probe.
    pub model: String,
    /// Round-trip latency in milliseconds.
    pub latency_ms: u64,
    /// HTTP status code from upstream (0 for network errors).
    pub http_status: u16,
}

/// Builds a dedicated reqwest client for health probes.
///
/// Uses shorter timeouts than the main proxy client since probes
/// should fail fast.
fn build_probe_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .read_timeout(std::time::Duration::from_secs(10))
        .http1_only()
        .build()
}

/// Finds the cheapest model bound to a channel.
///
/// Selects the enabled mapping whose model has the lowest
/// `price_input + price_output` sum. Returns the upstream model name.
async fn find_cheapest_model(storage: &dyn Storage, channel_id: &str) -> Option<String> {
    let mappings: Vec<ModelMapping> = storage.list_mappings(channel_id).await.ok()?;
    let enabled: Vec<&ModelMapping> = mappings.iter().filter(|m| m.enabled).collect();
    if enabled.is_empty() {
        return None;
    }

    let all_models = storage.list_models(None).await.ok()?;

    let mut cheapest: Option<(f64, &str)> = None;
    for mapping in &enabled {
        if let Some(model) = all_models
            .iter()
            .find(|m| m.client_name == mapping.client_name)
        {
            let total_price = model.price_input + model.price_output;
            if cheapest.is_none_or(|(best, _)| total_price < best) {
                cheapest = Some((total_price, mapping.upstream_name.as_str()));
            }
        }
    }

    cheapest.map(|(_, name)| name.to_string())
}

/// Sends a health probe to the channel's upstream.
///
/// Selects the cheapest bound model, constructs a minimal API request,
/// and returns the probe result with latency and HTTP status.
///
/// Returns a [`ProbeResponse`] with [`ProbeResult::NoModels`] or
/// [`ProbeResult::NoProtocols`] when the channel is misconfigured.
pub async fn probe_channel(storage: &dyn Storage, channel: &Channel) -> ProbeResponse {
    let protocols: Vec<ProtocolEntry> =
        serde_json::from_str(&channel.protocols).unwrap_or_default();
    let Some(first_protocol) = protocols.first() else {
        return no_protocol_response();
    };

    let Some(model_name) = find_cheapest_model(storage, &channel.id).await else {
        return no_model_response();
    };

    let Some((url, body)) = build_probe_request(first_protocol, &model_name) else {
        return ProbeResponse {
            result: ProbeResult::NoProtocols,
            model: model_name,
            latency_ms: 0,
            http_status: 0,
        };
    };
    send_probe(channel, first_protocol, &url, &body, &model_name).await
}

fn no_protocol_response() -> ProbeResponse {
    ProbeResponse {
        result: ProbeResult::NoProtocols,
        model: String::new(),
        latency_ms: 0,
        http_status: 0,
    }
}

fn no_model_response() -> ProbeResponse {
    ProbeResponse {
        result: ProbeResult::NoModels,
        model: String::new(),
        latency_ms: 0,
        http_status: 0,
    }
}

/// Builds the probe URL and request body for the given protocol.
///
/// Returns `None` for unsupported protocols.
fn build_probe_request(
    protocol: &ProtocolEntry,
    model_name: &str,
) -> Option<(String, serde_json::Value)> {
    let base_url = protocol.base_url.trim_end_matches('/');
    match protocol.protocol.as_str() {
        "openai_chat" => Some((
            format!("{base_url}/v1/chat/completions"),
            serde_json::json!({
                "model": model_name,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        )),
        "anthropic_messages" => Some((
            format!("{base_url}/v1/messages"),
            serde_json::json!({
                "model": model_name,
                "max_tokens": 1,
                "messages": [{"role": "user", "content": "hi"}]
            }),
        )),
        "openai_responses" => Some((
            format!("{base_url}/v1/responses"),
            serde_json::json!({
                "model": model_name,
                "input": "hi"
            }),
        )),
        _ => None,
    }
}

/// Sends the probe request and interprets the response.
async fn send_probe(
    channel: &Channel,
    protocol: &ProtocolEntry,
    url: &str,
    body: &serde_json::Value,
    model_name: &str,
) -> ProbeResponse {
    let Ok(client) = build_probe_client() else {
        return ProbeResponse {
            result: ProbeResult::Unreachable,
            model: model_name.to_string(),
            latency_ms: 0,
            http_status: 0,
        };
    };

    let api_key = channel.api_key.expose_secret();
    let mut req_builder = client
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.to_string());

    if !api_key.is_empty() {
        if protocol.protocol == "anthropic_messages" {
            req_builder = req_builder
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01");
        } else {
            req_builder = req_builder.header("Authorization", format!("Bearer {api_key}"));
        }
    }

    let start = Instant::now();
    match req_builder.send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            let latency = start.elapsed().as_millis() as u64;
            let result = match status {
                200..=299 => ProbeResult::Healthy,
                401 | 403 => ProbeResult::InvalidKey,
                429 => ProbeResult::RateLimited,
                _ => ProbeResult::Unknown,
            };
            log_probe_result(channel, result, latency, status);
            ProbeResponse {
                result,
                model: model_name.to_string(),
                latency_ms: latency,
                http_status: status,
            }
        }
        Err(e) => {
            let latency = start.elapsed().as_millis() as u64;
            let status = e.status().map_or(0, |s| s.as_u16());
            tracing::debug!(
                channel = %channel.id,
                error = %e,
                "health probe network error"
            );
            ProbeResponse {
                result: ProbeResult::Unreachable,
                model: model_name.to_string(),
                latency_ms: latency,
                http_status: status,
            }
        }
    }
}

fn log_probe_result(channel: &Channel, result: ProbeResult, latency_ms: u64, http_status: u16) {
    tracing::info!(
        channel = %channel.id,
        result = ?result,
        latency_ms,
        http_status,
        "health probe completed"
    );
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn test_should_report_healthy_for_rate_limited() {
        assert!(ProbeResult::RateLimited.is_healthy());
    }

    #[test]
    fn test_should_report_healthy_for_healthy() {
        assert!(ProbeResult::Healthy.is_healthy());
    }

    #[test]
    fn test_should_report_unhealthy_for_invalid_key() {
        assert!(!ProbeResult::InvalidKey.is_healthy());
    }

    #[test]
    fn test_should_report_unhealthy_for_unreachable() {
        assert!(!ProbeResult::Unreachable.is_healthy());
    }

    #[test]
    fn test_should_map_health_status_correctly() {
        assert_eq!(ProbeResult::Healthy.as_health_status(), "Healthy");
        assert_eq!(ProbeResult::RateLimited.as_health_status(), "Healthy");
        assert_eq!(ProbeResult::InvalidKey.as_health_status(), "Degraded");
        assert_eq!(ProbeResult::Unreachable.as_health_status(), "Degraded");
        assert_eq!(ProbeResult::NoModels.as_health_status(), "Unavailable");
    }
}
