-- Nur neue Community-Scoutkopien, keine allgemeine Twitch-Plattformlöschung.
CREATE TABLE community.scout_privacy_epochs (
    subject_hash BYTEA PRIMARY KEY CHECK (octet_length(subject_hash) = 32),
    epoch BIGINT NOT NULL CHECK (epoch > 0),
    action TEXT NOT NULL CHECK (action IN ('erase', 'consent'))
);
CREATE TABLE community.scout_privacy_outbox (
    operation_id TEXT PRIMARY KEY CHECK (operation_id::uuid IS NOT NULL),
    discord_id BIGINT NOT NULL,
    epoch BIGINT NOT NULL CHECK (epoch > 0),
    action TEXT NOT NULL CHECK (action IN ('erase', 'consent')),
    activity_since TIMESTAMPTZ,
    UNIQUE(discord_id, epoch),
    CHECK ((action = 'consent') = (activity_since IS NOT NULL))
);
ALTER TABLE community.streamer_suggestions ADD COLUMN privacy_epoch BIGINT NOT NULL DEFAULT 0;
INSERT INTO core.privacy_field_registry (
 schema_name, table_name, column_name, data_category, retention_action,
 erasure_action, owner_service, reason
) VALUES
 ('community', 'scout_privacy_epochs', 'subject_hash', 'pseudonym', 'retain_operational',
  'retain_hash_only', 'dl-community', 'Minimaler domaingetrennter Epochenschutz gegen verspätete Scoutoperationen'),
 ('community', 'scout_privacy_outbox', 'discord_id', 'user_id', 'retain_operational',
  'manual_review', 'dl-community', 'Ausstehender Lösch-/Einwilligungsauftrag; nach authentifizierter Bestätigung entfernt');
DO $roles$
BEGIN
 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
  GRANT SELECT, INSERT, UPDATE ON community.scout_privacy_epochs TO deadlock;
  GRANT SELECT, INSERT, DELETE ON community.scout_privacy_outbox TO deadlock;
 END IF;
END
$roles$;
