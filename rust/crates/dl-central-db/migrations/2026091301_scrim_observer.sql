-- Persistenter Zustand fuer den automatischen Scrim-/Turnier-Observer.
-- Additiv: bestehende Scrim- und Draft-Pfade bleiben unveraendert.

CREATE TABLE IF NOT EXISTS scrim.observer_sessions (
    id BIGSERIAL PRIMARY KEY,
    session_key TEXT NOT NULL UNIQUE,
    scrim_match_id INTEGER REFERENCES scrim.matches(id) ON DELETE SET NULL,
    draft_code TEXT,
    steam_match_id BIGINT,
    lobby_party_id BIGINT,
    bot_account_id SMALLINT NOT NULL DEFAULT 2,
    mode TEXT NOT NULL DEFAULT 'shadow'
        CHECK (mode IN ('shadow', 'assist', 'auto', 'manual')),
    state TEXT NOT NULL DEFAULT 'waiting'
        CHECK (state IN ('waiting', 'pairing', 'live', 'degraded', 'finished', 'error')),
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    current_account_id BIGINT,
    current_score DOUBLE PRECISION,
    recommended_account_id BIGINT,
    recommended_score DOUBLE PRECISION,
    fallback_reason TEXT,
    last_live_event_at TIMESTAMPTZ,
    last_agent_heartbeat_at TIMESTAMPTZ,
    last_agent_version TEXT,
    last_vconsole_ok BOOLEAN,
    last_game_connected BOOLEAN,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    CHECK (scrim_match_id IS NOT NULL OR draft_code IS NOT NULL OR steam_match_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS idx_scrim_observer_sessions_active
    ON scrim.observer_sessions (state, enabled, updated_at DESC)
    WHERE finished_at IS NULL;
CREATE INDEX IF NOT EXISTS idx_scrim_observer_sessions_steam_match
    ON scrim.observer_sessions (steam_match_id)
    WHERE steam_match_id IS NOT NULL;

-- Ein gerenderter Deadlock-Client kann nur genau eine laufende Partie beobachten.
-- Diese Barriere verhindert konkurrierende Auto-Observer auf Steam Bot 2.
CREATE UNIQUE INDEX IF NOT EXISTS uq_scrim_observer_one_active_bot
    ON scrim.observer_sessions (bot_account_id)
    WHERE enabled = TRUE AND finished_at IS NULL;

CREATE TABLE IF NOT EXISTS scrim.observer_commands (
    id BIGSERIAL PRIMARY KEY,
    observer_session_id BIGINT NOT NULL REFERENCES scrim.observer_sessions(id) ON DELETE CASCADE,
    action TEXT NOT NULL CHECK (action IN ('spectate_lobby', 'directed', 'hero_chase', 'player_view')),
    account_id BIGINT,
    lobby_id BIGINT,
    reason TEXT NOT NULL,
    score DOUBLE PRECISION,
    issued_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    claimed_at TIMESTAMPTZ,
    acked_at TIMESTAMPTZ,
    ack_ok BOOLEAN,
    ack_detail TEXT,
    CHECK (
        (action = 'spectate_lobby' AND lobby_id IS NOT NULL AND account_id IS NULL)
        OR (action = 'directed' AND lobby_id IS NULL AND account_id IS NULL)
        OR (action IN ('hero_chase', 'player_view') AND lobby_id IS NULL AND account_id IS NOT NULL)
    )
);

CREATE INDEX IF NOT EXISTS idx_scrim_observer_commands_poll
    ON scrim.observer_commands (observer_session_id, id)
    WHERE acked_at IS NULL;

CREATE TABLE IF NOT EXISTS scrim.observer_decisions (
    id BIGSERIAL PRIMARY KEY,
    observer_session_id BIGINT NOT NULL REFERENCES scrim.observer_sessions(id) ON DELETE CASCADE,
    observed_at TIMESTAMPTZ NOT NULL,
    account_id BIGINT,
    hero_id INTEGER,
    score DOUBLE PRECISION NOT NULL,
    current_score DOUBLE PRECISION,
    switched BOOLEAN NOT NULL DEFAULT FALSE,
    command_id BIGINT REFERENCES scrim.observer_commands(id) ON DELETE SET NULL,
    reason TEXT NOT NULL,
    factors JSONB NOT NULL DEFAULT '{}'::jsonb,
    frame_age_ms BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_scrim_observer_decisions_session_time
    ON scrim.observer_decisions (observer_session_id, observed_at DESC);
