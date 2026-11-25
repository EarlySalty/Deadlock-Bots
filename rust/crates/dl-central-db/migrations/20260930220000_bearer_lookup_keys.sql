-- Koordinierter Cutover der zugehörigen Rust-Leser und -Schreiber erforderlich.
-- Die externen Links/Cookies bleiben unverändert; nur der DB-Lookup wird ersetzt.
-- Keine Zeilen und keine fachlichen Metadaten werden entfernt.
UPDATE turnier.sessions
SET token = 'sha256:' || encode(sha256(convert_to(token, 'UTF8')), 'hex')
WHERE token !~ '^sha256:[0-9a-f]{64}$';

UPDATE turnier.draft_sessions
SET team1_token = CASE WHEN team1_token IS NOT NULL AND team1_token !~ '^sha256:[0-9a-f]{64}$'
        THEN 'sha256:' || encode(sha256(convert_to(team1_token, 'UTF8')), 'hex') ELSE team1_token END,
    team2_token = CASE WHEN team2_token IS NOT NULL AND team2_token !~ '^sha256:[0-9a-f]{64}$'
        THEN 'sha256:' || encode(sha256(convert_to(team2_token, 'UTF8')), 'hex') ELSE team2_token END;

UPDATE steam.steam_launch_tokens
SET token = 'sha256:' || encode(sha256(convert_to(token, 'UTF8')), 'hex')
WHERE token !~ '^sha256:[0-9a-f]{64}$';

ALTER TABLE turnier.sessions ADD CONSTRAINT sessions_token_hash_only
    CHECK (token ~ '^sha256:[0-9a-f]{64}$');
ALTER TABLE turnier.draft_sessions ADD CONSTRAINT draft_tokens_hash_only
    CHECK ((team1_token IS NULL OR team1_token ~ '^sha256:[0-9a-f]{64}$')
       AND (team2_token IS NULL OR team2_token ~ '^sha256:[0-9a-f]{64}$'));
ALTER TABLE steam.steam_launch_tokens ADD CONSTRAINT launch_tokens_hash_only
    CHECK (token ~ '^sha256:[0-9a-f]{64}$');
