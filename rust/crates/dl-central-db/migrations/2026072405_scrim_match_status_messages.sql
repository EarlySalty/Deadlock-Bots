ALTER TABLE scrim.match_requests
    ADD COLUMN IF NOT EXISTS team_status_message_ids JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS status_message_state TEXT NOT NULL DEFAULT 'pending'
        CHECK (status_message_state IN ('pending', 'posting', 'posted', 'post_failed')),
    ADD COLUMN IF NOT EXISTS status_message_last_error TEXT,
    ADD COLUMN IF NOT EXISTS status_message_posted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS status_message_updated_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS match_requests_status_message_claim_idx
    ON scrim.match_requests (released_at, id)
    WHERE released_at IS NOT NULL AND status_message_state = 'pending';
