CREATE TABLE IF NOT EXISTS bot.concierge_pate_requests (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    message_id BIGINT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'claimed', 'closed_unbesetzt')),
    pate_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL,
    escalated_2h_at TIMESTAMPTZ,
    escalated_24h_at TIMESTAMPTZ,
    claimed_at TIMESTAMPTZ,
    closed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS concierge_pate_requests_status_created_idx
    ON bot.concierge_pate_requests (status, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS concierge_pate_requests_one_open_per_user
    ON bot.concierge_pate_requests (user_id)
    WHERE status = 'open';
