use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// An upstream AI provider (e.g. "Anthropic", "`OpenAI`").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// Unique provider identifier.
    pub id: String,
    /// Human-readable name (e.g. "Anthropic").
    pub name: String,
    /// When the provider was added (RFC 3339).
    #[serde(default)]
    pub created_at: String,
}

/// A model offered by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// Unique model identifier.
    pub id: String,
    /// Provider this model belongs to.
    pub provider_id: String,
    /// Client-facing model name.
    pub client_name: String,
    /// Input price per 1M tokens.
    pub price_input: f64,
    /// Output price per 1M tokens.
    pub price_output: f64,
    /// Default pricing currency.
    pub currency: String,
    /// Maximum context window in tokens.
    pub context_window: i64,
    /// When the model was added (RFC 3339).
    pub created_at: String,
    /// Number of channels that can serve this model.
    #[serde(default)]
    pub channel_count: u32,
}

/// An upstream AI provider channel with its API key and protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// Unique channel identifier (e.g. "anthropic-official").
    pub id: String,
    /// Human-readable name (e.g. "Anthropic Official").
    pub name: String,
    /// API key for authenticating with the upstream.
    #[serde(
        rename = "apiKeyRef",
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub api_key: SecretString,
    /// Protocol spoken by the upstream (default).
    ///
    /// **Deprecated for runtime use.** Protocol selection is now derived from
    /// [`protocols`](Self::protocols) at request time. This field is retained
    /// for DB backward compatibility and admin UI display only.
    pub protocol: String,
    /// Supported protocols JSON: `[{"protocol":"...","baseUrl":"...","rewritePath":"..."}]`.
    pub protocols: String,
    /// Whether this channel was seeded by the system.
    pub is_builtin: bool,
    /// Whether this channel is active.
    pub enabled: bool,
    /// When the channel was first created (unix timestamp).
    pub created_at: i64,
    /// When the channel was last modified (unix timestamp).
    pub updated_at: i64,
    /// Current health status: "Healthy", "Degraded", or "Cooldown".
    pub health_status: String,
    /// If in cooldown, when it ends (RFC 3339).
    pub cooldown_until: Option<String>,
    /// Number of consecutive failures.
    pub consecutive_failures: u32,
    /// Billing type: "Metered" or "`FlatFee`".
    pub billing_type: String,
    /// Monthly request quota (if applicable).
    pub monthly_quota: Option<u64>,
    /// Quota exhaustion policy: "fallback" or "block".
    pub quota_policy: String,
    /// Channel priority for weighted selection.
    pub priority: u32,
    /// When set, forces all traffic through this protocol regardless of client format.
    /// Used for testing protocol bridge functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_protocol: Option<String>,
}

/// Maps a client-facing model name to an upstream model name with pricing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMapping {
    /// Unique mapping identifier.
    pub id: String,
    /// The channel this mapping belongs to.
    pub channel_id: String,
    /// Model name as seen by the client (e.g. "claude-sonnet").
    pub client_name: String,
    /// Model name sent to the upstream API (e.g. "claude-sonnet-4-7").
    pub upstream_name: String,
    /// Billing model: "metered" or "flatfee".
    pub billing: String,
    /// Serialized pricing configuration (JSON).
    pub pricing_json: String,
    /// Selection weight for weighted random (Phase 2). Default 100.
    pub weight: u32,
    /// Whether this mapping is active.
    pub enabled: bool,
    /// Protocols this mapping is valid for (JSON array of strings).
    ///
    /// Empty array `[]` means the mapping works with all protocols the channel
    /// supports (backward compatible). When non-empty, only the listed protocols
    /// are considered valid for this mapping — the router will bridge to a
    /// compatible protocol when the client's format does not match.
    ///
    /// Example: `'["openai_chat"]'` constrains a mapping to the `OpenAI` Chat
    /// protocol only, preventing the router from forwarding requests via
    /// `anthropic_messages` when the upstream model does not support that
    /// protocol.
    #[serde(default, skip_serializing_if = "is_empty_protocols")]
    pub protocols: String,
}

/// Returns `true` when the protocols string is empty or represents an empty
/// JSON array — used by serde `skip_serializing_if` to keep the wire format
/// compact.
fn is_empty_protocols(s: &str) -> bool {
    s.is_empty() || s == "[]"
}

/// A single proxied request with token usage and cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    /// UUID v7 primary key.
    pub id: String,
    /// Channel used for this request (proxy channel ID, e.g. "deepseek").
    pub channel_id: String,
    /// Human-readable upstream channel name (e.g. "`DeepSeek` Official").
    #[serde(default)]
    pub upstream_channel: String,
    /// Upstream model name sent to the API (e.g. "deepseek-v4-pro").
    #[serde(default)]
    pub upstream_model: String,
    /// Request processing time in milliseconds (from request arrival to response return).
    #[serde(default)]
    pub request_time_ms: i64,
    /// Project path or identifier.
    pub project: String,
    /// User who made the request.
    pub user_id: String,
    /// Agent type: "`ClaudeCode`", "Codex", etc.
    pub agent_type: String,
    /// Input/prompt tokens consumed.
    pub input_tokens: i64,
    /// Output/completion tokens consumed.
    pub output_tokens: i64,
    /// Tokens written to the provider's prompt cache.
    pub cache_write_tokens: i64,
    /// Tokens read from the provider's prompt cache.
    pub cache_read_tokens: i64,
    /// Extended thinking tokens consumed.
    pub thinking_tokens: i64,
    /// Actual monetary cost of this request.
    pub cost: f64,
    /// Tokens saved by schema compression.
    pub schema_saved_tokens: i64,
    /// Tokens saved by response compression.
    pub response_saved_tokens: i64,
    /// Tokens saved by RTK token optimization.
    pub rtk_saved_tokens: i64,
    /// Token count before tokenless compression.
    pub pre_compress_tokens: i64,
    /// Token count after tokenless compression.
    pub post_compress_tokens: i64,
    /// Tokens saved by tokenless compression.
    pub compression_tokens_saved: i64,
    /// Currency of `cost`: "USD", "CNY", "credits".
    pub unit: String,
    /// Serialized pricing snapshot for audit trail.
    #[serde(default)]
    pub pricing_snapshot_json: String,
    /// When the request was completed (RFC 3339).
    pub timestamp: String,
    /// Session ID from `X-Claude-Code-Session-Id` header (for billing correlation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Estimated tokens before any compression (tokenless + proxy layers).
    #[serde(default)]
    pub before_tokens: i64,
    /// Actual tokens consumed by the upstream API (input + output).
    #[serde(default)]
    pub after_tokens: i64,
    /// Total tokens saved across all compression layers.
    #[serde(default)]
    pub tokens_saved: i64,
    /// JSON array breakdown of each compression operation.
    ///
    /// Each element:
    /// ```json
    /// {
    ///   "op": "compress-schema",      // opType — see report.rs for full enum
    ///   "method": "ToonHrv",          // compression strategy
    ///   "beforeTokens": 1500,
    ///   "afterTokens": 700,
    ///   "savedTokens": 800,
    ///   "beforeBytes": 6000,
    ///   "afterBytes": 2800,
    ///   "savedBytes": 3200
    /// }
    /// ```
    ///
    /// See [`crate::report`] module docs for the complete opType / method lookup table.
    #[serde(default)]
    pub compression_breakdown_json: String,
}

/// A monthly subscription fee for a flat-fee channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionFee {
    /// Auto-increment primary key.
    pub id: i64,
    /// Channel display name.
    pub channel_name: String,
    /// Month in "YYYY-MM" format.
    pub month: String,
    /// Monthly subscription price.
    pub monthly_price: f64,
    /// Currency code (e.g. "USD").
    pub currency: String,
}

/// A switch/redirect event between channels.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchLog {
    /// UUID primary key.
    pub id: String,
    /// Channel that traffic was switched from.
    pub from_channel_id: String,
    /// Channel that traffic was switched to.
    pub to_channel_id: String,
    /// Reason for the switch.
    pub reason: String,
    /// Optional reference to a cost record at switch time.
    pub cost_record_id: Option<String>,
    /// When the switch occurred (RFC 3339).
    pub created_at: String,
}

/// Optional filters for querying cost records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostFilter {
    /// Filter by project path.
    pub project_path: Option<String>,
    /// Filter by model name.
    pub model_name: Option<String>,
    /// Filter by channel name.
    pub channel_name: Option<String>,
    /// Filter by time range.
    pub time_range: Option<TimeRange>,
    /// Maximum number of records to return.
    pub limit: Option<u32>,
    /// Number of records to skip.
    pub offset: Option<u32>,
}

/// A time range bounded by start and end timestamps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    /// Start of the range (inclusive).
    pub start: i64,
    /// End of the range (exclusive).
    pub end: i64,
}

/// Grouping dimension for cost aggregation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CostGroupBy {
    /// Aggregate per project path.
    Project,
    /// Aggregate per model name.
    Model,
    /// Aggregate per channel name.
    Channel,
    /// Aggregate per project × model × month.
    ProjectModelMonth,
    /// Aggregate per project × model × hour (for trend charts).
    ProjectModelHour,
    /// Aggregate per hour only (for trend chart, all models merged).
    Hourly,
}

/// Aggregated cost summary row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAggregate {
    /// The group key (depends on `CostGroupBy`).
    pub group_key: String,
    /// Sum of input tokens.
    pub total_input_tokens: i64,
    /// Sum of output tokens.
    pub total_output_tokens: i64,
    /// Sum of actual costs.
    pub total_actual_cost: f64,
    /// Sum of compression tokens saved.
    pub total_compression_tokens_saved: i64,
    /// Number of requests in this group.
    pub request_count: i64,
}

/// An enabled channel with its bound models, used by the Admin API
/// `GET /admin/available-channels` endpoint for Claude direct-connect mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableChannelInfo {
    /// Channel ID.
    pub channel_id: String,
    /// Human-readable channel name.
    pub channel_name: String,
    /// Default protocol for this channel.
    pub protocol: String,
    /// Protocols JSON with `base_url` and `rewrite_path`.
    pub protocols: String,
    /// Current health status.
    pub health_status: String,
    /// Whether the channel is enabled.
    pub enabled: bool,
    /// Models bound to this channel.
    pub models: Vec<AvailableModelInfo>,
}

/// Compression savings report for a project.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressionSavingsReport {
    /// Total tokens saved by schema compression.
    pub schema_saved_tokens: i64,
    /// Total tokens saved by response compression.
    pub response_saved_tokens: i64,
    /// Total tokens saved by RTK optimization.
    pub rtk_saved_tokens: i64,
    /// Total tokens saved across all compression layers.
    pub total_saved_tokens: i64,
}

/// A model bound to a channel, returned as part of [`AvailableChannelInfo`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableModelInfo {
    /// Model mapping ID.
    pub mapping_id: String,
    /// Client-facing model name.
    pub client_name: String,
    /// Upstream model name sent to the provider.
    pub upstream_name: String,
}

// ── Seed Data ────────────────────────────────────────────────────────

/// Status of the local seed data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedStatus {
    /// Current local version.
    pub local_version: u32,
    /// Latest remote version (if checked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_version: Option<u32>,
    /// Whether a newer remote version is available.
    pub update_available: bool,
    /// Data source: `"embedded"`, `"remote"`, or `"cache"`.
    pub source: String,
    /// Per-entry status detail.
    pub entries: Vec<SeedEntryStatus>,
    /// When the last refresh was attempted (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<String>,
    /// Last error message, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// Status of a single seed data entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedEntryStatus {
    /// Entry name (e.g. `"providers"`, `"models"`).
    pub name: String,
    /// Local SHA-256 hash.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local_sha256: Option<String>,
    /// Remote SHA-256 hash (if checked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_sha256: Option<String>,
    /// Whether local and remote differ.
    pub changed: bool,
}

/// Deserialized seed manifest from `seed-manifest.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedManifest {
    /// Manifest version (monotonically increasing).
    pub version: u32,
    /// Minimum schema version required.
    pub min_schema_version: u32,
    /// When this manifest was published (RFC 3339).
    pub updated_at: String,
    /// File entries with SHA-256 checksums.
    pub entries: std::collections::HashMap<String, SeedManifestEntry>,
}

/// A single file entry in the seed manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedManifestEntry {
    /// File name (e.g. `"providers.json"`).
    pub file: String,
    /// SHA-256 hex digest of the file contents.
    pub sha256: String,
}

// ── Serde helpers for SecretString ────────────────────────────────

/// A single protocol entry in a channel's `protocols` JSON.
///
/// Each entry describes how the channel connects to an upstream for a specific protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProtocolEntry {
    /// Protocol identifier: `"anthropic_messages"`, `"openai_chat"`, `"openai_responses"`.
    pub protocol: String,
    /// Upstream base URL for this protocol.
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// Optional path rewrite. When set, overrides the client request path entirely.
    /// When `None` or empty, the original request path is passed through.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rewrite_path: Option<String>,
}

/// Serializes a [`SecretString`] as its exposed string value.
fn serialize_secret<S>(secret: &SecretString, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(secret.expose_secret())
}

/// Deserializes a [`SecretString`] from a string.
fn deserialize_secret<'de, D>(deserializer: D) -> Result<SecretString, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(SecretString::new(s.into_boxed_str()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    // ── ProtocolEntry ──────────────────────────────────────────

    #[test]
    fn test_protocol_entry_deserialize_full() {
        let json = r#"{"protocol":"openai_chat","baseUrl":"https://api.deepseek.com","rewritePath":"/chat/completions"}"#;
        let entry: ProtocolEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.protocol, "openai_chat");
        assert_eq!(entry.base_url, "https://api.deepseek.com");
        assert_eq!(entry.rewrite_path, Some("/chat/completions".to_string()));
    }

    #[test]
    fn test_protocol_entry_deserialize_without_rewrite_path() {
        let json = r#"{"protocol":"openai_chat","baseUrl":"https://api.deepseek.com"}"#;
        let entry: ProtocolEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.protocol, "openai_chat");
        assert_eq!(entry.base_url, "https://api.deepseek.com");
        assert_eq!(entry.rewrite_path, None);
    }

    #[test]
    fn test_protocol_entry_serialize_skips_none_rewrite_path() {
        let entry = ProtocolEntry {
            protocol: "anthropic_messages".into(),
            base_url: "https://api.anthropic.com".into(),
            rewrite_path: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            !json.contains("rewritePath"),
            "None rewrite_path should be skipped"
        );
        assert!(json.contains("baseUrl"));
    }

    #[test]
    fn test_protocol_entry_serialize_with_rewrite_path() {
        let entry = ProtocolEntry {
            protocol: "anthropic_messages".into(),
            base_url: "https://api.anthropic.com".into(),
            rewrite_path: Some("/anthropic/v1/messages".into()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("rewritePath"));
        assert!(json.contains("/anthropic/v1/messages"));
    }

    #[test]
    fn test_protocols_json_array_deserialize() {
        let json = r#"[
            {"protocol":"anthropic_messages","baseUrl":"https://api.anthropic.com","rewritePath":"/anthropic/v1/messages"},
            {"protocol":"openai_chat","baseUrl":"https://api.deepseek.com"}
        ]"#;
        let entries: Vec<ProtocolEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].protocol, "anthropic_messages");
        assert_eq!(entries[0].base_url, "https://api.anthropic.com");
        assert_eq!(
            entries[0].rewrite_path.as_deref(),
            Some("/anthropic/v1/messages")
        );
        assert_eq!(entries[1].protocol, "openai_chat");
        assert_eq!(entries[1].rewrite_path, None);
    }

    // ── Channel force_protocol ─────────────────────────────────

    #[test]
    fn test_channel_force_protocol_none_by_default() {
        let json = r#"{"id":"test","name":"Test","apiKeyRef":"sk-test","protocol":"openai_chat","protocols":"[]","isBuiltin":false,"enabled":true,"createdAt":0,"updatedAt":0,"healthStatus":"Healthy","consecutiveFailures":0,"billingType":"Metered","monthlyQuota":null,"quotaPolicy":"fallback","priority":1}"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        assert_eq!(ch.force_protocol, None);
    }

    #[test]
    fn test_channel_force_protocol_some() {
        let json = r#"{"id":"test","name":"Test","apiKeyRef":"sk-test","protocol":"anthropic_messages","protocols":"[]","isBuiltin":false,"enabled":true,"createdAt":0,"updatedAt":0,"healthStatus":"Healthy","consecutiveFailures":0,"billingType":"Metered","monthlyQuota":null,"quotaPolicy":"fallback","priority":1,"forceProtocol":"openai_chat"}"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        assert_eq!(ch.force_protocol, Some("openai_chat".to_string()));
    }

    #[test]
    fn test_channel_no_base_url_field() {
        // base_url field should NOT exist in Channel anymore
        let json = r#"{"id":"test","name":"Test","apiKeyRef":"sk-test","protocol":"openai_chat","protocols":"[]","isBuiltin":false,"enabled":true,"createdAt":0,"updatedAt":0,"healthStatus":"Healthy","consecutiveFailures":0,"billingType":"Metered","monthlyQuota":null,"quotaPolicy":"fallback","priority":1}"#;
        let ch: Channel = serde_json::from_str(json).unwrap();
        // Channel deserialization should succeed without base_url
        assert_eq!(ch.id, "test");
    }
}
