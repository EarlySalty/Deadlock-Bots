-- Viewer-Tagesdaten sind personenbezogen. Der minimale Hash-Grabstein
-- verhindert Wiederimport auch nach dem Entfernen der Kontoverknüpfung.
CREATE TABLE community_points.twitch_viewer_privacy_blocks (
    subject_hash BYTEA PRIMARY KEY CHECK (octet_length(subject_hash) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Bestehende Opt-outs mit noch vorhandener Verknüpfung ebenfalls schützen.
INSERT INTO community_points.twitch_viewer_privacy_blocks(subject_hash)
SELECT sha256(convert_to('community-points:twitch-viewer-privacy:v1:' || c.platform_user_id, 'UTF8'))
  FROM core.discord_platform_connections c
  JOIN core.user_privacy p ON p.user_id = c.discord_id
 WHERE c.platform = 'twitch' AND (p.opted_out OR p.deleted_at IS NOT NULL)
ON CONFLICT DO NOTHING;

DELETE FROM community_points.twitch_viewer_daily v
 USING community_points.twitch_viewer_privacy_blocks b
 WHERE b.subject_hash = sha256(convert_to('community-points:twitch-viewer-privacy:v1:' || v.twitch_user_id, 'UTF8'));

UPDATE core.privacy_field_registry
   SET data_category = 'user_id', erasure_action = 'delete_row_on_user_delete',
       reason = 'Personenbezogene Twitch-Zuschauer-ID; Export und Löschung über die Discord-Verknüpfung vor deren Entfernung, Wiederimport durch minimalen Hash-Grabstein gesperrt'
 WHERE schema_name = 'community_points' AND table_name = 'twitch_viewer_daily'
   AND column_name = 'twitch_user_id';

INSERT INTO core.privacy_field_registry(
    schema_name, table_name, column_name, data_category, retention_action,
    erasure_action, owner_service, reason
) VALUES (
    'community_points', 'twitch_viewer_privacy_blocks', 'subject_hash',
    'pseudonym', 'retain_operational', 'retain_hash_only', 'dl-community',
    'Domaingetrennter SHA256-Twitch-ID-Hash als notwendiger dauerhafter Importschutz; keine Tagesdaten oder Discordzuordnung, keine Anonymisierung'
);

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT ON community_points.twitch_viewer_privacy_blocks TO deadlock;
    END IF;
END
$roles$;
