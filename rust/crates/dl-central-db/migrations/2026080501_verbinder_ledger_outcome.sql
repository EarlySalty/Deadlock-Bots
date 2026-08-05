-- Verbinder-Shadow (Agent R1): Grundwahrheits-Spalten am Entscheidungs-Ledger.
-- outcome = was die Realitaet tat (trafen sich die Vorgeschlagenen binnen 24 h),
-- aufgeloest vom dl-verbinder-Resolver aus activity.voice_session_log.
ALTER TABLE bot.ai_decision_ledger
    ADD COLUMN outcome TEXT,
    ADD COLUMN outcome_at TIMESTAMPTZ;

ALTER TABLE bot.ai_decision_ledger
    ADD CONSTRAINT ai_decision_ledger_outcome_check
    CHECK (outcome IS NULL OR outcome IN ('met', 'not_met'));

-- Hotpath des Resolvers: unaufgeloeste yes-Urteile einer Quelle.
CREATE INDEX ai_decision_ledger_outcome_pending_idx
    ON bot.ai_decision_ledger (source, decided_at)
    WHERE decision = 'yes' AND outcome IS NULL;
