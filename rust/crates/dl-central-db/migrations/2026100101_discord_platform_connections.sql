-- Plattform-Verknuepfungen, die ein Mitglied selbst aus seinem Discord-Profil
-- freigibt (Discord-Verbindungen). Erste Plattform ist Twitch: der Button
-- "Twitch verknuepfen" im Verify-Panel schreibt hier Discord-ID und
-- Twitch-User-ID. Logins sind nur Anzeige, Schluessel sind die IDs.
--
-- Rein additiv: core.steam_links bleibt unberuehrt. Kein Fremdschluessel auf
-- core.users, damit die Verknuepfung nicht an einem fehlenden Nutzer-Datensatz
-- scheitert (wie core.discord_role_connection_sync_state). Loeschantraege
-- erfasst dl-community/src/privacy.rs ueber discord_id.
CREATE TABLE IF NOT EXISTS core.discord_platform_connections (
    discord_id BIGINT NOT NULL CHECK (discord_id > 0),
    platform TEXT NOT NULL CHECK (platform ~ '^[a-z][a-z0-9_]{0,31}$'),
    platform_user_id TEXT NOT NULL CHECK (platform_user_id <> ''),
    platform_login TEXT NOT NULL CHECK (platform_login <> ''),
    verified BOOLEAN NOT NULL DEFAULT FALSE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (discord_id, platform),
    CONSTRAINT discord_platform_connections_twitch_id_numeric
        CHECK (platform <> 'twitch' OR platform_user_id ~ '^[1-9][0-9]{0,19}$')
);

-- Ein Plattform-Konto gehoert hoechstens einer Discord-ID. Punkte und
-- Erkennung duerfen nie auf zwei Mitglieder gleichzeitig laufen; die
-- juengste Verknuepfung gewinnt (der Schreiber raeumt die alte Zeile ab).
CREATE UNIQUE INDEX IF NOT EXISTS discord_platform_connections_account_key
    ON core.discord_platform_connections (platform, platform_user_id);

CREATE INDEX IF NOT EXISTS discord_platform_connections_updated_idx
    ON core.discord_platform_connections (platform, updated_at);

INSERT INTO core.privacy_field_registry(
    schema_name,
    table_name,
    column_name,
    data_category,
    retention_action,
    erasure_action,
    owner_service,
    reason
)
VALUES
    ('core', 'discord_platform_connections', 'discord_id', 'user_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-dashboard', 'Discord user id of the member who shared a platform account; the row is deleted on user erasure'),
    ('core', 'discord_platform_connections', 'platform_user_id', 'domain_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-dashboard', 'platform account id shared by the member (Twitch user id)'),
    ('core', 'discord_platform_connections', 'platform_login', 'display_name', 'retain_operational', 'delete_row_on_user_delete', 'dl-dashboard', 'platform login shared by the member, display only')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE SET
    data_category = EXCLUDED.data_category,
    retention_action = EXCLUDED.retention_action,
    erasure_action = EXCLUDED.erasure_action,
    owner_service = EXCLUDED.owner_service,
    reason = EXCLUDED.reason;

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON core.discord_platform_connections TO deadlock';
    END IF;
END
$roles$;
