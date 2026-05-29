//! `SQLite` backend implementing the [`Storage`] trait.
//!
//! Uses `r2d2` connection pool with `rusqlite`. WAL mode is enabled
//! on every connection for concurrent read/write support.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

use std::fmt::Write;

use agent_proxy_rust_storage::{
    Channel, CostAggregate, CostFilter, CostGroupBy, CostRecord, Model, ModelMapping, Provider,
    Storage, StorageError, SubscriptionFee, SwitchLog, TimeRange,
};
use async_trait::async_trait;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use secrecy::{ExposeSecret, SecretString};
use tracing::debug;

const MIGRATION_V1: &str = include_str!("../migrations/001_init.sql");
const MIGRATION_V2: &str = include_str!("../migrations/002_health_fields.sql");
const MIGRATION_V3: &str = include_str!("../migrations/003_savings_fields.sql");
const MIGRATION_V4: &str = include_str!("../migrations/004_switch_logs_auth.sql");

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
    pub fn new(path: &std::path::Path) -> Result<Self, StorageError> {
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

impl SqliteStorage {
    fn now_unix() -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn get_pool(&self) -> Pool<SqliteConnectionManager> {
        self.pool.clone()
    }

    fn row_to_channel(row: &rusqlite::Row) -> rusqlite::Result<Channel> {
        Ok(Channel {
            id: row.get(0)?,
            name: row.get(1)?,
            base_url: row.get(2)?,
            api_key: SecretString::new(row.get::<_, String>(3)?.into_boxed_str()),
            protocol: row.get(4)?,
            is_builtin: row.get(5)?,
            enabled: row.get(6)?,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            health_status: row.get(9)?,
            cooldown_until: row.get(10)?,
            consecutive_failures: row.get(11)?,
            billing_type: row.get(12)?,
            monthly_quota: row.get(13)?,
            quota_policy: row.get(14)?,
            priority: row.get(15)?,
        })
    }

    const CHANNEL_COLS: &'static str = "id, name, url, api_key, protocol, is_builtin, enabled, \
                                        created_at, updated_at, health_status, cooldown_until, \
                                        consecutive_failures, billing_type, monthly_quota, \
                                        quota_policy, priority";
}

#[async_trait]
impl Storage for SqliteStorage {
    // ── Provider ────────────────────────────────────────────────────────

    async fn list_providers(&self) -> Result<Vec<Provider>, StorageError> {
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare("SELECT id, name FROM providers ORDER BY id")
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Provider {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut providers = Vec::new();
            for row in rows {
                providers.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(providers)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn get_provider(&self, id: &str) -> Result<Option<Provider>, StorageError> {
        let id = id.to_string();
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare("SELECT id, name FROM providers WHERE id = ?1")
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![id], |row| {
                    Ok(Provider {
                        id: row.get(0)?,
                        name: row.get(1)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            match rows.next() {
                Some(Ok(p)) => Ok(Some(p)),
                Some(Err(e)) => Err(StorageError::Backend(e.to_string())),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Model ──────────────────────────────────────────────────────────

    async fn list_models(&self, provider_id: Option<&str>) -> Result<Vec<Model>, StorageError> {
        let provider_id = provider_id.map(String::from);
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let (sql, param_values): (&str, Vec<String>) = match &provider_id {
                Some(pid) => (
                    "SELECT DISTINCT id, channel_id, client_name, 'USD' FROM model_mappings WHERE \
                     channel_id = ?1 ORDER BY id",
                    vec![pid.clone()],
                ),
                None => (
                    "SELECT DISTINCT id, channel_id, client_name, 'USD' FROM model_mappings ORDER \
                     BY id",
                    vec![],
                ),
            };
            let mut stmt = conn
                .prepare(sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> = param_values
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = stmt
                .query_map(params_refs.as_slice(), |row| {
                    Ok(Model {
                        id: row.get(0)?,
                        provider_id: row.get(1)?,
                        client_name: row.get(2)?,
                        currency: row.get(3)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut models = Vec::new();
            for row in rows {
                models.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(models)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    async fn get_model(&self, id: &str) -> Result<Option<Model>, StorageError> {
        let id = id.to_string();
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, channel_id, client_name, 'USD' FROM model_mappings WHERE id = ?1",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![id], |row| {
                    Ok(Model {
                        id: row.get(0)?,
                        provider_id: row.get(1)?,
                        client_name: row.get(2)?,
                        currency: row.get(3)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            match rows.next() {
                Some(Ok(m)) => Ok(Some(m)),
                Some(Err(e)) => Err(StorageError::Backend(e.to_string())),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Channel ────────────────────────────────────────────────────────

    async fn list_channels(&self, model_id: Option<&str>) -> Result<Vec<Channel>, StorageError> {
        let model_id = model_id.map(String::from);
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let (sql, params_vec) = match &model_id {
                Some(mid) => (
                    format!(
                        "SELECT {} FROM channels WHERE id IN (SELECT channel_id FROM \
                         model_mappings WHERE client_name = ?1) ORDER BY id",
                        SqliteStorage::CHANNEL_COLS
                    ),
                    vec![mid.clone()],
                ),
                None => (
                    format!(
                        "SELECT {} FROM channels ORDER BY id",
                        SqliteStorage::CHANNEL_COLS
                    ),
                    vec![],
                ),
            };
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = stmt
                .query_map(params_refs.as_slice(), SqliteStorage::row_to_channel)
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
            let sql = format!(
                "SELECT {} FROM channels WHERE id = ?1",
                SqliteStorage::CHANNEL_COLS
            );
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![id], SqliteStorage::row_to_channel)
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
        let url = channel.base_url.clone();
        let api_key = channel.api_key.expose_secret().to_string();
        let protocol = channel.protocol.clone();
        let is_builtin = channel.is_builtin;
        let enabled = channel.enabled;
        let now = Self::now_unix();
        let health_status = channel.health_status.clone();
        let billing_type = channel.billing_type.clone();
        let monthly_quota = channel.monthly_quota;
        let quota_policy = channel.quota_policy.clone();
        let priority = channel.priority;
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO channels (id, name, url, api_key, protocol, is_builtin, enabled, \
                 created_at, updated_at, health_status, billing_type, monthly_quota, \
                 quota_policy, priority)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   url = excluded.url,
                   api_key = excluded.api_key,
                   protocol = excluded.protocol,
                   is_builtin = excluded.is_builtin,
                   enabled = excluded.enabled,
                   updated_at = excluded.updated_at,
                   health_status = excluded.health_status,
                   billing_type = excluded.billing_type,
                   monthly_quota = excluded.monthly_quota,
                   quota_policy = excluded.quota_policy,
                   priority = excluded.priority",
                params![
                    id,
                    name,
                    url,
                    api_key,
                    protocol,
                    is_builtin,
                    enabled,
                    now,
                    now,
                    health_status,
                    billing_type,
                    monthly_quota,
                    quota_policy,
                    priority,
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

    async fn update_channel(
        &self,
        id: &str,
        name: Option<&str>,
        enabled: Option<bool>,
        priority: Option<u32>,
        monthly_quota: Option<u64>,
        quota_policy: Option<&str>,
    ) -> Result<Channel, StorageError> {
        let id = id.to_string();
        let name = name.map(String::from);
        let quota_policy = quota_policy.map(String::from);
        let now = Self::now_unix();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            // Build SET clause dynamically
            let mut sets = vec!["updated_at = ?1".to_string()];
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(now)];

            if let Some(ref n) = name {
                sets.push(format!("name = ?{}", param_values.len() + 1));
                param_values.push(Box::new(n.clone()));
            }
            if let Some(e) = enabled {
                sets.push(format!("enabled = ?{}", param_values.len() + 1));
                param_values.push(Box::new(e));
            }
            if let Some(p) = priority {
                sets.push(format!("priority = ?{}", param_values.len() + 1));
                param_values.push(Box::new(p));
            }
            if let Some(q) = monthly_quota {
                sets.push(format!("monthly_quota = ?{}", param_values.len() + 1));
                param_values.push(Box::new(i64::try_from(q).unwrap_or(i64::MAX)));
            }
            if let Some(ref qp) = quota_policy {
                sets.push(format!("quota_policy = ?{}", param_values.len() + 1));
                param_values.push(Box::new(qp.clone()));
            }

            let id_param_idx = param_values.len() + 1;
            param_values.push(Box::new(id.clone()));

            let sql = format!(
                "UPDATE channels SET {} WHERE id = ?{id_param_idx}",
                sets.join(", "),
            );

            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();

            let rows = conn
                .execute(&sql, params_refs.as_slice())
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            if rows == 0 {
                return Err(StorageError::NotFound(format!("channel not found: {id}")));
            }

            // Fetch updated channel
            let channel_sql = format!(
                "SELECT {} FROM channels WHERE id = ?1",
                SqliteStorage::CHANNEL_COLS
            );
            let mut stmt = conn
                .prepare(&channel_sql)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let updated = stmt
                .query_row(params![id], SqliteStorage::row_to_channel)
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(updated)
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

    async fn mark_channel_healthy(&self, id: &str) -> Result<(), StorageError> {
        let id = id.to_string();
        let now = Self::now_unix();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let rows = conn
                .execute(
                    "UPDATE channels SET health_status = 'Healthy', cooldown_until = NULL, \
                     consecutive_failures = 0, updated_at = ?1 WHERE id = ?2",
                    params![now, id],
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

    async fn record_channel_failure(&self, id: &str) -> Result<(), StorageError> {
        let id = id.to_string();
        let now = Self::now_unix();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            // Atomically increment consecutive_failures and update health_status
            conn.execute(
                "UPDATE channels SET
                   consecutive_failures = consecutive_failures + 1,
                   updated_at = ?1
                 WHERE id = ?2",
                params![now, id],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;

            // Read back failures count to determine health status
            let failures: i32 = conn
                .query_row(
                    "SELECT consecutive_failures FROM channels WHERE id = ?1",
                    params![id],
                    |row| row.get(0),
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            let status = if failures >= 3 {
                "Cooldown"
            } else if failures >= 1 {
                "Degraded"
            } else {
                "Healthy"
            };

            let cooldown_sql = if status == "Cooldown" {
                format!(
                    ", cooldown_until = '{}'",
                    chrono::Utc::now()
                        .checked_add_signed(chrono::Duration::minutes(5))
                        .unwrap_or(chrono::Utc::now())
                        .to_rfc3339()
                )
            } else {
                String::new()
            };

            conn.execute(
                &format!("UPDATE channels SET health_status = ?1 {cooldown_sql} WHERE id = ?2"),
                params![status, id],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;

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
        let id = record.id.clone();
        let channel_id = record.channel_id.clone();
        let project = record.project.clone();
        let user_id = record.user_id.clone();
        let agent_type = record.agent_type.clone();
        let input_tokens = record.input_tokens;
        let output_tokens = record.output_tokens;
        let cache_write_tokens = record.cache_write_tokens;
        let cache_read_tokens = record.cache_read_tokens;
        let thinking_tokens = record.thinking_tokens;
        let cost = record.cost;
        let schema_saved_tokens = record.schema_saved_tokens;
        let response_saved_tokens = record.response_saved_tokens;
        let rtk_saved_tokens = record.rtk_saved_tokens;
        let pre_compress_tokens = record.pre_compress_tokens;
        let post_compress_tokens = record.post_compress_tokens;
        let compression_tokens_saved = record.compression_tokens_saved;
        let unit = record.unit.clone();
        let timestamp = record.timestamp.clone();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO cost_records
                 (id, channel_id, project, user_id, agent_type,
                  input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
                  thinking_tokens, cost,
                  schema_saved_tokens, response_saved_tokens, rtk_saved_tokens,
                  pre_compress_tokens, post_compress_tokens, compression_tokens_saved,
                  unit, timestamp)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                params![
                    id,
                    channel_id,
                    project,
                    user_id,
                    agent_type,
                    input_tokens,
                    output_tokens,
                    cache_write_tokens,
                    cache_read_tokens,
                    thinking_tokens,
                    cost,
                    schema_saved_tokens,
                    response_saved_tokens,
                    rtk_saved_tokens,
                    pre_compress_tokens,
                    post_compress_tokens,
                    compression_tokens_saved,
                    unit,
                    timestamp,
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
                "SELECT id, channel_id, project, user_id, agent_type,
                        input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
                        thinking_tokens, cost,
                        schema_saved_tokens, response_saved_tokens, rtk_saved_tokens,
                        pre_compress_tokens, post_compress_tokens, compression_tokens_saved,
                        unit, timestamp
                 FROM cost_records WHERE 1=1",
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(project_path) = filter.project_path {
                sql.push_str(" AND project = ?");
                param_values.push(Box::new(project_path));
            }
            if let Some(model_name) = filter.model_name {
                sql.push_str(" AND channel_id = ?");
                param_values.push(Box::new(model_name));
            }
            if let Some(channel_name) = filter.channel_name {
                sql.push_str(" AND channel_id = ?");
                param_values.push(Box::new(channel_name));
            }
            if let Some(ref tr) = filter.time_range {
                sql.push_str(" AND timestamp >= ? AND timestamp < ?");
                param_values.push(Box::new(tr.start.to_string()));
                param_values.push(Box::new(tr.end.to_string()));
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
                        channel_id: row.get(1)?,
                        project: row.get(2)?,
                        user_id: row.get(3)?,
                        agent_type: row.get(4)?,
                        input_tokens: row.get(5)?,
                        output_tokens: row.get(6)?,
                        cache_write_tokens: row.get(7)?,
                        cache_read_tokens: row.get(8)?,
                        thinking_tokens: row.get(9)?,
                        cost: row.get(10)?,
                        schema_saved_tokens: row.get(11)?,
                        response_saved_tokens: row.get(12)?,
                        rtk_saved_tokens: row.get(13)?,
                        pre_compress_tokens: row.get(14)?,
                        post_compress_tokens: row.get(15)?,
                        compression_tokens_saved: row.get(16)?,
                        unit: row.get(17)?,
                        timestamp: row.get(18)?,
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
        let start_ts = range.start;
        let end_ts = range.end;

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            let (group_key_expr, group_clause): (&str, &str) = match group_by {
                CostGroupBy::Project => ("project", "project"),
                CostGroupBy::Model | CostGroupBy::Channel => ("channel_id", "channel_id"),
                CostGroupBy::ProjectModelMonth => (
                    "project || '|' || channel_id || '|' || substr(timestamp, 1, 7)",
                    "project, channel_id",
                ),
            };

            let sql = format!(
                "SELECT {group_key_expr} as group_key,
                        SUM(input_tokens) as total_input_tokens,
                        SUM(output_tokens) as total_output_tokens,
                        SUM(cost) as total_actual_cost,
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
                .query_map(params![start_ts.to_string(), end_ts.to_string()], |row| {
                    Ok(CostAggregate {
                        group_key: row.get(0)?,
                        total_input_tokens: row.get(1)?,
                        total_output_tokens: row.get(2)?,
                        total_actual_cost: row.get(3)?,
                        total_compression_tokens_saved: row.get(4)?,
                        request_count: row.get(5)?,
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

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            // Parse cutoff: current time minus N days as RFC 3339 string comparison
            let cutoff = chrono::Utc::now()
                .checked_sub_signed(chrono::Duration::days(i64::from(older_than_days)))
                .unwrap_or(chrono::Utc::now())
                .to_rfc3339();

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

    // ── Switch Log ─────────────────────────────────────────────────────

    async fn insert_switch_log(&self, log: &SwitchLog) -> Result<(), StorageError> {
        let id = log.id.clone();
        let from_channel_id = log.from_channel_id.clone();
        let to_channel_id = log.to_channel_id.clone();
        let reason = log.reason.clone();
        let cost_record_id = log.cost_record_id.clone();
        let created_at = log.created_at.clone();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO switch_logs (id, from_channel_id, to_channel_id, reason, \
                 cost_record_id, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    from_channel_id,
                    to_channel_id,
                    reason,
                    cost_record_id,
                    created_at
                ],
            )
            .map_err(|e| StorageError::Backend(e.to_string()))?;
            Ok(())
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

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            // Check current version
            let version: i64 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap_or(0);

            if version < 1 {
                conn.execute_batch(MIGRATION_V1)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
            }
            if version < 2 {
                conn.execute_batch(MIGRATION_V2)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
            }
            if version < 3 {
                conn.execute_batch(MIGRATION_V3)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
            }
            if version < 4 {
                conn.execute_batch(MIGRATION_V4)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
            }
            conn.pragma_update(None, "user_version", 4)
                .map_err(|e| StorageError::Migration(e.to_string()))?;

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
