-- Einmalige ID-basierte Reparatur nach Deploy des Gateway-Writer-Fixes.
-- CSV: channel_id,message_id,discord_type (REST-bestätigt: 7 MemberJoin, 8 NitroBoost, 10 NitroTier2).
-- Keine 404/Unknown-Message-Kandidaten und keine Vermutungen aus Leertext importieren.
-- Standard ist Preview mit ROLLBACK. Erst nach Writer-Deploy bewusst anwenden:
-- sudo -u postgres psql -X -v apply=true -d deadlock -f <diese Datei>
\set ON_ERROR_STOP on
\if :{?apply}
\else
\set apply false
\endif
BEGIN;
SET LOCAL lock_timeout = '10s';
CREATE TEMP TABLE verified_system_messages (
    channel_id BIGINT NOT NULL, message_id BIGINT PRIMARY KEY,
    discord_type INTEGER NOT NULL CHECK (discord_type IN (7,8,10))
);
\copy verified_system_messages FROM '/tmp/discord-insights-verified-system-messages.csv' WITH (FORMAT csv, HEADER true)
LOCK TABLE activity.message_metadata_events, activity.journey_events,
    activity.journey_user_state IN SHARE ROW EXCLUSIVE MODE;
CREATE TEMP TABLE bad_messages AS
SELECT m.* FROM activity.message_metadata_events m
JOIN verified_system_messages v USING (channel_id,message_id);
CREATE TEMP TABLE first_affected AS
SELECT j.guild_id,j.user_id,j.occurred_at AS bad_first_at FROM activity.journey_events j
JOIN bad_messages b ON b.guild_id=j.guild_id AND b.message_id=j.message_id
    AND b.channel_id=j.channel_id AND b.user_id=j.user_id
WHERE j.event_type='first_message';
CREATE TEMP TABLE bad_last_state AS
SELECT s.guild_id,s.user_id FROM activity.journey_user_state s
JOIN first_affected a USING(guild_id,user_id)
WHERE s.last_event_type='first_message' AND s.last_event_at=a.bad_first_at;
DO $$ BEGIN
    IF NOT EXISTS(SELECT 1 FROM bad_messages) THEN
        RAISE EXCEPTION 'Keine unreparierten bestätigten IDs; vorhandenes Backup bleibt erhalten';
    END IF;
    IF EXISTS(SELECT 1 FROM activity.message_daily_aggregates)
       OR EXISTS(SELECT 1 FROM activity.journey_daily_aggregates) THEN
        RAISE EXCEPTION 'Verdichtete Historie vorhanden; Reparatur für diesen Bestand neu prüfen';
    END IF;
END $$;
CREATE TEMP TABLE repair_backup AS
SELECT 'message_metadata_events' AS source_table,to_jsonb(b) AS row_data FROM bad_messages b
UNION ALL SELECT 'journey_events',to_jsonb(j) FROM activity.journey_events j JOIN first_affected a USING(guild_id,user_id) WHERE j.event_type='first_message'
UNION ALL SELECT 'journey_user_state',to_jsonb(j) FROM activity.journey_user_state j JOIN first_affected a USING(guild_id,user_id);
SELECT '/tmp/discord-insights-systemmessage-backup-' || to_char(clock_timestamp(),'YYYYMMDDHH24MISSUS') || '.csv' AS backup_file \gset
SELECT format('COPY repair_backup TO %L WITH (FORMAT csv, HEADER true)', :'backup_file') \gexec
\echo Backup: :backup_file
DELETE FROM activity.message_metadata_events m USING bad_messages b WHERE m.id=b.id;
DELETE FROM activity.journey_events j USING first_affected a
WHERE j.guild_id=a.guild_id AND j.user_id=a.user_id AND j.event_type='first_message';
INSERT INTO activity.journey_events(user_id,guild_id,event_type,event_source,actor_kind,occurred_at,channel_id,message_id,metadata)
SELECT DISTINCT ON (m.guild_id,m.user_id) m.user_id,m.guild_id,'first_message','gateway',NULL,m.occurred_at,m.channel_id,m.message_id,'{}'::jsonb
FROM activity.message_metadata_events m JOIN first_affected a USING(guild_id,user_id)
ORDER BY m.guild_id,m.user_id,m.occurred_at,m.message_id;
UPDATE activity.journey_user_state s SET first_message_at=(
    SELECT min(m.occurred_at) FROM activity.message_metadata_events m
    WHERE m.guild_id=s.guild_id AND m.user_id=s.user_id), updated_at=now()
FROM first_affected a WHERE s.guild_id=a.guild_id AND s.user_id=a.user_id
    AND s.first_message_at=a.bad_first_at;
-- Nur einen tatsächlich vergifteten last_event-Zeiger ersetzen. Andere, später
-- registrierte Interaktionen bleiben unberührt. Auch Interaktionen aus ihrem
-- eigenen Rohdatenpfad berücksichtigen; ohne Ersatz ist NULL ehrlich.
UPDATE activity.journey_user_state s SET (last_event_at,last_event_type)=(
    SELECT e.occurred_at,e.event_type FROM (
        SELECT j.occurred_at,j.event_type FROM activity.journey_events j
        WHERE j.guild_id=s.guild_id AND j.user_id=s.user_id
        UNION ALL
        SELECT i.occurred_at,'interaction' FROM activity.interaction_events i
        WHERE i.guild_id=s.guild_id AND i.user_id=s.user_id
    ) e ORDER BY e.occurred_at DESC,e.event_type LIMIT 1
) FROM bad_last_state a WHERE s.guild_id=a.guild_id AND s.user_id=a.user_id;
SELECT (SELECT count(*) FROM bad_messages) AS korrigierte_nachrichten,
       (SELECT count(*) FROM first_affected) AS korrigierte_erstnachrichten,
       (SELECT count(*) FROM bad_last_state) AS korrigierte_letzte_ereignisse;
-- Legacy-Zähler/Textsessions speichern keine Message-IDs. Gleiche Summen belegen
-- keine Identität: keinerlei geschätzter Abzug von Zählern, Zeitpunkten oder Punkten.
SELECT count(DISTINCT l.user_id) AS konten_mit_ungeklaerter_text_historie,
       count(*) AS textsession_zeilen_im_betroffenen_kanal,
       min(l.started_at) AS von,max(l.ended_at) AS bis
FROM activity.text_conversation_log l
WHERE EXISTS(SELECT 1 FROM bad_messages b WHERE b.user_id=l.user_id AND b.guild_id=l.guild_id AND b.channel_id=l.channel_id);
SELECT count(*) AS ungeklaerte_legacy_konten
FROM activity.message_activity l WHERE EXISTS(
    SELECT 1 FROM bad_messages b WHERE b.user_id=l.user_id AND b.guild_id=l.guild_id);
\if :apply
COMMIT;
\else
ROLLBACK;
\echo Vorschau abgeschlossen, keine Datenänderung gespeichert.
\endif
