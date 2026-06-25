//! Model routing and channel selection middleware.
//!
//! Implements the selection strategy from specs/0003-channel-model.md Phase 1:
//! `FlatFee` channels with quota > 0 and healthy are preferred; `Metered` channels
//! serve as fallback. Health tracking uses a simple binary model with 60s
//! cooldown.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

mod types;

use std::sync::Arc;

use agent_proxy_rust_core::{
    ProxyError,
    extensions::{EXT_SELECTED_CHANNEL, EXT_SELECTED_MAPPING},
    middleware::ProxyMiddleware,
    types::{ApiFormat, ChannelConfig, ConnectionContext, ProxyRequest, ProxyResponse},
};
use agent_proxy_rust_storage::{ProtocolEntry, Storage};
use arc_swap::ArcSwap;
use async_trait::async_trait;
use dashmap::DashMap;
use secrecy::ExposeSecret;
use tracing::{debug, warn};
pub use types::{
    BillingDimension, ChannelBilling, ChannelHealth, ChannelState, ExhaustedAction, Pricing,
    PricingTier, Quota, QuotaUsage, TierPrice,
};

/// Parsed in-memory representation of a channel with its model mappings.
#[derive(Debug, Clone)]
pub struct ResolvedChannel {
    /// Channel ID.
    pub channel_id: String,
    /// Human-readable channel name.
    pub channel_name: String,
    /// API key for upstream requests.
    pub api_key: secrecy::SecretString,
    /// Supported protocols parsed from the channel's JSON configuration.
    pub protocols: Vec<ProtocolEntry>,
    /// Whether the channel is enabled.
    pub enabled: bool,
    /// Optional protocol override.
    pub force_protocol: Option<String>,
    /// Routing priority (higher = selected first).
    pub priority: u32,
    /// Model mappings bound to this channel.
    pub mappings: Vec<ResolvedMapping>,
}

impl ResolvedChannel {
    /// Returns the protocol identifiers supported by this channel.
    #[allow(dead_code)]
    fn supported_protocols(&self) -> Vec<&str> {
        self.protocols.iter().map(|p| p.protocol.as_str()).collect()
    }
}

/// Parsed in-memory representation of a model mapping.
#[derive(Debug, Clone)]
pub struct ResolvedMapping {
    /// Mapping ID for quota tracking.
    pub mapping_id: String,
    /// Client-facing model name.
    pub client_name: String,
    /// Upstream model name sent to the API.
    pub upstream_name: String,
    /// Billing type (flat-fee or metered).
    pub billing: ChannelBilling,
    /// Protocols this mapping is valid for. Empty = all protocols (backward compatible).
    pub allowed_protocols: Vec<String>,
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
    /// Pricing snapshot at selection time (metered only).
    pub pricing: Option<Pricing>,
    /// Serialized pricing for audit trail.
    pub pricing_snapshot_json: String,
}

/// Channel selection and model routing middleware.
pub struct ModelRouterMiddleware {
    channels: Arc<ArcSwap<Vec<ResolvedChannel>>>,
    health: Arc<DashMap<String, ChannelState>>,
    /// Per-mapping-id quota consumption counters. Keys match `model_mappings.id`.
    quota_usage: Arc<DashMap<String, QuotaUsage>>,
    /// Shared API key overrides: populated at startup from DB, updated at
    /// runtime by the admin API. The router looks up keys here first,
    /// falling back to the `ResolvedChannel::api_key`.
    channel_api_keys: Arc<DashMap<String, secrecy::SecretString>>,
}

impl std::fmt::Debug for ModelRouterMiddleware {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRouterMiddleware")
            .field("channels", &self.channels.load())
            .field("health", &self.health)
            .field("quota_usage", &self.quota_usage)
            .field("channel_api_keys", &"<DashMap>")
            .finish()
    }
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
            // Parse protocols JSON into typed entries; skip channels with no protocols
            let protocols: Vec<ProtocolEntry> =
                serde_json::from_str(&ch.protocols).unwrap_or_default();
            if protocols.is_empty() {
                warn!(
                    channel = %ch.id,
                    "channel has no protocols configured, skipping"
                );
                continue;
            }

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
                    let allowed_protocols: Vec<String> =
                        serde_json::from_str(&m.protocols).unwrap_or_default();
                    Some(ResolvedMapping {
                        mapping_id: m.id,
                        client_name: m.client_name,
                        upstream_name: m.upstream_name,
                        billing,
                        allowed_protocols,
                    })
                })
                .collect();

            // Normalize: trim trailing slashes and treat empty rewrite_path as None
            let protocols: Vec<ProtocolEntry> = protocols
                .into_iter()
                .map(|mut p| {
                    p.base_url = p.base_url.trim_end_matches('/').to_string();
                    p.rewrite_path = p.rewrite_path.filter(|rp| !rp.is_empty());
                    p
                })
                .collect();

            channels.push(ResolvedChannel {
                channel_id: ch.id,
                channel_name: ch.name,
                api_key: ch.api_key,
                protocols,
                enabled: ch.enabled,
                force_protocol: ch.force_protocol,
                priority: ch.priority,
                mappings,
            });
        }

        // Channels without API keys are naturally skipped by the router
        // (`has_api_key` check in `select_channel`). No need to simulate
        // health-check failures — they are simply unavailable.
        let health: Arc<DashMap<String, ChannelState>> = Arc::new(DashMap::new());
        for ch in &channels {
            if ch.api_key.expose_secret().is_empty() {
                tracing::info!(channel=%ch.channel_id, name=%ch.channel_name, "no API key — skipped");
            }
        }

        // Build the shared API-key override map from the DB values.
        // When the admin API updates a key, it writes here so the router
        // picks up the new key without a restart.
        let channel_api_keys: Arc<DashMap<String, secrecy::SecretString>> =
            Arc::new(DashMap::new());
        for ch in &channels {
            if !ch.api_key.expose_secret().is_empty() {
                channel_api_keys.insert(ch.channel_id.clone(), ch.api_key.clone());
            }
        }

        Ok(Self {
            channels: Arc::new(ArcSwap::from_pointee(channels)),
            health,
            quota_usage: Arc::new(DashMap::new()),
            channel_api_keys,
        })
    }

    /// Returns a reference to the in-memory health map.
    #[must_use]
    pub fn health_map(&self) -> &Arc<DashMap<String, ChannelState>> {
        &self.health
    }

    /// Returns a reference to the shared API-key override map.
    ///
    /// The admin API writes updated keys here so the router picks them up
    /// at request time without needing a restart.
    #[must_use]
    pub fn api_key_map(&self) -> &Arc<DashMap<String, secrecy::SecretString>> {
        &self.channel_api_keys
    }

    /// Returns a clone of the atomic channel list Arc so the admin API can
    /// trigger a hot-reload after mutations (priority, enabled, etc.).
    #[must_use]
    pub fn channels_swap(&self) -> Arc<ArcSwap<Vec<ResolvedChannel>>> {
        Arc::clone(&self.channels)
    }

    /// Finds all candidate mappings for a given client model name.
    fn find_candidates<'c>(
        channels: &'c [ResolvedChannel],
        client_name: &str,
    ) -> Vec<(&'c ResolvedChannel, &'c ResolvedMapping)> {
        let mut candidates = Vec::new();
        for ch in channels {
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
        let (mut flatfee, mut metered): (Vec<_>, Vec<_>) = candidates
            .iter()
            .partition(|(_, m)| m.billing.is_flat_fee());

        // Sort by channel priority: higher = selected first
        flatfee.sort_by_key(|(ch, _)| std::cmp::Reverse(ch.priority));
        metered.sort_by_key(|(ch, _)| std::cmp::Reverse(ch.priority));

        // Phase 1: try FlatFee channels first
        for (ch, m) in &flatfee {
            if !self.has_api_key(&ch.channel_id) {
                debug!(
                    channel = %ch.channel_id,
                    "skipping flat-fee channel: no API key configured"
                );
                continue;
            }
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
            if !self.has_api_key(&ch.channel_id) {
                debug!(
                    channel = %ch.channel_id,
                    "skipping metered channel: no API key configured"
                );
                continue;
            }
            if self.is_healthy(&ch.channel_id) {
                return Ok((ch, m));
            }
        }

        // All unhealthy — try any channel past cooldown that has a valid key
        for (ch, m) in candidates {
            if !self.has_api_key(&ch.channel_id) {
                continue;
            }
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

    /// Returns `true` when a channel has a usable API key.
    ///
    /// Checks the runtime override map first (so admin API key updates take
    /// effect immediately), then falls back to the key loaded from storage
    /// at startup. Channels without any key are permanently excluded from
    /// selection to avoid repeated 401 failures.
    fn has_api_key(&self, channel_id: &str) -> bool {
        // Runtime override from admin API takes precedence
        if let Some(key) = self.channel_api_keys.get(channel_id) {
            return !key.expose_secret().is_empty();
        }
        // Check the key loaded from storage at startup
        self.channels
            .load()
            .iter()
            .any(|ch| ch.channel_id == channel_id && !ch.api_key.expose_secret().is_empty())
    }

    fn is_healthy(&self, channel_id: &str) -> bool {
        self.health
            .get(channel_id)
            .is_none_or(|s| s.is_tryable_past_cooldown())
    }

    fn is_tryable_past_cooldown(&self, channel_id: &str) -> bool {
        self.health
            .get(channel_id)
            .is_none_or(|s| s.is_tryable_past_cooldown())
    }

    fn mark_healthy(&self, channel_id: &str) {
        if let Some(mut state) = self.health.get_mut(channel_id) {
            state.record_success();
        }
    }

    /// Records a request failure. After 3 consecutive failures the
    /// channel is marked Unhealthy with exponential backoff.
    #[allow(dead_code)]
    fn record_failure(&self, channel_id: &str) {
        let mut state = self.health.entry(channel_id.to_owned()).or_default();
        state.record_failure();
    }

    /// Marks a channel as rate-limited with a short cooldown.
    ///
    /// The channel will be retried after 30 seconds instead of the
    /// regular exponential backoff.
    fn mark_rate_limited(&self, channel_id: &str) {
        let mut state = self.health.entry(channel_id.to_owned()).or_default();
        state.mark_rate_limited();
    }

    /// Forces a channel to Unhealthy immediately (e.g. 5xx server error).
    fn mark_unhealthy_immediate(&self, channel_id: &str) {
        self.health
            .entry(channel_id.to_owned())
            .or_default()
            .mark_unhealthy();
    }
}

/// Hot-reloads the in-memory channel list from storage and atomically swaps
/// it into `channels_swap`.
///
/// Called by the admin API after channel mutations (create, update, delete)
/// so that priority, enabled, protocol, and mapping changes take effect
/// immediately without requiring a proxy restart.
///
/// # Errors
///
/// Returns `ProxyError::Internal` if the storage backend fails or if any
/// channel has an unrecognized protocol string.
pub async fn reload_channels_from_storage(
    storage: &dyn Storage,
    channels_swap: &ArcSwap<Vec<ResolvedChannel>>,
) -> Result<(), ProxyError> {
    let storage_channels = storage
        .list_channels(None)
        .await
        .map_err(|e| ProxyError::Internal(e.into()))?;

    let mut channels = Vec::with_capacity(storage_channels.len());

    for ch in storage_channels {
        let protocols: Vec<ProtocolEntry> = serde_json::from_str(&ch.protocols).unwrap_or_default();
        if protocols.is_empty() {
            warn!(
                channel = %ch.id,
                "channel has no protocols configured, skipping"
            );
            continue;
        }

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
                let allowed_protocols: Vec<String> =
                    serde_json::from_str(&m.protocols).unwrap_or_default();
                Some(ResolvedMapping {
                    mapping_id: m.id,
                    client_name: m.client_name,
                    upstream_name: m.upstream_name,
                    billing,
                    allowed_protocols,
                })
            })
            .collect();

        let protocols: Vec<ProtocolEntry> = protocols
            .into_iter()
            .map(|mut p| {
                p.base_url = p.base_url.trim_end_matches('/').to_string();
                p.rewrite_path = p.rewrite_path.filter(|rp| !rp.is_empty());
                p
            })
            .collect();

        channels.push(ResolvedChannel {
            channel_id: ch.id,
            channel_name: ch.name,
            api_key: ch.api_key,
            protocols,
            enabled: ch.enabled,
            force_protocol: ch.force_protocol,
            priority: ch.priority,
            mappings,
        });
    }

    channels_swap.store(Arc::new(channels));
    tracing::info!(count = channels_swap.load().len(), "channels hot-reloaded");
    Ok(())
}

#[async_trait]
impl ProxyMiddleware for ModelRouterMiddleware {
    #[allow(clippy::too_many_lines)]
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
            debug!(
                path = %req.path,
                body = %String::from_utf8_lossy(&req.body),
                "request body missing 'model' field"
            );
            return Err(ProxyError::BadRequest(
                "request body missing 'model' field".into(),
            ));
        }

        // Hold the Arc guard for the entire request so references remain valid.
        let channels = self.channels.load();
        let candidates = Self::find_candidates(&channels, &client_name);

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

        // Determine target protocol using the 3-step resolution
        let mut target_protocol = resolve_target_protocol(
            channel.force_protocol.as_deref(),
            ctx.detected_format,
            &channel.protocols,
        )?;

        // ── Protocol-model compatibility check ───────────────────────
        //
        // If the mapping declares protocol constraints (e.g. a model only
        // works on openai_chat), validate that the resolved protocol is
        // compatible. When it isn't, switch to the first protocol that
        // both the mapping and the channel support — the bridge middleware
        // will handle format conversion.
        if !mapping.allowed_protocols.is_empty() {
            let target_str = protocol_to_str(target_protocol);
            if !mapping.allowed_protocols.iter().any(|p| p == target_str) {
                // Resolved protocol is not in the mapping's allowed list.
                // Find the first protocol the channel supports that the
                // mapping also allows.
                let compatible = channel.protocols.iter().find(|pe| {
                    mapping
                        .allowed_protocols
                        .iter()
                        .any(|ap| ap == &pe.protocol)
                });
                if let Some(entry) = compatible {
                    debug!(
                        channel = %channel.channel_id,
                        mapping = %mapping.mapping_id,
                        resolved = %target_str,
                        switched_to = %entry.protocol,
                        "mapping protocol constraint: switching target protocol"
                    );
                    target_protocol = parse_protocol(&entry.protocol)?;
                } else {
                    let channel_prots: Vec<&str> = channel
                        .protocols
                        .iter()
                        .map(|p| p.protocol.as_str())
                        .collect();
                    return Err(ProxyError::Internal(anyhow::anyhow!(
                        "mapping '{}' protocol constraint {:?} incompatible with channel \
                         protocols {channel_prots:?}",
                        mapping.mapping_id,
                        mapping.allowed_protocols,
                    )));
                }
            }
        }

        ctx.target_protocol = Some(target_protocol);

        // Resolve upstream URL from protocols entries
        let (base_url, rewrite_path) = resolve_upstream_url(target_protocol, &channel.protocols)?;

        // Look up the API key from the shared override map first (so admin
        // API updates take effect without a restart), falling back to the
        // key that was loaded at startup.
        let api_key = self
            .channel_api_keys
            .get(&channel.channel_id)
            .map_or_else(|| channel.api_key.clone(), |r| r.clone());

        // Write ChannelConfig to extensions
        ctx.insert(
            EXT_SELECTED_CHANNEL,
            ChannelConfig {
                url: base_url,
                api_key,
                protocol: target_protocol,
                name: channel.channel_name.clone(),
                rewrite_path,
            },
        );

        // Extract pricing snapshot from billing
        let (pricing, snapshot_json) = match &mapping.billing {
            ChannelBilling::Metered { pricing } => {
                let json = serde_json::to_string(pricing).unwrap_or_default();
                (Some(pricing.clone()), json)
            }
            ChannelBilling::FlatFee { .. } => (None, r#"{"type":"flat_fee"}"#.to_string()),
        };

        // Write selected mapping info to extensions
        ctx.insert(
            EXT_SELECTED_MAPPING,
            SelectedMappingInfo {
                channel_id: channel.channel_id.clone(),
                mapping_id: mapping.mapping_id.clone(),
                client_name: mapping.client_name.clone(),
                upstream_name: mapping.upstream_name.clone(),
                is_flat_fee: mapping.billing.is_flat_fee(),
                pricing,
                pricing_snapshot_json: snapshot_json,
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

        if res.status.is_server_error() || res.status == http::StatusCode::UNAUTHORIZED {
            // 5xx: immediate unhealthy — server is down
            // 401: authentication failure — API key is missing, invalid, or expired
            warn!(
                channel = %channel_id,
                status = %res.status,
                "upstream {}, marking channel unhealthy immediately",
                if res.status.is_server_error() { "5xx" } else { "401 Unauthorized" }
            );
            self.mark_unhealthy_immediate(&channel_id);
        } else if res.status.is_client_error() && res.status.as_u16() != 429 {
            // 4xx (except 401, 429): client errors — not the channel's fault
            debug!(
                channel = %channel_id,
                status = %res.status,
                "client error, not counting as channel failure"
            );
        } else if res.status == http::StatusCode::TOO_MANY_REQUESTS {
            // 429: rate limit — short cooldown, retry quickly
            warn!(
                channel = %channel_id,
                "upstream 429 rate limit, applying 30s cooldown"
            );
            self.mark_rate_limited(&channel_id);
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

/// Resolves the target protocol for a request using a 3-step strategy:
///
/// 1. If `force_protocol` is set, validate it exists in `protocols` and use it.
/// 2. Otherwise, if the client's `detected_format` matches a protocol in `protocols`, use it
///    (passthrough, no conversion).
/// 3. Otherwise, fall back to the first protocol in `protocols`.
///
/// # Errors
///
/// Returns `ProxyError::Internal` if `force_protocol` is set but not found in
/// `protocols`, if the matched protocol string is unrecognized, or if
/// `protocols` is empty.
fn resolve_target_protocol(
    force_protocol: Option<&str>,
    detected_format: Option<ApiFormat>,
    protocols: &[ProtocolEntry],
) -> Result<ApiFormat, ProxyError> {
    // Step 1: force_protocol must exist in protocols
    if let Some(fp) = force_protocol {
        let target = parse_protocol(fp)?;
        let target_str = protocol_to_str(target);
        if !protocols.iter().any(|p| p.protocol == target_str) {
            return Err(ProxyError::Internal(anyhow::anyhow!(
                "force_protocol '{fp}' not found in channel protocols"
            )));
        }
        return Ok(target);
    }

    // Step 2: if client protocol is supported, passthrough
    if let Some(df) = detected_format {
        let df_str = protocol_to_str(df);
        if !df_str.is_empty() && protocols.iter().any(|p| p.protocol == df_str) {
            return Ok(df);
        }
    }

    // Step 3: fallback to first protocol
    if let Some(first) = protocols.first()
        && !first.protocol.is_empty()
    {
        return parse_protocol(&first.protocol);
    }

    Err(ProxyError::Internal(anyhow::anyhow!(
        "channel has no protocols configured"
    )))
}

/// Resolves the upstream URL for a given protocol from the channel's `protocols` entries.
///
/// Returns the `(base_url, rewrite_path)` tuple for the matching protocol entry.
/// `rewrite_path` is `None` when the entry does not specify a path rewrite — the
/// original request path should be passed through.
///
/// # Errors
///
/// Returns `ProxyError::Internal` if no entry matches the target protocol or
/// if the matched entry has an empty `base_url`.
fn resolve_upstream_url(
    protocol: ApiFormat,
    protocols: &[ProtocolEntry],
) -> Result<(String, Option<String>), ProxyError> {
    let target = protocol_to_str(protocol);

    let entry = protocols
        .iter()
        .find(|e| e.protocol == target)
        .ok_or_else(|| {
            ProxyError::Internal(anyhow::anyhow!(
                "no protocol entry for '{target}' in channel protocols"
            ))
        })?;

    if entry.base_url.is_empty() {
        return Err(ProxyError::Internal(anyhow::anyhow!(
            "protocol entry '{target}' has empty base_url"
        )));
    }

    Ok((entry.base_url.clone(), entry.rewrite_path.clone()))
}

/// Returns the `snake_case` string representation for an [`ApiFormat`] variant,
/// matching the format used in the `protocols` JSON column.
fn protocol_to_str(protocol: ApiFormat) -> &'static str {
    match protocol {
        ApiFormat::AnthropicMessages => "anthropic_messages",
        ApiFormat::OpenaiChat => "openai_chat",
        ApiFormat::OpenaiResponses => "openai_responses",
        _ => "",
    }
}

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
    clippy::unchecked_time_subtraction,
    clippy::panic
)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::types::ChannelHealth;

    fn make_channel(
        id: &str,
        name: &str,
        protocols: Vec<ProtocolEntry>,
        mappings: Vec<ResolvedMapping>,
    ) -> ResolvedChannel {
        ResolvedChannel {
            channel_id: id.into(),
            channel_name: name.into(),
            api_key: secrecy::SecretString::from("sk-test"),
            protocols,
            enabled: true,
            force_protocol: None,
            priority: 0,
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
            allowed_protocols: Vec::new(),
        }
    }

    fn make_protocols(protocol: ApiFormat, base_url: &str) -> Vec<ProtocolEntry> {
        vec![ProtocolEntry {
            protocol: protocol_to_str(protocol).to_string(),
            base_url: base_url.to_string(),
            rewrite_path: None,
        }]
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
                    currency: "USD".to_string(),
                },
            },
            allowed_protocols: Vec::new(),
        }
    }

    fn make_middleware(channels: Vec<ResolvedChannel>) -> ModelRouterMiddleware {
        ModelRouterMiddleware {
            channels: Arc::new(ArcSwap::from_pointee(channels)),
            health: Arc::new(DashMap::new()),
            quota_usage: Arc::new(DashMap::new()),
            channel_api_keys: Arc::new(DashMap::new()),
        }
    }

    // ── Selection strategy ──────────────────────────────────────

    #[test]
    fn test_select_flatfee_has_quota_and_healthy() {
        let mw = make_middleware(vec![
            make_channel(
                "sub",
                "Subscription",
                make_protocols(ApiFormat::AnthropicMessages, "https://sub.example.com"),
                vec![make_mapping_flatfee(
                    "claude-sonnet",
                    "claude-sonnet-4-7",
                    ExhaustedAction::FallbackToMetered,
                )],
            ),
            make_channel(
                "metered",
                "Metered",
                make_protocols(ApiFormat::AnthropicMessages, "https://metered.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            ),
        ]);

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
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
                make_protocols(ApiFormat::AnthropicMessages, "https://sub.example.com"),
                vec![ResolvedMapping {
                    mapping_id: "flatfee-exhausted".into(),
                    client_name: "claude-sonnet".into(),
                    upstream_name: "claude-sonnet-4-7".into(),
                    billing: ChannelBilling::FlatFee {
                        monthly_cost_hint: None,
                        quota: Some(Quota::MaxRequests { per_month: 0 }),
                        on_exhausted: ExhaustedAction::FallbackToMetered,
                    },
                    allowed_protocols: Vec::new(),
                }],
            ),
            make_channel(
                "metered",
                "Metered",
                make_protocols(ApiFormat::AnthropicMessages, "https://metered.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            ),
        ]);

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        let (ch, _m) = mw.select_channel(&candidates, "claude-sonnet").unwrap();
        assert_eq!(ch.channel_id, "metered");
    }

    #[test]
    fn test_select_block_when_flatfee_exhausted_block() {
        let mw = make_middleware(vec![make_channel(
            "sub-blocked",
            "Subscription",
            make_protocols(ApiFormat::AnthropicMessages, "https://sub.example.com"),
            vec![ResolvedMapping {
                mapping_id: "flatfee-blocked".into(),
                client_name: "claude-sonnet".into(),
                upstream_name: "claude-sonnet-4-7".into(),
                billing: ChannelBilling::FlatFee {
                    monthly_cost_hint: None,
                    quota: Some(Quota::MaxRequests { per_month: 0 }),
                    on_exhausted: ExhaustedAction::Block,
                },
                allowed_protocols: Vec::new(),
            }],
        )]);

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        let err = mw.select_channel(&candidates, "claude-sonnet").unwrap_err();
        assert!(matches!(err, ProxyError::ChannelSelection { .. }));
    }

    #[test]
    fn test_select_all_unhealthy_returns_error() {
        let mw = make_middleware(vec![
            make_channel(
                "m1",
                "Metered1",
                make_protocols(ApiFormat::AnthropicMessages, "https://m1.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            ),
            make_channel(
                "m2",
                "Metered2",
                make_protocols(ApiFormat::AnthropicMessages, "https://m2.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-haiku-4-5")],
            ),
        ]);

        mw.mark_unhealthy_immediate("m1");
        mw.mark_unhealthy_immediate("m2");

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        let err = mw.select_channel(&candidates, "claude-sonnet").unwrap_err();
        assert!(matches!(err, ProxyError::ChannelSelection { .. }));
    }

    #[test]
    fn test_no_candidates_for_unknown_model() {
        let mw = make_middleware(vec![make_channel(
            "m1",
            "Metered1",
            make_protocols(ApiFormat::AnthropicMessages, "https://m1.example.com"),
            vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
        )]);

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "nonexistent-model");
        assert!(candidates.is_empty());
    }

    #[test]
    fn test_disabled_channel_skipped() {
        let mw = ModelRouterMiddleware {
            quota_usage: Arc::new(DashMap::new()),
            channels: Arc::new(ArcSwap::from_pointee(vec![ResolvedChannel {
                channel_id: "disabled".into(),
                channel_name: "Disabled".into(),
                api_key: secrecy::SecretString::from("sk-test"),
                protocols: make_protocols(
                    ApiFormat::AnthropicMessages,
                    "https://disabled.example.com",
                ),
                enabled: false,
                force_protocol: None,
                priority: 0,
                mappings: vec![make_mapping_metered("claude-sonnet", "claude-opus-4-7")],
            }])),
            health: Arc::new(DashMap::new()),
            channel_api_keys: Arc::new(DashMap::new()),
        };

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        assert!(candidates.is_empty());
    }

    // ── resolve_upstream_url ────────────────────────────────────

    #[test]
    fn test_resolve_upstream_url_returns_base_url_and_rewrite_path() {
        let protocols = vec![ProtocolEntry {
            protocol: "openai_chat".into(),
            base_url: "https://api.deepseek.com".into(),
            rewrite_path: Some("/chat/completions".into()),
        }];
        let (base_url, rewrite_path) =
            resolve_upstream_url(ApiFormat::OpenaiChat, &protocols).unwrap();
        assert_eq!(base_url, "https://api.deepseek.com");
        assert_eq!(rewrite_path, Some("/chat/completions".into()));
    }

    #[test]
    fn test_resolve_upstream_url_no_rewrite_path() {
        let protocols = vec![ProtocolEntry {
            protocol: "openai_chat".into(),
            base_url: "https://api.deepseek.com".into(),
            rewrite_path: None,
        }];
        let (base_url, rewrite_path) =
            resolve_upstream_url(ApiFormat::OpenaiChat, &protocols).unwrap();
        assert_eq!(base_url, "https://api.deepseek.com");
        assert_eq!(rewrite_path, None);
    }

    #[test]
    fn test_resolve_upstream_url_no_matching_protocol() {
        let protocols = vec![ProtocolEntry {
            protocol: "openai_chat".into(),
            base_url: "https://api.deepseek.com".into(),
            rewrite_path: None,
        }];
        let result = resolve_upstream_url(ApiFormat::AnthropicMessages, &protocols);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_upstream_url_empty_base_url() {
        let protocols = vec![ProtocolEntry {
            protocol: "openai_chat".into(),
            base_url: String::new(),
            rewrite_path: None,
        }];
        let result = resolve_upstream_url(ApiFormat::OpenaiChat, &protocols);
        assert!(result.is_err());
    }

    // ── resolve_target_protocol ─────────────────────────────────

    fn make_protocol_entries(entries: &[(&str, &str)]) -> Vec<ProtocolEntry> {
        entries
            .iter()
            .map(|&(protocol, base_url)| ProtocolEntry {
                protocol: protocol.to_owned(),
                base_url: base_url.to_owned(),
                rewrite_path: None,
            })
            .collect()
    }

    #[test]
    fn test_resolve_target_protocol_force_valid() {
        let protocols = make_protocol_entries(&[
            ("openai_chat", "https://api.example.com"),
            ("anthropic_messages", "https://api.example.com/anthropic"),
        ]);
        let result = resolve_target_protocol(
            Some("openai_chat"),
            Some(ApiFormat::AnthropicMessages),
            &protocols,
        )
        .unwrap();
        assert_eq!(result, ApiFormat::OpenaiChat);
    }

    #[test]
    fn test_resolve_target_protocol_force_not_in_protocols() {
        let protocols = make_protocol_entries(&[("openai_chat", "https://api.example.com")]);
        let result = resolve_target_protocol(Some("anthropic_messages"), None, &protocols);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_target_protocol_passthrough_client_match() {
        let protocols = make_protocol_entries(&[
            ("openai_chat", "https://api.example.com"),
            ("anthropic_messages", "https://api.example.com/anthropic"),
        ]);
        let result =
            resolve_target_protocol(None, Some(ApiFormat::AnthropicMessages), &protocols).unwrap();
        assert_eq!(result, ApiFormat::AnthropicMessages);
    }

    #[test]
    fn test_resolve_target_protocol_fallback_to_first() {
        let protocols = make_protocol_entries(&[
            ("openai_chat", "https://api.example.com"),
            ("anthropic_messages", "https://api.example.com/anthropic"),
        ]);
        // Client sends a protocol not in the list → fallback to first
        let result =
            resolve_target_protocol(None, Some(ApiFormat::OpenaiResponses), &protocols).unwrap();
        assert_eq!(result, ApiFormat::OpenaiChat);
    }

    #[test]
    fn test_resolve_target_protocol_no_client_format() {
        let protocols = make_protocol_entries(&[("openai_chat", "https://api.example.com")]);
        // No detected_format → fallback to first
        let result = resolve_target_protocol(None, None, &protocols).unwrap();
        assert_eq!(result, ApiFormat::OpenaiChat);
    }

    #[test]
    fn test_resolve_target_protocol_empty_protocols() {
        let result = resolve_target_protocol(None, Some(ApiFormat::AnthropicMessages), &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_resolve_target_protocol_force_with_empty_protocols() {
        let result = resolve_target_protocol(Some("anthropic_messages"), None, &[]);
        assert!(result.is_err());
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

    // ── API key filtering ───────────────────────────────────────

    fn make_channel_with_key(
        id: &str,
        api_key: &str,
        protocols: Vec<ProtocolEntry>,
        mappings: Vec<ResolvedMapping>,
    ) -> ResolvedChannel {
        ResolvedChannel {
            channel_id: id.into(),
            channel_name: id.into(),
            api_key: secrecy::SecretString::from(api_key),
            protocols,
            enabled: true,
            force_protocol: None,
            priority: 10,
            mappings,
        }
    }

    #[test]
    fn test_channel_with_empty_api_key_is_skipped() {
        let mw = make_middleware(vec![
            make_channel_with_key(
                "no-key",
                "",
                make_protocols(ApiFormat::AnthropicMessages, "https://no-key.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-sonnet-v1")],
            ),
            make_channel_with_key(
                "has-key",
                "sk-valid",
                make_protocols(ApiFormat::AnthropicMessages, "https://has-key.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-sonnet-v2")],
            ),
        ]);

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        assert_eq!(candidates.len(), 2);
        let (ch, _m) = mw.select_channel(&candidates, "claude-sonnet").unwrap();
        assert_eq!(
            ch.channel_id, "has-key",
            "should skip channel with empty API key"
        );
    }

    #[test]
    fn test_all_channels_empty_key_returns_error() {
        let mw = make_middleware(vec![make_channel_with_key(
            "no-key-1",
            "",
            make_protocols(ApiFormat::AnthropicMessages, "https://no1.example.com"),
            vec![make_mapping_metered("claude-sonnet", "claude-sonnet-v1")],
        )]);

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        let err = mw.select_channel(&candidates, "claude-sonnet").unwrap_err();
        assert!(
            matches!(err, ProxyError::ChannelSelection { .. }),
            "should error when no channel has a valid API key"
        );
    }

    #[test]
    fn test_has_api_key_runtime_override() {
        let mw = make_middleware(vec![make_channel_with_key(
            "no-key-stored",
            "",
            make_protocols(ApiFormat::AnthropicMessages, "https://no-key.example.com"),
            vec![make_mapping_metered("claude-sonnet", "claude-sonnet-v1")],
        )]);

        // Without override: should be excluded
        assert!(!mw.has_api_key("no-key-stored"));

        // With runtime override: should be available
        mw.channel_api_keys.insert(
            "no-key-stored".into(),
            secrecy::SecretString::from("sk-override"),
        );
        assert!(mw.has_api_key("no-key-stored"));
    }

    #[test]
    fn test_empty_key_skipped_in_fallback_phase() {
        // All channels unhealthy but "has-key" past cooldown, "no-key" also past cooldown
        let mw = make_middleware(vec![
            make_channel_with_key(
                "no-key",
                "",
                make_protocols(ApiFormat::AnthropicMessages, "https://no-key.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-sonnet-v1")],
            ),
            make_channel_with_key(
                "has-key",
                "sk-valid",
                make_protocols(ApiFormat::AnthropicMessages, "https://has-key.example.com"),
                vec![make_mapping_metered("claude-sonnet", "claude-sonnet-v2")],
            ),
        ]);

        // Mark both unhealthy (past cooldown: 61s ago)
        for ch_id in ["no-key", "has-key"] {
            mw.health.insert(
                ch_id.to_owned(),
                ChannelState {
                    health: ChannelHealth::Unhealthy,
                    consecutive_failures: 1,
                    failed_at: Some(std::time::Instant::now() - Duration::from_secs(61)),
                },
            );
        }

        let channels = mw.channels.load();
        let candidates = ModelRouterMiddleware::find_candidates(&channels, "claude-sonnet");
        // Fallback should skip "no-key" and pick "has-key"
        let (ch, _m) = mw.select_channel(&candidates, "claude-sonnet").unwrap();
        assert_eq!(
            ch.channel_id, "has-key",
            "fallback should skip empty-key channel"
        );
    }
}
