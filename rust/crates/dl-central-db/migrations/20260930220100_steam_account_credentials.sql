-- Kontogebundene Runtime-Zugänge. Der Master-Key bleibt außerhalb der DB.
-- Bot-ID alleine reicht nicht: ein neu zugeordnetes Steam-Konto darf die
-- Freigaben seines Vorgängers nicht übernehmen.
CREATE TABLE steam.account_credentials (
    bot_account_id SMALLINT NOT NULL CHECK (bot_account_id > 0),
    steam_id BIGINT NOT NULL CHECK (steam_id > 0),
    login_hash TEXT NOT NULL CHECK (login_hash ~ '^sha256:[0-9a-f]{64}$'),
    guard_token_enc BYTEA,
    refresh_token_enc BYTEA,
    enc_version INTEGER NOT NULL DEFAULT 1 CHECK (enc_version = 1),
    revoked_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (bot_account_id, steam_id, login_hash),
    CHECK (guard_token_enc IS NULL OR octet_length(guard_token_enc) >= 32),
    CHECK (refresh_token_enc IS NULL OR octet_length(refresh_token_enc) >= 32)
);
