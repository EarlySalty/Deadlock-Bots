-- Additive Migration fuer den Scrim-Draft-Warteraum und die Lobby-Automatik:
-- legt nur Spalten auf turnier.draft_sessions an, Bestandszeilen bleiben
-- unberuehrt (lobby_status 'keine' heisst: alter Vertrag, kein Warteraum).
ALTER TABLE turnier.draft_sessions
    ADD COLUMN IF NOT EXISTS bans_per_team INTEGER NOT NULL DEFAULT 2,
    ADD COLUMN IF NOT EXISTS team1_claimed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS team2_claimed_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS team1_ready BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS team2_ready BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS lobby_status TEXT NOT NULL DEFAULT 'keine',
    ADD COLUMN IF NOT EXISTS lobby_party_id TEXT,
    ADD COLUMN IF NOT EXISTS lobby_join_code TEXT,
    ADD COLUMN IF NOT EXISTS lobby_error TEXT,
    ADD COLUMN IF NOT EXISTS lobby_match_id TEXT,
    ADD COLUMN IF NOT EXISTS lobby_result JSONB,
    ADD COLUMN IF NOT EXISTS discord_lobby_posted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS discord_result_posted_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS rematch_of_code TEXT;