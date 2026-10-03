-- Einmalnachweis für öffentliche Kanäle, ohne Autor, Zeitpunkt oder Ledgerverweis.
CREATE TABLE community_points.streamer_suggestion_credits (
    channel_id TEXT PRIMARY KEY CHECK (channel_id ~ '^[1-9][0-9]*$'),
    awarded BOOLEAN NOT NULL DEFAULT TRUE CHECK (awarded)
);
INSERT INTO community_points.streamer_suggestion_credits(channel_id)
SELECT DISTINCT substring(ref FROM length('streamer_suggestion:') + 1)
  FROM community_points.ledger
 WHERE source = 'streamer_suggestion' AND ref ~ '^streamer_suggestion:[1-9][0-9]*$';

-- Dieselbe Consentgrenze bleibt nach der authentifizierten Outboxbestätigung erhalten.
ALTER TABLE community.scout_privacy_epochs ADD COLUMN activity_since TIMESTAMPTZ;
UPDATE community.scout_privacy_epochs e SET activity_since = o.activity_since
  FROM community.scout_privacy_outbox o
 WHERE e.subject_hash = sha256(convert_to('scout-community:discord-privacy:v1:' || o.discord_id::text, 'UTF8'))
   AND e.epoch = o.epoch AND e.action = 'consent' AND o.action = 'consent';
-- Ohne belegte ursprüngliche Grenze bleibt ein geerbter Consent geschlossen.
ALTER TABLE community.scout_privacy_epochs ADD CHECK (action = 'consent' OR activity_since IS NULL);
INSERT INTO core.privacy_field_registry
 (schema_name, table_name, column_name, data_category, retention_action, erasure_action, owner_service, reason)
VALUES ('community_points', 'streamer_suggestion_credits', 'channel_id', 'domain_id',
 'retain_operational', 'retain_non_personal', 'dl-community-points-sync',
 'Öffentlicher Kanal hat seinen einmaligen Vorschlagscredit erhalten; keinerlei Autorbezug');
DO $roles$
BEGIN
 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
  GRANT SELECT, INSERT ON community_points.streamer_suggestion_credits TO deadlock;
 END IF;
END
$roles$;
