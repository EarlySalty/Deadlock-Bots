SET lock_timeout TO '2s';

CREATE TABLE IF NOT EXISTS scrim.command_receipts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    command_scope TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    idempotency_generation INTEGER NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    state TEXT NOT NULL DEFAULT 'received'
        CHECK (state IN ('received', 'processing', 'completed', 'failed', 'retry', 'uncertain', 'dead', 'cancelled')),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    remote_system TEXT,
    remote_message_id TEXT,
    remote_task_id TEXT,
    result_payload JSONB,
    last_error_code TEXT CHECK (last_error_code IS NULL OR last_error_code ~ '^err_[a-z0-9_]{2,64}$'),
    last_error_hash BYTEA CHECK (last_error_hash IS NULL OR octet_length(last_error_hash) = 32),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    CONSTRAINT command_receipts_lease_pair_check CHECK (
        (lease_owner IS NULL AND lease_until IS NULL)
        OR (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT command_receipts_processing_lease_check CHECK (
        (state = 'processing') = (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT command_receipts_retry_time_check CHECK (state <> 'retry' OR next_attempt_at IS NOT NULL),
    CONSTRAINT command_receipts_terminal_time_check CHECK (
        (completed_at IS NULL AND state NOT IN ('completed', 'failed', 'uncertain', 'dead', 'cancelled'))
        OR (completed_at IS NOT NULL AND state IN ('completed', 'failed', 'uncertain', 'dead', 'cancelled'))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS command_receipts_idempotency_generation_uidx
    ON scrim.command_receipts (command_scope, idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS command_receipts_one_active_idempotency_uidx
    ON scrim.command_receipts (command_scope, idempotency_key)
    WHERE state IN ('received', 'processing', 'retry');

CREATE INDEX IF NOT EXISTS command_receipts_due_idx
    ON scrim.command_receipts (state, next_attempt_at, id)
    WHERE state IN ('received', 'retry');

CREATE INDEX IF NOT EXISTS command_receipts_lease_idx
    ON scrim.command_receipts (lease_until, id)
    WHERE lease_owner IS NOT NULL;

CALL scrim.ensure_check_constraint('scrim.command_receipts'::regclass, 'command_receipts_command_scope_machine_code_check', 'scrim.is_machine_code(command_scope)');
CALL scrim.ensure_check_constraint('scrim.command_receipts'::regclass, 'command_receipts_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('scrim.command_receipts'::regclass, 'command_receipts_lease_owner_domain_ref_check', 'lease_owner IS NULL OR scrim.is_domain_ref(lease_owner)');
CALL scrim.ensure_check_constraint('scrim.command_receipts'::regclass, 'command_receipts_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.command_receipts'::regclass, 'command_receipts_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.ensure_check_constraint('scrim.command_receipts'::regclass, 'command_receipts_remote_task_id_domain_ref_check', 'remote_task_id IS NULL OR scrim.is_domain_ref(remote_task_id)');
CALL scrim.validate_constraints('scrim.command_receipts'::regclass, ARRAY[
    'command_receipts_command_scope_machine_code_check',
    'command_receipts_idempotency_key_domain_ref_check',
    'command_receipts_lease_owner_domain_ref_check',
    'command_receipts_remote_system_machine_code_check',
    'command_receipts_remote_message_id_domain_ref_check',
    'command_receipts_remote_task_id_domain_ref_check'
]);

CREATE OR REPLACE FUNCTION scrim.jsonb_contains_user_ref(value JSONB, user_ref TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    element JSONB;
    item JSONB;
    object_key TEXT;
BEGIN
    IF value IS NULL OR user_ref IS NULL OR user_ref = '' THEN
        RETURN false;
    END IF;

    CASE jsonb_typeof(value)
        WHEN 'object' THEN
            FOR object_key, element IN SELECT key, jsonb_each.value FROM jsonb_each(value) LOOP
                IF object_key = user_ref THEN
                    RETURN true;
                END IF;
                IF scrim.jsonb_contains_user_ref(element, user_ref) THEN
                    RETURN true;
                END IF;
            END LOOP;
            RETURN false;
        WHEN 'array' THEN
            FOR item IN SELECT jsonb_array_elements.value FROM jsonb_array_elements(value) LOOP
                IF scrim.jsonb_contains_user_ref(item, user_ref) THEN
                    RETURN true;
                END IF;
            END LOOP;
            RETURN false;
        WHEN 'string' THEN
            RETURN value #>> '{}' = user_ref;
        WHEN 'number' THEN
            RETURN value #>> '{}' = user_ref;
        ELSE
            RETURN false;
    END CASE;
END;
$$;

CREATE OR REPLACE FUNCTION scrim.jsonb_redact_user_ref(value JSONB, user_ref TEXT)
RETURNS JSONB
LANGUAGE plpgsql
IMMUTABLE
AS $$
DECLARE
    redacted JSONB;
    object_contains_target BOOLEAN;
BEGIN
    IF value IS NULL OR user_ref IS NULL OR user_ref = '' THEN
        RETURN value;
    END IF;

    CASE jsonb_typeof(value)
        WHEN 'object' THEN
            SELECT COALESCE(
                       bool_or(
                           key = user_ref
                           OR (
                               jsonb_typeof(child) IN ('string', 'number')
                               AND child #>> '{}' = user_ref
                           )
                       ),
                       false
                   )
              INTO object_contains_target
              FROM jsonb_each(value) AS e(key, child);

            SELECT COALESCE(
                       jsonb_object_agg(
                           key,
                           CASE
                               WHEN object_contains_target
                                    AND (
                                        lower(key) = 'name'
                                        OR lower(key) LIKE '%display%'
                                        OR lower(key) LIKE '%nickname%'
                                        OR lower(key) LIKE '%tag%'
                                        OR lower(key) LIKE '%\_name' ESCAPE '\'
                                    )
                                   THEN '"redacted"'::jsonb
                               ELSE scrim.jsonb_redact_user_ref(child, user_ref)
                           END
                       ),
                       '{}'::jsonb
                   )
              INTO redacted
              FROM jsonb_each(value) AS e(key, child)
             WHERE key <> user_ref;
            RETURN redacted;
        WHEN 'array' THEN
            SELECT COALESCE(
                       jsonb_agg(scrim.jsonb_redact_user_ref(jsonb_array_elements.value, user_ref)),
                       '[]'::jsonb
                   )
              INTO redacted
              FROM jsonb_array_elements(value);
            RETURN redacted;
        WHEN 'string' THEN
            IF value #>> '{}' = user_ref THEN
                RETURN '"redacted"'::jsonb;
            END IF;
            RETURN value;
        WHEN 'number' THEN
            IF value #>> '{}' = user_ref THEN
                RETURN '"redacted"'::jsonb;
            END IF;
            RETURN value;
        ELSE
            RETURN value;
    END CASE;
END;
$$;

CREATE OR REPLACE FUNCTION scrim.privacy_erasure_authorized(target_ref TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    privacy_user_ref TEXT := NULLIF(current_setting('scrim.privacy_erasure_user_id', true), '');
    privacy_user_id BIGINT;
BEGIN
    IF privacy_user_ref IS NULL OR target_ref IS NULL OR target_ref = '' THEN
        RETURN false;
    END IF;

    IF privacy_user_ref !~ '^[0-9]+$' THEN
        RETURN false;
    END IF;

    privacy_user_id := privacy_user_ref::BIGINT;

    IF NOT EXISTS (
        SELECT 1
          FROM core.user_privacy
         WHERE user_id = privacy_user_id
           AND opted_out = true
           AND deleted_at IS NOT NULL
    ) THEN
        RETURN false;
    END IF;

    IF target_ref = privacy_user_ref THEN
        RETURN true;
    END IF;

    RETURN EXISTS (
        SELECT 1
          FROM core.steam_links
         WHERE discord_id = privacy_user_id
           AND (
                 steam_id = target_ref
                 OR steam_id64::TEXT = target_ref
                 OR (
                     target_ref ~ '^[0-9]{1,10}$'
                     AND steam_id64 IS NOT NULL
                     AND steam_id64::NUMERIC - 76561197960265728 BETWEEN 0 AND 4294967295
                     AND ((steam_id64::NUMERIC - 76561197960265728)::BIGINT)::TEXT = target_ref
                 )
            )
    );
END;
$$;

CREATE OR REPLACE FUNCTION scrim.current_privacy_erasure_target_ref()
RETURNS TEXT
LANGUAGE sql
STABLE
AS $$
    SELECT COALESCE(
        NULLIF(current_setting('scrim.privacy_erasure_target_ref', true), ''),
        NULLIF(current_setting('scrim.privacy_erasure_user_id', true), '')
    )
$$;

CREATE OR REPLACE FUNCTION scrim.privacy_erasure_target_namespace(target_ref TEXT)
RETURNS TEXT
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    privacy_user_ref TEXT := NULLIF(current_setting('scrim.privacy_erasure_user_id', true), '');
    privacy_user_id BIGINT;
BEGIN
    IF NOT scrim.privacy_erasure_authorized(target_ref) THEN
        RETURN NULL;
    END IF;

    IF privacy_user_ref IS NULL OR privacy_user_ref !~ '^[0-9]+$' THEN
        RETURN NULL;
    END IF;

    privacy_user_id := privacy_user_ref::BIGINT;

    IF target_ref = privacy_user_ref THEN
        RETURN 'discord_user';
    END IF;

    IF EXISTS (
        SELECT 1
          FROM core.steam_links
         WHERE discord_id = privacy_user_id
           AND (
                 steam_id = target_ref
                 OR steam_id64::TEXT = target_ref
                 OR (
                     target_ref ~ '^[0-9]{1,10}$'
                     AND steam_id64 IS NOT NULL
                     AND steam_id64::NUMERIC - 76561197960265728 BETWEEN 0 AND 4294967295
                     AND ((steam_id64::NUMERIC - 76561197960265728)::BIGINT)::TEXT = target_ref
                 )
            )
    ) THEN
        RETURN 'steam_user';
    END IF;

    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION scrim.is_privacy_redaction(old_value JSONB, new_value JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    privacy_target_ref TEXT := scrim.current_privacy_erasure_target_ref();
BEGIN
    IF privacy_target_ref IS NULL OR NOT scrim.privacy_erasure_authorized(privacy_target_ref) THEN
        RETURN false;
    END IF;

    IF jsonb_typeof(old_value) IN ('object', 'array') THEN
        RETURN new_value IS NOT DISTINCT FROM scrim.jsonb_redact_user_ref(old_value, privacy_target_ref);
    END IF;

    IF jsonb_typeof(old_value) IN ('string', 'number') THEN
        RETURN old_value #>> '{}' = privacy_target_ref
           AND new_value IS NOT DISTINCT FROM '"redacted"'::jsonb;
    END IF;

    RETURN false;
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_logical_identity_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    field_name TEXT;
    field_index INTEGER;
BEGIN
    FOR field_index IN 0..TG_NARGS - 1 LOOP
        field_name := TG_ARGV[field_index];
        IF to_jsonb(NEW) -> field_name IS DISTINCT FROM to_jsonb(OLD) -> field_name THEN
            IF scrim.is_privacy_redaction(to_jsonb(OLD) -> field_name, to_jsonb(NEW) -> field_name) THEN
                CONTINUE;
            END IF;
            RAISE EXCEPTION 'logical idempotency identity field % is immutable', field_name
                USING ERRCODE = '55000';
        END IF;
    END LOOP;

    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_terminal_state_reopen()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    state_column TEXT;
    old_state TEXT;
    new_state TEXT;
    terminal_states TEXT[];
    arg_index INTEGER;
BEGIN
    state_column := TG_ARGV[0];
    terminal_states := ARRAY[]::TEXT[];
    FOR arg_index IN 1..TG_NARGS - 1 LOOP
        terminal_states := terminal_states || TG_ARGV[arg_index];
    END LOOP;
    old_state := to_jsonb(OLD) ->> state_column;
    new_state := to_jsonb(NEW) ->> state_column;

    IF old_state = ANY(terminal_states) AND new_state IS DISTINCT FROM old_state THEN
        RAISE EXCEPTION 'terminal journal state % cannot transition to %', old_state, new_state
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_journal_delete_or_truncate()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'journal rows are append-only and cannot be deleted or truncated'
        USING ERRCODE = '55000';
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_append_only_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'append-only rows cannot be updated, deleted, or truncated'
        USING ERRCODE = '55000';
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_append_only_update_except_redaction()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    allowed_fields TEXT[] := TG_ARGV;
    field_name TEXT;
    paired_user_field TEXT;
    old_value JSONB;
    new_value JSONB;
    privacy_target_ref TEXT := scrim.current_privacy_erasure_target_ref();
BEGIN
    FOR field_name IN
        SELECT key
          FROM jsonb_object_keys(to_jsonb(OLD) || to_jsonb(NEW)) AS key
    LOOP
        old_value := to_jsonb(OLD) -> field_name;
        new_value := to_jsonb(NEW) -> field_name;
        IF new_value IS DISTINCT FROM old_value THEN
            IF privacy_target_ref IS NULL
               OR NOT scrim.privacy_erasure_authorized(privacy_target_ref)
               OR NOT field_name = ANY(allowed_fields) THEN
                RAISE EXCEPTION 'append-only rows can only change privacy-redacted actor fields'
                    USING ERRCODE = '55000';
            END IF;

            IF scrim.is_privacy_redaction(old_value, new_value) THEN
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

            RAISE EXCEPTION 'append-only rows can only change privacy-redacted actor fields'
                USING ERRCODE = '55000';
        END IF;
    END LOOP;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_command_receipt_idempotency_mutation ON scrim.command_receipts;
CREATE TRIGGER prevent_command_receipt_idempotency_mutation
    BEFORE UPDATE ON scrim.command_receipts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_logical_identity_mutation(
        'command_scope',
        'idempotency_key',
        'idempotency_generation',
        'payload_hash',
        'payload'
    );

DROP TRIGGER IF EXISTS prevent_command_receipt_terminal_reopen ON scrim.command_receipts;
CREATE TRIGGER prevent_command_receipt_terminal_reopen
    BEFORE UPDATE ON scrim.command_receipts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'completed',
        'failed',
        'uncertain',
        'dead',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_command_receipt_delete ON scrim.command_receipts;
CREATE TRIGGER prevent_command_receipt_delete
    BEFORE DELETE ON scrim.command_receipts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_command_receipt_truncate ON scrim.command_receipts;
CREATE TRIGGER prevent_command_receipt_truncate
    BEFORE TRUNCATE ON scrim.command_receipts
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE TABLE IF NOT EXISTS scrim.inbox_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_source TEXT NOT NULL,
    source_event_id TEXT,
    idempotency_key TEXT NOT NULL,
    idempotency_generation INTEGER NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    state TEXT NOT NULL DEFAULT 'received'
        CHECK (state IN ('received', 'processing', 'processed', 'retry', 'uncertain', 'dead', 'ignored')),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    remote_system TEXT,
    remote_message_id TEXT,
    remote_task_id TEXT,
    last_error_code TEXT CHECK (last_error_code IS NULL OR last_error_code ~ '^err_[a-z0-9_]{2,64}$'),
    last_error_hash BYTEA CHECK (last_error_hash IS NULL OR octet_length(last_error_hash) = 32),
    received_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    processed_at TIMESTAMPTZ,
    CONSTRAINT inbox_events_lease_pair_check CHECK (
        (lease_owner IS NULL AND lease_until IS NULL)
        OR (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT inbox_events_processing_lease_check CHECK (
        (state = 'processing') = (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT inbox_events_retry_time_check CHECK (state <> 'retry' OR next_attempt_at IS NOT NULL),
    CONSTRAINT inbox_events_terminal_time_check CHECK (
        (processed_at IS NULL AND state NOT IN ('processed', 'uncertain', 'dead', 'ignored'))
        OR (processed_at IS NOT NULL AND state IN ('processed', 'uncertain', 'dead', 'ignored'))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS inbox_events_source_event_uidx
    ON scrim.inbox_events (event_source, source_event_id)
    WHERE source_event_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS inbox_events_idempotency_generation_uidx
    ON scrim.inbox_events (event_source, idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS inbox_events_one_active_idempotency_uidx
    ON scrim.inbox_events (event_source, idempotency_key)
    WHERE state IN ('received', 'processing', 'retry');

CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_event_source_machine_code_check', 'scrim.is_machine_code(event_source)');
CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_source_event_id_domain_ref_check', 'source_event_id IS NULL OR scrim.is_domain_ref(source_event_id)');
CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_lease_owner_domain_ref_check', 'lease_owner IS NULL OR scrim.is_domain_ref(lease_owner)');
CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.ensure_check_constraint('scrim.inbox_events'::regclass, 'inbox_events_remote_task_id_domain_ref_check', 'remote_task_id IS NULL OR scrim.is_domain_ref(remote_task_id)');
CALL scrim.validate_constraints('scrim.inbox_events'::regclass, ARRAY[
    'inbox_events_event_source_machine_code_check',
    'inbox_events_source_event_id_domain_ref_check',
    'inbox_events_idempotency_key_domain_ref_check',
    'inbox_events_lease_owner_domain_ref_check',
    'inbox_events_remote_system_machine_code_check',
    'inbox_events_remote_message_id_domain_ref_check',
    'inbox_events_remote_task_id_domain_ref_check'
]);

CREATE INDEX IF NOT EXISTS inbox_events_due_idx
    ON scrim.inbox_events (state, next_attempt_at, id)
    WHERE state IN ('received', 'retry');

DROP TRIGGER IF EXISTS prevent_inbox_event_idempotency_mutation ON scrim.inbox_events;
CREATE TRIGGER prevent_inbox_event_idempotency_mutation
    BEFORE UPDATE ON scrim.inbox_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_logical_identity_mutation(
        'event_source',
        'source_event_id',
        'idempotency_key',
        'idempotency_generation',
        'payload_hash',
        'payload'
    );

DROP TRIGGER IF EXISTS prevent_inbox_event_terminal_reopen ON scrim.inbox_events;
CREATE TRIGGER prevent_inbox_event_terminal_reopen
    BEFORE UPDATE ON scrim.inbox_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'processed',
        'uncertain',
        'dead',
        'ignored'
    );

DROP TRIGGER IF EXISTS prevent_inbox_event_delete ON scrim.inbox_events;
CREATE TRIGGER prevent_inbox_event_delete
    BEFORE DELETE ON scrim.inbox_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_inbox_event_truncate ON scrim.inbox_events;
CREATE TRIGGER prevent_inbox_event_truncate
    BEFORE TRUNCATE ON scrim.inbox_events
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE TABLE IF NOT EXISTS scrim.outbox_effects (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    effect_type TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    idempotency_generation INTEGER NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'leased', 'delivered', 'retry', 'uncertain', 'dead', 'cancelled')),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    remote_system TEXT,
    remote_message_id TEXT,
    remote_task_id TEXT,
    command_receipt_id BIGINT REFERENCES scrim.command_receipts(id) ON DELETE SET NULL,
    inbox_event_id BIGINT REFERENCES scrim.inbox_events(id) ON DELETE SET NULL,
    last_error_code TEXT CHECK (last_error_code IS NULL OR last_error_code ~ '^err_[a-z0-9_]{2,64}$'),
    last_error_hash BYTEA CHECK (last_error_hash IS NULL OR octet_length(last_error_hash) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    CONSTRAINT outbox_effects_lease_pair_check CHECK (
        (lease_owner IS NULL AND lease_until IS NULL)
        OR (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT outbox_effects_leased_state_check CHECK (
        (state = 'leased') = (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT outbox_effects_retry_time_check CHECK (state <> 'retry' OR next_attempt_at IS NOT NULL),
    CONSTRAINT outbox_effects_delivered_time_check CHECK (
        (delivered_at IS NULL AND state <> 'delivered')
        OR (delivered_at IS NOT NULL AND state = 'delivered')
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS outbox_effects_idempotency_generation_uidx
    ON scrim.outbox_effects (effect_type, idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS outbox_effects_one_active_idempotency_uidx
    ON scrim.outbox_effects (effect_type, idempotency_key)
    WHERE state IN ('pending', 'leased', 'retry');

CREATE INDEX IF NOT EXISTS outbox_effects_due_idx
    ON scrim.outbox_effects (state, next_attempt_at, id)
    WHERE state IN ('pending', 'retry');

CREATE INDEX IF NOT EXISTS outbox_effects_remote_task_idx
    ON scrim.outbox_effects (remote_system, remote_task_id)
    WHERE remote_task_id IS NOT NULL;

CALL scrim.ensure_check_constraint('scrim.outbox_effects'::regclass, 'outbox_effects_effect_type_machine_code_check', 'scrim.is_machine_code(effect_type)');
CALL scrim.ensure_check_constraint('scrim.outbox_effects'::regclass, 'outbox_effects_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('scrim.outbox_effects'::regclass, 'outbox_effects_lease_owner_domain_ref_check', 'lease_owner IS NULL OR scrim.is_domain_ref(lease_owner)');
CALL scrim.ensure_check_constraint('scrim.outbox_effects'::regclass, 'outbox_effects_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.outbox_effects'::regclass, 'outbox_effects_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.ensure_check_constraint('scrim.outbox_effects'::regclass, 'outbox_effects_remote_task_id_domain_ref_check', 'remote_task_id IS NULL OR scrim.is_domain_ref(remote_task_id)');
CALL scrim.validate_constraints('scrim.outbox_effects'::regclass, ARRAY[
    'outbox_effects_effect_type_machine_code_check',
    'outbox_effects_idempotency_key_domain_ref_check',
    'outbox_effects_lease_owner_domain_ref_check',
    'outbox_effects_remote_system_machine_code_check',
    'outbox_effects_remote_message_id_domain_ref_check',
    'outbox_effects_remote_task_id_domain_ref_check'
]);

DROP TRIGGER IF EXISTS prevent_outbox_effect_idempotency_mutation ON scrim.outbox_effects;
CREATE TRIGGER prevent_outbox_effect_idempotency_mutation
    BEFORE UPDATE ON scrim.outbox_effects
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_logical_identity_mutation(
        'effect_type',
        'idempotency_key',
        'idempotency_generation',
        'payload_hash',
        'payload'
    );

DROP TRIGGER IF EXISTS prevent_outbox_effect_terminal_reopen ON scrim.outbox_effects;
CREATE TRIGGER prevent_outbox_effect_terminal_reopen
    BEFORE UPDATE ON scrim.outbox_effects
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'delivered',
        'uncertain',
        'dead',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_outbox_effect_delete ON scrim.outbox_effects;
CREATE TRIGGER prevent_outbox_effect_delete
    BEFORE DELETE ON scrim.outbox_effects
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_outbox_effect_truncate ON scrim.outbox_effects;
CREATE TRIGGER prevent_outbox_effect_truncate
    BEFORE TRUNCATE ON scrim.outbox_effects
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE TABLE IF NOT EXISTS scrim.effect_receipts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    outbox_effect_id BIGINT REFERENCES scrim.outbox_effects(id) ON DELETE SET NULL,
    remote_system TEXT NOT NULL,
    remote_message_id TEXT,
    remote_task_id TEXT,
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    receipt_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL
        CHECK (status IN ('observed', 'confirmed', 'failed', 'uncertain')),
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT effect_receipts_remote_ref_check CHECK (remote_message_id IS NOT NULL OR remote_task_id IS NOT NULL)
);

CREATE UNIQUE INDEX IF NOT EXISTS effect_receipts_remote_message_uidx
    ON scrim.effect_receipts (remote_system, remote_message_id)
    WHERE remote_message_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS effect_receipts_remote_task_uidx
    ON scrim.effect_receipts (remote_system, remote_task_id)
    WHERE remote_task_id IS NOT NULL;

CALL scrim.ensure_check_constraint('scrim.effect_receipts'::regclass, 'effect_receipts_remote_system_machine_code_check', 'scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.effect_receipts'::regclass, 'effect_receipts_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.ensure_check_constraint('scrim.effect_receipts'::regclass, 'effect_receipts_remote_task_id_domain_ref_check', 'remote_task_id IS NULL OR scrim.is_domain_ref(remote_task_id)');
CALL scrim.validate_constraints('scrim.effect_receipts'::regclass, ARRAY[
    'effect_receipts_remote_system_machine_code_check',
    'effect_receipts_remote_message_id_domain_ref_check',
    'effect_receipts_remote_task_id_domain_ref_check'
]);

DROP TRIGGER IF EXISTS prevent_effect_receipt_update ON scrim.effect_receipts;
CREATE TRIGGER prevent_effect_receipt_update
    BEFORE UPDATE ON scrim.effect_receipts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_update_except_redaction('receipt_payload');

DROP TRIGGER IF EXISTS prevent_effect_receipt_delete ON scrim.effect_receipts;
CREATE TRIGGER prevent_effect_receipt_delete
    BEFORE DELETE ON scrim.effect_receipts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_effect_receipt_truncate ON scrim.effect_receipts;
CREATE TRIGGER prevent_effect_receipt_truncate
    BEFORE TRUNCATE ON scrim.effect_receipts
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

RESET lock_timeout;
