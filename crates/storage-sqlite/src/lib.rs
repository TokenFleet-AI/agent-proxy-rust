//! `SQLite` backend implementing the [`Storage`] trait.
//!
//! Uses `r2d2` connection pool with `rusqlite`. WAL mode is enabled
//! on every connection for concurrent read/write support.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

use std::{fmt::Write, path::Path};

use agent_proxy_rust_storage::{
    Channel, CostAggregate, CostFilter, CostGroupBy, CostRecord, ModelMapping, Storage,
    StorageError, SubscriptionFee, TimeRange,
};
use async_trait::async_trait;
use chrono::{DateTime, TimeZone, Utc};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use secrecy::{ExposeSecret, SecretString};
use tracing::debug;

const MIGRATION_SQL: &str = include_str!("../migrations/001_init.sql");

/// SQLite-backed storage implementation.
///
/// Wraps an `r2d2` connection pool. Use [`new`](SqliteStorage::new) for
/// file-backed storage or [`new_in_memory`](SqliteStorage::new_in_memory) for
/// testing with isolated in-memory databases.
#[derive(Debug, Clone)]
pub struct SqliteStorage {
    pool: Pool<SqliteConnectionManager>,
}

impl SqliteStorage {
    /// Opens a file-backed `SQLite` database at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Connection`] if the database file cannot be
    /// opened or the pool cannot be created.
    pub fn new(path: &Path) -> Result<Self, StorageError> {
        let manager = SqliteConnectionManager::file(path);
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(|e| StorageError::Connection(format!("failed to create pool: {e}")))?;
        debug!(path = %path.display(), "SQLite database opened");
        Ok(Self { pool })
    }

    /// Opens an in-memory `SQLite` database with shared cache.
    ///
    /// Suitable for testing — each call creates an isolated database.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Connection`] if the pool cannot be created.
    pub fn new_in_memory() -> Result<Self, StorageError> {
        let manager = SqliteConnectionManager::memory();
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .map_err(|e| StorageError::Connection(format!("failed to create pool: {e}")))?;
        debug!("SQLite in-memory database opened");
        Ok(Self { pool })
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Safely converts a `u64` to `i64`, saturating at [`i64::MAX`].
/// Token counts are non-negative and fit in `i64`; this is used for `SQLite` `INTEGER` columns.
fn u64_to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Safely converts an `i64` (from `SQLite`) to `u64`, clamping at 0.
/// `SQLite` `INTEGER` columns for token counts are never negative.
fn i64_to_u64(v: i64) -> u64 {
    u64::try_from(v).unwrap_or(0)
}

impl SqliteStorage {
    fn now_unix() -> i64 {
        Utc::now().timestamp()
    }

    fn unix_to_dt(ts: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(ts, 0).single().unwrap_or_else(|| {
            Utc.timestamp_opt(0, 0)
                .single()
                .unwrap_or(DateTime::UNIX_EPOCH)
        })
    }

    fn get_pool(&self) -> Pool<SqliteConnectionManager> {
        self.pool.clone()
    }
}

#[async_trait]
impl Storage for SqliteStorage {
    // ── Channel ────────────────────────────────────────────────────────

    async fn list_channels(&self) -> Result<Vec<Channel>, StorageError> {
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, api_key, protocol, is_builtin, enabled, created_at, \
                     updated_at
                     FROM channels ORDER BY id",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Channel {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        url: row.get(2)?,
                        api_key: SecretString::new(row.get::<_, String>(3)?.into_boxed_str()),
                        protocol: row.get(4)?,
                        is_builtin: row.get(5)?,
                        enabled: row.get(6)?,
                        created_at: Self::unix_to_dt(row.get(7)?),
                        updated_at: Self::unix_to_dt(row.get(8)?),
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut channels = Vec::new();
            for row in rows {
                channels.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(channels)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn get_channel(&self, id: &str) -> Result<Option<Channel>, StorageError> {
        let id = id.to_string();
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, name, url, api_key, protocol, is_builtin, enabled, created_at, \
                     updated_at
                     FROM channels WHERE id = ?1",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![id], |row| {
                    Ok(Channel {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        url: row.get(2)?,
                        api_key: SecretString::new(row.get::<_, String>(3)?.into_boxed_str()),
                        protocol: row.get(4)?,
                        is_builtin: row.get(5)?,
                        enabled: row.get(6)?,
                        created_at: Self::unix_to_dt(row.get(7)?),
                        updated_at: Self::unix_to_dt(row.get(8)?),
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            match rows.next() {
                Some(Ok(ch)) => Ok(Some(ch)),
                Some(Err(e)) => Err(StorageError::Backend(e.to_string())),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn upsert_channel(&self, channel: &Channel) -> Result<(), StorageError> {
        let id = channel.id.clone();
        let name = channel.name.clone();
        let url = channel.url.clone();
        let api_key = channel.api_key.expose_secret().to_string();
        let protocol = channel.protocol.clone();
        let is_builtin = channel.is_builtin;
        let enabled = channel.enabled;
        let now = Self::now_unix();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO channels (id, name, url, api_key, protocol, is_builtin, enabled, \
                 created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   url = excluded.url,
                   api_key = excluded.api_key,
                   protocol = excluded.protocol,
                   is_builtin = excluded.is_builtin,
                   enabled = excluded.enabled,
                   updated_at = excluded.updated_at",
                params![
                    id, name, url, api_key, protocol, is_builtin, enabled, now, now
                ],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn set_channel_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError> {
        let id = id.to_string();
        let now = Self::now_unix();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let rows = conn
                .execute(
                    "UPDATE channels SET enabled = ?1, updated_at = ?2 WHERE id = ?3",
                    params![enabled, now, id],
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            if rows == 0 {
                return Err(StorageError::NotFound(format!("channel not found: {id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn set_channel_api_key(&self, id: &str, key: &SecretString) -> Result<(), StorageError> {
        let id = id.to_string();
        let api_key = key.expose_secret().to_string();
        let now = Self::now_unix();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let rows = conn
                .execute(
                    "UPDATE channels SET api_key = ?1, updated_at = ?2 WHERE id = ?3",
                    params![api_key, now, id],
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            if rows == 0 {
                return Err(StorageError::NotFound(format!("channel not found: {id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn delete_channel(&self, id: &str) -> Result<(), StorageError> {
        let id = id.to_string();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let rows = conn
                .execute("DELETE FROM channels WHERE id = ?1", params![id])
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            if rows == 0 {
                return Err(StorageError::NotFound(format!("channel not found: {id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Model Mapping ──────────────────────────────────────────────────

    async fn list_mappings(&self, channel_id: &str) -> Result<Vec<ModelMapping>, StorageError> {
        let channel_id = channel_id.to_string();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, channel_id, client_name, upstream_name, billing, pricing_json, \
                     weight, enabled
                     FROM model_mappings WHERE channel_id = ?1 ORDER BY id",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map(params![channel_id], |row| {
                    Ok(ModelMapping {
                        id: row.get(0)?,
                        channel_id: row.get(1)?,
                        client_name: row.get(2)?,
                        upstream_name: row.get(3)?,
                        billing: row.get(4)?,
                        pricing_json: row.get(5)?,
                        weight: row.get(6)?,
                        enabled: row.get(7)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut mappings = Vec::new();
            for row in rows {
                mappings.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(mappings)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn upsert_mapping(&self, mapping: &ModelMapping) -> Result<(), StorageError> {
        let id = mapping.id.clone();
        let channel_id = mapping.channel_id.clone();
        let client_name = mapping.client_name.clone();
        let upstream_name = mapping.upstream_name.clone();
        let billing = mapping.billing.clone();
        let pricing_json = mapping.pricing_json.clone();
        let weight = mapping.weight;
        let enabled = mapping.enabled;
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO model_mappings (id, channel_id, client_name, upstream_name, billing, \
                 pricing_json, weight, enabled)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(id) DO UPDATE SET
                   channel_id = excluded.channel_id,
                   client_name = excluded.client_name,
                   upstream_name = excluded.upstream_name,
                   billing = excluded.billing,
                   pricing_json = excluded.pricing_json,
                   weight = excluded.weight,
                   enabled = excluded.enabled",
                params![
                    id,
                    channel_id,
                    client_name,
                    upstream_name,
                    billing,
                    pricing_json,
                    weight,
                    enabled,
                ],
            )
            .map_err(|e| {
                // Check for foreign key constraint violation
                let msg = e.to_string();
                if msg.contains("FOREIGN KEY") {
                    StorageError::NotFound(format!("channel not found: {channel_id}"))
                } else {
                    StorageError::Backend(msg)
                }
            })?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn set_mapping_enabled(&self, id: &str, enabled: bool) -> Result<(), StorageError> {
        let id = id.to_string();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let rows = conn
                .execute(
                    "UPDATE model_mappings SET enabled = ?1 WHERE id = ?2",
                    params![enabled, id],
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            if rows == 0 {
                return Err(StorageError::NotFound(format!("mapping not found: {id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn delete_mapping(&self, id: &str) -> Result<(), StorageError> {
        let id = id.to_string();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let rows = conn
                .execute("DELETE FROM model_mappings WHERE id = ?1", params![id])
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            if rows == 0 {
                return Err(StorageError::NotFound(format!("mapping not found: {id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Cost Records ───────────────────────────────────────────────────

    async fn insert_cost_record(&self, record: &CostRecord) -> Result<(), StorageError> {
        let timestamp = record.timestamp.timestamp();
        let user_name = record.user_name.clone();
        let project_path = record.project_path.clone();
        let project_name = record.project_name.clone();
        let agent_type = record.agent_type.clone();
        let agent_role = record.agent_role.clone();
        let channel_name = record.channel_name.clone();
        let channel_kind = record.channel_kind.clone();
        let model_name = record.model_name.clone();
        let input_tokens = u64_to_i64(record.input_tokens);
        let output_tokens = u64_to_i64(record.output_tokens);
        let cache_write_tokens = u64_to_i64(record.cache_write_tokens);
        let cache_read_tokens = u64_to_i64(record.cache_read_tokens);
        let thinking_tokens = u64_to_i64(record.thinking_tokens);
        let actual_cost = record.actual_cost;
        let unit = record.unit.clone();
        let pre_compress_tokens = u64_to_i64(record.pre_compress_tokens);
        let post_compress_tokens = u64_to_i64(record.post_compress_tokens);
        let compression_tokens_saved = u64_to_i64(record.compression_tokens_saved);
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO cost_records
                 (timestamp, user_name, project_path, project_name, agent_type, agent_role, \
                 channel_name, channel_kind, model_name,
                  input_tokens, output_tokens, cache_write_tokens, cache_read_tokens, \
                 thinking_tokens,
                  actual_cost, unit, pre_compress_tokens, post_compress_tokens, \
                 compression_tokens_saved)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                params![
                    timestamp,
                    user_name,
                    project_path,
                    project_name,
                    agent_type,
                    agent_role,
                    channel_name,
                    channel_kind,
                    model_name,
                    input_tokens,
                    output_tokens,
                    cache_write_tokens,
                    cache_read_tokens,
                    thinking_tokens,
                    actual_cost,
                    unit,
                    pre_compress_tokens,
                    post_compress_tokens,
                    compression_tokens_saved,
                ],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn query_cost_records(
        &self,
        filter: CostFilter,
    ) -> Result<Vec<CostRecord>, StorageError> {
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            let mut sql = String::from(
                "SELECT id, timestamp, user_name, project_path, project_name, agent_type, \
                 agent_role, channel_name, channel_kind, model_name,
                        input_tokens, output_tokens, cache_write_tokens, cache_read_tokens, \
                 thinking_tokens,
                        actual_cost, unit, pre_compress_tokens, post_compress_tokens, \
                 compression_tokens_saved
                 FROM cost_records WHERE 1=1",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(project_path) = filter.project_path {
                sql.push_str(" AND project_path = ?");
                param_values.push(Box::new(project_path));
            }
            if let Some(model_name) = filter.model_name {
                sql.push_str(" AND model_name = ?");
                param_values.push(Box::new(model_name));
            }
            if let Some(channel_name) = filter.channel_name {
                sql.push_str(" AND channel_name = ?");
                param_values.push(Box::new(channel_name));
            }
            if let Some(ref tr) = filter.time_range {
                sql.push_str(" AND timestamp >= ? AND timestamp < ?");
                param_values.push(Box::new(tr.start.timestamp()));
                param_values.push(Box::new(tr.end.timestamp()));
            }

            sql.push_str(" ORDER BY timestamp DESC");

            let limit = filter.limit.unwrap_or(100);
            let offset = filter.offset.unwrap_or(0);
            let _ = write!(sql, " LIMIT {limit} OFFSET {offset}");

            let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
                .iter()
                .map(std::convert::AsRef::as_ref)
                .collect();

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    Ok(CostRecord {
                        id: row.get(0)?,
                        timestamp: Self::unix_to_dt(row.get::<_, i64>(1)?),
                        user_name: row.get(2)?,
                        project_path: row.get(3)?,
                        project_name: row.get(4)?,
                        agent_type: row.get(5)?,
                        agent_role: row.get(6)?,
                        channel_name: row.get(7)?,
                        channel_kind: row.get(8)?,
                        model_name: row.get(9)?,
                        input_tokens: i64_to_u64(row.get::<_, i64>(10)?),
                        output_tokens: i64_to_u64(row.get::<_, i64>(11)?),
                        cache_write_tokens: i64_to_u64(row.get::<_, i64>(12)?),
                        cache_read_tokens: i64_to_u64(row.get::<_, i64>(13)?),
                        thinking_tokens: i64_to_u64(row.get::<_, i64>(14)?),
                        actual_cost: row.get(15)?,
                        unit: row.get(16)?,
                        pre_compress_tokens: i64_to_u64(row.get::<_, i64>(17)?),
                        post_compress_tokens: i64_to_u64(row.get::<_, i64>(18)?),
                        compression_tokens_saved: i64_to_u64(row.get::<_, i64>(19)?),
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            let mut records = Vec::new();
            for row in rows {
                records.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(records)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn aggregate_costs(
        &self,
        group_by: CostGroupBy,
        range: TimeRange,
    ) -> Result<Vec<CostAggregate>, StorageError> {
        let pool = self.get_pool();
        let start_ts = range.start.timestamp();
        let end_ts = range.end.timestamp();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            let (group_key_expr, group_clause): (&str, &str) = match group_by {
                CostGroupBy::Project => ("project_path", "project_path"),
                CostGroupBy::Model => ("model_name", "model_name"),
                CostGroupBy::Channel => ("channel_name", "channel_name"),
                CostGroupBy::ProjectModelMonth => (
                    "project_path || '|' || model_name || '|' || strftime('%Y-%m', \
                     datetime(timestamp, 'unixepoch'))",
                    "project_path, model_name, strftime('%Y-%m', datetime(timestamp, 'unixepoch'))",
                ),
            };

            let sql = format!(
                "SELECT {group_key_expr} as group_key,
                        SUM(input_tokens) as total_input_tokens,
                        SUM(output_tokens) as total_output_tokens,
                        SUM(actual_cost) as total_actual_cost,
                        SUM(compression_tokens_saved) as total_compression_tokens_saved,
                        COUNT(*) as request_count
                 FROM cost_records
                 WHERE timestamp >= ?1 AND timestamp < ?2
                 GROUP BY {group_clause}
                 ORDER BY total_actual_cost DESC"
            );

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map(params![start_ts, end_ts], |row| {
                    Ok(CostAggregate {
                        group_key: row.get(0)?,
                        total_input_tokens: i64_to_u64(row.get::<_, i64>(1)?),
                        total_output_tokens: i64_to_u64(row.get::<_, i64>(2)?),
                        total_actual_cost: row.get(3)?,
                        total_compression_tokens_saved: i64_to_u64(row.get::<_, i64>(4)?),
                        request_count: i64_to_u64(row.get::<_, i64>(5)?),
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(results)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn prune_cost_records(&self, older_than_days: u32) -> Result<u64, StorageError> {
        let pool = self.get_pool();
        let cutoff = Utc::now().timestamp() - i64::from(older_than_days) * 86400;

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let deleted = conn
                .execute(
                    "DELETE FROM cost_records WHERE timestamp < ?1",
                    params![cutoff],
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(deleted as u64)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Subscription Fees ──────────────────────────────────────────────

    async fn insert_subscription_fee(&self, fee: &SubscriptionFee) -> Result<(), StorageError> {
        let channel_name = fee.channel_name.clone();
        let month = fee.month.clone();
        let monthly_price = fee.monthly_price;
        let currency = fee.currency.clone();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO subscription_fees (channel_name, month, monthly_price, currency)
                 VALUES (?1, ?2, ?3, ?4)",
                params![channel_name, month, monthly_price, currency],
            )
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("UNIQUE constraint") {
                    StorageError::Duplicate(msg)
                } else {
                    StorageError::Backend(msg)
                }
            })?;
            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn query_subscription_fees(
        &self,
        channel: Option<&str>,
        month: Option<&str>,
    ) -> Result<Vec<SubscriptionFee>, StorageError> {
        let channel = channel.map(String::from);
        let month = month.map(String::from);
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            let mut sql = String::from(
                "SELECT id, channel_name, month, monthly_price, currency FROM subscription_fees \
                 WHERE 1=1",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(ref ch) = channel {
                sql.push_str(" AND channel_name = ?");
                param_values.push(Box::new(ch.clone()));
            }
            if let Some(ref mo) = month {
                sql.push_str(" AND month = ?");
                param_values.push(Box::new(mo.clone()));
            }

            sql.push_str(" ORDER BY month DESC, channel_name");

            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();

            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    Ok(SubscriptionFee {
                        id: row.get(0)?,
                        channel_name: row.get(1)?,
                        month: row.get(2)?,
                        monthly_price: row.get(3)?,
                        currency: row.get(4)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            let mut fees = Vec::new();
            for row in rows {
                fees.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(fees)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Lifecycle ──────────────────────────────────────────────────────

    async fn migrate(&self) -> Result<(), StorageError> {
        let pool = self.get_pool();
        let sql = MIGRATION_SQL.to_string();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            // Check current version
            let version: i64 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap_or(0);

            if version < 1 {
                conn.execute_batch(&sql)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
                conn.pragma_update(None, "user_version", 1)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
            }

            Ok(())
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn health_check(&self) -> Result<bool, StorageError> {
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|_| StorageError::Connection("unable to get connection".into()))?;
            conn.execute_batch("SELECT 1")
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            Ok(true)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    fn max_connections(&self) -> usize {
        1
    }
}
