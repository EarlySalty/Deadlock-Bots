-- Gezielte, wiederholbare Reparatur des Befunds vom 21.09.2026.
-- Ausführung durch den bestehenden Betriebsprozess, nicht beim Schema-Migrator.
-- Verwendet den Journey-Abschluss (voice_metadata_events), ohne Punkte oder
-- eine zweite Session in voice_session_log zu erfinden. Geschlossene Zeilen
-- werden nicht verändert. Die Zeitgrenze schützt inzwischen erneuerte Sessions.
BEGIN;
SET LOCAL statement_timeout = '10s';
SET LOCAL lock_timeout = '5s';

-- Dieselbe User-/Privacy-Sperre wie dl-central-db::lock_user_privacy.
SELECT pg_advisory_xact_lock(754808003257172048::bigint # '-9223372036854775808'::bigint);

WITH removed AS (
    DELETE FROM activity.voice_open_sessions
    WHERE user_id = 754808003257172048
      AND guild_id = 1289721245281292288
      AND channel_id = 1470126503252721845
      AND joined_at = TIMESTAMPTZ '2026-08-02 20:44:20.594256+00'
      AND updated_at < TIMESTAMPTZ '2026-09-21 00:00:00+00'
    RETURNING user_id, guild_id, channel_id, joined_at,
              LEAST(updated_at, statement_timestamp()) AS ended_at
), closed AS (
    INSERT INTO activity.voice_metadata_events (
        user_id, guild_id, channel_id, event_type, occurred_at, duration_seconds
    )
    SELECT user_id, guild_id, channel_id, 'leave', ended_at,
           GREATEST(0, FLOOR(EXTRACT(EPOCH FROM ended_at - joined_at)))::bigint
    FROM removed
    WHERE NOT EXISTS (
        SELECT 1 FROM core.user_privacy privacy
        WHERE privacy.user_id = removed.user_id
          AND (privacy.opted_out = TRUE OR privacy.deleted_at IS NOT NULL)
    )
    RETURNING id
)
SELECT (SELECT COUNT(*) FROM removed) AS removed_open_sessions,
       (SELECT COUNT(*) FROM closed) AS appended_leave_events;
COMMIT;
