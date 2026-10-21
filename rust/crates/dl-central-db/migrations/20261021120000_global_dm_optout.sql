SET lock_timeout TO '5s';

CREATE TABLE IF NOT EXISTS core.dm_optout (
    user_id BIGINT PRIMARY KEY CHECK (user_id > 0),
    source TEXT NOT NULL CHECK (length(source) BETWEEN 1 AND 64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE OR REPLACE FUNCTION core.set_dm_optout(target_user_id BIGINT, optout_source TEXT)
RETURNS VOID
LANGUAGE sql
AS $$
    INSERT INTO core.dm_optout (user_id, source)
    SELECT target_user_id, optout_source
     WHERE target_user_id IS NOT NULL AND target_user_id > 0
    ON CONFLICT (user_id) DO NOTHING;
$$;

CREATE OR REPLACE FUNCTION core.dm_optout_snowflake(raw TEXT)
RETURNS BIGINT
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT CASE
        WHEN raw ~ '^[1-9][0-9]{0,18}$'
         AND (length(raw) < 19 OR raw <= '9223372036854775807')
        THEN raw::BIGINT
    END;
$$;

CREATE OR REPLACE FUNCTION core.dm_optout_from_kv_store()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.ns IN ('solo_lfg_never_ask', 'voice_mate_survey_never', 'voice_pair_never')
       AND NEW.v = 'true' THEN
        PERFORM core.set_dm_optout(core.dm_optout_snowflake(NEW.k), NEW.ns);
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS dm_optout_from_kv_store ON bot.kv_store;
CREATE TRIGGER dm_optout_from_kv_store
    AFTER INSERT OR UPDATE OF v ON bot.kv_store
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_from_kv_store();

CREATE OR REPLACE FUNCTION core.dm_optout_from_flag()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    target BIGINT;
BEGIN
    CASE TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME
        WHEN 'activity.user_retention_tracking' THEN
            IF NEW.opted_out THEN
                target := NEW.user_id;
            END IF;
        WHEN 'bot.concierge_profiles' THEN
            IF NEW.opted_out THEN
                target := NEW.user_id;
            END IF;
        WHEN 'steam.steam_nudge_state' THEN
            IF NEW.opt_out THEN
                target := NEW.user_id;
            END IF;
        WHEN 'turnier.user_profiles' THEN
            IF NEW.notify_discord_dm IS FALSE THEN
                target := NEW.discord_id;
            END IF;
        WHEN 'turnier.tournament_dm_optout' THEN
            target := NEW.discord_id;
        ELSE
            target := NULL;
    END CASE;
    IF target IS NOT NULL THEN
        PERFORM core.set_dm_optout(target, TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME);
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS dm_optout_from_flag ON activity.user_retention_tracking;
CREATE TRIGGER dm_optout_from_flag
    AFTER INSERT OR UPDATE OF opted_out ON activity.user_retention_tracking
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_from_flag();

DROP TRIGGER IF EXISTS dm_optout_from_flag ON bot.concierge_profiles;
CREATE TRIGGER dm_optout_from_flag
    AFTER INSERT OR UPDATE OF opted_out ON bot.concierge_profiles
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_from_flag();

DROP TRIGGER IF EXISTS dm_optout_from_flag ON steam.steam_nudge_state;
CREATE TRIGGER dm_optout_from_flag
    AFTER INSERT OR UPDATE OF opt_out ON steam.steam_nudge_state
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_from_flag();

DROP TRIGGER IF EXISTS dm_optout_from_flag ON turnier.user_profiles;
CREATE TRIGGER dm_optout_from_flag
    AFTER INSERT OR UPDATE OF notify_discord_dm ON turnier.user_profiles
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_from_flag();

DROP TRIGGER IF EXISTS dm_optout_from_flag ON turnier.tournament_dm_optout;
CREATE TRIGGER dm_optout_from_flag
    AFTER INSERT ON turnier.tournament_dm_optout
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_from_flag();

INSERT INTO core.dm_optout (user_id, source)
SELECT DISTINCT ON (user_id) user_id, source
  FROM (
        SELECT core.dm_optout_snowflake(k) AS user_id, ns AS source, 1 AS rank
          FROM bot.kv_store
         WHERE ns IN ('solo_lfg_never_ask', 'voice_mate_survey_never', 'voice_pair_never')
           AND v = 'true'
        UNION ALL
        SELECT discord_id, 'turnier.tournament_dm_optout', 2
          FROM turnier.tournament_dm_optout
        UNION ALL
        SELECT discord_id, 'turnier.user_profiles', 3
          FROM turnier.user_profiles
         WHERE notify_discord_dm IS FALSE
        UNION ALL
        SELECT user_id, 'activity.user_retention_tracking', 4
          FROM activity.user_retention_tracking
         WHERE opted_out
        UNION ALL
        SELECT user_id, 'bot.concierge_profiles', 5
          FROM bot.concierge_profiles
         WHERE opted_out
        UNION ALL
        SELECT user_id, 'steam.steam_nudge_state', 6
          FROM steam.steam_nudge_state
         WHERE opt_out
       ) AS sources
 WHERE user_id IS NOT NULL AND user_id > 0
 ORDER BY user_id, rank
ON CONFLICT (user_id) DO NOTHING;

INSERT INTO core.privacy_field_registry (
    schema_name, table_name, column_name, data_category, retention_action,
    erasure_action, owner_service, reason
) VALUES
 ('core', 'dm_optout', 'user_id', 'user_id', 'retain_operational', 'manual_review', 'dl-discord',
  'Globaler Bot-DM-Opt-out; muss eine Datenlöschung überleben, sonst gingen wieder Bot-DMs raus'),
 ('core', 'dm_optout', 'source', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-discord',
  'Herkunft des Opt-outs als feste Kennung')
ON CONFLICT (schema_name, table_name, column_name) DO NOTHING;
