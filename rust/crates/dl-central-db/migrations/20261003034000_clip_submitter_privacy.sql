-- Einreicher sind eigenständige Zuschauer, unabhängig vom öffentlichen Kanal.
ALTER TABLE clips.clip_submissions ADD COLUMN submitted_at TIMESTAMPTZ;

INSERT INTO core.privacy_field_registry(
    schema_name, table_name, column_name, data_category, retention_action,
    erasure_action, owner_service, reason
) VALUES
 ('clips','clip_submissions','submitted_by_twitch_user_id','user_id','retain_operational',
  'redact_on_user_delete','dl-community',
  'Twitch-Einreicher über Discord-Verknüpfung exportieren und vor Linkentfernung nullen; Clip und Contestbelege bleiben erhalten'),
 ('bot','twitch_streamer_invites','twitch_user_id','domain_id','retain_operational',
  'retain_non_personal','dl-twitch-invite-sync',
  'Aktuelle öffentliche Partner-/Einladungskanalzuordnung für den Serverbetrieb; keine Ausnahme für historische Viewer-, Einreicher- oder Attributionsdaten');

CREATE FUNCTION clips.protect_twitch_submitter_privacy() RETURNS TRIGGER
LANGUAGE plpgsql SET search_path = pg_catalog AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        IF NEW.submitted_at IS DISTINCT FROM OLD.submitted_at THEN
            RAISE EXCEPTION 'Clip submission origin is immutable' USING ERRCODE='23514';
        END IF;
        IF OLD.submitted_by_twitch_user_id IS NULL THEN
            NEW.submitted_by_twitch_user_id := NULL;
        END IF;
    END IF;
    IF NEW.submitted_by_twitch_user_id IS NOT NULL AND (
        NOT bot.invite_privacy_twitch_allowed(NEW.submitted_by_twitch_user_id, NEW.submitted_at)
        OR NEW.submitted_at > clock_timestamp()
    ) THEN
        NEW.submitted_by_twitch_user_id := NULL;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER clip_submitter_privacy_statement BEFORE INSERT OR UPDATE OR DELETE
ON clips.clip_submissions FOR EACH STATEMENT EXECUTE FUNCTION core.lock_invite_privacy_statement();
CREATE TRIGGER clip_submitter_privacy_write BEFORE INSERT OR UPDATE
ON clips.clip_submissions FOR EACH ROW EXECUTE FUNCTION clips.protect_twitch_submitter_privacy();

-- Erasure vor dieser additiven Migration ebenfalls berücksichtigen.
UPDATE clips.clip_submissions s SET submitted_by_twitch_user_id = NULL
WHERE s.submitted_by_twitch_user_id IS NOT NULL AND EXISTS (
    SELECT 1 FROM community_points.twitch_viewer_privacy_blocks b
    WHERE b.subject_hash = sha256(convert_to(
        'community-points:twitch-viewer-privacy:v1:' || s.submitted_by_twitch_user_id, 'UTF8'))
);
