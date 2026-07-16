-- Plan-Lauf: jeder Lauf wird protokolliert, auch der gescheiterte.
CREATE TABLE brain.plan_runs (
    id                BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    period_start      TIMESTAMPTZ NOT NULL,
    period_end        TIMESTAMPTZ NOT NULL,
    plan_path         TEXT,
    modell            TEXT,
    lage              TEXT,
    vorgeschlagen     INTEGER NOT NULL DEFAULT 0,
    uebernommen       INTEGER NOT NULL DEFAULT 0,
    verworfen         INTEGER NOT NULL DEFAULT 0,
    verworfen_gruende JSONB,
    quellen_fehlend   TEXT[],
    status            TEXT NOT NULL CHECK (status IN ('ok','partial','error')),
    error             TEXT,
    committed         BOOLEAN NOT NULL DEFAULT false,
    pushed            BOOLEAN NOT NULL DEFAULT false
);

CREATE TABLE brain.plan_items (
    id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id         BIGINT NOT NULL REFERENCES brain.plan_runs(id) ON DELETE CASCADE,
    prioritaet     SMALLINT NOT NULL CHECK (prioritaet BETWEEN 1 AND 3),
    bereich        TEXT NOT NULL CHECK (bereich IN ('discord','twitch','steam','turniere','website','brain')),
    titel          TEXT NOT NULL,
    begruendung    TEXT NOT NULL,
    aktion         TEXT NOT NULL,
    beleg          TEXT NOT NULL,
    beleg_art      TEXT NOT NULL CHECK (beleg_art IN ('digest','wiki','pg','betrieb')),
    belegt_gemessen BOOLEAN NOT NULL,
    fingerprint    TEXT NOT NULL,
    status         TEXT NOT NULL DEFAULT 'offen' CHECK (status IN ('offen','angenommen','abgelehnt','erledigt')),
    kommentar      TEXT,
    entschieden_am TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX plan_items_run_idx ON brain.plan_items(run_id);
CREATE INDEX plan_items_offen_idx ON brain.plan_items(status) WHERE status = 'offen';
CREATE INDEX plan_items_fingerprint_idx ON brain.plan_items(fingerprint);
