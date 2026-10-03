-- Streamer-Vorschlaege aus der Community (Community-Streamer-Bruecke, Paket F).
-- Ein Mitglied schlaegt ueber den Button "Streamer vorschlagen" einen
-- Twitch-Kanal vor. dl-bot speichert den Vorschlag hier und reicht ihn an den
-- Twitch-Bot weiter (POST /internal/twitch/v1/scout/community-suggestion,
-- Idempotency-Key "discord-suggest-<id>"). Bis eine Antwort da ist, bleibt
-- forwarded_at NULL und ein Retry-Loop versucht es erneut.
--
-- Ein Vorschlag je Mitglied und Kanal (Unique auf discord_id, twitch_login).
-- Das Tageslimit je Mitglied prueft der Code ueber created_at.
-- Punkte vergibt Paket C aus dem Outcomes-Endpunkt des Twitch-Bots, nicht hier.
--
-- Rein additiv, kein Fremdschluessel auf core.users. Loeschantraege erfasst
-- dl-community/src/privacy.rs ueber discord_id (Zeile wird geloescht).
CREATE SCHEMA IF NOT EXISTS community;

CREATE TABLE IF NOT EXISTS community.streamer_suggestions (
    id BIGSERIAL PRIMARY KEY,
    discord_id BIGINT NOT NULL CHECK (discord_id > 0),
    twitch_login TEXT NOT NULL CHECK (twitch_login ~ '^[a-z0-9_]{1,25}$'),
    twitch_user_id TEXT NULL CHECK (twitch_user_id IS NULL OR twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    reason TEXT NOT NULL DEFAULT '' CHECK (char_length(reason) <= 500),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN (
        'pending',
        'created',
        'already_known',
        'already_partner',
        'blocked',
        'not_found',
        'rejected'
    )),
    forward_attempts INTEGER NOT NULL DEFAULT 0 CHECK (forward_attempts >= 0),
    last_attempt_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    forwarded_at TIMESTAMPTZ NULL,
    CONSTRAINT streamer_suggestions_member_channel_key UNIQUE (discord_id, twitch_login),
    CONSTRAINT streamer_suggestions_pending_not_forwarded
        CHECK ((status = 'pending') = (forwarded_at IS NULL))
);

-- Tageslimit je Mitglied.
CREATE INDEX IF NOT EXISTS streamer_suggestions_member_created_idx
    ON community.streamer_suggestions (discord_id, created_at);

-- Retry-Loop: nur offene Weitergaben.
CREATE INDEX IF NOT EXISTS streamer_suggestions_pending_idx
    ON community.streamer_suggestions (last_attempt_at NULLS FIRST, id)
    WHERE forwarded_at IS NULL;

-- Erster Vorschlagender je Kanal (Punkte-Abgleich Paket C).
CREATE INDEX IF NOT EXISTS streamer_suggestions_channel_first_idx
    ON community.streamer_suggestions (twitch_user_id, created_at, id)
    WHERE twitch_user_id IS NOT NULL;

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
    ('community', 'streamer_suggestions', 'discord_id', 'user_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-bot', 'Discord user id of the member who suggested a Twitch channel; the row is deleted on user erasure'),
    ('community', 'streamer_suggestions', 'reason', 'free_text', 'retain_operational', 'delete_row_on_user_delete', 'dl-bot', 'short free text why the suggested channel fits the community'),
    ('community', 'streamer_suggestions', 'twitch_user_id', 'domain_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-bot', 'Twitch user id of the suggested public channel, not of the member')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE SET
    data_category = EXCLUDED.data_category,
    retention_action = EXCLUDED.retention_action,
    erasure_action = EXCLUDED.erasure_action,
    owner_service = EXCLUDED.owner_service,
    reason = EXCLUDED.reason;

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON community.streamer_suggestions TO deadlock';
        EXECUTE 'GRANT USAGE, SELECT ON SEQUENCE community.streamer_suggestions_id_seq TO deadlock';
    END IF;
END
$roles$;
