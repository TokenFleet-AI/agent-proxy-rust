//! Seed data management for `SQLite` storage.
//!
//! Reads compile-time embedded JSON fallback and upserts seed data into the
//! database. Supports remote refresh from Git-tagged GitHub Raw URLs with
//! SHA-256 integrity verification.

use std::time::Duration;

use agent_proxy_rust_storage::{
    SeedEntryStatus, SeedManifest, SeedManifestEntry, SeedStatus, StorageError,
};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

// ── Embedded fallback data ──────────────────────────────────────────

const EMBEDDED_PROVIDERS: &str = include_str!("../seed/providers.json");
const EMBEDDED_MODELS: &str = include_str!("../seed/models.json");
const EMBEDDED_CHANNELS: &str = include_str!("../seed/channels.json");
const EMBEDDED_MODEL_MAPPINGS: &str = include_str!("../seed/model_mappings.json");
const EMBEDDED_MANIFEST: &str = include_str!("../seed/seed-manifest.json");

/// Environment variable name for the default seed remote URL.
const ENV_SEED_URL: &str = "AGENT_PROXY_SEED_URL";

// ── Intermediate deserialization types ──────────────────────────────

#[derive(Debug, Deserialize)]
struct SeedProvider {
    id: String,
    name: String,
    #[serde(default)]
    created_at: i64,
}

#[derive(Debug, Deserialize)]
struct SeedModel {
    id: String,
    provider_id: String,
    client_name: String,
    #[serde(default)]
    price_input: f64,
    #[serde(default)]
    price_output: f64,
    #[serde(default = "default_currency")]
    currency: String,
    #[serde(default)]
    context_window: i64,
    #[serde(default)]
    created_at: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SeedChannel {
    id: String,
    name: String,
    #[serde(default)]
    api_key: String,
    protocol: String,
    #[serde(default = "default_empty_array")]
    protocols: String,
    #[serde(default)]
    is_builtin: bool,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_metered")]
    billing_type: String,
    #[serde(default)]
    monthly_quota: Option<u64>,
    #[serde(default = "default_fallback")]
    quota_policy: String,
    #[serde(default)]
    priority: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SeedModelMapping {
    id: String,
    channel_id: String,
    client_name: String,
    upstream_name: String,
    #[serde(default = "default_metered")]
    billing: String,
    #[serde(default)]
    pricing_json: String,
    #[serde(default = "default_hundred")]
    weight: u32,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default = "default_empty_array")]
    protocols: String,
}

fn default_currency() -> String {
    "USD".to_string()
}

fn default_true() -> bool {
    true
}

fn default_metered() -> String {
    "metered".to_string()
}

fn default_fallback() -> String {
    "fallback".to_string()
}

fn default_hundred() -> u32 {
    100
}

fn default_empty_array() -> String {
    "[]".to_string()
}

// ── SHA-256 helper ──────────────────────────────────────────────────

fn sha256(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

// ── HTTP client ─────────────────────────────────────────────────────

fn http_client() -> Result<reqwest::blocking::Client, StorageError> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(format!("agent-proxy-rust/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| StorageError::Backend(format!("failed to create HTTP client: {e}")))
}

// ── SeedOps ─────────────────────────────────────────────────────────

/// Seed data operations backed by `SQLite`.
pub struct SeedOps {
    pool: Pool<SqliteConnectionManager>,
}

impl SeedOps {
    /// Creates a new `SeedOps` using the given connection pool.
    #[must_use]
    pub fn new(pool: Pool<SqliteConnectionManager>) -> Self {
        Self { pool }
    }

    fn now_unix() -> i64 {
        chrono::Utc::now().timestamp()
    }

    // ── Public API ──────────────────────────────────────────────

    /// Initialize seed data from embedded JSON fallback.
    ///
    /// Only inserts when `seed_metadata.version == 0` (first startup).
    pub fn seed_init(&self) -> Result<SeedStatus, StorageError> {
        let conn = self
            .pool
            .get()
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let version = Self::get_local_version_inner(&conn);

        if version > 0 {
            info!(version, "seed data already initialized, skipping");
            return self.seed_status();
        }

        info!("initializing seed data from embedded fallback");
        Self::apply_all_entries(
            &conn,
            EMBEDDED_PROVIDERS,
            EMBEDDED_MODELS,
            EMBEDDED_CHANNELS,
            EMBEDDED_MODEL_MAPPINGS,
            "embedded",
            false,
        )?;

        let manifest: SeedManifest = serde_json::from_str(EMBEDDED_MANIFEST).map_err(|e| {
            StorageError::Backend(format!("failed to parse embedded manifest: {e}"))
        })?;

        Self::set_metadata(&conn, "version", &manifest.version.to_string())?;

        info!(version = manifest.version, "seed data initialized");
        self.seed_status()
    }

    /// Query the current local seed status from `seed_metadata`.
    pub fn seed_status(&self) -> Result<SeedStatus, StorageError> {
        let conn = self
            .pool
            .get()
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        Ok(Self::build_status(&conn, None))
    }

    /// Fetch the remote manifest and compare with local status, without applying.
    pub fn seed_check_remote(&self, url: Option<&str>) -> Result<SeedStatus, StorageError> {
        let base_url = Self::resolve_seed_url(url)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        match Self::fetch_manifest(&base_url) {
            Ok(manifest) => Ok(Self::build_status(&conn, Some(&manifest))),
            Err(e) => {
                warn!(%base_url, error = %e, "failed to fetch remote manifest for status check");
                // Return local status with the error
                let mut status = Self::build_status(&conn, None);
                status.last_error = Some(e.to_string());
                Ok(status)
            }
        }
    }

    /// Fetch the remote manifest at `base_url` and compare with local.
    ///
    /// Returns the manifest if successfully fetched and parsed.
    #[allow(dead_code, clippy::unused_self)]
    pub fn check_remote_manifest(
        &self,
        base_url: &str,
    ) -> Result<Option<(SeedManifest, SeedStatus)>, StorageError> {
        let manifest = Self::fetch_manifest(base_url)?;
        let conn = self
            .pool
            .get()
            .map_err(|e| StorageError::Connection(e.to_string()))?;
        let status = Self::build_status(&conn, Some(&manifest));
        Ok(Some((manifest, status)))
    }

    /// Fetch seed data from a remote URL, verify integrity, and upsert.
    ///
    /// `url` is the base URL to the seed directory. When `None`, resolves
    /// from `AGENT_PROXY_SEED_URL` env var or last-used URL in metadata.
    pub fn seed_refresh(&self, url: Option<&str>) -> Result<SeedStatus, StorageError> {
        let base_url = Self::resolve_seed_url(url)?;

        info!(%base_url, "fetching remote seed data");

        let manifest = Self::fetch_manifest(&base_url)?;

        let conn = self
            .pool
            .get()
            .map_err(|e| StorageError::Connection(e.to_string()))?;

        let local_version = Self::get_local_version_inner(&conn);

        // Reject downgrade
        if manifest.version <= local_version {
            info!(
                local = local_version,
                remote = manifest.version,
                "seed data already up to date"
            );
            return Ok(Self::build_status(&conn, Some(&manifest)));
        }

        info!(
            local = local_version,
            remote = manifest.version,
            "applying seed update"
        );

        // Track which entries were updated
        let mut updated = Vec::new();
        let mut errors = Vec::new();

        for (name, entry) in &manifest.entries {
            let local_hash: Option<String> = Self::get_metadata(&conn, &format!("{name}:sha256"));

            if local_hash.as_deref() == Some(&entry.sha256) {
                info!(%name, "seed entry unchanged, skipping");
                continue;
            }

            info!(%name, remote_sha256 = %entry.sha256, "fetching seed entry");
            match Self::fetch_and_apply_entry(&conn, &base_url, name, entry) {
                Ok(()) => updated.push(name.clone()),
                Err(e) => {
                    let msg = format!("{name}: {e}");
                    warn!(error = %msg, "seed entry update failed");
                    errors.push(msg);
                }
            }
        }

        if errors.is_empty() {
            Self::set_metadata(&conn, "version", &manifest.version.to_string())?;
            Self::set_metadata(&conn, "last_refresh_at", &chrono::Utc::now().to_rfc3339())?;
            Self::set_metadata(&conn, "last_error", "")?;
            Self::set_metadata(&conn, "source", "remote")?;
            Self::set_metadata(&conn, "remote_url", &base_url)?;
        } else {
            let error_msg = errors.join("; ");
            Self::set_metadata(&conn, "last_error", &error_msg)?;
            warn!(%error_msg, "seed refresh completed with errors");
        }

        Ok(Self::build_status(&conn, Some(&manifest)))
    }

    // ── Remote fetch helpers ────────────────────────────────────

    /// Resolve the seed URL from explicit parameter, env var, or stored metadata.
    fn resolve_seed_url(explicit: Option<&str>) -> Result<String, StorageError> {
        if let Some(url) = explicit {
            let url = url.trim_end_matches('/');
            return Ok(url.to_string());
        }
        if let Ok(url) = std::env::var(ENV_SEED_URL) {
            let url = url.trim_end_matches('/');
            if !url.is_empty() {
                return Ok(url.to_string());
            }
        }
        Err(StorageError::Backend(
            "no seed URL provided — set AGENT_PROXY_SEED_URL or pass a URL".to_string(),
        ))
    }

    /// Fetch and parse `seed-manifest.json` from `base_url`.
    fn fetch_manifest(base_url: &str) -> Result<SeedManifest, StorageError> {
        let url = format!("{base_url}/seed-manifest.json");
        let client = http_client()?;
        let resp = client
            .get(&url)
            .send()
            .map_err(|e| StorageError::Backend(format!("failed to fetch manifest: {e}")))?;

        if !resp.status().is_success() {
            return Err(StorageError::Backend(format!(
                "manifest fetch returned {}",
                resp.status()
            )));
        }

        let manifest: SeedManifest = resp.json().map_err(|e| {
            StorageError::Backend(format!("failed to parse manifest from {url}: {e}"))
        })?;

        Ok(manifest)
    }

    /// Fetch a single seed JSON file, verify its SHA-256, and upsert into DB.
    fn fetch_and_apply_entry(
        conn: &r2d2::PooledConnection<SqliteConnectionManager>,
        base_url: &str,
        name: &str,
        entry: &SeedManifestEntry,
    ) -> Result<(), StorageError> {
        let url = format!("{base_url}/{}", entry.file);
        let client = http_client()?;
        let resp = client
            .get(&url)
            .send()
            .map_err(|e| StorageError::Backend(format!("failed to fetch {url}: {e}")))?;

        if !resp.status().is_success() {
            return Err(StorageError::Backend(format!(
                "fetch {url} returned {}",
                resp.status()
            )));
        }

        let body = resp.text().map_err(|e| {
            StorageError::Backend(format!("failed to read response body from {url}: {e}"))
        })?;

        // Verify SHA-256
        let actual_hash = sha256(&body);
        if actual_hash != entry.sha256 {
            return Err(StorageError::Backend(format!(
                "SHA-256 mismatch for {name}: expected {}, got {actual_hash}",
                entry.sha256
            )));
        }

        // Apply based on entry name
        Self::upsert_entry(conn, name, &body)?;

        // Store hash
        Self::set_metadata(conn, &format!("{name}:sha256"), &entry.sha256)?;

        Ok(())
    }

    // ── Entry application ───────────────────────────────────────

    /// Apply a single seed entry's JSON to the database.
    #[allow(clippy::too_many_lines)]
    fn upsert_entry(
        conn: &r2d2::PooledConnection<SqliteConnectionManager>,
        name: &str,
        json: &str,
    ) -> Result<(), StorageError> {
        let now = Self::now_unix();

        match name {
            "providers" => {
                let providers: Vec<SeedProvider> = serde_json::from_str(json)
                    .map_err(|e| StorageError::Backend(format!("invalid providers: {e}")))?;
                for p in &providers {
                    conn.execute(
                        "INSERT OR IGNORE INTO providers (id, name, created_at) \
                         VALUES (?1, ?2, ?3)",
                        rusqlite::params![p.id, p.name, p.created_at.max(now)],
                    )
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                }
            }
            "models" => {
                let models: Vec<SeedModel> = serde_json::from_str(json)
                    .map_err(|e| StorageError::Backend(format!("invalid models: {e}")))?;
                for m in &models {
                    conn.execute(
                        "INSERT OR IGNORE INTO models \
                         (id, provider_id, client_name, price_input, price_output, \
                          currency, context_window, created_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            m.id,
                            m.provider_id,
                            m.client_name,
                            m.price_input,
                            m.price_output,
                            m.currency,
                            m.context_window,
                            m.created_at.max(now),
                        ],
                    )
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                }
            }
            "channels" => {
                let channels: Vec<SeedChannel> = serde_json::from_str(json)
                    .map_err(|e| StorageError::Backend(format!("invalid channels: {e}")))?;
                for ch in &channels {
                    conn.execute(
                        "INSERT INTO channels \
                         (id, name, api_key, protocol, protocols, is_builtin, enabled, \
                          created_at, updated_at, billing_type, monthly_quota, \
                          quota_policy, priority) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
                         ON CONFLICT(id) DO UPDATE SET \
                           name = excluded.name, \
                           protocol = excluded.protocol, \
                           protocols = excluded.protocols, \
                           is_builtin = excluded.is_builtin, \
                           updated_at = excluded.updated_at, \
                           billing_type = excluded.billing_type, \
                           monthly_quota = excluded.monthly_quota, \
                           quota_policy = excluded.quota_policy, \
                           priority = excluded.priority",
                        rusqlite::params![
                            ch.id,
                            ch.name,
                            ch.api_key,
                            ch.protocol,
                            ch.protocols,
                            ch.is_builtin,
                            ch.enabled,
                            now,
                            now,
                            ch.billing_type,
                            ch.monthly_quota,
                            ch.quota_policy,
                            ch.priority,
                        ],
                    )
                    .map_err(|e| StorageError::Backend(e.to_string()))?;
                }
            }
            "modelMappings" | "model_mappings" => {
                let mappings: Vec<SeedModelMapping> = serde_json::from_str(json)
                    .map_err(|e| StorageError::Backend(format!("invalid model_mappings: {e}")))?;
                for mm in &mappings {
                    conn.execute(
                        "INSERT INTO model_mappings \
                         (id, channel_id, client_name, upstream_name, billing, \
                          pricing_json, weight, enabled, protocols) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                         ON CONFLICT(id) DO UPDATE SET \
                           channel_id = excluded.channel_id, \
                           client_name = excluded.client_name, \
                           upstream_name = excluded.upstream_name, \
                           billing = excluded.billing, \
                           pricing_json = excluded.pricing_json, \
                           weight = excluded.weight, \
                           enabled = excluded.enabled, \
                           protocols = excluded.protocols",
                        rusqlite::params![
                            mm.id,
                            mm.channel_id,
                            mm.client_name,
                            mm.upstream_name,
                            mm.billing,
                            mm.pricing_json,
                            mm.weight,
                            mm.enabled,
                            mm.protocols,
                        ],
                    )
                    .map_err(|e| {
                        let msg = e.to_string();
                        if msg.contains("FOREIGN KEY") {
                            StorageError::Backend(format!(
                                "channel not found for mapping {}: {}",
                                mm.id, msg
                            ))
                        } else {
                            StorageError::Backend(msg)
                        }
                    })?;
                }
            }
            other => {
                return Err(StorageError::Backend(format!(
                    "unknown seed entry type: {other}"
                )));
            }
        }

        Ok(())
    }

    /// Apply all four seed entries to the database (for embedded fallback).
    fn apply_all_entries(
        conn: &r2d2::PooledConnection<SqliteConnectionManager>,
        providers_json: &str,
        models_json: &str,
        channels_json: &str,
        mappings_json: &str,
        source: &str,
        upsert: bool,
    ) -> Result<(), StorageError> {
        if upsert {
            Self::upsert_entry(conn, "providers", providers_json)?;
            Self::upsert_entry(conn, "models", models_json)?;
            Self::upsert_entry(conn, "channels", channels_json)?;
            Self::upsert_entry(conn, "model_mappings", mappings_json)?;
        } else {
            // For initial seed, use the batch upsert with separate insert paths
            Self::upsert_entry(conn, "providers", providers_json)?;
            Self::upsert_entry(conn, "models", models_json)?;
            Self::upsert_entry(conn, "channels", channels_json)?;
            Self::upsert_entry(conn, "model_mappings", mappings_json)?;
        }

        // Record per-entry SHA-256 hashes
        let hashes = [
            ("providers:sha256", providers_json),
            ("models:sha256", models_json),
            ("channels:sha256", channels_json),
            ("model_mappings:sha256", mappings_json),
        ];
        for (key, data) in &hashes {
            let hash = sha256(data);
            Self::set_metadata(conn, key, &hash)?;
        }

        Self::set_metadata(conn, "source", source)?;

        Ok(())
    }

    // ── Metadata helpers ────────────────────────────────────────

    fn get_local_version_inner(conn: &r2d2::PooledConnection<SqliteConnectionManager>) -> u32 {
        conn.query_row(
            "SELECT value FROM seed_metadata WHERE key = 'version'",
            [],
            |row| {
                let v: String = row.get(0)?;
                Ok(v.parse::<u32>().unwrap_or(0))
            },
        )
        .unwrap_or(0)
    }

    fn get_metadata(
        conn: &r2d2::PooledConnection<SqliteConnectionManager>,
        key: &str,
    ) -> Option<String> {
        conn.query_row(
            "SELECT value FROM seed_metadata WHERE key = ?1",
            rusqlite::params![key],
            |row| row.get(0),
        )
        .ok()
    }

    fn set_metadata(
        conn: &r2d2::PooledConnection<SqliteConnectionManager>,
        key: &str,
        value: &str,
    ) -> Result<(), StorageError> {
        let now = Self::now_unix();
        conn.execute(
            "INSERT OR REPLACE INTO seed_metadata (key, value, updated_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![key, value, now],
        )
        .map_err(|e| StorageError::Backend(e.to_string()))?;
        Ok(())
    }

    /// Build a `SeedStatus` from the database, optionally comparing with a remote manifest.
    fn build_status(
        conn: &r2d2::PooledConnection<SqliteConnectionManager>,
        remote: Option<&SeedManifest>,
    ) -> SeedStatus {
        let local_version = Self::get_local_version_inner(conn);

        let source: String = Self::get_metadata(conn, "source").unwrap_or_else(|| "unknown".into());

        let last_refresh_at = Self::get_metadata(conn, "last_refresh_at");

        let last_error = Self::get_metadata(conn, "last_error").filter(|s| !s.is_empty());

        let remote_version = remote.map(|m| m.version);
        let update_available = remote_version.is_some_and(|rv| rv > local_version);

        let entry_names = ["providers", "models", "channels", "model_mappings"];
        let mut entry_statuses = Vec::new();
        for name in &entry_names {
            let local_sha256 = Self::get_metadata(conn, &format!("{name}:sha256"));
            let remote_entry = remote.and_then(|m| m.entries.get(*name));
            let remote_sha256 = remote_entry.map(|e| e.sha256.clone());
            let changed = match (&local_sha256, &remote_sha256) {
                (Some(l), Some(r)) => l != r,
                (_, Some(_)) => true,
                _ => false,
            };
            entry_statuses.push(SeedEntryStatus {
                name: (*name).to_string(),
                local_sha256,
                remote_sha256,
                changed,
            });
        }

        SeedStatus {
            local_version,
            remote_version,
            update_available,
            source,
            entries: entry_statuses,
            last_refresh_at,
            last_error,
        }
    }
}
