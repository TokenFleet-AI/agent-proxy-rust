-- V1: Initial schema for agent-proxy-rust storage backend.
-- Applied idempotently via PRAGMA user_version check.

PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA foreign_keys=ON;

-- ── Providers & Models (single source of truth for UI + proxy) ──

CREATE TABLE IF NOT EXISTS providers (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

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

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    protocol TEXT NOT NULL,
    protocols TEXT NOT NULL DEFAULT '[]',
    is_builtin BOOLEAN DEFAULT 0,
    enabled BOOLEAN DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS model_mappings (
    id TEXT PRIMARY KEY,
    channel_id TEXT NOT NULL REFERENCES channels(id) ON DELETE CASCADE,
    client_name TEXT NOT NULL,
    upstream_name TEXT NOT NULL,
    billing TEXT NOT NULL,
    pricing_json TEXT NOT NULL,
    weight INTEGER DEFAULT 100,
    enabled BOOLEAN DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_mappings_channel ON model_mappings(channel_id);

CREATE TABLE IF NOT EXISTS cost_records (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp INTEGER NOT NULL,
    user_name TEXT NOT NULL DEFAULT '',
    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL DEFAULT '',
    agent_type TEXT NOT NULL,
    agent_role TEXT,
    channel_name TEXT NOT NULL,
    channel_kind TEXT NOT NULL,
    model_name TEXT NOT NULL,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_write_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens INTEGER NOT NULL DEFAULT 0,
    thinking_tokens INTEGER NOT NULL DEFAULT 0,
    actual_cost REAL NOT NULL DEFAULT 0.0,
    unit TEXT NOT NULL DEFAULT 'USD',
    pre_compress_tokens INTEGER NOT NULL DEFAULT 0,
    post_compress_tokens INTEGER NOT NULL DEFAULT 0,
    compression_tokens_saved INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_cost_project_date ON cost_records(project_path, timestamp);
CREATE INDEX IF NOT EXISTS idx_cost_user_date ON cost_records(user_name, timestamp);
CREATE INDEX IF NOT EXISTS idx_cost_model_date ON cost_records(model_name, timestamp);
CREATE INDEX IF NOT EXISTS idx_cost_role_date ON cost_records(agent_role, timestamp);

CREATE TABLE IF NOT EXISTS subscription_fees (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_name TEXT NOT NULL,
    month TEXT NOT NULL,
    monthly_price REAL NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD'
);

CREATE INDEX IF NOT EXISTS idx_sub_fees_channel_month ON subscription_fees(channel_name, month);

CREATE TABLE IF NOT EXISTS cost_records_daily (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    date TEXT NOT NULL,
    project_path TEXT NOT NULL,
    project_name TEXT NOT NULL,
    channel_name TEXT NOT NULL,
    model_name TEXT NOT NULL,
    total_input_tokens INTEGER NOT NULL,
    total_output_tokens INTEGER NOT NULL,
    total_cache_write_tokens INTEGER NOT NULL,
    total_cache_read_tokens INTEGER NOT NULL,
    total_thinking_tokens INTEGER NOT NULL,
    total_actual_cost REAL NOT NULL,
    total_compression_tokens_saved INTEGER NOT NULL,
    request_count INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cost_daily_date ON cost_records_daily(date);

-- ── Seed providers ──
INSERT OR IGNORE INTO providers (id, name, created_at) VALUES
    ('deepseek', 'DeepSeek', strftime('%s', 'now')),
    ('alibaba-bailian', 'Alibaba Bailian', strftime('%s', 'now')),
    ('moonshot', 'Moonshot (Kimi)', strftime('%s', 'now')),
    ('zhipu', 'Zhipu AI', strftime('%s', 'now')),
    ('minimax', 'MiniMax', strftime('%s', 'now'));

-- ── Seed models (only models with channel mappings) ──
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

-- Seed default builtin channels (multi-protocol: one channel per provider).
INSERT OR IGNORE INTO channels (id, name, url, api_key, protocol, protocols, is_builtin, enabled, created_at, updated_at)
VALUES
    -- DeepSeek (OpenAI + Anthropic)
    ('deepseek', 'DeepSeek Official', 'https://api.deepseek.com', '', 'anthropic_messages',
     '[{"protocol":"openai_chat","path":"/v1/chat/completions"},{"protocol":"anthropic_messages","path":"/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Token Plan
    ('dashscope-token', 'DashScope Token Plan', 'https://token-plan.cn-beijing.maas.aliyuncs.com', '', 'openai_chat',
     '[{"protocol":"openai_chat","path":"/compatible-mode/v1"},{"protocol":"anthropic_messages","path":"/apps/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Coding Plan
    ('dashscope-coding', 'DashScope Coding Plan', 'https://coding.dashscope.aliyuncs.com', '', 'openai_chat',
     '[{"protocol":"openai_chat","path":"/v1"},{"protocol":"anthropic_messages","path":"/apps/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Pay-as-you-go
    ('dashscope-payg', 'DashScope Pay-as-you-go', 'https://dashscope.aliyuncs.com', '', 'openai_chat',
     '[{"protocol":"openai_chat","path":"/compatible-mode/v1"},{"protocol":"anthropic_messages","path":"/apps/anthropic"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- GLM
    ('glm-official', 'Zhipu GLM Official', 'https://open.bigmodel.cn', '', 'openai_chat',
     '[{"protocol":"openai_chat","path":"/api/paas/v4"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- Kimi
    ('kimi-official', 'Kimi Official', 'https://api.moonshot.cn', '', 'openai_chat',
     '[{"protocol":"openai_chat","path":"/v1"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- MiniMax
    ('minimax-official', 'MiniMax Official', 'https://api.minimax.chat', '', 'openai_chat',
     '[{"protocol":"openai_chat","path":"/v1"}]',
     1, 1, strftime('%s', 'now'), strftime('%s', 'now'));

-- Seed model mappings (matching token-fleet-switch channel_models design).
-- Only 26 precise bindings — each model is only on channels it actually supports.
INSERT OR IGNORE INTO model_mappings (id, channel_id, client_name, upstream_name, billing, pricing_json, weight, enabled)
VALUES
    -- DeepSeek
    ('deepseek:deepseek-v4-flash', 'deepseek', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1),
    ('deepseek:deepseek-v4-pro', 'deepseek', 'deepseek-v4-pro', 'deepseek-v4-pro', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":3.0,"output_per_mtok":6.0,"cache_read_per_mtok":0.025}', 100, 1),
    -- DashScope Token Plan
    ('dashscope-token:qwen3.6-plus', 'dashscope-token', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-token:deepseek-v4-flash', 'dashscope-token', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1),
    ('dashscope-token:glm-5', 'dashscope-token', 'glm-5', 'glm-5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('dashscope-token:kimi-k2.5', 'dashscope-token', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    ('dashscope-token:minimax-m2.7', 'dashscope-token', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1),
    -- DashScope Coding Plan (metered)
    ('dashscope-coding:qwen3.6-plus', 'dashscope-coding', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-coding:qwen3.5-plus', 'dashscope-coding', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1),
    ('dashscope-coding:glm-5', 'dashscope-coding', 'glm-5', 'glm-5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('dashscope-coding:kimi-k2.5', 'dashscope-coding', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    ('dashscope-coding:minimax-m2.7', 'dashscope-coding', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1),
    ('dashscope-coding:glm-4.7-flash', 'dashscope-coding', 'glm-4.7-flash', 'glm-4.7-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.0,"output_per_mtok":0.0,"cache_read_per_mtok":0.0}', 100, 1),
    -- DashScope Pay-as-you-go
    ('dashscope-payg:qwen3.7-max', 'dashscope-payg', 'qwen3.7-max', 'qwen3.7-max', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":18.0,"cache_write_per_mtok":7.50,"cache_read_per_mtok":1.20}', 100, 1),
    ('dashscope-payg:qwen3.6-max', 'dashscope-payg', 'qwen3.6-max', 'qwen3.6-max', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":9.0,"output_per_mtok":54.0,"cache_write_per_mtok":11.25,"cache_read_per_mtok":0.90}', 100, 1),
    ('dashscope-payg:qwen3.6-plus', 'dashscope-payg', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-payg:qwen3.6-flash', 'dashscope-payg', 'qwen3.6-flash', 'qwen3.6-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":1.20,"output_per_mtok":7.20,"cache_write_per_mtok":1.50,"cache_read_per_mtok":0.12}', 100, 1),
    ('dashscope-payg:qwen3.5-plus', 'dashscope-payg', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1),
    ('dashscope-payg:qwen3.5-flash', 'dashscope-payg', 'qwen3.5-flash', 'qwen3.5-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.20,"output_per_mtok":2.0,"cache_write_per_mtok":0.25,"cache_read_per_mtok":0.02}', 100, 1),
    -- Zhipu GLM
    ('glm-official:glm-5.1', 'glm-official', 'glm-5.1', 'glm-5.1', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":24.0,"cache_read_per_mtok":1.30}', 100, 1),
    ('glm-official:glm-5-turbo', 'glm-official', 'glm-5-turbo', 'glm-5-turbo', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":5.0,"output_per_mtok":22.0,"cache_read_per_mtok":1.20}', 100, 1),
    ('glm-official:glm-5', 'glm-official', 'glm-5', 'glm-5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('glm-official:glm-4.7-flash', 'glm-official', 'glm-4.7-flash', 'glm-4.7-flash', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":0.0,"output_per_mtok":0.0,"cache_read_per_mtok":0.0}', 100, 1),
    -- Kimi
    ('kimi-official:kimi-k2.6', 'kimi-official', 'kimi-k2.6', 'kimi-k2.6', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":6.50,"output_per_mtok":27.0,"cache_read_per_mtok":1.10}', 100, 1),
    ('kimi-official:kimi-k2.5', 'kimi-official', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    -- MiniMax
    ('minimax-official:minimax-m2.7', 'minimax-official', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"type":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1);
