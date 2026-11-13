SET lock_timeout TO '5s';

CREATE OR REPLACE FUNCTION core.supporter_sources_privacy_gate() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(NEW.discord_id # (-9223372036854775808)::bigint);
    IF NOT EXISTS (
        SELECT 1 FROM core.user_privacy
         WHERE user_id = NEW.discord_id
           AND (opted_out OR deleted_at IS NOT NULL)
    ) THEN
        RETURN NEW;
    END IF;
    IF TG_OP = 'UPDATE'
       AND NEW.discord_id = OLD.discord_id
       AND NEW.source = OLD.source
       AND NEW.source_ref = OLD.source_ref
       AND NEW.valid_until IS NOT NULL
       AND (OLD.valid_until IS NULL OR NEW.valid_until <= OLD.valid_until) THEN
        RETURN NEW;
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
    'Takes the per-user privacy lock that delete_user_data takes and drops every insert or update for a member who opted out or was deleted; only shortening valid_until of an existing row passes, so ending a subscription still works.';

COMMENT ON TABLE core.supporter_sources IS
    'One row per Discord member and supporter source. By convention each source owner (dl-bot boost and manual, Steam bot stripe, Twitch bot twitch_sub and twitch_bot_customer) writes and deletes only its own rows; the grants do not enforce that split. The trigger supporter_sources_privacy_gate drops writes for members who opted out or were deleted. Any row with valid_until NULL or in the future makes the member a supporter.';
