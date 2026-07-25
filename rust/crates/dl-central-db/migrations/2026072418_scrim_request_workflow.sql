SET lock_timeout TO '2s';

CREATE OR REPLACE PROCEDURE scrim.ensure_check_constraint(
    relation REGCLASS,
    constraint_name TEXT,
    check_expression TEXT
)
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conrelid = relation
           AND conname = constraint_name
    ) THEN
        EXECUTE format(
            'ALTER TABLE %s ADD CONSTRAINT %I CHECK (%s) NOT VALID',
            relation,
            constraint_name,
            check_expression
        );
    END IF;
END;
$$;

CREATE OR REPLACE PROCEDURE scrim.validate_constraints(
    relation REGCLASS,
    constraint_names TEXT[]
)
LANGUAGE plpgsql
AS $$
DECLARE
    constraint_name TEXT;
BEGIN
    FOREACH constraint_name IN ARRAY constraint_names LOOP
        EXECUTE format(
            'ALTER TABLE %s VALIDATE CONSTRAINT %I',
            relation,
            constraint_name
        );
    END LOOP;
END;
$$;

ALTER TABLE scrim.replacement_needs
    ADD COLUMN IF NOT EXISTS match_request_id INTEGER REFERENCES scrim.match_requests(id),
    ADD COLUMN IF NOT EXISTS slot_index INTEGER;

CALL scrim.ensure_check_constraint(
    'scrim.replacement_needs'::regclass,
    'replacement_needs_match_request_slot_pair_check',
    '(match_request_id IS NULL AND slot_index IS NULL) OR (match_request_id IS NOT NULL AND slot_index IS NOT NULL)'
);
CALL scrim.ensure_check_constraint(
    'scrim.replacement_needs'::regclass,
    'replacement_needs_slot_index_domain_check',
    'slot_index IS NULL OR slot_index::TEXT ~ ''^[0-9]{1,9}$'''
);
CALL scrim.validate_constraints('scrim.replacement_needs'::regclass, ARRAY[
    'replacement_needs_match_request_slot_pair_check',
    'replacement_needs_slot_index_domain_check'
]);

CREATE UNIQUE INDEX IF NOT EXISTS replacement_needs_match_request_slot_uidx
    ON scrim.replacement_needs (match_request_id, team_id, participant_id, slot_index)
    WHERE match_request_id IS NOT NULL
      AND team_id IS NOT NULL
      AND participant_id IS NOT NULL
      AND slot_index IS NOT NULL;
COMMENT ON INDEX scrim.replacement_needs_match_request_slot_uidx IS
    'Predicate: match_request_id IS NOT NULL AND team_id IS NOT NULL AND participant_id IS NOT NULL AND slot_index IS NOT NULL';

ALTER TABLE scrim.replacement_candidates
    DROP CONSTRAINT IF EXISTS replacement_candidates_status_check;
ALTER TABLE scrim.replacement_candidates
    ADD CONSTRAINT replacement_candidates_status_check
    CHECK (status IN ('candidate', 'shortlisted', 'requested', 'accepted', 'declined', 'expired', 'rejected', 'selected')) NOT VALID;
CALL scrim.validate_constraints('scrim.replacement_candidates'::regclass, ARRAY[
    'replacement_candidates_status_check'
]);

CREATE UNIQUE INDEX IF NOT EXISTS replacement_candidates_one_selected_need_uidx
    ON scrim.replacement_candidates (need_id)
    WHERE status = 'selected';

ALTER TABLE scrim.replacement_requests
    DROP CONSTRAINT IF EXISTS replacement_requests_status_check,
    DROP CONSTRAINT IF EXISTS replacement_requests_response_time_check;
ALTER TABLE scrim.replacement_requests
    ADD CONSTRAINT replacement_requests_status_check
    CHECK (status IN ('pending', 'sent', 'accepted', 'declined', 'expired', 'cancelled', 'failed', 'uncertain', 'selected')) NOT VALID,
    ADD CONSTRAINT replacement_requests_response_time_check
    CHECK (
        (responded_at IS NULL AND status IN ('pending', 'sent', 'uncertain', 'cancelled', 'failed'))
        OR (responded_at IS NOT NULL AND status IN ('accepted', 'declined', 'expired', 'selected', 'cancelled'))
    ) NOT VALID;
CALL scrim.validate_constraints('scrim.replacement_requests'::regclass, ARRAY[
    'replacement_requests_status_check',
    'replacement_requests_response_time_check'
]);

ALTER TABLE scrim.status_publication_approvals
    DROP CONSTRAINT IF EXISTS status_publication_approvals_decision_check;
ALTER TABLE scrim.status_publication_approvals
    ADD CONSTRAINT status_publication_approvals_decision_check
    CHECK (decision IN ('pending', 'approved', 'rejected', 'cancelled', 'published', 'failed', 'uncertain')) NOT VALID;
CALL scrim.validate_constraints('scrim.status_publication_approvals'::regclass, ARRAY[
    'status_publication_approvals_decision_check'
]);

ALTER TABLE scrim.match_request_reminders
    DROP CONSTRAINT IF EXISTS match_request_reminders_status_check;
ALTER TABLE scrim.match_request_reminders
    ADD CONSTRAINT match_request_reminders_status_check
    CHECK (status IN ('approved', 'queued', 'posting', 'posted', 'failed', 'uncertain', 'cancelled')) NOT VALID;
CALL scrim.ensure_check_constraint(
    'scrim.match_request_reminders'::regclass,
    'match_request_reminders_posted_time_check',
    'status <> ''posted'' OR posted_at IS NOT NULL'
);
CALL scrim.validate_constraints('scrim.match_request_reminders'::regclass, ARRAY[
    'match_request_reminders_status_check',
    'match_request_reminders_posted_time_check'
]);

ALTER TABLE scrim.match_requests
    DROP CONSTRAINT IF EXISTS match_requests_status_message_state_check;
ALTER TABLE scrim.match_requests
    ADD CONSTRAINT match_requests_status_message_state_check
    CHECK (status_message_state IN ('pending', 'queued', 'posting', 'posted', 'post_failed', 'failed', 'uncertain', 'cancelled')) NOT VALID;
CALL scrim.ensure_check_constraint(
    'scrim.match_requests'::regclass,
    'match_requests_status_message_posted_time_check',
    'status_message_state <> ''posted'' OR status_message_posted_at IS NOT NULL'
);
CALL scrim.validate_constraints('scrim.match_requests'::regclass, ARRAY[
    'match_requests_status_message_state_check',
    'match_requests_status_message_posted_time_check'
]);

CREATE TABLE IF NOT EXISTS scrim.match_request_reminder_effects (
    reminder_id BIGINT NOT NULL REFERENCES scrim.match_request_reminders(id),
    outbox_effect_id BIGINT NOT NULL REFERENCES scrim.outbox_effects(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reconciled_at TIMESTAMPTZ,
    PRIMARY KEY (reminder_id, outbox_effect_id),
    UNIQUE (reminder_id),
    UNIQUE (outbox_effect_id)
);

CREATE TABLE IF NOT EXISTS scrim.status_publication_effects (
    status_publication_approval_id BIGINT NOT NULL REFERENCES scrim.status_publication_approvals(id),
    outbox_effect_id BIGINT NOT NULL REFERENCES scrim.outbox_effects(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reconciled_at TIMESTAMPTZ,
    PRIMARY KEY (status_publication_approval_id, outbox_effect_id),
    UNIQUE (outbox_effect_id)
);

CREATE TABLE IF NOT EXISTS scrim.replacement_request_effects (
    replacement_request_id BIGINT NOT NULL REFERENCES scrim.replacement_requests(id),
    outbox_effect_id BIGINT NOT NULL REFERENCES scrim.outbox_effects(id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    reconciled_at TIMESTAMPTZ,
    PRIMARY KEY (replacement_request_id, outbox_effect_id),
    UNIQUE (replacement_request_id),
    UNIQUE (outbox_effect_id)
);

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
    ('scrim', 'replacement_needs', 'match_request_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'match request domain id; not a user id'),
    ('scrim', 'replacement_needs', 'slot_index', 'domain_id', 'retain_operational', 'retain_non_personal', 'turniere', 'slot index scoped to a match request; not a user id'),
    ('scrim', 'match_request_reminder_effects', 'reminder_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'match request reminder domain id'),
    ('scrim', 'match_request_reminder_effects', 'outbox_effect_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'outbox effect domain id'),
    ('scrim', 'match_request_reminder_effects', 'reconciled_at', 'machine_timestamp', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'non-personal reconciliation marker timestamp'),
    ('scrim', 'status_publication_effects', 'status_publication_approval_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'status publication approval domain id'),
    ('scrim', 'status_publication_effects', 'outbox_effect_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'outbox effect domain id'),
    ('scrim', 'status_publication_effects', 'reconciled_at', 'machine_timestamp', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'non-personal reconciliation marker timestamp'),
    ('scrim', 'replacement_request_effects', 'replacement_request_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'replacement request domain id'),
    ('scrim', 'replacement_request_effects', 'outbox_effect_id', 'domain_id', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'outbox effect domain id'),
    ('scrim', 'replacement_request_effects', 'reconciled_at', 'machine_timestamp', 'retain_operational', 'retain_non_personal', 'dl-bots-discord-adapter', 'non-personal reconciliation marker timestamp')
ON CONFLICT (schema_name, table_name, column_name) DO UPDATE
    SET data_category = EXCLUDED.data_category,
        retention_action = EXCLUDED.retention_action,
        erasure_action = EXCLUDED.erasure_action,
        owner_service = EXCLUDED.owner_service,
        reason = EXCLUDED.reason;

DROP PROCEDURE IF EXISTS scrim.validate_constraints(REGCLASS, TEXT[]);
DROP PROCEDURE IF EXISTS scrim.ensure_check_constraint(REGCLASS, TEXT, TEXT);

RESET lock_timeout;
