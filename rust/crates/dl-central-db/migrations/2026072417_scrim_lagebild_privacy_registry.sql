SET lock_timeout TO '2s';

CREATE OR REPLACE FUNCTION scrim.is_privacy_text_redaction(old_value TEXT, new_value TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    privacy_target_ref TEXT := scrim.current_privacy_erasure_target_ref();
BEGIN
    IF privacy_target_ref IS NULL
       OR NOT scrim.privacy_erasure_authorized(privacy_target_ref)
       OR old_value IS NULL THEN
        RETURN false;
    END IF;

    RETURN new_value IS NOT DISTINCT FROM replace(old_value, privacy_target_ref, 'redacted');
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_lagebild_update_except_redaction()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    allowed_fields TEXT[] := TG_ARGV;
    field_name TEXT;
    paired_user_field TEXT;
    old_value JSONB;
    new_value JSONB;
BEGIN
    FOR field_name IN
        SELECT key
          FROM jsonb_object_keys(to_jsonb(OLD) || to_jsonb(NEW)) AS key
    LOOP
        old_value := to_jsonb(OLD) -> field_name;
        new_value := to_jsonb(NEW) -> field_name;
        IF new_value IS NOT DISTINCT FROM old_value THEN
            CONTINUE;
        END IF;

        IF NOT field_name = ANY(allowed_fields) THEN
            RAISE EXCEPTION 'lagebild rows can only change privacy-redacted fields'
                USING ERRCODE = '55000';
        END IF;

        IF scrim.is_privacy_redaction(old_value, new_value) THEN
            CONTINUE;
        END IF;

        IF jsonb_typeof(old_value) = 'string'
           AND jsonb_typeof(new_value) = 'string'
           AND scrim.is_privacy_text_redaction(old_value #>> '{}', new_value #>> '{}') THEN
            CONTINUE;
        END IF;

        IF field_name LIKE '%\_display_name' ESCAPE '\' THEN
            paired_user_field := regexp_replace(field_name, '_display_name$', '_user_id');
            IF paired_user_field = ANY(allowed_fields)
               AND jsonb_typeof(new_value) = 'string'
               AND new_value #>> '{}' = 'redacted'
               AND scrim.is_privacy_redaction(
                    to_jsonb(OLD) -> paired_user_field,
                    to_jsonb(NEW) -> paired_user_field
               ) THEN
                CONTINUE;
            END IF;
        END IF;

        RAISE EXCEPTION 'lagebild rows can only change privacy-redacted fields'
            USING ERRCODE = '55000';
    END LOOP;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_lagebild_snapshot_update_except_redaction ON scrim.lagebild_snapshots;
CREATE TRIGGER prevent_lagebild_snapshot_update_except_redaction
    BEFORE UPDATE ON scrim.lagebild_snapshots
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_lagebild_update_except_redaction(
        'lagebild_text',
        'data_summary',
        'error'
    );

DROP TRIGGER IF EXISTS prevent_lagebild_evidence_update_except_redaction ON scrim.lagebild_evidences;
CREATE TRIGGER prevent_lagebild_evidence_update_except_redaction
    BEFORE UPDATE ON scrim.lagebild_evidences
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_lagebild_update_except_redaction(
        'label',
        'url',
        'reference_id',
        'payload'
    );

DROP TRIGGER IF EXISTS prevent_lagebild_correction_update_except_redaction ON scrim.lagebild_corrections;
CREATE TRIGGER prevent_lagebild_correction_update_except_redaction
    BEFORE UPDATE ON scrim.lagebild_corrections
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_lagebild_update_except_redaction(
        'author_user_id',
        'author_display_name',
        'message'
    );

INSERT INTO core.privacy_field_registry(
    schema_name,
    table_name,
    column_name,
    data_category,
    retention_action,
    erasure_action,
    owner_service,
    reason
)
VALUES
    ('scrim', 'lagebild_snapshots', 'team_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'team domain id; not a user id'),
    ('scrim', 'lagebild_snapshots', 'generated_for', 'free_text', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'bounded generation scope stored as text'),
    ('scrim', 'lagebild_snapshots', 'source', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'snapshot source code'),
    ('scrim', 'lagebild_snapshots', 'status', 'machine_code', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'snapshot status code'),
    ('scrim', 'lagebild_snapshots', 'lagebild_text', 'free_text', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'generated Lagebild text may mention user-entered or user-identifying facts'),
    ('scrim', 'lagebild_snapshots', 'data_summary', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'snapshot summary JSON with nested counters and possible future user references'),
    ('scrim', 'lagebild_snapshots', 'model', 'free_text', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'AI model label'),
    ('scrim', 'lagebild_snapshots', 'error', 'free_text', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'AI error text may include bounded user refs and is redaction-guarded'),
    ('scrim', 'lagebild_evidences', 'evidence_type', 'free_text', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'bounded evidence type label'),
    ('scrim', 'lagebild_evidences', 'label', 'free_text', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'evidence label may include user-entered text'),
    ('scrim', 'lagebild_evidences', 'url', 'free_text', 'retain_operational', 'retain_non_personal', 'dl-bots-ai', 'Discord message URL; not treated as a user id'),
    ('scrim', 'lagebild_evidences', 'reference_id', 'free_text', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'evidence reference may contain Discord or Steam ids'),
    ('scrim', 'lagebild_evidences', 'payload', 'json_payload', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'nested evidence JSON supports automated user-ref redaction'),
    ('scrim', 'lagebild_corrections', 'author_user_id', 'user_id', 'retain_audit', 'redact_on_user_delete', 'dl-bots-ai', 'human correction author Discord id'),
    ('scrim', 'lagebild_corrections', 'author_display_name', 'display_name', 'retain_audit', 'redact_on_user_delete', 'dl-bots-ai', 'human correction author display name'),
    ('scrim', 'lagebild_corrections', 'message', 'free_text', 'retain_operational', 'redact_on_user_delete', 'dl-bots-ai', 'human or assistant correction text')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE SET
    data_category = EXCLUDED.data_category,
    retention_action = EXCLUDED.retention_action,
    erasure_action = EXCLUDED.erasure_action,
    owner_service = EXCLUDED.owner_service,
    reason = EXCLUDED.reason;

RESET lock_timeout;
