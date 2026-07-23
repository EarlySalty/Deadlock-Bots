ALTER TABLE scrim.match_request_batches
    DROP CONSTRAINT IF EXISTS match_request_batches_status_check,
    ADD CONSTRAINT match_request_batches_status_check
        CHECK (status IN ('draft', 'posting', 'open', 'post_failed', 'closed', 'cancelled'));

ALTER TABLE scrim.match_requests
    DROP CONSTRAINT IF EXISTS match_requests_status_check,
    ADD CONSTRAINT match_requests_status_check
        CHECK (status IN ('draft', 'posting', 'open', 'post_failed', 'closed', 'cancelled')),
    ADD COLUMN IF NOT EXISTS team_query_message_ids JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS posted_at TIMESTAMPTZ;
