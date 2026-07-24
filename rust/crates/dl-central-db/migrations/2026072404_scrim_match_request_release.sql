ALTER TABLE scrim.match_requests
    ADD COLUMN IF NOT EXISTS released_slot_index INTEGER CHECK (released_slot_index IS NULL OR released_slot_index >= 0),
    ADD COLUMN IF NOT EXISTS released_slot JSONB,
    ADD COLUMN IF NOT EXISTS released_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS released_by_user_id TEXT,
    ADD COLUMN IF NOT EXISTS released_by_display_name TEXT,
    ADD COLUMN IF NOT EXISTS override_reason TEXT;
