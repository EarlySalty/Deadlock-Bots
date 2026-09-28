CREATE TABLE IF NOT EXISTS bot.devfeed_mirror (
    source_message_id text PRIMARY KEY,
    discord_message_id bigint NOT NULL DEFAULT 0,
    skipped boolean NOT NULL DEFAULT false,
    posted_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CONSTRAINT devfeed_mirror_source_digits CHECK (source_message_id ~ '^[0-9]+$')
);

COMMENT ON TABLE bot.devfeed_mirror IS
    'Zuordnung DevFeed-Quellnachricht zu Spiegelung in #Allgemein.';
