ALTER TABLE bot.concierge_pate_requests
    ADD COLUMN IF NOT EXISTS dm_pending BOOLEAN NOT NULL DEFAULT FALSE;
