CREATE SCHEMA IF NOT EXISTS steam;

CREATE TABLE steam.bot_event_log (
    id BIGSERIAL PRIMARY KEY,
    occurred_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    event_type TEXT NOT NULL,
    decision TEXT NOT NULL,
    discord_id BIGINT NULL,
    steam_id BIGINT NULL,
    reason TEXT NULL,
    detail JSONB NULL
);

CREATE INDEX bot_event_log_occurred_at_idx ON steam.bot_event_log (occurred_at);
CREATE INDEX bot_event_log_event_type_idx ON steam.bot_event_log (event_type);
CREATE INDEX bot_event_log_discord_id_idx ON steam.bot_event_log (discord_id);
