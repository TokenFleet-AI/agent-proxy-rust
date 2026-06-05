//! `SQLite` backend implementing the [`Storage`] trait.
//!
//! Uses `r2d2` connection pool with `rusqlite`. WAL mode is enabled
//! on every connection for concurrent read/write support.

#![forbid(unsafe_code)]
#![warn(missing_docs, missing_debug_implementations)]

use std::fmt::Write;

use agent_proxy_rust_storage::{
    AvailableChannelInfo, AvailableModelInfo, Channel, CostAggregate, CostFilter, CostGroupBy,
    CostRecord, Model, ModelMapping, Provider, Storage, StorageError, SubscriptionFee, SwitchLog,
    TimeRange,
};
use async_trait::async_trait;
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::params;
use secrecy::{ExposeSecret, SecretString};
use tracing::debug;

const MIGRATION_V1: &str = include_str!("../migrations/001_init.sql");

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
            .max_size(4)
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
            .max_size(4)
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
            api_key: SecretString::new(row.get::<_, String>(2)?.into_boxed_str()),
            protocol: row.get(3)?,
            protocols: row.get::<_, String>(4).unwrap_or_default(),
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
            force_protocol: row.get(16)?,
        })
    }

    const CHANNEL_COLS: &'static str = "id, name, api_key, protocol, protocols, is_builtin, \
                                        enabled, created_at, updated_at, health_status, \
                                        cooldown_until, consecutive_failures, billing_type, \
                                        monthly_quota, quota_policy, priority, force_protocol";
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
                .prepare("SELECT id, name, created_at FROM providers ORDER BY id")
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(Provider {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get::<_, i64>(2).map_or_else(
                            |_| String::new(),
                            |ts| {
                                chrono::DateTime::from_timestamp(ts, 0)
                                    .unwrap_or_default()
                                    .to_rfc3339()
                            },
                        ),
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
                .prepare("SELECT id, name, created_at FROM providers WHERE id = ?1")
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![id], |row| {
                    Ok(Provider {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        created_at: row.get::<_, i64>(2).map_or_else(
                            |_| String::new(),
                            |ts| {
                                chrono::DateTime::from_timestamp(ts, 0)
                                    .unwrap_or_default()
                                    .to_rfc3339()
                            },
                        ),
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
                    "SELECT m.id, m.provider_id, m.client_name, m.price_input, m.price_output, \
                     m.currency, m.context_window, m.created_at, \
                     COALESCE((SELECT COUNT(*) FROM model_mappings WHERE client_name = m.client_name), 0) as channel_count \
                     FROM models m WHERE m.provider_id = ?1 ORDER BY m.client_name",
                    vec![pid.clone()],
                ),
                None => (
                    "SELECT m.id, m.provider_id, m.client_name, m.price_input, m.price_output, \
                     m.currency, m.context_window, m.created_at, \
                     COALESCE((SELECT COUNT(*) FROM model_mappings WHERE client_name = m.client_name), 0) as channel_count \
                     FROM models m ORDER BY m.provider_id, m.client_name",
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
                        price_input: row.get(3)?,
                        price_output: row.get(4)?,
                        currency: row.get(5)?,
                        context_window: row.get(6)?,
                        created_at: row.get::<_, i64>(7).map(|ts| {
                            chrono::DateTime::from_timestamp(ts, 0)
                                .unwrap_or_default()
                                .to_rfc3339()
                        }).unwrap_or_default(),
                        channel_count: row.get(8)?,
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
                    "SELECT m.id, m.provider_id, m.client_name, m.price_input, m.price_output, \
                     m.currency, m.context_window, m.created_at, \
                     COALESCE((SELECT COUNT(*) FROM model_mappings WHERE client_name = m.client_name), 0) \
                     FROM models m WHERE m.id = ?1",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut rows = stmt
                .query_map(params![id], |row| {
                    Ok(Model {
                        id: row.get(0)?,
                        provider_id: row.get(1)?,
                        client_name: row.get(2)?,
                        price_input: row.get(3)?,
                        price_output: row.get(4)?,
                        currency: row.get(5)?,
                        context_window: row.get(6)?,
                        created_at: row.get::<_, i64>(7).map(|ts| {
                            chrono::DateTime::from_timestamp(ts, 0)
                                .unwrap_or_default()
                                .to_rfc3339()
                        }).unwrap_or_default(),
                        channel_count: row.get(8)?,
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
                        "SELECT {} FROM channels ORDER BY CASE health_status WHEN 'Healthy' THEN 0 WHEN 'Degraded' THEN 1 WHEN 'Cooldown' THEN 2 ELSE 3 END, priority, id",
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
        let api_key = channel.api_key.expose_secret().to_string();
        let protocol = channel.protocol.clone();
        let protocols = channel.protocols.clone();
        let is_builtin = channel.is_builtin;
        let enabled = channel.enabled;
        let now = Self::now_unix();
        let health_status = channel.health_status.clone();
        let billing_type = channel.billing_type.clone();
        let monthly_quota = channel.monthly_quota;
        let quota_policy = channel.quota_policy.clone();
        let priority = channel.priority;
        let force_protocol = channel.force_protocol.clone();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO channels (id, name, api_key, protocol, protocols, is_builtin, \
                 enabled, created_at, updated_at, health_status, billing_type, \
                 monthly_quota, quota_policy, priority, force_protocol)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(id) DO UPDATE SET
                   name = excluded.name,
                   api_key = excluded.api_key,
                   protocol = excluded.protocol,
                   protocols = excluded.protocols,
                   is_builtin = excluded.is_builtin,
                   enabled = excluded.enabled,
                   updated_at = excluded.updated_at,
                   health_status = excluded.health_status,
                   billing_type = excluded.billing_type,
                   monthly_quota = excluded.monthly_quota,
                   quota_policy = excluded.quota_policy,
                   priority = excluded.priority,
                   force_protocol = excluded.force_protocol",
                params![
                    id,
                    name,
                    api_key,
                    protocol,
                    protocols,
                    is_builtin,
                    enabled,
                    now,
                    now,
                    health_status,
                    billing_type,
                    monthly_quota,
                    quota_policy,
                    priority,
                    force_protocol,
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

    #[allow(clippy::too_many_arguments)]
    async fn update_channel(
        &self,
        id: &str,
        name: Option<&str>,
        enabled: Option<bool>,
        priority: Option<u32>,
        monthly_quota: Option<u64>,
        quota_policy: Option<&str>,
        protocols: Option<&str>,
        force_protocol: Option<&str>,
    ) -> Result<Channel, StorageError> {
        let id = id.to_string();
        let name = name.map(String::from);
        let quota_policy = quota_policy.map(String::from);
        let protocols = protocols.map(String::from);
        let force_protocol = force_protocol.map(String::from);
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
            if let Some(ref p) = protocols {
                sets.push(format!("protocols = ?{}", param_values.len() + 1));
                param_values.push(Box::new(p.clone()));
            }
            if let Some(ref fp) = force_protocol {
                sets.push(format!("force_protocol = ?{}", param_values.len() + 1));
                param_values.push(Box::new(fp.clone()));
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
                     weight, enabled, protocols
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
                        protocols: row.get(8)?,
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
        let protocols = mapping.protocols.clone();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO model_mappings (id, channel_id, client_name, upstream_name, billing, \
                 pricing_json, weight, enabled, protocols)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                   channel_id = excluded.channel_id,
                   client_name = excluded.client_name,
                   upstream_name = excluded.upstream_name,
                   billing = excluded.billing,
                   pricing_json = excluded.pricing_json,
                   weight = excluded.weight,
                   enabled = excluded.enabled,
                   protocols = excluded.protocols",
                params![
                    id,
                    channel_id,
                    client_name,
                    upstream_name,
                    billing,
                    pricing_json,
                    weight,
                    enabled,
                    protocols,
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

    async fn list_all_mappings(&self) -> Result<Vec<ModelMapping>, StorageError> {
        let pool = self.get_pool();
        tokio::task::spawn_blocking(move || {
            let conn = pool.get().map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare("SELECT id, channel_id, client_name, upstream_name, billing, pricing_json, weight, enabled, protocols FROM model_mappings ORDER BY channel_id, client_name")
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(ModelMapping {
                        id: row.get(0)?,
                        channel_id: row.get(1)?,
                        client_name: row.get(2)?,
                        upstream_name: row.get(3)?,
                        billing: row.get(4)?,
                        pricing_json: row.get(5)?,
                        weight: row.get(6)?,
                        enabled: row.get(7)?,
                        protocols: row.get(8)?,
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

    // ── Cost Records ───────────────────────────────────────────────────

    async fn insert_cost_record(&self, record: &CostRecord) -> Result<(), StorageError> {
        let id = record.id.clone();
        let channel_id = record.channel_id.clone();
        let upstream_channel = record.upstream_channel.clone();
        let upstream_model = record.upstream_model.clone();
        let request_time_ms = record.request_time_ms;
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
        let pricing_snapshot_json = record.pricing_snapshot_json.clone();
        let unit = record.unit.clone();
        let timestamp = record.timestamp.clone();
        let session_id = record.session_id.clone();
        let before_tokens = record.before_tokens;
        let after_tokens = record.after_tokens;
        let tokens_saved = record.tokens_saved;
        let compression_breakdown = record.compression_breakdown_json.clone();
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            conn.execute(
                "INSERT INTO cost_records
                 (id, channel_id, upstream_channel, upstream_model, request_time_ms, project, user_id, agent_type,
                  input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
                  thinking_tokens, cost,
                  schema_saved_tokens, response_saved_tokens, rtk_saved_tokens,
                  pre_compress_tokens, post_compress_tokens, compression_tokens_saved,
                  unit, pricing_snapshot_json, timestamp,
                  session_id, before_tokens, after_tokens, tokens_saved, compression_breakdown_json)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,
                         ?24,?25,?26,?27,?28)",
                params![
                    id,
                    channel_id,
                    upstream_channel,
                    upstream_model,
                    request_time_ms,
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
                    pricing_snapshot_json,
                    timestamp,
                    session_id,
                    before_tokens,
                    after_tokens,
                    tokens_saved,
                    compression_breakdown,
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
                "SELECT id, channel_id, upstream_channel, upstream_model, request_time_ms, project, user_id, agent_type,
                        input_tokens, output_tokens, cache_write_tokens, cache_read_tokens,
                        thinking_tokens, cost,
                        schema_saved_tokens, response_saved_tokens, rtk_saved_tokens,
                        pre_compress_tokens, post_compress_tokens, compression_tokens_saved,
                        unit, pricing_snapshot_json, timestamp,
                        session_id, before_tokens, after_tokens, tokens_saved, compression_breakdown_json
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
                        upstream_channel: row.get(2)?,
                        upstream_model: row.get(3)?,
                        request_time_ms: row.get(4)?,
                        project: row.get(5)?,
                        user_id: row.get(6)?,
                        agent_type: row.get(7)?,
                        input_tokens: row.get(8)?,
                        output_tokens: row.get(9)?,
                        cache_write_tokens: row.get(10)?,
                        cache_read_tokens: row.get(11)?,
                        thinking_tokens: row.get(12)?,
                        cost: row.get(13)?,
                        schema_saved_tokens: row.get(14)?,
                        response_saved_tokens: row.get(15)?,
                        rtk_saved_tokens: row.get(16)?,
                        pre_compress_tokens: row.get(17)?,
                        post_compress_tokens: row.get(18)?,
                        compression_tokens_saved: row.get(19)?,
                        unit: row.get(20)?,
                        pricing_snapshot_json: row.get(21)?,
                        timestamp: row.get(22)?,
                        session_id: row.get(23)?,
                        before_tokens: row.get(24)?,
                        after_tokens: row.get(25)?,
                        tokens_saved: row.get(26)?,
                        compression_breakdown_json: row.get(27)?,
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

    // ── Switch Log Queries ───────────────────────────────────────────────

    async fn query_switch_logs(&self, limit: Option<u32>) -> Result<Vec<SwitchLog>, StorageError> {
        let limit = limit.unwrap_or(20).min(100);
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;
            let mut stmt = conn
                .prepare(
                    "SELECT id, from_channel_id, to_channel_id, reason, cost_record_id, created_at
                     FROM switch_logs ORDER BY created_at DESC LIMIT ?1",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let rows = stmt
                .query_map(params![limit], |row| {
                    Ok(SwitchLog {
                        id: row.get(0)?,
                        from_channel_id: row.get(1)?,
                        to_channel_id: row.get(2)?,
                        reason: row.get(3)?,
                        cost_record_id: row.get(4)?,
                        created_at: row.get(5)?,
                    })
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?;
            let mut logs = Vec::new();
            for row in rows {
                logs.push(row.map_err(|e| StorageError::Backend(e.to_string()))?);
            }
            Ok(logs)
        })
        .await
        .map_err(|e| StorageError::Backend(format!("join error: {e}")))?
    }

    // ── Available Channels ────────────────────────────────────────────

    async fn list_available_channels(&self) -> Result<Vec<AvailableChannelInfo>, StorageError> {
        let pool = self.get_pool();

        tokio::task::spawn_blocking(move || {
            let conn = pool
                .get()
                .map_err(|e| StorageError::Connection(e.to_string()))?;

            // Get all enabled channels
            let mut ch_stmt = conn
                .prepare(
                    "SELECT id, name, protocol, protocols, health_status
                     FROM channels WHERE enabled = 1 ORDER BY priority, id",
                )
                .map_err(|e| StorageError::Backend(e.to_string()))?;

            let channels: Vec<(String, String, String, String, String)> = ch_stmt
                .query_map([], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get::<_, String>(3).unwrap_or_default(),
                        row.get::<_, String>(4).unwrap_or_default(),
                    ))
                })
                .map_err(|e| StorageError::Backend(e.to_string()))?
                .flatten()
                .collect();

            let mut result = Vec::new();
            for (ch_id, ch_name, protocol, protocols, health) in channels {
                // Get bound models for each channel
                let mut m_stmt = conn
                    .prepare(
                        "SELECT id, client_name, upstream_name
                         FROM model_mappings WHERE channel_id = ?1 AND enabled = 1
                         ORDER BY client_name",
                    )
                    .map_err(|e| StorageError::Backend(e.to_string()))?;

                let models: Vec<AvailableModelInfo> = m_stmt
                    .query_map(params![ch_id], |row| {
                        Ok(AvailableModelInfo {
                            mapping_id: row.get(0)?,
                            client_name: row.get(1)?,
                            upstream_name: row.get(2)?,
                        })
                    })
                    .map_err(|e| StorageError::Backend(e.to_string()))?
                    .flatten()
                    .collect();

                // Skip channels with no models
                if models.is_empty() {
                    continue;
                }

                result.push(AvailableChannelInfo {
                    channel_id: ch_id,
                    channel_name: ch_name,
                    protocol,
                    protocols,
                    health_status: health,
                    enabled: true,
                    models,
                });
            }
            Ok(result)
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

            let version: i64 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap_or(0);

            if version < 1 {
                conn.execute_batch(MIGRATION_V1)
                    .map_err(|e| StorageError::Migration(e.to_string()))?;
            }

            conn.pragma_update(None, "user_version", 1)
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
        4
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Sync setup for non-async tests.
    fn setup_in_memory() -> SqliteStorage {
        let storage = SqliteStorage::new_in_memory().expect("failed to create in-memory storage");
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(storage.migrate())
            .expect("migration failed");
        storage
    }

    /// Async setup for `#[tokio::test]` tests.
    async fn setup_in_memory_async() -> SqliteStorage {
        let storage = SqliteStorage::new_in_memory().expect("failed to create in-memory storage");
        storage.migrate().await.expect("migration failed");
        storage
    }

    // ── Migration tests ──────────────────────────────────────────────

    #[test]
    fn test_providers_table_exists() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        // Verify providers table exists by querying its schema
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='providers'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "providers table should exist");
    }

    #[test]
    fn test_models_table_exists() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='models'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "models table should exist");
    }

    #[test]
    fn test_providers_table_has_correct_columns() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info('providers')").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .collect();

        assert!(
            columns.contains(&"id".to_string()),
            "providers should have 'id' column"
        );
        assert!(
            columns.contains(&"name".to_string()),
            "providers should have 'name' column"
        );
        assert!(
            columns.contains(&"created_at".to_string()),
            "providers should have 'created_at' column"
        );
    }

    #[test]
    fn test_models_table_has_correct_columns() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let mut stmt = conn.prepare("PRAGMA table_info('models')").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .collect();

        assert!(
            columns.contains(&"id".to_string()),
            "models should have 'id' column"
        );
        assert!(
            columns.contains(&"provider_id".to_string()),
            "models should have 'provider_id' column"
        );
        assert!(
            columns.contains(&"client_name".to_string()),
            "models should have 'client_name' column"
        );
        assert!(
            columns.contains(&"price_input".to_string()),
            "models should have 'price_input' column"
        );
        assert!(
            columns.contains(&"price_output".to_string()),
            "models should have 'price_output' column"
        );
        assert!(
            columns.contains(&"currency".to_string()),
            "models should have 'currency' column"
        );
        assert!(
            columns.contains(&"context_window".to_string()),
            "models should have 'context_window' column"
        );
    }

    #[test]
    fn test_models_foreign_key_to_providers() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        // Verify foreign key exists
        let mut stmt = conn.prepare("PRAGMA foreign_key_list('models')").unwrap();
        let fk_refs: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(2))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .collect();

        assert!(
            fk_refs.contains(&"providers".to_string()),
            "models.provider_id should reference providers(id)"
        );
    }

    // ── Seed data tests ─────────────────────────────────────────────

    #[test]
    fn test_seed_providers_populated() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM providers", [], |row| row.get(0))
            .unwrap();
        assert!(count >= 5, "should have 5 seeded providers, got {count}");
    }

    #[test]
    fn test_seed_models_populated() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM models", [], |row| row.get(0))
            .unwrap();
        assert!(
            count >= 15,
            "should have at least 15 seeded models, got {count}"
        );
    }

    #[test]
    fn test_seed_providers_include_deepseek() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let name: String = conn
            .query_row(
                "SELECT name FROM providers WHERE id = 'deepseek'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "DeepSeek");
    }

    #[test]
    fn test_seed_models_linked_to_providers() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        // All models should reference a valid provider
        let orphan_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM models WHERE provider_id NOT IN (SELECT id FROM providers)",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_count, 0,
            "all models must reference a valid provider"
        );
    }

    #[test]
    fn test_seed_models_include_deepseek_flash() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM models WHERE client_name = 'deepseek-v4-flash'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "deepseek-v4-flash should exist in models");
    }

    #[test]
    fn test_seed_models_include_deepseek() {
        let storage = setup_in_memory();
        let pool = storage.get_pool();
        let conn = pool.get().unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM models WHERE client_name IN ('deepseek-v4-pro', 'deepseek-v4-flash')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "deepseek models should be seeded");
    }

    // ── Storage trait tests ─────────────────────────────────────────

    #[tokio::test]
    async fn test_storage_list_providers() {
        let storage = setup_in_memory_async().await;
        let providers = storage.list_providers().await.unwrap();
        assert!(!providers.is_empty(), "should return seeded providers");
        assert!(
            providers.iter().any(|p| p.name == "DeepSeek"),
            "should include DeepSeek"
        );
        assert!(
            providers.iter().any(|p| p.name == "Zhipu AI"),
            "should include Zhipu AI"
        );
    }

    #[tokio::test]
    async fn test_storage_get_provider_found() {
        let storage = setup_in_memory_async().await;
        let provider = storage.get_provider("deepseek").await.unwrap();
        assert!(provider.is_some(), "should find deepseek provider");
        assert_eq!(provider.unwrap().name, "DeepSeek");
    }

    #[tokio::test]
    async fn test_storage_get_provider_not_found() {
        let storage = setup_in_memory_async().await;
        let provider = storage.get_provider("nonexistent").await.unwrap();
        assert!(
            provider.is_none(),
            "should return None for unknown provider"
        );
    }

    #[tokio::test]
    async fn test_storage_list_models_unfiltered() {
        let storage = setup_in_memory_async().await;
        let models = storage.list_models(None).await.unwrap();
        assert!(!models.is_empty(), "should return seeded models");
    }

    #[tokio::test]
    async fn test_storage_list_models_filtered_by_provider() {
        let storage = setup_in_memory_async().await;
        let models = storage.list_models(Some("deepseek")).await.unwrap();
        assert!(!models.is_empty(), "should return models for deepseek");
        for m in &models {
            assert_eq!(
                m.provider_id, "deepseek",
                "all models should belong to deepseek"
            );
        }
    }
}
