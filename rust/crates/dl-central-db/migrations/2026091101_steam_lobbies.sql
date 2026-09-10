CREATE TABLE IF NOT EXISTS steam.steam_lobbies (
    id BIGINT PRIMARY KEY REFERENCES steam.v1_operations(id),
    bot_account_id SMALLINT NOT NULL CHECK (bot_account_id > 0),
    party_id TEXT UNIQUE,
    phase TEXT NOT NULL CHECK (phase IN ('provisioning','offen','gestartet','beendet','verlassen')),
    state JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS steam_lobbies_open_idx ON steam.steam_lobbies(bot_account_id,phase);
