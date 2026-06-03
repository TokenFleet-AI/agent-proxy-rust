-- Migration 006: billing correlation fields for tokenless ↔ agent-proxy integration
--
-- Adds session-level billing tracking:
--   session_id: X-Claude-Code-Session-Id from Claude Code header
--   before_tokens: estimated tokens before all compression layers
--   after_tokens: actual tokens consumed by the upstream API
--   tokens_saved: total tokens saved across all layers
--   compression_breakdown_json: JSON array of per-operation savings

ALTER TABLE cost_records ADD COLUMN session_id TEXT;
ALTER TABLE cost_records ADD COLUMN before_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cost_records ADD COLUMN after_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cost_records ADD COLUMN tokens_saved INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cost_records ADD COLUMN compression_breakdown_json TEXT NOT NULL DEFAULT '[]';

CREATE INDEX IF NOT EXISTS idx_cost_session ON cost_records(session_id);
