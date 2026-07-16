-- Freie Draft-Lobbys: Draft-Sessions ohne Turnier-Bindung, mit eigener
-- Sequenz, Captain-Tokens und optionalem Zug-Timer.
-- Rein additiv: Bestands-Turnier-Drafts behalten mit NULL in allen neuen
-- Spalten ihr bisheriges Verhalten.

-- NULL kennzeichnet eine freie Lobby; gesetzte Werte bleiben Turnier-Drafts.
ALTER TABLE turnier.draft_sessions
    ALTER COLUMN bracket_match_id DROP NOT NULL,
    ADD COLUMN IF NOT EXISTS code TEXT,
    ADD COLUMN IF NOT EXISTS team1_name TEXT,
    ADD COLUMN IF NOT EXISTS team2_name TEXT,
    ADD COLUMN IF NOT EXISTS team1_token TEXT,
    ADD COLUMN IF NOT EXISTS team2_token TEXT,
    ADD COLUMN IF NOT EXISTS sequence JSONB,
    ADD COLUMN IF NOT EXISTS round_seconds INTEGER,
    ADD COLUMN IF NOT EXISTS reserve_seconds INTEGER,
    ADD COLUMN IF NOT EXISTS team1_reserve_left INTEGER,
    ADD COLUMN IF NOT EXISTS team2_reserve_left INTEGER,
    ADD COLUMN IF NOT EXISTS deadline_at TIMESTAMPTZ;

-- Nur freie Lobbys tragen einen Code. Der partielle Index hält Codes eindeutig,
-- ohne die beliebig vielen NULL-Werte reiner Turnier-Drafts zu indizieren.
CREATE UNIQUE INDEX IF NOT EXISTS draft_sessions_code_unique_idx
    ON turnier.draft_sessions (code)
    WHERE code IS NOT NULL;

-- Automatisch aufgelöste Timer-Züge bleiben von manuellen Aktionen unterscheidbar.
ALTER TABLE turnier.draft_actions
    ADD COLUMN IF NOT EXISTS is_auto BOOLEAN NOT NULL DEFAULT FALSE;
