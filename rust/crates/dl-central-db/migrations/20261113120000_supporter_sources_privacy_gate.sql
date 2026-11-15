SET LOCAL lock_timeout TO '5s';

CREATE OR REPLACE FUNCTION core.supporter_sources_privacy_gate() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    privacy_key CONSTANT bigint := NEW.discord_id # (-9223372036854775808)::bigint;
BEGIN
    IF TG_OP = 'UPDATE'
       AND NEW.discord_id = OLD.discord_id
       AND NEW.source = OLD.source
       AND NEW.source_ref = OLD.source_ref
       AND NEW.valid_until IS NOT NULL
       AND (OLD.valid_until IS NULL OR NEW.valid_until <= OLD.valid_until) THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'INSERT' THEN
        PERFORM pg_advisory_xact_lock(privacy_key);
    ELSIF NOT pg_try_advisory_xact_lock(privacy_key) THEN
        RAISE EXCEPTION USING
            ERRCODE = 'serialization_failure',
            MESSAGE = 'core.supporter_sources: an update that grants or extends support needs the member privacy lock before the row lock',
            HINT = 'Take lock_user_privacy before the update, or write through INSERT ... ON CONFLICT, and retry the transaction.';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM core.user_privacy
         WHERE user_id = NEW.discord_id
           AND (opted_out OR deleted_at IS NOT NULL)
    ) THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'INSERT' THEN
        PERFORM 1 FROM core.supporter_sources
          WHERE discord_id = NEW.discord_id
            AND source = NEW.source
            AND source_ref = NEW.source_ref
          FOR UPDATE;
        IF FOUND THEN
            RETURN NEW;
        END IF;
    END IF;
    RETURN NULL;
END;
$$;

REVOKE ALL ON FUNCTION core.supporter_sources_privacy_gate() FROM PUBLIC;

DROP TRIGGER IF EXISTS supporter_sources_privacy_gate ON core.supporter_sources;
CREATE TRIGGER supporter_sources_privacy_gate
    BEFORE INSERT OR UPDATE ON core.supporter_sources
    FOR EACH ROW EXECUTE FUNCTION core.supporter_sources_privacy_gate();

COMMENT ON FUNCTION core.supporter_sources_privacy_gate() IS
    'Privacy gate for writes to core.supporter_sources. An update that keeps the key and only shortens or ends valid_until always passes without any lock. Every other write needs the per-user privacy lock that delete_user_data takes: an insert waits for it before any row lock, an update only tries it and fails with serialization_failure, because its row lock is already held and waiting would invert the order delete_user_data uses. For a member who opted out or was deleted, a new row is dropped; an insert whose key already exists locks that row and passes on to its ON CONFLICT update, which may only shorten or end valid_until.';

COMMENT ON TABLE core.supporter_sources IS
    'One row per Discord member and supporter source. Each source owner (dl-bot boost and manual, Steam bot stripe, Twitch dashboard twitch_sub and twitch_bot_customer) writes and deletes only its own rows; the table owner bypasses row level security, and the policy supporter_sources_twitch_rows limits twitchdash to Twitch rows without an amount. The trigger supporter_sources_privacy_gate drops new or extended grants for members who opted out or were deleted and lets their sources only shorten or end. Any row with valid_until NULL or in the future makes the member a supporter.';
