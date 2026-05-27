-- V1: Initial schema for agent-proxy-rust storage backend.
-- Applied idempotently via PRAGMA user_version check.

PRAGMA journal_mode=WAL;
PRAGMA busy_timeout=5000;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS channels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    url TEXT NOT NULL,
    api_key TEXT NOT NULL,
    protocol TEXT NOT NULL,
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

-- Seed default builtin channels (api_key is empty, user must set it).
INSERT OR IGNORE INTO channels (id, name, url, api_key, protocol, is_builtin, enabled, created_at, updated_at)
VALUES
    -- Anthropic
    ('anthropic-official', 'Anthropic Official', 'https://api.anthropic.com', '', 'anthropic_messages', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- OpenAI
    ('openai-official', 'OpenAI Official', 'https://api.openai.com', '', 'openai_responses', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DeepSeek
    ('deepseek-openai', 'DeepSeek Official (OpenAI)', 'https://api.deepseek.com', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    ('deepseek-anthropic', 'DeepSeek Official (Anthropic)', 'https://api.deepseek.com/anthropic', '', 'anthropic_messages', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Token Plan
    ('dashscope-token-openai', 'DashScope Token Plan (OpenAI)', 'https://token-plan.cn-beijing.maas.aliyuncs.com/compatible-mode/v1', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    ('dashscope-token-anthropic', 'DashScope Token Plan (Anthropic)', 'https://token-plan.cn-beijing.maas.aliyuncs.com/apps/anthropic', '', 'anthropic_messages', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Coding Plan
    ('dashscope-coding-openai', 'DashScope Coding Plan (OpenAI)', 'https://coding.dashscope.aliyuncs.com/v1', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    ('dashscope-coding-anthropic', 'DashScope Coding Plan (Anthropic)', 'https://coding.dashscope.aliyuncs.com/apps/anthropic', '', 'anthropic_messages', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- DashScope Pay-as-you-go (北京)
    ('dashscope-payg-openai', 'DashScope Pay-as-you-go (OpenAI)', 'https://dashscope.aliyuncs.com/compatible-mode/v1', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    ('dashscope-payg-anthropic', 'DashScope Pay-as-you-go (Anthropic)', 'https://dashscope.aliyuncs.com/apps/anthropic', '', 'anthropic_messages', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- GLM
    ('glm-official', 'Zhipu GLM Official', 'https://open.bigmodel.cn/api/paas/v4', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- Kimi
    ('kimi-official', 'Kimi Official', 'https://api.moonshot.cn/v1', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now')),
    -- MiniMax
    ('minimax-official', 'MiniMax Official', 'https://api.minimax.chat/v1', '', 'openai_chat', 1, 1, strftime('%s', 'now'), strftime('%s', 'now'));

-- Seed model mappings.
-- id = {channel_id}:{model_id}
INSERT OR IGNORE INTO model_mappings (id, channel_id, client_name, upstream_name, billing, pricing_json, weight, enabled)
VALUES
    -- Anthropic (USD)
    ('anthropic-official:claude-opus-4-7', 'anthropic-official', 'claude-opus-4-7', 'claude-opus-4-7', 'metered', '{"mode":"per_token","currency":"USD","input_per_mtok":5.0,"output_per_mtok":25.0,"cache_write_per_mtok":6.25,"cache_read_per_mtok":0.50,"thinking_per_mtok":12.0}', 100, 1),
    ('anthropic-official:claude-sonnet-4-6', 'anthropic-official', 'claude-sonnet-4-6', 'claude-sonnet-4-6', 'metered', '{"mode":"per_token","currency":"USD","input_per_mtok":3.0,"output_per_mtok":15.0,"cache_write_per_mtok":3.75,"cache_read_per_mtok":0.30}', 100, 1),
    ('anthropic-official:claude-haiku-4-5', 'anthropic-official', 'claude-haiku-4-5', 'claude-haiku-4-5', 'metered', '{"mode":"per_token","currency":"USD","input_per_mtok":1.0,"output_per_mtok":5.0,"cache_write_per_mtok":1.25,"cache_read_per_mtok":0.10}', 100, 1),
    -- OpenAI (USD)
    ('openai-official:gpt-5.5', 'openai-official', 'gpt-5.5', 'gpt-5.5', 'metered', '{"mode":"per_token","currency":"USD","input_per_mtok":5.0,"output_per_mtok":30.0,"cache_read_per_mtok":1.25}', 100, 1),
    ('openai-official:gpt-5.4', 'openai-official', 'gpt-5.4', 'gpt-5.4', 'metered', '{"mode":"per_token","currency":"USD","input_per_mtok":2.5,"output_per_mtok":15.0,"cache_read_per_mtok":0.25}', 100, 1),
    ('openai-official:gpt-5-codex', 'openai-official', 'gpt-5-codex', 'gpt-5-codex', 'metered', '{"mode":"per_token","currency":"USD","input_per_mtok":1.25,"output_per_mtok":10.0,"cache_read_per_mtok":0.125}', 100, 1),
    -- DeepSeek (CNY) — both protocols
    ('deepseek-openai:deepseek-v4-flash', 'deepseek-openai', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1),
    ('deepseek-openai:deepseek-v4-pro', 'deepseek-openai', 'deepseek-v4-pro', 'deepseek-v4-pro', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":3.0,"output_per_mtok":6.0,"cache_read_per_mtok":0.025}', 100, 1),
    ('deepseek-anthropic:deepseek-v4-flash', 'deepseek-anthropic', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1),
    ('deepseek-anthropic:deepseek-v4-pro', 'deepseek-anthropic', 'deepseek-v4-pro', 'deepseek-v4-pro', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":3.0,"output_per_mtok":6.0,"cache_read_per_mtok":0.025}', 100, 1),
    -- Bailian/Qwen (CNY) — Pay-as-you-go channels
    ('dashscope-payg-openai:qwen3.7-max', 'dashscope-payg-openai', 'qwen3.7-max', 'qwen3.7-max', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":18.0,"cache_write_per_mtok":7.50,"cache_read_per_mtok":1.20}', 100, 1),
    ('dashscope-payg-openai:qwen3.6-max', 'dashscope-payg-openai', 'qwen3.6-max', 'qwen3.6-max', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":9.0,"output_per_mtok":54.0,"cache_write_per_mtok":11.25,"cache_read_per_mtok":0.90}', 100, 1),
    ('dashscope-payg-openai:qwen3.6-plus', 'dashscope-payg-openai', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-payg-openai:qwen3.6-flash', 'dashscope-payg-openai', 'qwen3.6-flash', 'qwen3.6-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.20,"output_per_mtok":7.20,"cache_write_per_mtok":1.50,"cache_read_per_mtok":0.12}', 100, 1),
    ('dashscope-payg-openai:qwen3.5-plus', 'dashscope-payg-openai', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1),
    ('dashscope-payg-openai:qwen3.5-flash', 'dashscope-payg-openai', 'qwen3.5-flash', 'qwen3.5-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.20,"output_per_mtok":2.0,"cache_write_per_mtok":0.25,"cache_read_per_mtok":0.02}', 100, 1),
    ('dashscope-payg-anthropic:qwen3.7-max', 'dashscope-payg-anthropic', 'qwen3.7-max', 'qwen3.7-max', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":18.0,"cache_write_per_mtok":7.50,"cache_read_per_mtok":1.20}', 100, 1),
    ('dashscope-payg-anthropic:qwen3.6-max', 'dashscope-payg-anthropic', 'qwen3.6-max', 'qwen3.6-max', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":9.0,"output_per_mtok":54.0,"cache_write_per_mtok":11.25,"cache_read_per_mtok":0.90}', 100, 1),
    ('dashscope-payg-anthropic:qwen3.6-plus', 'dashscope-payg-anthropic', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-payg-anthropic:qwen3.6-flash', 'dashscope-payg-anthropic', 'qwen3.6-flash', 'qwen3.6-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.20,"output_per_mtok":7.20,"cache_write_per_mtok":1.50,"cache_read_per_mtok":0.12}', 100, 1),
    ('dashscope-payg-anthropic:qwen3.5-plus', 'dashscope-payg-anthropic', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1),
    ('dashscope-payg-anthropic:qwen3.5-flash', 'dashscope-payg-anthropic', 'qwen3.5-flash', 'qwen3.5-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.20,"output_per_mtok":2.0,"cache_write_per_mtok":0.25,"cache_read_per_mtok":0.02}', 100, 1),
    -- Zhipu GLM (CNY) — tier 1 pricing [0,32K)
    ('glm-official:glm-5.1', 'glm-official', 'glm-5.1', 'glm-5.1', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":24.0,"cache_read_per_mtok":1.30}', 100, 1),
    ('glm-official:glm-5-turbo', 'glm-official', 'glm-5-turbo', 'glm-5-turbo', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":5.0,"output_per_mtok":22.0,"cache_read_per_mtok":1.20}', 100, 1),
    ('glm-official:glm-5', 'glm-official', 'glm-5', 'glm-5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('glm-official:glm-4.7', 'glm-official', 'glm-4.7', 'glm-4.7', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":8.0,"cache_read_per_mtok":0.40}', 100, 1),
    ('glm-official:glm-4.5-air', 'glm-official', 'glm-4.5-air', 'glm-4.5-air', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":2.0,"cache_read_per_mtok":0.16}', 100, 1),
    ('glm-official:glm-4.7-flashx', 'glm-official', 'glm-4.7-flashx', 'glm-4.7-flashx', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.50,"output_per_mtok":3.0,"cache_read_per_mtok":0.10}', 100, 1),
    ('glm-official:glm-4.7-flash', 'glm-official', 'glm-4.7-flash', 'glm-4.7-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.0,"output_per_mtok":0.0,"cache_read_per_mtok":0.0}', 100, 1),
    -- Kimi (CNY)
    ('kimi-official:kimi-k2.6', 'kimi-official', 'kimi-k2.6', 'kimi-k2.6', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.50,"output_per_mtok":27.0,"cache_read_per_mtok":1.10}', 100, 1),
    ('kimi-official:kimi-k2.5', 'kimi-official', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    -- MiniMax (CNY)
    ('minimax-official:minimax-m2.7', 'minimax-official', 'minimax-m2.7', 'minimax-m2.7', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1),
    ('minimax-official:minimax-m2.7-highspeed', 'minimax-official', 'minimax-m2.7-highspeed', 'minimax-m2.7-highspeed', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.20,"output_per_mtok":16.80,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.42}', 100, 1),
    ('minimax-official:minimax-m2.5', 'minimax-official', 'minimax-m2.5', 'minimax-m2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.21}', 100, 1),
    ('minimax-official:minimax-m2.5-highspeed', 'minimax-official', 'minimax-m2.5-highspeed', 'minimax-m2.5-highspeed', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.20,"output_per_mtok":16.80,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.21}', 100, 1),
    -- DashScope Token Plan (10 models × 2 protocols)
    -- Qwen models
    ('dashscope-token-openai:qwen3.7-max', 'dashscope-token-openai', 'qwen3.7-max', 'qwen3.7-max', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":18.0,"cache_write_per_mtok":7.50,"cache_read_per_mtok":1.20}', 100, 1),
    ('dashscope-token-openai:qwen3.6-plus', 'dashscope-token-openai', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-token-openai:qwen3.6-flash', 'dashscope-token-openai', 'qwen3.6-flash', 'qwen3.6-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.20,"output_per_mtok":7.20,"cache_write_per_mtok":1.50,"cache_read_per_mtok":0.12}', 100, 1),
    -- DeepSeek models
    ('dashscope-token-openai:deepseek-v4-pro', 'dashscope-token-openai', 'deepseek-v4-pro', 'deepseek-v4-pro', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":3.0,"output_per_mtok":6.0,"cache_read_per_mtok":0.025}', 100, 1),
    ('dashscope-token-openai:deepseek-v4-flash', 'dashscope-token-openai', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1),
    -- Kimi models
    ('dashscope-token-openai:kimi-k2.6', 'dashscope-token-openai', 'kimi-k2.6', 'kimi-k2.6', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.50,"output_per_mtok":27.0,"cache_read_per_mtok":1.10}', 100, 1),
    ('dashscope-token-openai:kimi-k2.5', 'dashscope-token-openai', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    -- GLM models
    ('dashscope-token-openai:glm-5.1', 'dashscope-token-openai', 'glm-5.1', 'glm-5.1', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":24.0,"cache_read_per_mtok":1.30}', 100, 1),
    ('dashscope-token-openai:glm-5', 'dashscope-token-openai', 'glm-5', 'glm-5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    -- MiniMax models
    ('dashscope-token-openai:minimax-m2.5', 'dashscope-token-openai', 'minimax-m2.5', 'minimax-m2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.21}', 100, 1),
    -- Same models for Token Plan Anthropic protocol
    ('dashscope-token-anthropic:qwen3.7-max', 'dashscope-token-anthropic', 'qwen3.7-max', 'qwen3.7-max', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":18.0,"cache_write_per_mtok":7.50,"cache_read_per_mtok":1.20}', 100, 1),
    ('dashscope-token-anthropic:qwen3.6-plus', 'dashscope-token-anthropic', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-token-anthropic:qwen3.6-flash', 'dashscope-token-anthropic', 'qwen3.6-flash', 'qwen3.6-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.20,"output_per_mtok":7.20,"cache_write_per_mtok":1.50,"cache_read_per_mtok":0.12}', 100, 1),
    ('dashscope-token-anthropic:deepseek-v4-pro', 'dashscope-token-anthropic', 'deepseek-v4-pro', 'deepseek-v4-pro', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":3.0,"output_per_mtok":6.0,"cache_read_per_mtok":0.025}', 100, 1),
    ('dashscope-token-anthropic:deepseek-v4-flash', 'dashscope-token-anthropic', 'deepseek-v4-flash', 'deepseek-v4-flash', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":1.0,"output_per_mtok":2.0,"cache_read_per_mtok":0.02}', 100, 1),
    ('dashscope-token-anthropic:kimi-k2.6', 'dashscope-token-anthropic', 'kimi-k2.6', 'kimi-k2.6', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.50,"output_per_mtok":27.0,"cache_read_per_mtok":1.10}', 100, 1),
    ('dashscope-token-anthropic:kimi-k2.5', 'dashscope-token-anthropic', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    ('dashscope-token-anthropic:glm-5.1', 'dashscope-token-anthropic', 'glm-5.1', 'glm-5.1', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":6.0,"output_per_mtok":24.0,"cache_read_per_mtok":1.30}', 100, 1),
    ('dashscope-token-anthropic:glm-5', 'dashscope-token-anthropic', 'glm-5', 'glm-5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('dashscope-token-anthropic:minimax-m2.5', 'dashscope-token-anthropic', 'minimax-m2.5', 'minimax-m2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.21}', 100, 1),
    -- DashScope Coding Plan (5 models × 2 protocols)
    ('dashscope-coding-openai:qwen3.6-plus', 'dashscope-coding-openai', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-coding-openai:qwen3.5-plus', 'dashscope-coding-openai', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1),
    ('dashscope-coding-openai:glm-5', 'dashscope-coding-openai', 'glm-5', 'glm-5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('dashscope-coding-openai:kimi-k2.5', 'dashscope-coding-openai', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    ('dashscope-coding-openai:minimax-m2.5', 'dashscope-coding-openai', 'minimax-m2.5', 'minimax-m2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.21}', 100, 1),
    ('dashscope-coding-anthropic:qwen3.6-plus', 'dashscope-coding-anthropic', 'qwen3.6-plus', 'qwen3.6-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.0,"output_per_mtok":12.0,"cache_write_per_mtok":2.50,"cache_read_per_mtok":0.20}', 100, 1),
    ('dashscope-coding-anthropic:qwen3.5-plus', 'dashscope-coding-anthropic', 'qwen3.5-plus', 'qwen3.5-plus', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":0.80,"output_per_mtok":4.80,"cache_write_per_mtok":1.0,"cache_read_per_mtok":0.08}', 100, 1),
    ('dashscope-coding-anthropic:glm-5', 'dashscope-coding-anthropic', 'glm-5', 'glm-5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":18.0,"cache_read_per_mtok":1.0}', 100, 1),
    ('dashscope-coding-anthropic:kimi-k2.5', 'dashscope-coding-anthropic', 'kimi-k2.5', 'kimi-k2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":4.0,"output_per_mtok":21.0,"cache_read_per_mtok":0.70}', 100, 1),
    ('dashscope-coding-anthropic:minimax-m2.5', 'dashscope-coding-anthropic', 'minimax-m2.5', 'minimax-m2.5', 'metered', '{"mode":"per_token","currency":"CNY","input_per_mtok":2.10,"output_per_mtok":8.40,"cache_write_per_mtok":2.625,"cache_read_per_mtok":0.21}', 100, 1);
