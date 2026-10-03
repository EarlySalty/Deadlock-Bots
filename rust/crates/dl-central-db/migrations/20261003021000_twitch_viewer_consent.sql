-- Erneute Einwilligung gilt erst nach bewusster Twitchverknüpfung.
-- Der Hash-Grabstein sperrt weiterhin ungefilterte kumulative Altwerte.
CREATE TABLE community_points.twitch_viewer_consents (
    subject_hash BYTEA PRIMARY KEY CHECK (octet_length(subject_hash) = 32),
    discord_id BIGINT NOT NULL,
    activity_since TIMESTAMPTZ NOT NULL
);
CREATE TABLE community_points.twitch_viewer_activity_state (
    subject_hash BYTEA NOT NULL CHECK (octet_length(subject_hash) = 32),
    day DATE NOT NULL,
    activity_since TIMESTAMPTZ NOT NULL,
    computed_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY(subject_hash, day)
);
INSERT INTO core.privacy_field_registry (
    schema_name, table_name, column_name, data_category, retention_action,
    erasure_action, owner_service, reason
) VALUES
 ('community_points', 'twitch_viewer_consents', 'discord_id', 'user_id', 'retain_operational',
  'delete_row_on_user_delete', 'dl-community', 'Bewusste erneute Einwilligung nach Twitchverknüpfung'),
 ('community_points', 'twitch_viewer_consents', 'subject_hash', 'pseudonym', 'retain_operational',
  'delete_row_on_user_delete', 'dl-community', 'Einwilligungszuordnung, Löschung über Discord-ID'),
 ('community_points', 'twitch_viewer_activity_state', 'subject_hash', 'pseudonym', 'retain_operational',
  'delete_row_on_user_delete', 'dl-community', 'Stand gefilterter Rohaktivität; Löschung vor Linkentfernung');
DO $roles$
BEGIN
 IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
  GRANT SELECT, INSERT, UPDATE, DELETE ON community_points.twitch_viewer_consents,
    community_points.twitch_viewer_activity_state TO deadlock;
 END IF;
END
$roles$;
