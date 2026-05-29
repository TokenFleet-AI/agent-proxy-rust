use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// An upstream AI provider (e.g. "Anthropic", "`OpenAI`").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    /// Unique provider identifier.
    pub id: String,
    /// Human-readable name (e.g. "Anthropic").
    pub name: String,
}

/// A model offered by a provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    /// Unique model identifier.
    pub id: String,
    /// Provider this model belongs to.
    pub provider_id: String,
    /// Client-facing model name.
    pub client_name: String,
    /// Default pricing currency.
    pub currency: String,
}

/// An upstream AI provider channel with its API key and protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    /// Unique channel identifier (e.g. "anthropic-official").
    pub id: String,
    /// Human-readable name (e.g. "Anthropic Official").
    pub name: String,
    /// Base URL of the upstream API.
    pub base_url: String,
    /// API key for authenticating with the upstream.
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub api_key: SecretString,
    /// Protocol spoken by the upstream.
    pub protocol: String,
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
}

/// Maps a client-facing model name to an upstream model name with pricing.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
}

/// A single proxied request with token usage and cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostRecord {
    /// UUID v7 primary key.
    pub id: String,
    /// Channel used for this request.
    pub channel_id: String,
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
    /// When the request was completed (RFC 3339).
    pub timestamp: String,
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

// ── Serde helpers for SecretString ────────────────────────────────

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
