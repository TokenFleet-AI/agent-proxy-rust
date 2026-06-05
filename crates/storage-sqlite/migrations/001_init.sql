-- V1: Initial schema for agent-proxy-rust storage backend.
-- Consolidates all schema into a single migration (project not yet live).

PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA foreign_keys=ON;

-- ── Providers ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

-- ── Models ────────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS models (
    id TEXT PRIMARY KEY,
    provider_id TEXT NOT NULL REFERENCES providers(id),
    client_name TEXT NOT NULL UNIQUE,
    price_input REAL NOT NULL DEFAULT 0,
    price_output REAL NOT NULL DEFAULT 0,
    currency TEXT NOT NULL DEFAULT 'USD',
    context_window INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_models_provider ON models(provider_id);

-- ── Channels ──────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    api_key TEXT NOT NULL,
    protocol TEXT NOT NULL,
    protocols TEXT NOT NULL DEFAULT '[]',
    is_builtin BOOLEAN DEFAULT 0,
    enabled BOOLEAN DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    health_status TEXT NOT NULL DEFAULT 'Healthy',
    cooldown_until TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    billing_type TEXT NOT NULL DEFAULT 'metered',
    monthly_quota INTEGER,
    quota_policy TEXT NOT NULL DEFAULT 'fallback',
    priority INTEGER NOT NULL DEFAULT 0,
    force_protocol TEXT
);

CREATE INDEX IF NOT EXISTS idx_channels_health ON channels(enabled, health_status);

-- ── Model Mappings ────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS model_mappings (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    client_name TEXT NOT NULL,
    upstream_name TEXT NOT NULL,
    billing TEXT NOT NULL,
    pricing_json TEXT NOT NULL,
    weight INTEGER DEFAULT 100,
    enabled BOOLEAN DEFAULT 1,
    protocols TEXT NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS idx_mappings_channel ON model_mappings(channel_id);

-- ── Cost Records ──────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cost_records (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL DEFAULT '',
    upstream_channel TEXT NOT NULL DEFAULT '',
    upstream_model TEXT NOT NULL DEFAULT '',
    request_time_ms INTEGER NOT NULL DEFAULT 0,
    project TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL DEFAULT '',
    agent_type TEXT NOT NULL DEFAULT '',
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    thinking_tokens INTEGER NOT NULL DEFAULT 0,
    cost REAL NOT NULL DEFAULT 0.0,
    schema_saved_tokens INTEGER NOT NULL DEFAULT 0,
    response_saved_tokens INTEGER NOT NULL DEFAULT 0,
    rtk_saved_tokens INTEGER NOT NULL DEFAULT 0,
    pre_compress_tokens INTEGER NOT NULL DEFAULT 0,
    post_compress_tokens INTEGER NOT NULL DEFAULT 0,
    compression_tokens_saved INTEGER NOT NULL DEFAULT 0,
    unit TEXT NOT NULL DEFAULT 'USD',
    pricing_snapshot_json TEXT NOT NULL DEFAULT '',
    timestamp TEXT NOT NULL,
    session_id TEXT,
    before_tokens INTEGER NOT NULL DEFAULT 0,
    after_tokens INTEGER NOT NULL DEFAULT 0,
    tokens_saved INTEGER NOT NULL DEFAULT 0,
    compression_breakdown_json TEXT NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS idx_cost_records_channel ON cost_records(channel_id);
CREATE INDEX IF NOT EXISTS idx_cost_records_project ON cost_records(project);
CREATE INDEX IF NOT EXISTS idx_cost_records_user ON cost_records(user_id);
CREATE INDEX IF NOT EXISTS idx_cost_records_time ON cost_records(timestamp);
CREATE INDEX IF NOT EXISTS idx_cost_session ON cost_records(session_id);

-- ── Cost Records Daily ────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS cost_records_daily (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    channel_id TEXT NOT NULL,
    project TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    total_input_tokens INTEGER NOT NULL,
    total_output_tokens INTEGER NOT NULL,
    total_cache_write_tokens INTEGER NOT NULL,
    total_cache_read_tokens INTEGER NOT NULL,
    total_thinking_tokens INTEGER NOT NULL,
    total_cost REAL NOT NULL,
    total_compression_tokens_saved INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cost_daily_date ON cost_records_daily(date);

-- ── Switch Logs ───────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS switch_logs (
    id TEXT PRIMARY KEY,
    from_channel_id TEXT NOT NULL,
    to_channel_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    cost_record_id TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_switch_logs_time ON switch_logs(created_at);

-- ── Auth Keys ─────────────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS auth_keys (
    key_hash TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked INTEGER DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_auth_keys_role ON auth_keys(role);

-- ── Subscription Fees ─────────────────────────────────────────────────────

CREATE TABLE IF NOT EXISTS subscription_fees (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_name TEXT NOT NULL,
    month TEXT NOT NULL,
    monthly_price REAL NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD'
);

CREATE INDEX IF NOT EXISTS idx_sub_fees_channel_month ON subscription_fees(channel_name, month);

-- ══════════════════════════════════════════════════════════════════════════
-- Seed Metadata (version tracking)
-- ══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS seed_metadata (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT OR IGNORE INTO seed_metadata (key, value, updated_at) VALUES
    ('version', '0', strftime('%s', 'now')),
    ('source', 'pending', strftime('%s', 'now'));

-- ══════════════════════════════════════════════════════════════════════════
-- Note: Seed data (providers, models, channels, model_mappings) is now
-- managed by the SeedManager trait. See crates/storage-sqlite/seed/ for
-- embedded fallback JSON files, and docs/specs/0019-remote-seed-data.md
-- for the remote-update design.
-- ══════════════════════════════════════════════════════════════════════════
