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
    Channel, CostAggregate, CostFilter, CostGroupBy, CostRecord, ModelMapping, SubscriptionFee,
    TimeRange,
};

/// Backend-agnostic storage for channels, cost records, and subscription fees.
///
/// Every method except [`max_connections`](Self::max_connections) is async and
/// returns `Result<T, StorageError>`. Implementations must be `Send + Sync`
/// so the trait object can be shared across Tokio tasks behind an `Arc`.
#[async_trait]
pub trait Storage: Send + Sync + Debug {
    // ── Channel ─────────────────────────────────────────────

    /// List all channels.
    async fn list_channels(&self) -> Result<Vec<Channel>, StorageError>;

    /// Get a single channel by ID.
    async fn get_channel(&self, id: &str) -> Result<Option<Channel>, StorageError>;

    /// Insert or replace a channel (upsert).
    async fn upsert_channel(&self, channel: &Channel) -> Result<(), StorageError>;

    /// Toggle a channel enabled/disabled.
    async fn set_channel_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError>;

    /// Update just the API key for a channel.
    async fn set_channel_api_key(&self, id: &str, key: &SecretString) -> Result<(), StorageError>;

    /// Delete a channel and its model mappings (cascade).
    async fn delete_channel(&self, id: &str) -> Result<(), StorageError>;

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
    /// `SQLite` returns 1; `PostgreSQL` returns pool size.
    fn max_connections(&self) -> usize;
}
