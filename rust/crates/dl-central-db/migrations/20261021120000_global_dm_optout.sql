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
    ON CONFLICT (user_id) DO UPDATE SET source = EXCLUDED.source
     WHERE EXCLUDED.source = 'dm_optout_button';
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

CREATE OR REPLACE FUNCTION core.dm_optout_active_source(target_user_id BIGINT)
RETURNS TEXT
LANGUAGE sql
STABLE
AS $$
    SELECT source
      FROM (
            SELECT ns AS source, 1 AS rank
              FROM bot.kv_store
             WHERE ns IN ('solo_lfg_never_ask', 'voice_mate_survey_never', 'voice_pair_never')
               AND k = target_user_id::TEXT
               AND v = 'true'
            UNION ALL
            SELECT 'turnier.tournament_dm_optout', 2
              FROM turnier.tournament_dm_optout
             WHERE discord_id = target_user_id
            UNION ALL
            SELECT 'turnier.user_profiles', 3
              FROM turnier.user_profiles
             WHERE discord_id = target_user_id
               AND notify_discord_dm IS FALSE
            UNION ALL
            SELECT 'activity.user_retention_tracking', 4
              FROM activity.user_retention_tracking
             WHERE user_id = target_user_id
               AND opted_out
            UNION ALL
            SELECT 'bot.concierge_profiles', 5
              FROM bot.concierge_profiles
             WHERE user_id = target_user_id
               AND opted_out
            UNION ALL
            SELECT 'steam.steam_nudge_state', 6
              FROM steam.steam_nudge_state
             WHERE user_id = target_user_id
               AND opt_out
           ) AS sources
     ORDER BY rank
     LIMIT 1;
$$;

CREATE OR REPLACE FUNCTION core.refresh_dm_optout(target_user_id BIGINT)
RETURNS VOID
LANGUAGE plpgsql
AS $$
DECLARE
    active TEXT;
BEGIN
    IF target_user_id IS NULL OR target_user_id <= 0 THEN
        RETURN;
    END IF;
    active := core.dm_optout_active_source(target_user_id);
    IF active IS NULL THEN
        DELETE FROM core.dm_optout
         WHERE user_id = target_user_id
           AND source <> 'dm_optout_button'
           AND NOT EXISTS (
                SELECT 1
                  FROM core.user_privacy
                 WHERE user_id = target_user_id
                   AND deleted_at IS NOT NULL
           );
    ELSE
        INSERT INTO core.dm_optout AS current (user_id, source)
        VALUES (target_user_id, active)
        ON CONFLICT (user_id) DO UPDATE SET source = EXCLUDED.source
         WHERE current.source <> 'dm_optout_button'
           AND current.source <> EXCLUDED.source;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION core.dm_optout_row_user(relation TEXT, row_data JSONB)
RETURNS BIGINT
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT CASE relation
        WHEN 'bot.kv_store' THEN
            CASE WHEN row_data->>'ns' IN ('solo_lfg_never_ask', 'voice_mate_survey_never', 'voice_pair_never')
                 THEN core.dm_optout_snowflake(row_data->>'k')
            END
        WHEN 'turnier.tournament_dm_optout' THEN (row_data->>'discord_id')::BIGINT
        WHEN 'turnier.user_profiles' THEN (row_data->>'discord_id')::BIGINT
        ELSE (row_data->>'user_id')::BIGINT
    END;
$$;

CREATE OR REPLACE FUNCTION core.dm_optout_sync()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    relation TEXT := TG_TABLE_SCHEMA || '.' || TG_TABLE_NAME;
    old_user BIGINT;
    new_user BIGINT;
BEGIN
    IF TG_OP <> 'INSERT' THEN
        old_user := core.dm_optout_row_user(relation, to_jsonb(OLD));
    END IF;
    IF TG_OP <> 'DELETE' THEN
        new_user := core.dm_optout_row_user(relation, to_jsonb(NEW));
    END IF;
    PERFORM core.refresh_dm_optout(new_user);
    IF old_user IS DISTINCT FROM new_user THEN
        PERFORM core.refresh_dm_optout(old_user);
    END IF;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS dm_optout_sync ON bot.kv_store;
CREATE TRIGGER dm_optout_sync
    AFTER INSERT OR DELETE OR UPDATE OF ns, k, v ON bot.kv_store
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_sync();

DROP TRIGGER IF EXISTS dm_optout_sync ON activity.user_retention_tracking;
CREATE TRIGGER dm_optout_sync
    AFTER INSERT OR DELETE OR UPDATE OF user_id, opted_out ON activity.user_retention_tracking
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_sync();

DROP TRIGGER IF EXISTS dm_optout_sync ON bot.concierge_profiles;
CREATE TRIGGER dm_optout_sync
    AFTER INSERT OR DELETE OR UPDATE OF user_id, opted_out ON bot.concierge_profiles
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_sync();

DROP TRIGGER IF EXISTS dm_optout_sync ON steam.steam_nudge_state;
CREATE TRIGGER dm_optout_sync
    AFTER INSERT OR DELETE OR UPDATE OF user_id, opt_out ON steam.steam_nudge_state
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_sync();

DROP TRIGGER IF EXISTS dm_optout_sync ON turnier.user_profiles;
CREATE TRIGGER dm_optout_sync
    AFTER INSERT OR DELETE OR UPDATE OF discord_id, notify_discord_dm ON turnier.user_profiles
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_sync();

DROP TRIGGER IF EXISTS dm_optout_sync ON turnier.tournament_dm_optout;
CREATE TRIGGER dm_optout_sync
    AFTER INSERT OR DELETE OR UPDATE OF discord_id ON turnier.tournament_dm_optout
    FOR EACH ROW
    EXECUTE FUNCTION core.dm_optout_sync();


SELECT core.refresh_dm_optout(user_id)
  FROM (
        SELECT core.dm_optout_snowflake(k) AS user_id
          FROM bot.kv_store
         WHERE ns IN ('solo_lfg_never_ask', 'voice_mate_survey_never', 'voice_pair_never')
           AND v = 'true'
        UNION
        SELECT discord_id FROM turnier.tournament_dm_optout
        UNION
        SELECT discord_id FROM turnier.user_profiles WHERE notify_discord_dm IS FALSE
        UNION
        SELECT user_id FROM activity.user_retention_tracking WHERE opted_out
        UNION
        SELECT user_id FROM bot.concierge_profiles WHERE opted_out
        UNION
        SELECT user_id FROM steam.steam_nudge_state WHERE opt_out
       ) AS candidates
 WHERE user_id IS NOT NULL;

INSERT INTO core.privacy_field_registry (
    schema_name, table_name, column_name, data_category, retention_action,
    erasure_action, owner_service, reason
) VALUES
 ('core', 'dm_optout', 'user_id', 'user_id', 'retain_operational', 'manual_review', 'dl-discord',
  'Globaler Bot-DM-Opt-out; muss eine Datenlöschung überleben, sonst gingen wieder Bot-DMs raus'),
 ('core', 'dm_optout', 'source', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-discord',
  'Herkunft des Opt-outs als feste Kennung')
ON CONFLICT (schema_name, table_name, column_name) DO NOTHING;
