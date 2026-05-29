-- V3: Add compression savings columns to cost_records.
ALTER TABLE cost_records ADD COLUMN schema_saved_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cost_records ADD COLUMN response_saved_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE cost_records ADD COLUMN rtk_saved_tokens INTEGER NOT NULL DEFAULT 0;
