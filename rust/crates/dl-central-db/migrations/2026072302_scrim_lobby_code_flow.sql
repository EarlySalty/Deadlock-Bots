ALTER TABLE scrim.matches
    ADD COLUMN IF NOT EXISTS lobby_code_source_user_id TEXT,
    ADD COLUMN IF NOT EXISTS lobby_code_source_display_name TEXT,
    ADD COLUMN IF NOT EXISTS lobby_code_updated_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS lobby_code_message_ids JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS lobby_code_corrections JSONB NOT NULL DEFAULT '[]'::jsonb;
