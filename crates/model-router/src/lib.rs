//! Model routing and channel selection middleware.
//!
//! Implements the selection strategy from specs/0003-channel-model.md Phase 1:
//! `FlatFee` channels with quota > 0 and healthy are preferred; `Metered` channels
//! serve as fallback. Health tracking uses a simple binary model with 60s
//! cooldown.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

mod types;

use std::{sync::Arc, time::Duration};

use agent_proxy_rust_core::{
    ProxyError,
    extensions::{EXT_SELECTED_CHANNEL, EXT_SELECTED_MAPPING},
    middleware::ProxyMiddleware,
    types::{ApiFormat, ChannelConfig, ConnectionContext, ProxyRequest, ProxyResponse},
};
use agent_proxy_rust_storage::Storage;
use async_trait::async_trait;
use dashmap::DashMap;
use secrecy::ExposeSecret;
use tracing::{debug, warn};
pub use types::{
    ChannelBilling, ChannelHealth, ChannelState, ExhaustedAction, Pricing, Quota, QuotaUsage,
};

/// Cooldown period before an unhealthy channel is retried.
const COOLDOWN: Duration = Duration::from_secs(60);

/// Parsed in-memory representation of a channel with its model mappings.
#[derive(Debug, Clone)]
struct ResolvedChannel {
    channel_id: String,
    channel_name: String,
    url: String,
    api_key: String,
    protocol: ApiFormat,
    enabled: bool,
    mappings: Vec<ResolvedMapping>,
}

/// Parsed in-memory representation of a model mapping.
#[derive(Debug, Clone)]
struct ResolvedMapping {
    mapping_id: String,
    client_name: String,
    upstream_name: String,
    billing: ChannelBilling,
}

/// Lightweight mapping info stored in the context extension.
#[derive(Debug, Clone)]
pub struct SelectedMappingInfo {
    /// Channel ID for cost tracking.
    pub channel_id: String,
    /// Model mapping ID for quota tracking.
    pub mapping_id: String,
    /// Client-facing model name.
    pub client_name: String,
    /// Upstream model name sent to the API.
    pub upstream_name: String,
    /// Whether this mapping uses flat-fee billing.
    pub is_flat_fee: bool,
}

/// Channel selection and model routing middleware.
#[derive(Debug)]
pub struct ModelRouterMiddleware {
    channels: Vec<ResolvedChannel>,
    health: Arc<DashMap<String, ChannelState>>,
    /// Per-mapping-id quota consumption counters. Keys match `model_mappings.id`.
    quota_usage: Arc<DashMap<String, QuotaUsage>>,
}

impl ModelRouterMiddleware {
    /// Creates a new [`ModelRouterMiddleware`] from a storage backend.
    ///
    /// Loads all enabled channels and their model mappings, parsing the
    /// storage-layer string fields into typed enums.
    ///
    /// # Errors
    ///
    /// Returns `ProxyError` if the storage backend fails or if any channel
    /// has an unrecognized protocol string.
    pub async fn from_storage(storage: Arc<dyn Storage>) -> Result<Self, ProxyError> {
        let storage_channels = storage
            .list_channels(None)
            .await
            .map_err(|e| ProxyError::Internal(e.into()))?;

        let mut channels = Vec::with_capacity(storage_channels.len());

        for ch in storage_channels {
            let protocol = parse_protocol(&ch.protocol)?;

            let storage_mappings = storage
                .list_mappings(&ch.id)
                .await
                .map_err(|e| ProxyError::Internal(e.into()))?;

            let mappings: Vec<ResolvedMapping> = storage_mappings
                .into_iter()
                .filter(|m| m.enabled)
                .filter_map(|m| {
                    let billing = ChannelBilling::from_storage(&m.billing, &m.pricing_json)
                        .map_err(|e| {
                            warn!(
                                channel = %ch.id,
                                mapping = %m.id,
                                error = %e,
                                "failed to parse mapping billing/pricing, skipping"
                            );
                        })
                        .ok()?;
                    Some(ResolvedMapping {
                        mapping_id: m.id,
                        client_name: m.client_name,
                        upstream_name: m.upstream_name,
                        billing,
                    })
                })
                .collect();

            channels.push(ResolvedChannel {
                channel_id: ch.id,
                channel_name: ch.name,
                url: ch.base_url,
                api_key: ch.api_key.expose_secret().to_owned(),
                protocol,
                enabled: ch.enabled,
                mappings,
            });
        }

        Ok(Self {
            channels,
            health: Arc::new(DashMap::new()),
            quota_usage: Arc::new(DashMap::new()),
        })
    }

    /// Returns a reference to the in-memory health map.
    #[must_use]
    pub fn health_map(&self) -> &Arc<DashMap<String, ChannelState>> {
        &self.health
    }

    /// Finds all candidate mappings for a given client model name.
    fn find_candidates(&self, client_name: &str) -> Vec<(&ResolvedChannel, &ResolvedMapping)> {
        let mut candidates = Vec::new();
        for ch in &self.channels {
            if !ch.enabled {
                continue;
            }
            for m in &ch.mappings {
                if m.client_name == client_name {
                    candidates.push((ch, m));
                }
            }
        }
        candidates
    }

    /// Applies the Phase 1 selection strategy.
    fn select_channel<'a>(
        &self,
        candidates: &[(&'a ResolvedChannel, &'a ResolvedMapping)],
        client_name: &str,
    ) -> Result<(&'a ResolvedChannel, &'a ResolvedMapping), ProxyError> {
        let (flatfee, metered): (Vec<_>, Vec<_>) = candidates
            .iter()
            .partition(|(_, m)| m.billing.is_flat_fee());

        // Phase 1: try FlatFee channels first
        for (ch, m) in &flatfee {
            if !self.is_healthy(&ch.channel_id) {
                continue;
            }
            if let ChannelBilling::FlatFee {
                on_exhausted,
                quota,
                ..
            } = &m.billing
            {
                // Check actual monthly consumption against quota
                let within_quota = self
                    .quota_usage
                    .entry(m.mapping_id.clone())
                    .or_default()
                    .is_within_quota(quota.as_ref());

                if within_quota {
                    return Ok((ch, m));
                }
                if *on_exhausted == ExhaustedAction::Block {
                    debug!(
                        channel = %ch.channel_id,
                        model = %client_name,
                        "flat-fee channel quota exhausted, blocking"
                    );
                    return Err(ProxyError::ChannelSelection {
                        model: client_name.to_owned(),
                    });
                }
            }
        }

        // Phase 1: try Metered channels
        for (ch, m) in &metered {
            if self.is_healthy(&ch.channel_id) {
                return Ok((ch, m));
            }
        }

        // All unhealthy — try any channel past cooldown
        for (ch, m) in candidates {
            if self.is_tryable_past_cooldown(&ch.channel_id) {
                warn!(
                    channel = %ch.channel_id,
                    model = %client_name,
                    "all channels unhealthy, retrying past cooldown"
                );
                return Ok((ch, m));
            }
        }

        Err(ProxyError::ChannelSelection {
            model: client_name.to_owned(),
        })
    }

    fn is_healthy(&self, channel_id: &str) -> bool {
        self.health
            .get(channel_id)
            .is_none_or(|s| s.is_tryable(COOLDOWN))
    }

    fn is_tryable_past_cooldown(&self, channel_id: &str) -> bool {
        self.health
            .get(channel_id)
            .is_none_or(|s| s.is_tryable(COOLDOWN))
    }

    fn mark_healthy(&self, channel_id: &str) {
        if let Some(mut state) = self.health.get_mut(channel_id) {
            state.record_success();
        }
    }

    /// Records a request failure. After 3 consecutive failures the
    /// channel is marked Unhealthy with a 60 s cooldown.
    fn record_failure(&self, channel_id: &str) {
        let mut state = self.health.entry(channel_id.to_owned()).or_default();
        state.record_failure();
    }

    /// Forces a channel to Unhealthy immediately (e.g. 5xx server error).
    fn mark_unhealthy_immediate(&self, channel_id: &str) {
        self.health
            .entry(channel_id.to_owned())
            .or_default()
            .mark_unhealthy();
    }
}

#[async_trait]
impl ProxyMiddleware for ModelRouterMiddleware {
    async fn on_request(
        &self,
        req: &mut ProxyRequest,
        ctx: &mut ConnectionContext,
    ) -> Result<(), ProxyError> {
        let mut body: serde_json::Value =
            serde_json::from_slice(&req.body).map_err(|e| ProxyError::BadRequest(e.to_string()))?;

        let client_name = body
            .get("model")
            .and_then(|v| v.as_str())
            .map(String::from)
            .unwrap_or_default();

        if client_name.is_empty() {
            return Err(ProxyError::BadRequest(
                "request body missing 'model' field".into(),
            ));
        }

        let candidates = self.find_candidates(&client_name);

        if candidates.is_empty() {
            return Err(ProxyError::ChannelSelection { model: client_name });
        }

        let (channel, mapping) = self.select_channel(&candidates, &client_name)?;

        debug!(
            channel = %channel.channel_id,
            client_model = %client_name,
            upstream_model = %mapping.upstream_name,
            "selected channel"
        );

        // Replace model name in body
        if let Some(model_field) = body.get_mut("model") {
            *model_field = serde_json::Value::String(mapping.upstream_name.clone());
        }
        let new_body =
            serde_json::to_vec(&body).map_err(|e| ProxyError::BadRequest(e.to_string()))?;
        req.body = bytes::Bytes::from(new_body);

        // Set target protocol
        ctx.target_protocol = Some(channel.protocol);

        // Write ChannelConfig to extensions
        ctx.insert(
            EXT_SELECTED_CHANNEL,
            ChannelConfig {
                url: channel.url.clone(),
                api_key: channel.api_key.clone(),
                protocol: channel.protocol,
                name: channel.channel_name.clone(),
            },
        );

        // Write selected mapping info to extensions
        ctx.insert(
            EXT_SELECTED_MAPPING,
            SelectedMappingInfo {
                channel_id: channel.channel_id.clone(),
                mapping_id: mapping.mapping_id.clone(),
                client_name: mapping.client_name.clone(),
                upstream_name: mapping.upstream_name.clone(),
                is_flat_fee: mapping.billing.is_flat_fee(),
            },
        );

        Ok(())
    }

    async fn on_response(
        &self,
        res: &mut ProxyResponse,
        ctx: &ConnectionContext,
    ) -> Result<(), ProxyError> {
        let channel_id = ctx
            .get::<ChannelConfig>(EXT_SELECTED_CHANNEL)
            .map(|ch| ch.name.clone())
            .unwrap_or_default();

        if channel_id.is_empty() {
            return Ok(());
        }

        // Record quota usage for the selected mapping
        if let Some(mapping_info) = ctx.get::<SelectedMappingInfo>(EXT_SELECTED_MAPPING)
            && mapping_info.is_flat_fee
        {
            let token_count =
                serde_json::from_slice(&res.body).map_or(0, |body| extract_token_count(&body));
            self.quota_usage
                .entry(mapping_info.mapping_id.clone())
                .or_default()
                .record_usage(token_count);
        }

        if res.status.is_server_error() {
            // 5xx: immediate unhealthy — server is down
            warn!(
                channel = %channel_id,
                status = %res.status,
                "upstream 5xx, marking channel unhealthy immediately"
            );
            self.mark_unhealthy_immediate(&channel_id);
        } else if res.status.is_client_error() && res.status.as_u16() != 429 {
            // 4xx (except 429): client errors — not the channel's fault
            debug!(
                channel = %channel_id,
                status = %res.status,
                "client error, not counting as channel failure"
            );
        } else if res.status == http::StatusCode::TOO_MANY_REQUESTS {
            // 429: rate limit — counts as a failure
            warn!(
                channel = %channel_id,
                "upstream 429 rate limit, recording failure"
            );
            self.record_failure(&channel_id);
        } else {
            // 2xx: success
            self.mark_healthy(&channel_id);
        }

        Ok(())
    }

    fn name(&self) -> &'static str {
        "model-router"
    }
}

// ── Helpers ─────────────────────────────────────────────────────────

fn parse_protocol(s: &str) -> Result<ApiFormat, ProxyError> {
    match s {
        "anthropic_messages" => Ok(ApiFormat::AnthropicMessages),
        "openai_chat" => Ok(ApiFormat::OpenaiChat),
        "openai_responses" => Ok(ApiFormat::OpenaiResponses),
        other => Err(ProxyError::Internal(anyhow::anyhow!(
            "unknown protocol in storage: {other}"
        ))),
    }
}

/// Returns the total token count from an upstream response body for quota tracking.
fn extract_token_count(body: &serde_json::Value) -> u64 {
    body.get("usage").map_or(0, |u| {
        u.get("input_tokens")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            + u.get("output_tokens")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
    })
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::unwrap_in_result,
    clippy::unchecked_duration_subtraction,
    clippy::panic
)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::types::ChannelHealth;

    fn make_channel(
        id: &str,
        name: &str,
        protocol: ApiFormat,
        mappings: Vec<ResolvedMapping>,
    ) -> ResolvedChannel {
        ResolvedChannel {
            channel_id: id.into(),
            channel_name: name.into(),
            url: format!("https://{id}.example.com"),
            api_key: "sk-test".into(),
            protocol,
            enabled: true,
            mappings,
        }
    }

    fn make_mapping_flatfee(
        client: &str,
        upstream: &str,
        exhausted: ExhaustedAction,
    ) -> ResolvedMapping {
        ResolvedMapping {
            mapping_id: format!("test:{client}"),
            client_name: client.into(),
            upstream_name: upstream.into(),
            billing: ChannelBilling::FlatFee {
                monthly_cost_hint: None,
                quota: Some(Quota::Unlimited),
                on_exhausted: exhausted,
            },
        }
    }

    fn make_mapping_metered(client: &str, upstream: &str) -> ResolvedMapping {
        ResolvedMapping {
            mapping_id: format!("test:{client}"),
            client_name: client.into(),
            upstream_name: upstream.into(),
            billing: ChannelBilling::Metered {
                pricing: Pricing::PerToken {
                    input_per_mtok: 3.0,
                    output_per_mtok: 15.0,
                    cache_write_per_mtok: None,
                    cache_read_per_mtok: None,
                    thinking_per_mtok: None,
                },
            },
        }
    }

    fn make_middleware(channels: Vec<ResolvedChannel>) -> ModelRouterMiddleware {
        ModelRouterMiddleware {
            channels,
            health: Arc::new(DashMap::new()),
            quota_usage: Arc::new(DashMap::new()),
        }
    }

    // ── Selection strategy ──────────────────────────────────────

    #[test]
    fn test_select_flatfee_has_quota_and_healthy() {
        let mw = make_middleware(vec![
            make_channel(
                "sub",
                "Subscription",
                ApiFormat::AnthropicMessages,
                vec![make_mapping_flatfee(
                    "claude-sonnet",
                    "claude-sonnet-4-7",
                    ExhaustedAction::FallbackToMetered,
                )],
            ),
            make_channel(
                "metered",
                "Metered",
                ApiFormat::AnthropicMessages,
                vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            ),
        ]);

        let candidates = mw.find_candidates("claude-sonnet");
        let (ch, m) = mw.select_channel(&candidates, "claude-sonnet").unwrap();
        assert_eq!(ch.channel_id, "sub");
        assert!(m.billing.is_flat_fee());
    }

    #[test]
    fn test_select_metered_when_flatfee_exhausted_fallback() {
        let mw = make_middleware(vec![
            make_channel(
                "sub-exhausted",
                "Subscription",
                ApiFormat::AnthropicMessages,
                vec![ResolvedMapping {
                    mapping_id: "flatfee-exhausted".into(),
                    client_name: "claude-sonnet".into(),
                    upstream_name: "claude-sonnet-4-7".into(),
                    billing: ChannelBilling::FlatFee {
                        monthly_cost_hint: None,
                        quota: Some(Quota::MaxRequests { per_month: 0 }),
                        on_exhausted: ExhaustedAction::FallbackToMetered,
                    },
                }],
            ),
            make_channel(
                "metered",
                "Metered",
                ApiFormat::AnthropicMessages,
                vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            ),
        ]);

        let candidates = mw.find_candidates("claude-sonnet");
        let (ch, _m) = mw.select_channel(&candidates, "claude-sonnet").unwrap();
        assert_eq!(ch.channel_id, "metered");
    }

    #[test]
    fn test_select_block_when_flatfee_exhausted_block() {
        let mw = make_middleware(vec![make_channel(
            "sub-blocked",
            "Subscription",
            ApiFormat::AnthropicMessages,
            vec![ResolvedMapping {
                mapping_id: "flatfee-blocked".into(),
                client_name: "claude-sonnet".into(),
                upstream_name: "claude-sonnet-4-7".into(),
                billing: ChannelBilling::FlatFee {
                    monthly_cost_hint: None,
                    quota: Some(Quota::MaxRequests { per_month: 0 }),
                    on_exhausted: ExhaustedAction::Block,
                },
            }],
        )]);

        let candidates = mw.find_candidates("claude-sonnet");
        let err = mw.select_channel(&candidates, "claude-sonnet").unwrap_err();
        assert!(matches!(err, ProxyError::ChannelSelection { .. }));
    }

    #[test]
    fn test_select_all_unhealthy_returns_error() {
        let mw = make_middleware(vec![
            make_channel(
                "m1",
                "Metered1",
                ApiFormat::AnthropicMessages,
                vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            ),
            make_channel(
                "m2",
                "Metered2",
                ApiFormat::AnthropicMessages,
                vec![make_mapping_metered("claude-sonnet", "claude-haiku-4-5")],
            ),
        ]);

        mw.mark_unhealthy_immediate("m1");
        mw.mark_unhealthy_immediate("m2");

        let candidates = mw.find_candidates("claude-sonnet");
        let err = mw.select_channel(&candidates, "claude-sonnet").unwrap_err();
        assert!(matches!(err, ProxyError::ChannelSelection { .. }));
    }

    #[test]
    fn test_no_candidates_for_unknown_model() {
        let mw = make_middleware(vec![make_channel(
            "m1",
            "Metered1",
            ApiFormat::AnthropicMessages,
            vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
        )]);

        let candidates = mw.find_candidates("nonexistent-model");
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_disabled_channel_skipped() {
        let mw = ModelRouterMiddleware {
            quota_usage: Arc::new(DashMap::new()),
            channels: vec![ResolvedChannel {
                channel_id: "disabled".into(),
                channel_name: "Disabled".into(),
                url: "https://disabled.example.com".into(),
                api_key: "sk-test".into(),
                protocol: ApiFormat::AnthropicMessages,
                enabled: false,
                mappings: vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            }],
            health: Arc::new(DashMap::new()),
        };

        let candidates = mw.find_candidates("claude-sonnet");
        assert!(candidates.is_empty());
    }

    // ── Health tracking ─────────────────────────────────────────

    #[test]
    fn test_health_mark_unhealthy_then_healthy() {
        let mw = make_middleware(vec![]);
        mw.mark_unhealthy_immediate("ch1");
        assert!(!mw.is_healthy("ch1"));

        mw.mark_healthy("ch1");
        assert!(mw.is_healthy("ch1"));
    }

    #[test]
    fn test_health_cooldown_expired() {
        let mw = make_middleware(vec![]);
        mw.health.insert(
            "ch1".to_owned(),
            ChannelState {
                health: ChannelHealth::Unhealthy,
                consecutive_failures: 0,
                failed_at: Some(std::time::Instant::now() - Duration::from_secs(61)),
            },
        );
        assert!(mw.is_healthy("ch1"));
    }
}
