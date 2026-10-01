-- Community-Punkte (Community-Streamer-Bruecke, Paket C).
--
-- Spiegel der Tageswerte aus dem Twitch-Bot (interne API, Paket B), ein
-- Punkte-Ledger fuer Discord-Ereignisse und die Cursor des Sync-Bins
-- dl-community-points-sync. Rein additiv, eigenes Schema.
--
-- Schluessel sind ausschliesslich IDs. Twitch-Zuschauer werden nur ueber ihre
-- Twitch-User-ID gefuehrt (kein Login gespeichert). Zugerechnet wird erst
-- beim Lesen ueber core.discord_platform_connections (Paket A), dort greift
-- auch der Privacy-Grabstein. Loeschantraege erfasst dl-community/src/privacy.rs.
CREATE SCHEMA IF NOT EXISTS community_points;

-- Zuschauer je Partnerkanal und Berliner Tag. Wird beim Sync idempotent
-- ueberschrieben (Zeilen sind Tageswerte, juengere Quelle gewinnt).
CREATE TABLE IF NOT EXISTS community_points.twitch_viewer_daily (
    twitch_user_id TEXT NOT NULL CHECK (twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    channel_twitch_user_id TEXT NOT NULL CHECK (channel_twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    day DATE NOT NULL,
    watch_minutes INTEGER NOT NULL DEFAULT 0 CHECK (watch_minutes >= 0),
    chat_messages INTEGER NOT NULL DEFAULT 0 CHECK (chat_messages >= 0),
    points_watch INTEGER NOT NULL DEFAULT 0 CHECK (points_watch >= 0),
    points_chat INTEGER NOT NULL DEFAULT 0 CHECK (points_chat >= 0),
    points_discovery INTEGER NOT NULL DEFAULT 0 CHECK (points_discovery >= 0),
    source_updated_at TIMESTAMPTZ NOT NULL,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (twitch_user_id, channel_twitch_user_id, day)
);

CREATE INDEX IF NOT EXISTS twitch_viewer_daily_day_idx
    ON community_points.twitch_viewer_daily (day);

CREATE INDEX IF NOT EXISTS twitch_viewer_daily_channel_day_idx
    ON community_points.twitch_viewer_daily (channel_twitch_user_id, day);

-- Partner-Streamer je Berliner Tag. discord_user_id kommt aus der
-- Streamer-Identitaet des Twitch-Bots (kann fehlen).
CREATE TABLE IF NOT EXISTS community_points.twitch_streamer_daily (
    streamer_twitch_user_id TEXT NOT NULL CHECK (streamer_twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    day DATE NOT NULL,
    streamer_login TEXT NOT NULL CHECK (streamer_login <> ''),
    discord_user_id BIGINT CHECK (discord_user_id IS NULL OR discord_user_id > 0),
    viewer_minutes INTEGER NOT NULL DEFAULT 0 CHECK (viewer_minutes >= 0),
    unique_viewers INTEGER NOT NULL DEFAULT 0 CHECK (unique_viewers >= 0),
    raids_to_partners INTEGER NOT NULL DEFAULT 0 CHECK (raids_to_partners >= 0),
    source_updated_at TIMESTAMPTZ NOT NULL,
    synced_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (streamer_twitch_user_id, day)
);

CREATE INDEX IF NOT EXISTS twitch_streamer_daily_day_idx
    ON community_points.twitch_streamer_daily (day);

-- Punkte aus Discord-Ereignissen. Genau ein Empfaenger: ein Community-Mitglied
-- (discord_id) oder ein Partner-Streamer (streamer_twitch_user_id, zaehlt nur
-- im Streamer-Leaderboard). (source, ref) macht jeden Import idempotent.
CREATE TABLE IF NOT EXISTS community_points.ledger (
    id BIGSERIAL PRIMARY KEY,
    discord_id BIGINT CHECK (discord_id IS NULL OR discord_id > 0),
    streamer_twitch_user_id TEXT CHECK (streamer_twitch_user_id IS NULL OR streamer_twitch_user_id ~ '^[1-9][0-9]{0,19}$'),
    source TEXT NOT NULL CHECK (source IN ('clip_place', 'clip_vote', 'streamer_suggestion', 'streamer_qualified_join')),
    ref TEXT NOT NULL CHECK (ref <> ''),
    points INTEGER NOT NULL CHECK (points <> 0),
    occurred_at TIMESTAMPTZ NOT NULL,
    meta JSONB NOT NULL DEFAULT '{}'::jsonb CHECK (jsonb_typeof(meta) = 'object'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source, ref),
    CHECK (num_nonnulls(discord_id, streamer_twitch_user_id) = 1)
);

CREATE INDEX IF NOT EXISTS ledger_discord_occurred_idx
    ON community_points.ledger (discord_id, occurred_at)
    WHERE discord_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS ledger_streamer_occurred_idx
    ON community_points.ledger (streamer_twitch_user_id, occurred_at)
    WHERE streamer_twitch_user_id IS NOT NULL;

-- Cursor des Syncs je Quelle (Wert exakt so, wie die Quelle ihn liefert).
CREATE TABLE IF NOT EXISTS community_points.sync_state (
    name TEXT PRIMARY KEY CHECK (name <> ''),
    cursor TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

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
    ('community_points', 'twitch_viewer_daily', 'twitch_user_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-community-points-sync', 'Twitch user id from the Twitch bot daily values; only attributed to a Discord member through core.discord_platform_connections, which is deleted on user erasure'),
    ('community_points', 'twitch_viewer_daily', 'channel_twitch_user_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-community-points-sync', 'Twitch user id of the partner channel'),
    ('community_points', 'twitch_streamer_daily', 'streamer_twitch_user_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-community-points-sync', 'Twitch user id of the partner streamer'),
    ('community_points', 'twitch_streamer_daily', 'streamer_login', 'display_name', 'retain_operational', 'retain_non_personal', 'dl-community-points-sync', 'public Twitch channel login of the partner streamer, display only'),
    ('community_points', 'twitch_streamer_daily', 'discord_user_id', 'user_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-community-points-sync', 'Discord user id of the partner streamer; the rows are deleted on user erasure'),
    ('community_points', 'ledger', 'discord_id', 'user_id', 'retain_operational', 'delete_row_on_user_delete', 'dl-community-points-sync', 'Discord user id credited with community points; the rows are deleted on user erasure'),
    ('community_points', 'ledger', 'streamer_twitch_user_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-community-points-sync', 'Twitch user id of the partner streamer credited in the streamer leaderboard'),
    ('community_points', 'ledger', 'meta', 'json_payload', 'retain_operational', 'delete_row_on_user_delete', 'dl-community-points-sync', 'technical event references (window, place, join id), goes with the row')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE SET
    data_category = EXCLUDED.data_category,
    retention_action = EXCLUDED.retention_action,
    erasure_action = EXCLUDED.erasure_action,
    owner_service = EXCLUDED.owner_service,
    reason = EXCLUDED.reason;

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        EXECUTE 'GRANT USAGE ON SCHEMA community_points TO deadlock';
        EXECUTE 'GRANT SELECT, INSERT, UPDATE, DELETE ON
            community_points.twitch_viewer_daily,
            community_points.twitch_streamer_daily,
            community_points.ledger,
            community_points.sync_state TO deadlock';
        EXECUTE 'GRANT USAGE, SELECT ON SEQUENCE community_points.ledger_id_seq TO deadlock';
    END IF;
END
$roles$;
