use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// An upstream AI provider channel with its API key and protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    /// Unique channel identifier (e.g. "anthropic-official").
    pub id: String,
    /// Human-readable name (e.g. "Anthropic Official").
    pub name: String,
    /// Base URL of the upstream API.
    pub url: String,
    /// API key for authenticating with the upstream.
    #[serde(
        serialize_with = "serialize_secret",
        deserialize_with = "deserialize_secret"
    )]
    pub api_key: SecretString,
    /// Protocol spoken by the upstream: `"anthropic_messages"`, `"openai_chat"`, or
    /// `"openai_responses"`.
    pub protocol: String,
    /// Whether this channel was seeded by the system (cannot be deleted).
    pub is_builtin: bool,
    /// Whether this channel is active.
    pub enabled: bool,
    /// When the channel was first created.
    pub created_at: DateTime<Utc>,
    /// When the channel was last modified.
    pub updated_at: DateTime<Utc>,
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
    /// Auto-increment primary key.
    pub id: i64,
    /// When the request was completed.
    pub timestamp: DateTime<Utc>,
    /// Git user.name or OS username.
    pub user_name: String,
    /// Absolute path of the project directory.
    pub project_path: String,
    /// Project name extracted from git remote or directory name.
    pub project_name: String,
    /// Agent type: "claude", "codex", "gemini", etc.
    pub agent_type: String,
    /// Ruflo swarm role (architect, coder, tester, ...), if detected from API key mapping.
    pub agent_role: Option<String>,
    /// Channel display name.
    pub channel_name: String,
    /// "metered" or "subscription".
    pub channel_kind: String,
    /// Client-facing model name.
    pub model_name: String,
    /// Input/prompt tokens consumed.
    pub input_tokens: u64,
    /// Output/completion tokens consumed.
    pub output_tokens: u64,
    /// Tokens written to the provider's prompt cache.
    pub cache_write_tokens: u64,
    /// Tokens read from the provider's prompt cache.
    pub cache_read_tokens: u64,
    /// Extended thinking tokens consumed.
    pub thinking_tokens: u64,
    /// Actual monetary cost of this request.
    pub actual_cost: f64,
    /// Currency of `actual_cost`: "USD", "CNY", "credits".
    pub unit: String,
    /// Token count before tokenless compression.
    pub pre_compress_tokens: u64,
    /// Token count after tokenless compression.
    pub post_compress_tokens: u64,
    /// Tokens saved by tokenless compression.
    pub compression_tokens_saved: u64,
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
    pub start: DateTime<Utc>,
    /// End of the range (exclusive).
    pub end: DateTime<Utc>,
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
    pub total_input_tokens: u64,
    /// Sum of output tokens.
    pub total_output_tokens: u64,
    /// Sum of actual costs.
    pub total_actual_cost: f64,
    /// Sum of compression tokens saved.
    pub total_compression_tokens_saved: u64,
    /// Number of requests in this group.
    pub request_count: u64,
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
