//! Backend-agnostic storage abstraction for agent-proxy-rust.
//!
//! Defines the [`Storage`] trait and all data types shared across backends.
//! Middleware crates depend on `Box<dyn Storage>` injected at construction time,
//! and never know whether the backend is `SQLite`, `PostgreSQL`, or an in-memory mock.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

mod error;
mod types;

use std::fmt::Debug;

use async_trait::async_trait;
pub use error::StorageError;
use secrecy::SecretString;
pub use types::{
    Channel, CostAggregate, CostFilter, CostGroupBy, CostRecord, Model, ModelMapping, Provider,
    SubscriptionFee, SwitchLog, TimeRange,
};

/// Backend-agnostic storage for providers, models, channels, and cost records.
///
/// Every method except [`max_connections`](Self::max_connections) is async and
/// returns `Result<T, StorageError>`. Implementations must be `Send + Sync`
/// so the trait object can be shared across Tokio tasks behind an `Arc`.
#[async_trait]
pub trait Storage: Send + Sync + Debug {
    // ── Provider ────────────────────────────────────────────

    /// List all providers.
    async fn list_providers(&self) -> Result<Vec<Provider>, StorageError>;

    /// Get a single provider by ID.
    async fn get_provider(&self, id: &str) -> Result<Option<Provider>, StorageError>;

    // ── Model ───────────────────────────────────────────────

    /// List models, optionally filtered by provider.
    async fn list_models(&self, provider_id: Option<&str>) -> Result<Vec<Model>, StorageError>;

    /// Get a single model by ID.
    async fn get_model(&self, id: &str) -> Result<Option<Model>, StorageError>;

    // ── Channel ─────────────────────────────────────────────

    /// List all channels, optionally filtered by model ID.
    async fn list_channels(&self, model_id: Option<&str>) -> Result<Vec<Channel>, StorageError>;

    /// Get a single channel by ID.
    async fn get_channel(&self, id: &str) -> Result<Option<Channel>, StorageError>;

    /// Insert or replace a channel (upsert).
    async fn upsert_channel(&self, channel: &Channel) -> Result<(), StorageError>;

    /// Toggle a channel enabled/disabled.
    async fn set_channel_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError>;

    /// Update just the API key for a channel.
    async fn set_channel_api_key(&self, id: &str, key: &SecretString) -> Result<(), StorageError>;

    /// Update channel fields (name, enabled, priority, quota).
    async fn update_channel(
        &self,
        id: &str,
        name: Option<&str>,
        enabled: Option<bool>,
        priority: Option<u32>,
        monthly_quota: Option<u64>,
        quota_policy: Option<&str>,
    ) -> Result<Channel, StorageError>;

    /// Delete a channel and its model mappings (cascade).
    async fn delete_channel(&self, id: &str) -> Result<(), StorageError>;

    /// Mark a channel as healthy (reset failures).
    async fn mark_channel_healthy(&self, id: &str) -> Result<(), StorageError>;

    /// Record a channel failure (increments counter, may set Degraded/Cooldown).
    async fn record_channel_failure(&self, id: &str) -> Result<(), StorageError>;

    // ── Model Mapping ───────────────────────────────────────

    /// List all model mappings for a channel.
    async fn list_mappings(&self, channel_id: &str) -> Result<Vec<ModelMapping>, StorageError>;

    /// Upsert a single model mapping.
    async fn upsert_mapping(&self, mapping: &ModelMapping) -> Result<(), StorageError>;

    /// Toggle a model mapping enabled/disabled.
    async fn set_mapping_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError>;

    /// Delete a model mapping.
    async fn delete_mapping(&self, id: &str) -> Result<(), StorageError>;

    // ── Cost Records ────────────────────────────────────────

    /// Record a completed request.
    async fn insert_cost_record(&self, record: &CostRecord) -> Result<(), StorageError>;

    /// Query cost records with optional filters.
    async fn query_cost_records(&self, filter: CostFilter)
    -> Result<Vec<CostRecord>, StorageError>;

    /// Aggregate costs grouped by the given dimension within a time range.
    async fn aggregate_costs(
        &self,
        group_by: CostGroupBy,
        range: TimeRange,
    ) -> Result<Vec<CostAggregate>, StorageError>;

    /// Delete records older than N days, returning the count of deleted rows.
    async fn prune_cost_records(&self, older_than_days: u32) -> Result<u64, StorageError>;

    // ── Switch Log ──────────────────────────────────────────

    /// Record a channel switch event.
    async fn insert_switch_log(&self, log: &SwitchLog) -> Result<(), StorageError>;

    // ── Subscription Fees ───────────────────────────────────

    /// Record a monthly subscription fee.
    async fn insert_subscription_fee(&self, fee: &SubscriptionFee) -> Result<(), StorageError>;

    /// Query subscription fees optionally filtered by channel and/or month.
    async fn query_subscription_fees(
        &self,
        channel: Option<&str>,
        month: Option<&str>,
    ) -> Result<Vec<SubscriptionFee>, StorageError>;

    // ── Lifecycle ───────────────────────────────────────────

    /// Run migrations / schema init. Idempotent — safe to call on every startup.
    async fn migrate(&self) -> Result<(), StorageError>;

    /// Health check — returns `true` if the backend is reachable.
    async fn health_check(&self) -> Result<bool, StorageError>;

    /// Maximum number of concurrent connections this backend supports.
    fn max_connections(&self) -> usize;
}
