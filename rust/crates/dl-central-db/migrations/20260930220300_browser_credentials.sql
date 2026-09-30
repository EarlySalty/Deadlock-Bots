-- Browser-Sitzungszustand bleibt als Ganzes verschlüsselt, nach Anbieter und
-- ausdrücklich gewähltem Konto getrennt. Keine Cookies in Logs oder Dateien.
CREATE TABLE core.browser_credentials (
    provider TEXT NOT NULL CHECK (provider = 'gemini-browser'),
    account_id TEXT NOT NULL CHECK (account_id ~ '^[A-Za-z0-9_-]{1,128}$'),
    state_enc BYTEA NOT NULL CHECK (octet_length(state_enc) >= 32),
    enc_version INTEGER NOT NULL DEFAULT 1 CHECK (enc_version = 1),
    revision BIGINT NOT NULL DEFAULT 0 CHECK (revision >= 0),
    revoked_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, account_id)
);
