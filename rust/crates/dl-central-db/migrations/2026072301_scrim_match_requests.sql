CREATE TABLE IF NOT EXISTS scrim.match_request_batches (
    id INTEGER PRIMARY KEY,
    template TEXT NOT NULL CHECK (template IN ('regular_scrim', 'testmatch', 'training')),
    deadline_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'open', 'closed', 'cancelled')),
    created_by_user_id TEXT NOT NULL,
    created_by_display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS match_request_batches_deadline_status_idx
    ON scrim.match_request_batches (deadline_at, status);

CREATE TABLE IF NOT EXISTS scrim.match_requests (
    id INTEGER PRIMARY KEY,
    batch_id INTEGER NOT NULL REFERENCES scrim.match_request_batches(id) ON DELETE CASCADE,
    team_a_id INTEGER NOT NULL,
    team_b_id INTEGER,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'open', 'closed', 'cancelled')),
    slot_options JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (team_b_id IS NULL OR team_a_id <> team_b_id)
);

CREATE INDEX IF NOT EXISTS match_requests_batch_id_idx
    ON scrim.match_requests (batch_id);

CREATE INDEX IF NOT EXISTS match_requests_team_a_id_idx
    ON scrim.match_requests (team_a_id);

CREATE INDEX IF NOT EXISTS match_requests_team_b_id_idx
    ON scrim.match_requests (team_b_id)
    WHERE team_b_id IS NOT NULL;
