CREATE TABLE bot.twitch_streamer_invite_code_history (
    history_id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    source_login_snapshot TEXT NOT NULL,
    guild_id BIGINT,
    invite_code TEXT,
    twitch_user_id TEXT,
    channel_id BIGINT,
    valid_from TIMESTAMPTZ,
    valid_until TIMESTAMPTZ,
    attribution_safe BOOLEAN NOT NULL
);

CREATE INDEX twitch_streamer_invite_code_history_lookup_idx
    ON bot.twitch_streamer_invite_code_history
       (guild_id, invite_code, valid_from, valid_until);

-- Preserve the pre-migration mapping identity, but do not infer when it became
-- authoritative. Unbounded legacy snapshots remain fail-closed until a sync
-- observes a complete mapping.
INSERT INTO bot.twitch_streamer_invite_code_history (
    source_login_snapshot,
    guild_id,
    invite_code,
    twitch_user_id,
    channel_id,
    valid_from,
    valid_until,
    attribution_safe
)
SELECT streamer_login, guild_id, invite_code, twitch_user_id, channel_id,
       NULL, NULL, FALSE
FROM bot.twitch_streamer_invites;

CREATE FUNCTION bot.capture_twitch_streamer_invite_code_history()
RETURNS TRIGGER
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog, bot
AS $$
DECLARE
    observed_at TIMESTAMPTZ := clock_timestamp();
    mapping_changed BOOLEAN;
    new_mapping_safe BOOLEAN;
    safe_mapping_exists BOOLEAN;
BEGIN
    IF TG_OP = 'INSERT' THEN
        new_mapping_safe := NEW.guild_id IS NOT NULL
            AND NEW.invite_code IS NOT NULL
            AND btrim(NEW.invite_code) <> ''
            AND NEW.twitch_user_id IS NOT NULL
            AND NEW.channel_id IS NOT NULL
            AND left(NEW.streamer_login, 9) <> '__legacy_';

        INSERT INTO bot.twitch_streamer_invite_code_history (
            source_login_snapshot, guild_id, invite_code, twitch_user_id,
            channel_id, valid_from, attribution_safe
        ) VALUES (
            NEW.streamer_login, NEW.guild_id, NEW.invite_code, NEW.twitch_user_id,
            NEW.channel_id, observed_at, new_mapping_safe
        );
        RETURN NEW;
    END IF;

    IF TG_OP = 'DELETE' THEN
        UPDATE bot.twitch_streamer_invite_code_history
        SET valid_until = observed_at
        WHERE valid_until IS NULL
          AND guild_id IS NOT DISTINCT FROM OLD.guild_id
          AND invite_code IS NOT DISTINCT FROM OLD.invite_code
          AND twitch_user_id IS NOT DISTINCT FROM OLD.twitch_user_id
          AND channel_id IS NOT DISTINCT FROM OLD.channel_id
          AND (OLD.twitch_user_id IS NOT NULL
               OR source_login_snapshot = OLD.streamer_login);
        RETURN OLD;
    END IF;

    mapping_changed := left(NEW.streamer_login, 9) = '__legacy_'
        OR ROW(NEW.guild_id, NEW.invite_code, NEW.twitch_user_id, NEW.channel_id)
            IS DISTINCT FROM ROW(OLD.guild_id, OLD.invite_code, OLD.twitch_user_id, OLD.channel_id)
        OR ((OLD.twitch_user_id IS NULL OR NEW.twitch_user_id IS NULL)
            AND NEW.streamer_login IS DISTINCT FROM OLD.streamer_login);
    new_mapping_safe := NEW.guild_id IS NOT NULL
        AND NEW.invite_code IS NOT NULL
        AND btrim(NEW.invite_code) <> ''
        AND NEW.twitch_user_id IS NOT NULL
        AND NEW.channel_id IS NOT NULL
        AND left(NEW.streamer_login, 9) <> '__legacy_';

    SELECT EXISTS (
        SELECT 1
        FROM bot.twitch_streamer_invite_code_history AS history
        WHERE history.valid_until IS NULL
          AND history.guild_id IS NOT DISTINCT FROM NEW.guild_id
          AND history.invite_code IS NOT DISTINCT FROM NEW.invite_code
          AND history.twitch_user_id IS NOT DISTINCT FROM NEW.twitch_user_id
          AND history.channel_id IS NOT DISTINCT FROM NEW.channel_id
          AND (NEW.twitch_user_id IS NOT NULL
               OR history.source_login_snapshot = NEW.streamer_login)
          AND history.attribution_safe
    ) INTO safe_mapping_exists;

    IF NOT mapping_changed AND (NOT new_mapping_safe OR safe_mapping_exists) THEN
        RETURN NEW;
    END IF;

    UPDATE bot.twitch_streamer_invite_code_history
    SET valid_until = observed_at
    WHERE valid_until IS NULL
      AND guild_id IS NOT DISTINCT FROM OLD.guild_id
      AND invite_code IS NOT DISTINCT FROM OLD.invite_code
      AND twitch_user_id IS NOT DISTINCT FROM OLD.twitch_user_id
      AND channel_id IS NOT DISTINCT FROM OLD.channel_id
      AND (OLD.twitch_user_id IS NOT NULL
           OR source_login_snapshot = OLD.streamer_login);

    IF left(NEW.streamer_login, 9) = '__legacy_' THEN
        RETURN NEW;
    END IF;

    INSERT INTO bot.twitch_streamer_invite_code_history (
        source_login_snapshot, guild_id, invite_code, twitch_user_id,
        channel_id, valid_from, attribution_safe
    ) VALUES (
        NEW.streamer_login, NEW.guild_id, NEW.invite_code, NEW.twitch_user_id,
        NEW.channel_id, observed_at, new_mapping_safe
    );
    RETURN NEW;
END;
$$;

REVOKE ALL ON FUNCTION bot.capture_twitch_streamer_invite_code_history() FROM PUBLIC;

CREATE TRIGGER twitch_streamer_invite_code_history_trigger
AFTER INSERT OR UPDATE OR DELETE ON bot.twitch_streamer_invites
FOR EACH ROW EXECUTE FUNCTION bot.capture_twitch_streamer_invite_code_history();

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        EXECUTE 'GRANT SELECT ON bot.twitch_streamer_invite_code_history TO deadlock';
    END IF;
END
$roles$;
