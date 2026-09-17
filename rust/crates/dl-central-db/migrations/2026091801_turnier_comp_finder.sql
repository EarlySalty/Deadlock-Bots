-- Independent, anonymous Comp-Finder rooms. No tournament/draft/Steam side effects.
CREATE TABLE turnier.comp_lobbies (
    code TEXT PRIMARY KEY CHECK (code ~ '^[A-HJ-NP-Z2-9]{8}$'),
    host_member_id TEXT NOT NULL,
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '24 hours')
);
CREATE INDEX comp_lobbies_expiry_idx ON turnier.comp_lobbies (expires_at);

CREATE TABLE turnier.comp_members (
    id TEXT PRIMARY KEY,
    lobby_code TEXT NOT NULL REFERENCES turnier.comp_lobbies(code) ON DELETE CASCADE,
    name TEXT NOT NULL CHECK (char_length(btrim(name)) BETWEEN 1 AND 32),
    token_hash TEXT NOT NULL CHECK (token_hash ~ '^[0-9a-f]{64}$'),
    preferences JSONB NOT NULL DEFAULT '[]'::jsonb
        CHECK (jsonb_typeof(preferences) = 'array' AND jsonb_array_length(preferences) <= 128),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    joined_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (lobby_code, token_hash)
);
CREATE INDEX comp_members_lobby_idx ON turnier.comp_members (lobby_code, joined_at, id);
-- Capacity and all mutations are serialized on comp_lobbies FOR UPDATE.
-- Tokens are random browser capabilities; only SHA-256 hashes are persisted.
-- Expired rooms are immediately inaccessible and purged on subsequent creation.
