CREATE TABLE IF NOT EXISTS scrim.match_request_responses (
    request_id INTEGER NOT NULL REFERENCES scrim.match_requests(id) ON DELETE CASCADE,
    team_id INTEGER NOT NULL REFERENCES scrim.teams(id) ON DELETE CASCADE,
    participant_id INTEGER NOT NULL REFERENCES scrim.participants(id) ON DELETE CASCADE,
    discord_user_id BIGINT NOT NULL,
    slot_index INTEGER NOT NULL CHECK (slot_index >= -1),
    response TEXT NOT NULL CHECK (response IN ('available', 'unavailable')),
    source TEXT NOT NULL DEFAULT 'button' CHECK (source IN ('button', 'free_text', 'dashboard')),
    message_id BIGINT,
    channel_id BIGINT,
    responded_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (request_id, team_id, participant_id, slot_index)
);

CREATE INDEX IF NOT EXISTS match_request_responses_request_team_idx
    ON scrim.match_request_responses (request_id, team_id);

CREATE INDEX IF NOT EXISTS match_request_responses_discord_user_idx
    ON scrim.match_request_responses (discord_user_id, responded_at DESC);
