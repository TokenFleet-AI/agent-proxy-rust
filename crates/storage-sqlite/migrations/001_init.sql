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
-- Seed Data
-- ══════════════════════════════════════════════════════════════════════════

-- ── Seed providers ──
INSERT OR IGNORE INTO providers (id, name, created_at) VALUES
    ('anthropic', 'Anthropic', strftime('%s', 'now')),
    ('google', 'Google (Gemini)', strftime('%s', 'now')),
    ('openai', 'OpenAI', strftime('%s', 'now')),
    ('deepseek', 'DeepSeek', strftime('%s', 'now')),
    ('alibaba-bailian', 'Alibaba Bailian', strftime('%s', 'now')),
    ('moonshot', 'Moonshot (Kimi)', strftime('%s', 'now')),
    ('zhipu', 'Zhipu AI', strftime('%s', 'now')),
    ('minimax', 'MiniMax', strftime('%s', 'now'));

-- ── Seed models ──
INSERT OR IGNORE INTO models (id, provider_id, client_name, price_input, price_output, currency, context_window, created_at) VALUES
    -- DeepSeek
    ('deepseek:deepseek-v4-flash', 'deepseek', 'deepseek-v4-flash', 1.0, 2.0, 'CNY', 1000000, strftime('%s', 'now')),
    ('deepseek:deepseek-v4-pro', 'deepseek', 'deepseek-v4-pro', 3.0, 6.0, 'CNY', 1000000, strftime('%s', 'now')),
    -- Alibaba Bailian / Qwen
    ('alibaba-bailian:qwen3.7-max', 'alibaba-bailian', 'qwen3.7-max', 6.0, 18.0, 'CNY', 256000, strftime('%s', 'now')),
    ('alibaba-bailian:qwen3.6-max', 'alibaba-bailian', 'qwen3.6-max', 9.0, 54.0, 'CNY', 256000, strftime('%s', 'now')),
    ('alibaba-bailian:qwen3.6-plus', 'alibaba-bailian', 'qwen3.6-plus', 2.0, 12.0, 'CNY', 1000000, strftime('%s', 'now')),
    ('alibaba-bailian:qwen3.6-flash', 'alibaba-bailian', 'qwen3.6-flash', 1.20, 7.20, 'CNY', 1000000, strftime('%s', 'now')),
    ('alibaba-bailian:qwen3.5-plus', 'alibaba-bailian', 'qwen3.5-plus', 0.80, 4.80, 'CNY', 1000000, strftime('%s', 'now')),
    ('alibaba-bailian:qwen3.5-flash', 'alibaba-bailian', 'qwen3.5-flash', 0.20, 2.0, 'CNY', 1000000, strftime('%s', 'now')),
    -- Zhipu AI / GLM
    ('zhipu:glm-5.1', 'zhipu', 'glm-5.1', 6.0, 24.0, 'CNY', 200000, strftime('%s', 'now')),
    ('zhipu:glm-5-turbo', 'zhipu', 'glm-5-turbo', 5.0, 22.0, 'CNY', 200000, strftime('%s', 'now')),
    ('zhipu:glm-5', 'zhipu', 'glm-5', 4.0, 18.0, 'CNY', 200000, strftime('%s', 'now')),
    ('zhipu:glm-4.7-flash', 'zhipu', 'glm-4.7-flash', 0.0, 0.0, 'CNY', 200000, strftime('%s', 'now')),
    -- Moonshot / Kimi
    ('moonshot:kimi-k2.6', 'moonshot', 'kimi-k2.6', 6.50, 27.0, 'CNY', 256000, strftime('%s', 'now')),
    ('moonshot:kimi-k2.5', 'moonshot', 'kimi-k2.5', 4.0, 21.0, 'CNY', 256000, strftime('%s', 'now')),
    -- MiniMax
    ('minimax:minimax-m2.7', 'minimax', 'minimax-m2.7', 2.10, 8.40, 'CNY', 256000, strftime('%s', 'now'));

-- ── Seed channels (base_url now inside protocols JSON) ──
INSERT OR IGNORE INTO channels (id, name, api_key, protocol, protocols, is_builtin, enabled, created_at, updated_at)
VALUES
    -- DeepSeek (OpenAI + Anthropic)
    ('deepseek', 'DeepSeek Official', '', 'anthropic_messages',
     '[{"protocol":"openai_chat","baseUrl":"https://api.deepseek.com","rewritePath":"/chat/completions"},{"protocol":"anthropic_messages","baseUrl":"https://api.deepseek.com/anthropic","rewritePath":""}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Token Plan
    ('dashscope-token', 'DashScope Token Plan', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://token-plan.cn-beijing.maas.aliyuncs.com","rewritePath":"/compatible-mode/v1"},{"protocol":"anthropic_messages","baseUrl":"https://token-plan.cn-beijing.maas.aliyuncs.com","rewritePath":"/apps/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Coding Plan
    ('dashscope-coding', 'DashScope Coding Plan', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://coding.dashscope.aliyuncs.com","rewritePath":"/v1"},{"protocol":"anthropic_messages","baseUrl":"https://coding.dashscope.aliyuncs.com","rewritePath":"/apps/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Pay-as-you-go
    ('dashscope-payg', 'DashScope Pay-as-you-go', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://dashscope.aliyuncs.com","rewritePath":"/compatible-mode/v1"},{"protocol":"anthropic_messages","baseUrl":"https://dashscope.aliyuncs.com","rewritePath":"/apps/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- GLM
    ('glm-official', 'Zhipu GLM Official', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://open.bigmodel.cn","rewritePath":"/api/paas/v4"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- Kimi
    ('kimi-official', 'Kimi Official', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://api.moonshot.cn","rewritePath":"/v1"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- TokenFleet AI (Anthropic + OpenAI)
    ('tokenfleet-ai', 'TokenFleet AI', '', 'anthropic_messages',
     '[{"protocol":"anthropic_messages","baseUrl":"https://tokenfleet.ai"},{"protocol":"openai_chat","baseUrl":"https://tokenfleet.ai"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- TokenFleet CN (OpenAI Chat only)
    ('tokenfleet-cn', 'TokenFleet CN', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://tokenfleet.cn"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- MiniMax
    ('minimax-official', 'MiniMax Official', '', 'openai_chat',
     '[{"protocol":"openai_chat","baseUrl":"https://api.minimax.chat","rewritePath":"/v1"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now'));

-- ── Seed model mappings ──
INSERT OR IGNORE INTO model_mappings (id, channel_id, client_name, upstream_name, billing, pricing_json, weight, enabled, protocols)
VALUES
    -- TokenFleet AI - Claude (anthropic_messages + openai_chat)
    ('tokenfleet-ai:claude-opus-4-8','tokenfleet-ai','claude-opus-4-8','claude-opus-4-8','metered','{"type":"per_token","currency":"USD","input_per_mtok":5.0,"output_per_mtok":25.0,"cache_write_per_mtok":6.25,"cache_read_per_mtok":0.50}',100,1,'[]'),
    ('tokenfleet-ai:claude-opus-4-7','tokenfleet-ai','claude-opus-4-7','claude-opus-4-7','metered','{"type":"per_token","currency":"USD","input_per_mtok":5.0,"output_per_mtok":25.0,"cache_write_per_mtok":6.25,"cache_read_per_mtok":0.50}',100,1,'[]'),
    ('tokenfleet-ai:claude-opus-4-6','tokenfleet-ai','claude-opus-4-6','claude-opus-4-6','metered','{"type":"per_token","currency":"USD","input_per_mtok":5.0,"output_per_mtok":25.0,"cache_write_per_mtok":6.25,"cache_read_per_mtok":0.50}',100,1,'[]'),
    ('tokenfleet-ai:claude-sonnet-4-6','tokenfleet-ai','claude-sonnet-4-6','claude-sonnet-4-6','metered','{"type":"per_token","currency":"USD","input_per_mtok":5.0,"output_per_mtok":25.0,"cache_write_per_mtok":6.25,"cache_read_per_mtok":0.50}',100,1,'[]'),
    ('tokenfleet-ai:claude-sonnet-4-5','tokenfleet-ai','claude-sonnet-4-5','claude-sonnet-4-5','metered','{"type":"per_token","currency":"USD","input_per_mtok":3.0,"output_per_mtok":15.0,"cache_write_per_mtok":3.75,"cache_read_per_mtok":0.30}',100,1,'[]'),
    ('tokenfleet-ai:claude-opus-4-5-reverse','tokenfleet-ai','claude-opus-4-5-reverse','claude-opus-4-5-reverse','metered','{"type":"per_token","currency":"USD","input_per_mtok":2.5,"output_per_mtok":12.5,"cache_read_per_mtok":0.25}',100,1,'[]'),
    ('tokenfleet-ai:claude-sonnet-4-5-reverse','tokenfleet-ai','claude-sonnet-4-5-reverse','claude-sonnet-4-5-reverse','metered','{"type":"tiered","dimension":{"type":"tokens"},"currency":"USD","tiers":[{"up_to":200000,"price":{"type":"token","input_per_mtok":1.5,"output_per_mtok":7.5}},{"up_to":null,"price":{"type":"token","input_per_mtok":3.0,"output_per_mtok":11.25}}]}',100,1,'[]'),
    -- TokenFleet AI - non-Claude (openai_chat only)
    ('tokenfleet-ai:gpt-5.2','tokenfleet-ai','gpt-5.2','gpt-5.2','metered','{"type":"per_token","currency":"USD","input_per_mtok":1.75,"output_per_mtok":14.0,"cache_read_per_mtok":1.75}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:gpt-5.2-chat','tokenfleet-ai','gpt-5.2-chat','gpt-5.2-chat','metered','{"type":"per_token","currency":"USD","input_per_mtok":1.75,"output_per_mtok":14.0,"cache_read_per_mtok":1.75}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:gpt-5.2-codex','tokenfleet-ai','gpt-5.2-codex','gpt-5.2-codex','metered','{"type":"per_token","currency":"USD","input_per_mtok":1.75,"output_per_mtok":14.0,"cache_read_per_mtok":1.75}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:gpt-5.3-codex','tokenfleet-ai','gpt-5.3-codex','gpt-5.3-codex','metered','{"type":"per_token","currency":"USD","input_per_mtok":1.75,"output_per_mtok":14.0,"cache_read_per_mtok":0.175}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:gpt-5.4-mini','tokenfleet-ai','gpt-5.4-mini','gpt-5.4-mini','metered','{"type":"per_token","currency":"USD","input_per_mtok":0.75,"output_per_mtok":4.5,"cache_read_per_mtok":0.075}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:gpt-5.4','tokenfleet-ai','gpt-5.4','gpt-5.4','metered','{"type":"tiered","dimension":{"type":"tokens"},"currency":"USD","tiers":[{"up_to":272000,"price":{"type":"token","input_per_mtok":2.5,"output_per_mtok":15.0}},{"up_to":null,"price":{"type":"token","input_per_mtok":5.0,"output_per_mtok":22.5}}]}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:gpt-5.5','tokenfleet-ai','gpt-5.5','gpt-5.5','metered','{"type":"tiered","dimension":{"type":"tokens"},"currency":"USD","tiers":[{"up_to":272000,"price":{"type":"token","input_per_mtok":5.0,"output_per_mtok":30.0}},{"up_to":null,"price":{"type":"token","input_per_mtok":10.0,"output_per_mtok":45.0}}]}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:kimi-k2.5','tokenfleet-ai','kimi-k2.5','kimi-k2.5','metered','{"type":"per_token","currency":"USD","input_per_mtok":0.571,"output_per_mtok":3.0}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:kimi-k2.6','tokenfleet-ai','kimi-k2.6','kimi-k2.6','metered','{"type":"per_token","currency":"USD","input_per_mtok":0.929,"output_per_mtok":3.857,"cache_write_per_mtok":1.161,"cache_read_per_mtok":0.186}',100,1,'["openai_chat"]'),
    ('tokenfleet-ai:deepseek-v3.2','tokenfleet-ai','deepseek-v3.2','deepseek-v3.2','metered','{"type":"per_token","currency":"USD","input_per_mtok":0.571,"output_per_mtok":2.286,"cache_read_per_mtok":0.571}',100,1,'["openai_chat"]'),
    -- TokenFleet CN
    ('tokenfleet-cn:deepseek-v3.1','tokenfleet-cn','deepseek-v3.1','deepseek-v3.1','metered','{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":12.0}',100,1,'[]'),
    ('tokenfleet-cn:deepseek-v3.2','tokenfleet-cn','deepseek-v3.2','deepseek-v3.2','metered','{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":3.0}',100,1,'[]'),
    ('tokenfleet-cn:deepseek-v4-flash','tokenfleet-cn','deepseek-v4-flash','deepseek-v4-flash','metered','{"type":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}',100,1,'[]'),
    ('tokenfleet-cn:deepseek-v4-pro','tokenfleet-cn','deepseek-v4-pro','deepseek-v4-pro','metered','{"type":"per_token","currency":"CNY","input_per_mtok":12.0,"output_per_mtok":24.0,"cache_read_per_mtok":1.0}',100,1,'[]'),
    ('tokenfleet-cn:minimax-m2.5','tokenfleet-cn','minimax-m2.5','minimax-m2.5','metered','{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_read_per_mtok":0.21}',100,1,'[]'),
    ('tokenfleet-cn:minimax-m2.7','tokenfleet-cn','minimax-m2.7','minimax-m2.7','metered','{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_read_per_mtok":0.42}',100,1,'[]'),
    ('tokenfleet-cn:kimi-k2.5','tokenfleet-cn','kimi-k2.5','kimi-k2.5','metered','{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0}',100,1,'[]'),
    ('tokenfleet-cn:kimi-k2.6','tokenfleet-cn','kimi-k2.6','kimi-k2.6','metered','{"type":"per_token","currency":"CNY","input_per_mtok":6.5,"output_per_mtok":27.0,"cache_read_per_mtok":1.3}',100,1,'[]'),
    ('tokenfleet-cn:glm-5.1','tokenfleet-cn','glm-5.1','glm-5.1','metered','{"type":"tiered","dimension":{"type":"tokens"},"currency":"CNY","tiers":[{"up_to":32000,"price":{"type":"token","input_per_mtok":6.02,"output_per_mtok":24.01,"cache_read_per_mtok":1.33}},{"up_to":null,"price":{"type":"token","input_per_mtok":7.98,"output_per_mtok":28.00,"cache_read_per_mtok":2.03}}]}',100,1,'[]'),
    ('tokenfleet-cn:glm-5v-turbo','tokenfleet-cn','glm-5v-turbo','glm-5v-turbo','metered','{"type":"tiered","dimension":{"type":"tokens"},"currency":"CNY","tiers":[{"up_to":32000,"price":{"type":"token","input_per_mtok":4.97,"output_per_mtok":21.98,"cache_read_per_mtok":1.19}},{"up_to":null,"price":{"type":"token","input_per_mtok":7.00,"output_per_mtok":25.97,"cache_read_per_mtok":1.82}}]}',100,1,'[]'),
    -- DeepSeek
    ('deepseek:deepseek-v4-flash', 'deepseek', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1,'[]'),
    ('deepseek:deepseek-v4-pro', 'deepseek', 'deepseek-v4-pro', 'deepseek-v4-pro', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":3.0,"output_per_mtok":6.0,"cache_read_per_mtok":0.025}', 100, 1,'[]'),
    -- DashScope Token Plan
    ('dashscope-token:qwen3.6-plus', 'dashscope-token', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1,'[]'),
    ('dashscope-token:deepseek-v4-flash', 'dashscope-token', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1,'[]'),
    ('dashscope-token:glm-5', 'dashscope-token', 'glm-5', 'glm-5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1,'[]'),
    ('dashscope-token:kimi-k2.5', 'dashscope-token', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1,'[]'),
    ('dashscope-token:minimax-m2.7', 'dashscope-token', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1,'[]'),
    -- DashScope Coding Plan
    ('dashscope-coding:qwen3.6-plus', 'dashscope-coding', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1,'[]'),
    ('dashscope-coding:qwen3.5-plus', 'dashscope-coding', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1,'[]'),
    ('dashscope-coding:glm-5', 'dashscope-coding', 'glm-5', 'glm-5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1,'[]'),
    ('dashscope-coding:kimi-k2.5', 'dashscope-coding', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1,'[]'),
    ('dashscope-coding:minimax-m2.7', 'dashscope-coding', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1,'[]'),
    ('dashscope-coding:glm-4.7-flash', 'dashscope-coding', 'glm-4.7-flash', 'glm-4.7-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.0,"output_per_mtok":0.0,"cache_read_per_mtok":0.0}', 100, 1,'[]'),
    -- DashScope Pay-as-you-go
    ('dashscope-payg:qwen3.7-max', 'dashscope-payg', 'qwen3.7-max', 'qwen3.7-max', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":18.0,"cache_write_per_mtok":7.50,"cache_read_per_mtok":1.20}', 100, 1,'[]'),
    ('dashscope-payg:qwen3.6-max', 'dashscope-payg', 'qwen3.6-max', 'qwen3.6-max', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":9.0,"output_per_mtok":54.0,"cache_write_per_mtok":11.25,"cache_read_per_mtok":0.90}', 100, 1,'[]'),
    ('dashscope-payg:qwen3.6-plus', 'dashscope-payg', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1,'[]'),
    ('dashscope-payg:qwen3.6-flash', 'dashscope-payg', 'qwen3.6-flash', 'qwen3.6-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":1.20,"output_per_mtok":7.20,"cache_write_per_mtok":1.50,"cache_read_per_mtok":0.12}', 100, 1,'[]'),
    ('dashscope-payg:qwen3.5-plus', 'dashscope-payg', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1,'[]'),
    ('dashscope-payg:qwen3.5-flash', 'dashscope-payg', 'qwen3.5-flash', 'qwen3.5-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.20,"output_per_mtok":2.0,"cache_write_per_mtok":0.25,"cache_read_per_mtok":0.02}', 100, 1,'[]'),
    -- Zhipu GLM
    ('glm-official:glm-5.1', 'glm-official', 'glm-5.1', 'glm-5.1', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":24.0,"cache_read_per_mtok":1.30}', 100, 1,'[]'),
    ('glm-official:glm-5-turbo', 'glm-official', 'glm-5-turbo', 'glm-5-turbo', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":5.0,"output_per_mtok":22.0,"cache_read_per_mtok":1.20}', 100, 1,'[]'),
    ('glm-official:glm-5', 'glm-official', 'glm-5', 'glm-5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1,'[]'),
    ('glm-official:glm-4.7-flash', 'glm-official', 'glm-4.7-flash', 'glm-4.7-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.0,"output_per_mtok":0.0,"cache_read_per_mtok":0.0}', 100, 1,'[]'),
    -- Kimi
    ('kimi-official:kimi-k2.6', 'kimi-official', 'kimi-k2.6', 'kimi-k2.6', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":6.50,"output_per_mtok":27.0,"cache_read_per_mtok":1.10}', 100, 1,'[]'),
    ('kimi-official:kimi-k2.5', 'kimi-official', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1,'[]'),
    -- MiniMax
    ('minimax-official:minimax-m2.7', 'minimax-official', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1,'[]');
