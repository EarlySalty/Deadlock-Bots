CREATE TABLE IF NOT EXISTS scrim.lagebild_snapshots (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES scrim.teams(id) ON DELETE CASCADE,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    generated_for TEXT NOT NULL,
    source TEXT NOT NULL CHECK (source IN ('ai', 'match', 'correction')),
    status TEXT NOT NULL CHECK (status IN ('ok', 'error')),
    lagebild_text TEXT NOT NULL,
    data_summary JSONB NOT NULL DEFAULT '{}'::jsonb,
    model TEXT,
    error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS lagebild_snapshots_team_generated_idx
    ON scrim.lagebild_snapshots (team_id, generated_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS scrim.lagebild_evidences (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    snapshot_id BIGINT NOT NULL REFERENCES scrim.lagebild_snapshots(id) ON DELETE CASCADE,
    evidence_type TEXT NOT NULL,
    label TEXT NOT NULL,
    url TEXT,
    reference_id TEXT,
    occurred_at TIMESTAMPTZ,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS lagebild_evidences_snapshot_idx
    ON scrim.lagebild_evidences (snapshot_id, occurred_at DESC, id ASC);

CREATE TABLE IF NOT EXISTS scrim.lagebild_corrections (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    team_id INTEGER NOT NULL REFERENCES scrim.teams(id) ON DELETE CASCADE,
    snapshot_id BIGINT REFERENCES scrim.lagebild_snapshots(id) ON DELETE SET NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    author_user_id TEXT,
    author_display_name TEXT,
    message TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS lagebild_corrections_team_created_idx
    ON scrim.lagebild_corrections (team_id, created_at ASC, id ASC);
