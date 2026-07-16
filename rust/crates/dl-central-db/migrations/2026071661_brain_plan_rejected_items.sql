CREATE TABLE brain.plan_items_verworfen (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id           BIGINT NOT NULL REFERENCES brain.plan_runs(id) ON DELETE CASCADE,
    prioritaet       SMALLINT NOT NULL,
    bereich          TEXT NOT NULL,
    titel            TEXT NOT NULL,
    begruendung      TEXT NOT NULL,
    aktion           TEXT NOT NULL,
    beleg            TEXT NOT NULL,
    beleg_art        TEXT NOT NULL,
    verworfen_grund  TEXT NOT NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX plan_items_verworfen_run_idx ON brain.plan_items_verworfen(run_id);
