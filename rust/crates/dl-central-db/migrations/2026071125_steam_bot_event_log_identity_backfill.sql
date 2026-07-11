LOCK TABLE steam.bot_event_log IN SHARE ROW EXCLUSIVE MODE;

WITH manual_candidates AS (
    SELECT
        e.id,
        split_part(e.detail ->> 'trigger', ':', 2) AS actor_text
    FROM steam.bot_event_log AS e
    WHERE e.event_type = 'rank_sync_run'
      AND e.discord_id IS NULL
      AND jsonb_typeof(e.detail -> 'trigger') = 'string'
      AND e.detail ->> 'trigger' ~ '^(manual|subrank_manual):[1-9][0-9]*$'
),
manual_valid AS (
    SELECT
        id,
        actor_text::BIGINT AS actor_id
    FROM manual_candidates
    WHERE length(actor_text) < 19
       OR (
            length(actor_text) = 19
        AND actor_text <= '9223372036854775807'
       )
)
UPDATE steam.bot_event_log AS e
   SET discord_id = CASE
           WHEN COALESCE(p.opted_out, false) OR p.deleted_at IS NOT NULL THEN NULL
           ELSE m.actor_id
       END,
       detail = NULLIF(e.detail - 'trigger', '{}'::jsonb)
  FROM manual_valid AS m
  LEFT JOIN core.user_privacy AS p ON p.user_id = m.actor_id
 WHERE e.id = m.id;

WITH link_candidates AS (
    SELECT DISTINCT
        e.id,
        sl.discord_id
    FROM steam.bot_event_log AS e
    JOIN core.steam_links AS sl
      ON sl.discord_id <> 0
     AND (
            sl.steam_id64 = e.steam_id
         OR sl.steam_id = e.steam_id::text
     )
    WHERE e.discord_id IS NULL
      AND e.steam_id IS NOT NULL
),
link_resolution AS (
    SELECT
        c.id,
        CASE WHEN count(*) = 1 THEN min(c.discord_id) END AS discord_id,
        bool_or(COALESCE(p.opted_out, false) OR p.deleted_at IS NOT NULL) AS tombstoned
    FROM link_candidates AS c
    LEFT JOIN core.user_privacy AS p ON p.user_id = c.discord_id
    GROUP BY c.id
),
link_decisions AS (
    SELECT id, discord_id, tombstoned
    FROM link_resolution
    WHERE tombstoned OR discord_id IS NOT NULL
)
UPDATE steam.bot_event_log AS e
   SET discord_id = CASE WHEN lp.tombstoned THEN NULL ELSE lp.discord_id END,
       steam_id = CASE WHEN lp.tombstoned THEN NULL ELSE e.steam_id END,
       reason = CASE WHEN lp.tombstoned THEN NULL ELSE e.reason END,
       detail = CASE WHEN lp.tombstoned THEN NULL ELSE e.detail END
  FROM link_decisions AS lp
 WHERE e.id = lp.id
   AND e.discord_id IS NULL
   AND e.steam_id IS NOT NULL;
