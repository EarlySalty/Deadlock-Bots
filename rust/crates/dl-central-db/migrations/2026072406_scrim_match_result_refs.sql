CREATE TABLE IF NOT EXISTS scrim.match_result_refs (
    id BIGSERIAL PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES scrim.matches(id) ON DELETE CASCADE,
    steam_match_id BIGINT NOT NULL,
    source_user_id TEXT NOT NULL,
    source_display_name TEXT NOT NULL,
    entered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    fetch_status TEXT NOT NULL DEFAULT 'pending'
        CHECK (fetch_status IN ('pending', 'fetching', 'fetched', 'failed')),
    fetched_at TIMESTAMPTZ,
    last_error TEXT,
    winner_team_id INTEGER,
    raw_result_json JSONB,
    normalized_result_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS match_result_refs_steam_match_id_uidx
    ON scrim.match_result_refs (steam_match_id);

CREATE INDEX IF NOT EXISTS match_result_refs_match_status_idx
    ON scrim.match_result_refs (match_id, fetch_status, entered_at, id);
