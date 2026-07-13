CREATE TABLE bot.ai_decision_ledger (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    source TEXT NOT NULL,
    subject_user_id BIGINT,
    guild_id BIGINT,
    input_summary TEXT NOT NULL,
    decision TEXT NOT NULL,
    confidence REAL,
    reason TEXT NOT NULL,
    action_taken TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    CHECK (decision IN ('yes', 'no', 'unsure', 'timeout', 'error', 'suppressed'))
);

CREATE INDEX ai_decision_ledger_source_decided_idx
    ON bot.ai_decision_ledger (source, decided_at DESC);

CREATE INDEX ai_decision_ledger_subject_decided_idx
    ON bot.ai_decision_ledger (subject_user_id, decided_at DESC);

CREATE TABLE bot.brain_reports (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    period_start TIMESTAMPTZ NOT NULL,
    period_end TIMESTAMPTZ NOT NULL,
    kpis JSONB NOT NULL,
    report_text TEXT NOT NULL
);
