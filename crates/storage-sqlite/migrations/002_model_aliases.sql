-- Migration: Add model_aliases table for model name mapping
-- Used by ModelAliasMiddleware to translate official model names to custom models

CREATE TABLE IF NOT EXISTS model_aliases (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    alias_name TEXT NOT NULL UNIQUE,
    target_model TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Create index for fast lookups
CREATE INDEX IF NOT EXISTS idx_model_aliases_alias_name ON model_aliases(alias_name);
CREATE INDEX IF NOT EXISTS idx_model_aliases_enabled ON model_aliases(enabled);

-- Seed data: Codex official model → proxy model mapping (target TBD)
INSERT OR IGNORE INTO model_aliases (alias_name, target_model) VALUES ('gpt-5.5', '');
INSERT OR IGNORE INTO model_aliases (alias_name, target_model) VALUES ('gpt-5.4', '');
INSERT OR IGNORE INTO model_aliases (alias_name, target_model) VALUES ('gpt-5.4-mini', '');
INSERT OR IGNORE INTO model_aliases (alias_name, target_model) VALUES ('gpt-5.3-codex', '');
INSERT OR IGNORE INTO model_aliases (alias_name, target_model) VALUES ('gpt-5.2', '');
