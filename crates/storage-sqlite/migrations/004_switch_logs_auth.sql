-- V4: Recreate cost_records with new schema, add switch_logs, auth_keys, providers.
-- cost_records moves from integer AUTOINCREMENT id to TEXT (UUID v7) pk,
-- and columns are renamed to match the token-fleet-switch schema.

ALTER TABLE channels ADD COLUMN priority INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL
);

INSERT OR IGNORE INTO providers (id, name) VALUES
    ('019a0000-0000-7000-0000-000000000001', 'Anthropic'),
    ('019a0000-0000-7000-0000-000000000002', 'OpenAI'),
    ('019a0000-0000-7000-0000-000000000003', 'DeepSeek'),
    ('019a0000-0000-7000-0000-000000000004', 'DashScope'),
    ('019a0000-0000-7000-0000-000000000005', 'Zhipu GLM'),
    ('019a0000-0000-7000-0000-000000000006', 'Kimi'),
    ('019a0000-0000-7000-0000-000000000007', 'MiniMax');

DROP TABLE IF EXISTS cost_records_daily;
DROP TABLE IF EXISTS cost_records;
CREATE TABLE IF NOT EXISTS cost_records (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL DEFAULT '',
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
    timestamp TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cost_records_channel ON cost_records(channel_id);
CREATE INDEX IF NOT EXISTS idx_cost_records_project ON cost_records(project);
CREATE INDEX IF NOT EXISTS idx_cost_records_user ON cost_records(user_id);
CREATE INDEX IF NOT EXISTS idx_cost_records_time ON cost_records(timestamp);

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

CREATE TABLE IF NOT EXISTS switch_logs (
    id TEXT PRIMARY KEY,
    from_channel_id TEXT NOT NULL,
    to_channel_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    cost_record_id TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_switch_logs_time ON switch_logs(created_at);

CREATE TABLE IF NOT EXISTS auth_keys (
    key_hash TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    agent_type TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked INTEGER DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_auth_keys_role ON auth_keys(role);
