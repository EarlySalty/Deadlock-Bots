-- Lauf-Protokoll des Phase-2-Feeders (dl-brain-feeder). Ein Insert je Lauf,
-- auch im Fehlerfall — sofern die zentrale DB erreichbar ist; frühe
-- Verbindungsfehler stehen nur im Journal. Ausfälle müssen zählbar sein
-- (Judge-Regel).
CREATE TABLE brain.feeder_runs (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,
    digest_path TEXT,
    gesehen INTEGER,
    relevant INTEGER,
    kategorien TEXT[],
    status TEXT NOT NULL CHECK (status IN ('ok', 'partial', 'error')),
    error TEXT,
    committed BOOLEAN NOT NULL DEFAULT false,
    pushed BOOLEAN NOT NULL DEFAULT false
);

CREATE INDEX feeder_runs_run_at_idx ON brain.feeder_runs (run_at DESC);
