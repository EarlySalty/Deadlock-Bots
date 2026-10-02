CREATE TABLE IF NOT EXISTS bot.serverguide_discord_events (
    event_key TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS bot.serverguide_feedback_deliveries (
    delivery_id TEXT PRIMARY KEY,
    user_id BIGINT NOT NULL,
    sent_message_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
