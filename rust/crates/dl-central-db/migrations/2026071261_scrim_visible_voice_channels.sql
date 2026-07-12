CREATE TABLE IF NOT EXISTS scrim.voice_channels (
    match_id INTEGER NOT NULL,
    team_id INTEGER NOT NULL,
    channel_id BIGINT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (match_id, team_id)
);
