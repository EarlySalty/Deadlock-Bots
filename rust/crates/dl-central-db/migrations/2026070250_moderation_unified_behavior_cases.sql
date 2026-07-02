ALTER TABLE moderation.ai_moderation_cases
    ADD COLUMN IF NOT EXISTS source TEXT,
    ADD COLUMN IF NOT EXISTS trigger_type TEXT;

CREATE INDEX IF NOT EXISTS ai_moderation_cases_source_trigger_created_idx
    ON moderation.ai_moderation_cases (source, trigger_type, created_at DESC);
