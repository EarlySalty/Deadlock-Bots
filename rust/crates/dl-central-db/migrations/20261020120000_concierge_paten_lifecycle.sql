ALTER TABLE bot.concierge_pate_requests
    DROP CONSTRAINT concierge_pate_requests_status_check;

ALTER TABLE bot.concierge_pate_requests
    ADD CONSTRAINT concierge_pate_requests_status_check
    CHECK (status IN ('open', 'claimed', 'closed_unbesetzt', 'closed_opted_out', 'withdrawn'));

ALTER TABLE bot.concierge_patenschaften
    ADD COLUMN last_human_message_at TIMESTAMPTZ,
    ADD COLUMN inactivity_notice_at TIMESTAMPTZ,
    ADD COLUMN ended_at TIMESTAMPTZ,
    ADD COLUMN end_reason TEXT,
    ADD COLUMN end_message_at TIMESTAMPTZ,
    ADD COLUMN archived_at TIMESTAMPTZ,
    ADD COLUMN channel_deleted_at TIMESTAMPTZ;

ALTER TABLE bot.concierge_patenschaften
    ADD CONSTRAINT concierge_patenschaften_end_reason_check
    CHECK (end_reason IS NULL OR end_reason IN ('expired', 'inactive'));

CREATE INDEX IF NOT EXISTS concierge_patenschaften_channel_active_idx
    ON bot.concierge_patenschaften (channel_id)
    WHERE released_at IS NULL;

CREATE INDEX IF NOT EXISTS concierge_patenschaften_lifecycle_pending_idx
    ON bot.concierge_patenschaften (ended_at)
    WHERE ended_at IS NOT NULL AND channel_deleted_at IS NULL;
