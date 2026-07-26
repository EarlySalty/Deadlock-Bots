-- Nachtrag zu 2026072418.
--
-- 2026072418 war live bereits angewendet, als der Reconciliation-Marker
-- ergaenzt wurde. Eine angewendete Migration darf nicht nachtraeglich
-- geaendert werden, weil sqlx die Pruefsumme vergleicht und den Lauf sonst
-- verweigert. Das Delta steht deshalb hier als eigene Migration.

SET lock_timeout = '5s';

ALTER TABLE scrim.match_request_reminder_effects
    ADD COLUMN IF NOT EXISTS reconciled_at TIMESTAMPTZ;
ALTER TABLE scrim.status_publication_effects
    ADD COLUMN IF NOT EXISTS reconciled_at TIMESTAMPTZ;
ALTER TABLE scrim.replacement_request_effects
    ADD COLUMN IF NOT EXISTS reconciled_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS match_request_reminder_effects_unreconciled_outbox_idx
    ON scrim.match_request_reminder_effects (outbox_effect_id)
    WHERE reconciled_at IS NULL;
CREATE INDEX IF NOT EXISTS status_publication_effects_unreconciled_outbox_idx
    ON scrim.status_publication_effects (outbox_effect_id)
    WHERE reconciled_at IS NULL;
CREATE INDEX IF NOT EXISTS replacement_request_effects_unreconciled_outbox_idx
    ON scrim.replacement_request_effects (outbox_effect_id)
    WHERE reconciled_at IS NULL;

-- machine_timestamp ergaenzt die bisher erlaubten Kategorien, alle
-- bestehenden Werte bleiben zulaessig.
ALTER TABLE core.privacy_field_registry
    DROP CONSTRAINT IF EXISTS privacy_field_registry_data_category_check;
ALTER TABLE core.privacy_field_registry
    ADD CONSTRAINT privacy_field_registry_data_category_check
    CHECK (data_category IN ('user_id', 'display_name', 'json_payload', 'free_text', 'domain_id', 'machine_code', 'machine_timestamp', 'content_hash', 'pseudonym')) NOT VALID;
ALTER TABLE core.privacy_field_registry
    VALIDATE CONSTRAINT privacy_field_registry_data_category_check;

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
    ('scrim', 'match_request_reminder_effects', 'reconciled_at', 'machine_timestamp', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'non-personal reconciliation marker timestamp'),
    ('scrim', 'status_publication_effects', 'reconciled_at', 'machine_timestamp', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'non-personal reconciliation marker timestamp'),
    ('scrim', 'replacement_request_effects', 'reconciled_at', 'machine_timestamp', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'non-personal reconciliation marker timestamp')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE
    SET data_category = EXCLUDED.data_category,
        retention_action = EXCLUDED.retention_action,
        erasure_action = EXCLUDED.erasure_action,
        owner_service = EXCLUDED.owner_service,
        reason = EXCLUDED.reason;

RESET lock_timeout;
