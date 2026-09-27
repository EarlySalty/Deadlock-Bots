ALTER TABLE bot.concierge_pate_requests
    ADD COLUMN IF NOT EXISTS dm_pending BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS owner_alert_message_id BIGINT;
