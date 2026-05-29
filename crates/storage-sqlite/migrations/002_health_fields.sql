-- V2: Add health tracking columns to channels.
ALTER TABLE channels ADD COLUMN health_status TEXT NOT NULL DEFAULT 'Healthy';
ALTER TABLE channels ADD COLUMN cooldown_until TEXT;
ALTER TABLE channels ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE channels ADD COLUMN billing_type TEXT NOT NULL DEFAULT 'metered';
ALTER TABLE channels ADD COLUMN monthly_quota INTEGER;
ALTER TABLE channels ADD COLUMN quota_policy TEXT NOT NULL DEFAULT 'fallback';
