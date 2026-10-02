-- Twitch-Konten bleiben auch ohne Discord-Zuordnung personenbezogen.
-- Ein Hash dient ausschließlich als dauerhafte Importsperre nach Löschung.
CREATE TABLE community_points.twitch_erasure_tombstones (
    account_hash TEXT PRIMARY KEY CHECK (account_hash ~ '^[a-f0-9]{64}$'),
    erased_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE FUNCTION community_points.twitch_erasure_guard() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    payload JSONB := to_jsonb(NEW);
    identity TEXT;
BEGIN
    PERFORM pg_advisory_xact_lock(2061030115);
    FOREACH identity IN ARRAY ARRAY[
        payload->>'twitch_user_id', payload->>'channel_twitch_user_id',
        payload->>'streamer_twitch_user_id', payload->>'submitted_by_twitch_user_id'
    ] LOOP
        IF identity IS NOT NULL AND EXISTS (
            SELECT 1 FROM community_points.twitch_erasure_tombstones
             WHERE account_hash = encode(sha256(convert_to(identity, 'UTF8')), 'hex')
        ) THEN
            RETURN NULL;
        END IF;
    END LOOP;
    RETURN NEW;
END
$$;

CREATE TRIGGER twitch_viewer_erasure_guard BEFORE INSERT OR UPDATE
    ON community_points.twitch_viewer_daily FOR EACH ROW
    EXECUTE FUNCTION community_points.twitch_erasure_guard();
CREATE TRIGGER twitch_streamer_erasure_guard BEFORE INSERT OR UPDATE
    ON community_points.twitch_streamer_daily FOR EACH ROW
    EXECUTE FUNCTION community_points.twitch_erasure_guard();
CREATE TRIGGER twitch_ledger_erasure_guard BEFORE INSERT OR UPDATE
    ON community_points.ledger FOR EACH ROW
    EXECUTE FUNCTION community_points.twitch_erasure_guard();
CREATE TRIGGER twitch_clip_erasure_guard BEFORE INSERT OR UPDATE
    ON clips.clip_submissions FOR EACH ROW
    EXECUTE FUNCTION community_points.twitch_erasure_guard();
CREATE TRIGGER twitch_result_erasure_guard BEFORE INSERT OR UPDATE
    ON clips.clip_contest_results FOR EACH ROW
    EXECUTE FUNCTION community_points.twitch_erasure_guard();
CREATE TRIGGER twitch_suggestion_erasure_guard BEFORE INSERT OR UPDATE
    ON community.streamer_suggestions FOR EACH ROW
    EXECUTE FUNCTION community_points.twitch_erasure_guard();

CREATE FUNCTION community_points.erase_linked_twitch_accounts() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE identity TEXT;
BEGIN
    IF NOT NEW.opted_out AND NEW.deleted_at IS NULL THEN
        RETURN NEW;
    END IF;
    PERFORM pg_advisory_xact_lock(2061030115);
    FOR identity IN
        SELECT platform_user_id FROM core.discord_platform_connections
         WHERE discord_id = NEW.user_id AND platform = 'twitch'
        UNION
        SELECT streamer_twitch_user_id FROM community_points.twitch_streamer_daily
         WHERE discord_user_id = NEW.user_id
    LOOP
        INSERT INTO community_points.twitch_erasure_tombstones(account_hash)
        VALUES (encode(sha256(convert_to(identity, 'UTF8')), 'hex'))
        ON CONFLICT (account_hash) DO NOTHING;
        DELETE FROM community_points.twitch_viewer_daily
         WHERE twitch_user_id = identity OR channel_twitch_user_id = identity;
        DELETE FROM community_points.twitch_streamer_daily
         WHERE streamer_twitch_user_id = identity;
        DELETE FROM community_points.ledger WHERE streamer_twitch_user_id = identity;
        DELETE FROM clips.clip_contest_results WHERE streamer_twitch_user_id = identity;
        DELETE FROM clips.clip_submissions
         WHERE streamer_twitch_user_id = identity OR submitted_by_twitch_user_id = identity;
        DELETE FROM community.streamer_suggestions WHERE twitch_user_id = identity;
    END LOOP;
    RETURN NEW;
END
$$;
CREATE TRIGGER linked_twitch_erasure AFTER INSERT OR UPDATE
    ON core.user_privacy FOR EACH ROW
    EXECUTE FUNCTION community_points.erase_linked_twitch_accounts();

UPDATE core.privacy_field_registry
   SET erasure_action = 'delete_row_on_user_delete',
       reason = 'Personenbezogene Twitch-Kontodaten; die verknüpfte Kontolöschung entfernt die Zeile und sperrt erneute Imports.'
 WHERE (schema_name = 'community_points' AND table_name IN ('twitch_viewer_daily', 'twitch_streamer_daily', 'ledger')
        AND column_name IN ('twitch_user_id', 'channel_twitch_user_id', 'streamer_twitch_user_id'));
INSERT INTO core.privacy_field_registry(schema_name, table_name, column_name, data_category,
    retention_action, erasure_action, owner_service, reason)
VALUES ('community_points', 'twitch_erasure_tombstones', 'account_hash', 'domain_id',
    'retain_operational', 'retain_hash_only', 'dl-bot', 'SHA-256-Kontoreferenz ausschließlich zur Verhinderung erneuter Speicherung nach Löschung');
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT ON community_points.twitch_erasure_tombstones TO deadlock;
    END IF;
END $$;
