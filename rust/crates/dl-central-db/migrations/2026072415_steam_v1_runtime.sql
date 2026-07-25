SET lock_timeout TO '2s';

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION steam.random_v1_operation_id()
RETURNS TEXT
LANGUAGE sql
VOLATILE
AS $$
    SELECT 'op:' || encode(gen_random_bytes(16), 'hex')
$$;

CREATE OR REPLACE FUNCTION steam.v1_identity_redaction_allowed(
    identity_kind TEXT,
    old_value TEXT,
    new_value TEXT
)
RETURNS BOOLEAN
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    target_ref TEXT := scrim.current_privacy_erasure_target_ref();
    target_namespace TEXT;
    required_namespace TEXT;
BEGIN
    IF identity_kind IS NULL
       OR old_value IS NULL
       OR new_value IS DISTINCT FROM 'redacted'
       OR target_ref IS NULL
       OR old_value IS DISTINCT FROM target_ref THEN
        RETURN false;
    END IF;

    target_namespace := scrim.privacy_erasure_target_namespace(target_ref);
    required_namespace := CASE identity_kind
        WHEN 'discord_user' THEN 'discord_user'
        WHEN 'steam_user' THEN 'steam_user'
        WHEN 'player' THEN 'steam_user'
        ELSE NULL
    END;

    RETURN required_namespace IS NOT NULL AND target_namespace = required_namespace;
END;
$$;

CREATE OR REPLACE FUNCTION steam.prevent_v1_workflow_identity_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.contract_version IS DISTINCT FROM OLD.contract_version
       OR NEW.workflow_key IS DISTINCT FROM OLD.workflow_key
       OR NEW.workflow_type IS DISTINCT FROM OLD.workflow_type
       OR NEW.workflow_generation IS DISTINCT FROM OLD.workflow_generation
       OR NEW.aggregate_kind IS DISTINCT FROM OLD.aggregate_kind
       OR NEW.subject_kind IS DISTINCT FROM OLD.subject_kind THEN
        RAISE EXCEPTION 'steam v1 workflow identity fields are immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.aggregate_id IS DISTINCT FROM OLD.aggregate_id
       AND NOT steam.v1_identity_redaction_allowed(OLD.aggregate_kind, OLD.aggregate_id, NEW.aggregate_id) THEN
        RAISE EXCEPTION 'steam v1 workflow aggregate identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.subject_id IS DISTINCT FROM OLD.subject_id
       AND NOT steam.v1_identity_redaction_allowed(OLD.subject_kind, OLD.subject_id, NEW.subject_id) THEN
        RAISE EXCEPTION 'steam v1 workflow subject identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.payload IS DISTINCT FROM OLD.payload
       AND NOT scrim.is_privacy_redaction(OLD.payload, NEW.payload) THEN
        RAISE EXCEPTION 'steam v1 workflow payload identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION steam.prevent_v1_operation_identity_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.contract_version IS DISTINCT FROM OLD.contract_version
       OR NEW.operation_id IS DISTINCT FROM OLD.operation_id
       OR NEW.operation_type IS DISTINCT FROM OLD.operation_type
       OR NEW.workflow_key IS DISTINCT FROM OLD.workflow_key
       OR NEW.workflow_generation IS DISTINCT FROM OLD.workflow_generation
       OR NEW.aggregate_kind IS DISTINCT FROM OLD.aggregate_kind
       OR NEW.subject_kind IS DISTINCT FROM OLD.subject_kind
       OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
       OR NEW.idempotency_generation IS DISTINCT FROM OLD.idempotency_generation
       OR NEW.payload_hash IS DISTINCT FROM OLD.payload_hash THEN
        RAISE EXCEPTION 'steam v1 operation identity fields are immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.aggregate_id IS DISTINCT FROM OLD.aggregate_id
       AND NOT steam.v1_identity_redaction_allowed(OLD.aggregate_kind, OLD.aggregate_id, NEW.aggregate_id) THEN
        RAISE EXCEPTION 'steam v1 operation aggregate identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.subject_id IS DISTINCT FROM OLD.subject_id
       AND NOT steam.v1_identity_redaction_allowed(OLD.subject_kind, OLD.subject_id, NEW.subject_id) THEN
        RAISE EXCEPTION 'steam v1 operation subject identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.payload IS DISTINCT FROM OLD.payload
       AND NOT scrim.is_privacy_redaction(OLD.payload, NEW.payload) THEN
        RAISE EXCEPTION 'steam v1 operation payload identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TABLE IF NOT EXISTS steam.v1_workflows (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    contract_version INTEGER NOT NULL DEFAULT 1 CHECK (contract_version >= 1),
    workflow_key TEXT NOT NULL CHECK (btrim(workflow_key) <> ''),
    workflow_type TEXT NOT NULL,
    workflow_generation BIGINT NOT NULL DEFAULT 0 CHECK (workflow_generation >= 0),
    aggregate_kind TEXT,
    aggregate_id TEXT,
    state TEXT NOT NULL DEFAULT 'open'
        CHECK (state IN ('open', 'running', 'succeeded', 'failed', 'retry', 'uncertain', 'dead', 'cancelled')),
    subject_kind TEXT,
    subject_id TEXT,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    CONSTRAINT v1_workflows_aggregate_kind_check CHECK (
        aggregate_kind IS NULL OR aggregate_kind IN ('match', 'match_request', 'team', 'player', 'lobby', 'party')
    ),
    CONSTRAINT v1_workflows_subject_kind_check CHECK (
        subject_kind IS NULL OR subject_kind IN ('discord_user', 'steam_user')
    ),
    CONSTRAINT v1_workflows_finished_time_check CHECK (
        (finished_at IS NULL AND state NOT IN ('succeeded', 'failed', 'uncertain', 'dead', 'cancelled'))
        OR (finished_at IS NOT NULL AND state IN ('succeeded', 'failed', 'uncertain', 'dead', 'cancelled'))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS v1_workflows_key_generation_uidx
    ON steam.v1_workflows (workflow_key, workflow_generation);

CREATE UNIQUE INDEX IF NOT EXISTS v1_workflows_one_active_key_uidx
    ON steam.v1_workflows (workflow_key)
    WHERE state IN ('open', 'running', 'retry');

CALL scrim.ensure_check_constraint('steam.v1_workflows'::regclass, 'v1_workflows_workflow_key_domain_ref_check', 'scrim.is_domain_ref(workflow_key)');
CALL scrim.ensure_check_constraint('steam.v1_workflows'::regclass, 'v1_workflows_workflow_type_machine_code_check', 'scrim.is_machine_code(workflow_type)');
CALL scrim.validate_constraints('steam.v1_workflows'::regclass, ARRAY[
    'v1_workflows_workflow_key_domain_ref_check',
    'v1_workflows_workflow_type_machine_code_check'
]);

CREATE INDEX IF NOT EXISTS v1_workflows_state_idx
    ON steam.v1_workflows (state, updated_at, id)
    WHERE state IN ('open', 'running', 'retry');

CREATE INDEX IF NOT EXISTS v1_workflows_aggregate_idx
    ON steam.v1_workflows (aggregate_kind, aggregate_id, updated_at DESC, id DESC)
    WHERE aggregate_kind IS NOT NULL AND aggregate_id IS NOT NULL;

DROP TRIGGER IF EXISTS prevent_v1_workflow_identity_mutation ON steam.v1_workflows;
CREATE TRIGGER prevent_v1_workflow_identity_mutation
    BEFORE UPDATE ON steam.v1_workflows
    FOR EACH ROW
    EXECUTE FUNCTION steam.prevent_v1_workflow_identity_mutation();

DROP TRIGGER IF EXISTS prevent_v1_workflow_terminal_reopen ON steam.v1_workflows;
CREATE TRIGGER prevent_v1_workflow_terminal_reopen
    BEFORE UPDATE ON steam.v1_workflows
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'succeeded',
        'failed',
        'uncertain',
        'dead',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_v1_workflow_delete ON steam.v1_workflows;
CREATE TRIGGER prevent_v1_workflow_delete
    BEFORE DELETE ON steam.v1_workflows
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_v1_workflow_truncate ON steam.v1_workflows;
CREATE TRIGGER prevent_v1_workflow_truncate
    BEFORE TRUNCATE ON steam.v1_workflows
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE TABLE IF NOT EXISTS steam.v1_operations (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    contract_version INTEGER NOT NULL DEFAULT 1 CHECK (contract_version >= 1),
    operation_id TEXT NOT NULL UNIQUE DEFAULT steam.random_v1_operation_id() CHECK (btrim(operation_id) <> ''),
    operation_type TEXT NOT NULL,
    workflow_key TEXT,
    workflow_generation BIGINT NOT NULL DEFAULT 0 CHECK (workflow_generation >= 0),
    aggregate_kind TEXT,
    aggregate_id TEXT,
    subject_kind TEXT,
    subject_id TEXT,
    idempotency_key TEXT NOT NULL,
    idempotency_generation BIGINT NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'leased', 'running', 'succeeded', 'failed', 'retry', 'uncertain', 'dead', 'cancelled')),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    legacy_task_id BIGINT REFERENCES steam.steam_tasks(id) ON DELETE SET NULL,
    result_payload JSONB,
    last_error_code TEXT CHECK (last_error_code IS NULL OR last_error_code ~ '^err_[a-z0-9_]{2,64}$'),
    last_error_hash BYTEA CHECK (last_error_hash IS NULL OR octet_length(last_error_hash) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    CONSTRAINT v1_operations_aggregate_kind_check CHECK (
        aggregate_kind IS NULL OR aggregate_kind IN ('match', 'match_request', 'team', 'player', 'lobby', 'party')
    ),
    CONSTRAINT v1_operations_subject_kind_check CHECK (
        subject_kind IS NULL OR subject_kind IN ('discord_user', 'steam_user')
    ),
    CONSTRAINT v1_operations_lease_pair_check CHECK (
        (lease_owner IS NULL AND lease_until IS NULL)
        OR (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT v1_operations_running_lease_check CHECK (
        (state IN ('leased', 'running')) = (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT v1_operations_retry_time_check CHECK (state <> 'retry' OR next_attempt_at IS NOT NULL),
    CONSTRAINT v1_operations_start_time_check CHECK (started_at IS NULL OR state <> 'pending'),
    CONSTRAINT v1_operations_finish_time_check CHECK (
        (finished_at IS NULL AND state NOT IN ('succeeded', 'failed', 'uncertain', 'dead', 'cancelled'))
        OR (finished_at IS NOT NULL AND state IN ('succeeded', 'failed', 'uncertain', 'dead', 'cancelled'))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS v1_operations_operation_generation_uidx
    ON steam.v1_operations (operation_id, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS v1_operations_idempotency_generation_uidx
    ON steam.v1_operations (operation_type, idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS v1_operations_one_active_idempotency_uidx
    ON steam.v1_operations (operation_type, idempotency_key)
    WHERE state IN ('pending', 'leased', 'running', 'retry');

CREATE INDEX IF NOT EXISTS v1_operations_due_idx
    ON steam.v1_operations (state, next_attempt_at, id)
    WHERE state IN ('pending', 'retry');

CREATE INDEX IF NOT EXISTS v1_operations_workflow_idx
    ON steam.v1_operations (workflow_key, workflow_generation, created_at DESC, id DESC)
    WHERE workflow_key IS NOT NULL;

CREATE INDEX IF NOT EXISTS v1_operations_aggregate_idx
    ON steam.v1_operations (aggregate_kind, aggregate_id, created_at DESC, id DESC)
    WHERE aggregate_kind IS NOT NULL AND aggregate_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS v1_operations_legacy_task_idx
    ON steam.v1_operations (legacy_task_id)
    WHERE legacy_task_id IS NOT NULL;

CALL scrim.ensure_check_constraint('steam.v1_operations'::regclass, 'v1_operations_operation_id_domain_ref_check', 'scrim.is_domain_ref(operation_id)');
CALL scrim.ensure_check_constraint('steam.v1_operations'::regclass, 'v1_operations_operation_type_machine_code_check', 'scrim.is_machine_code(operation_type)');
CALL scrim.ensure_check_constraint('steam.v1_operations'::regclass, 'v1_operations_workflow_key_domain_ref_check', 'workflow_key IS NULL OR scrim.is_domain_ref(workflow_key)');
CALL scrim.ensure_check_constraint('steam.v1_operations'::regclass, 'v1_operations_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('steam.v1_operations'::regclass, 'v1_operations_lease_owner_domain_ref_check', 'lease_owner IS NULL OR scrim.is_domain_ref(lease_owner)');
CALL scrim.validate_constraints('steam.v1_operations'::regclass, ARRAY[
    'v1_operations_operation_id_domain_ref_check',
    'v1_operations_operation_type_machine_code_check',
    'v1_operations_workflow_key_domain_ref_check',
    'v1_operations_idempotency_key_domain_ref_check',
    'v1_operations_lease_owner_domain_ref_check'
]);

DROP TRIGGER IF EXISTS prevent_v1_operation_idempotency_mutation ON steam.v1_operations;
CREATE TRIGGER prevent_v1_operation_idempotency_mutation
    BEFORE UPDATE ON steam.v1_operations
    FOR EACH ROW
    EXECUTE FUNCTION steam.prevent_v1_operation_identity_mutation();

DROP TRIGGER IF EXISTS prevent_v1_operation_terminal_reopen ON steam.v1_operations;
CREATE TRIGGER prevent_v1_operation_terminal_reopen
    BEFORE UPDATE ON steam.v1_operations
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'succeeded',
        'failed',
        'uncertain',
        'dead',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_v1_operation_delete ON steam.v1_operations;
CREATE TRIGGER prevent_v1_operation_delete
    BEFORE DELETE ON steam.v1_operations
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_v1_operation_truncate ON steam.v1_operations;
CREATE TRIGGER prevent_v1_operation_truncate
    BEFORE TRUNCATE ON steam.v1_operations
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE TABLE IF NOT EXISTS steam.v1_operation_results (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    operation_id TEXT NOT NULL,
    operation_generation BIGINT NOT NULL DEFAULT 0 CHECK (operation_generation >= 0),
    result_hash BYTEA CHECK (result_hash IS NULL OR octet_length(result_hash) = 32),
    result_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL CHECK (status IN ('succeeded', 'failed', 'uncertain')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT v1_operation_results_operation_generation_fkey
        FOREIGN KEY (operation_id, operation_generation)
        REFERENCES steam.v1_operations(operation_id, idempotency_generation)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS v1_operation_results_operation_idx
    ON steam.v1_operation_results (operation_id, operation_generation, created_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS steam.v1_operation_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    operation_id TEXT,
    operation_generation BIGINT NOT NULL DEFAULT 0 CHECK (operation_generation >= 0),
    event_type TEXT NOT NULL,
    payload_hash BYTEA CHECK (payload_hash IS NULL OR octet_length(payload_hash) = 32),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT v1_operation_events_operation_generation_fkey
        FOREIGN KEY (operation_id, operation_generation)
        REFERENCES steam.v1_operations(operation_id, idempotency_generation)
        ON DELETE RESTRICT
);

CREATE INDEX IF NOT EXISTS v1_operation_events_operation_idx
    ON steam.v1_operation_events (operation_id, operation_generation, created_at DESC, id DESC);

CALL scrim.ensure_check_constraint('steam.v1_operation_events'::regclass, 'v1_operation_events_event_type_machine_code_check', 'scrim.is_machine_code(event_type)');
CALL scrim.validate_constraints('steam.v1_operation_events'::regclass, ARRAY[
    'v1_operation_events_event_type_machine_code_check'
]);

CREATE TABLE IF NOT EXISTS steam.v1_deliveries (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    operation_id TEXT,
    operation_generation BIGINT NOT NULL DEFAULT 0 CHECK (operation_generation >= 0),
    delivery_type TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    idempotency_generation BIGINT NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    remote_system TEXT NOT NULL,
    remote_message_id TEXT,
    remote_task_id TEXT,
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    state TEXT NOT NULL DEFAULT 'pending'
        CHECK (state IN ('pending', 'leased', 'delivered', 'failed', 'retry', 'uncertain', 'dead', 'cancelled')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    CONSTRAINT v1_deliveries_operation_generation_fkey
        FOREIGN KEY (operation_id, operation_generation)
        REFERENCES steam.v1_operations(operation_id, idempotency_generation)
        ON DELETE RESTRICT,
    CONSTRAINT v1_deliveries_lease_pair_check CHECK (
        (lease_owner IS NULL AND lease_until IS NULL)
        OR (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT v1_deliveries_leased_state_check CHECK (
        (state = 'leased') = (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT v1_deliveries_delivered_time_check CHECK (
        (delivered_at IS NULL AND state <> 'delivered')
        OR (delivered_at IS NOT NULL AND state = 'delivered')
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS v1_deliveries_idempotency_generation_uidx
    ON steam.v1_deliveries (delivery_type, idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS v1_deliveries_one_active_idempotency_uidx
    ON steam.v1_deliveries (delivery_type, idempotency_key)
    WHERE state IN ('pending', 'leased', 'retry');

CREATE UNIQUE INDEX IF NOT EXISTS v1_deliveries_remote_message_uidx
    ON steam.v1_deliveries (remote_system, remote_message_id)
    WHERE remote_message_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS v1_deliveries_remote_task_uidx
    ON steam.v1_deliveries (remote_system, remote_task_id)
    WHERE remote_task_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS v1_deliveries_due_idx
    ON steam.v1_deliveries (state, updated_at, id)
    WHERE state IN ('pending', 'retry');

CREATE INDEX IF NOT EXISTS v1_deliveries_operation_idx
    ON steam.v1_deliveries (operation_id, operation_generation, created_at DESC, id DESC)
    WHERE operation_id IS NOT NULL;

CALL scrim.ensure_check_constraint('steam.v1_deliveries'::regclass, 'v1_deliveries_delivery_type_machine_code_check', 'scrim.is_machine_code(delivery_type)');
CALL scrim.ensure_check_constraint('steam.v1_deliveries'::regclass, 'v1_deliveries_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('steam.v1_deliveries'::regclass, 'v1_deliveries_remote_system_machine_code_check', 'scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('steam.v1_deliveries'::regclass, 'v1_deliveries_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.ensure_check_constraint('steam.v1_deliveries'::regclass, 'v1_deliveries_remote_task_id_domain_ref_check', 'remote_task_id IS NULL OR scrim.is_domain_ref(remote_task_id)');
CALL scrim.ensure_check_constraint('steam.v1_deliveries'::regclass, 'v1_deliveries_lease_owner_domain_ref_check', 'lease_owner IS NULL OR scrim.is_domain_ref(lease_owner)');
CALL scrim.validate_constraints('steam.v1_deliveries'::regclass, ARRAY[
    'v1_deliveries_delivery_type_machine_code_check',
    'v1_deliveries_idempotency_key_domain_ref_check',
    'v1_deliveries_remote_system_machine_code_check',
    'v1_deliveries_remote_message_id_domain_ref_check',
    'v1_deliveries_remote_task_id_domain_ref_check',
    'v1_deliveries_lease_owner_domain_ref_check'
]);

DROP TRIGGER IF EXISTS prevent_v1_delivery_idempotency_mutation ON steam.v1_deliveries;
CREATE TRIGGER prevent_v1_delivery_idempotency_mutation
    BEFORE UPDATE ON steam.v1_deliveries
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_logical_identity_mutation(
        'operation_id',
        'operation_generation',
        'delivery_type',
        'remote_system',
        'idempotency_key',
        'idempotency_generation',
        'payload_hash',
        'payload'
    );

DROP TRIGGER IF EXISTS prevent_v1_delivery_terminal_reopen ON steam.v1_deliveries;
CREATE TRIGGER prevent_v1_delivery_terminal_reopen
    BEFORE UPDATE ON steam.v1_deliveries
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'delivered',
        'failed',
        'uncertain',
        'dead',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_v1_delivery_delete ON steam.v1_deliveries;
CREATE TRIGGER prevent_v1_delivery_delete
    BEFORE DELETE ON steam.v1_deliveries
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_v1_delivery_truncate ON steam.v1_deliveries;
CREATE TRIGGER prevent_v1_delivery_truncate
    BEFORE TRUNCATE ON steam.v1_deliveries
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE OR REPLACE FUNCTION steam.prevent_v1_operation_result_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'steam v1 operation results are immutable'
        USING ERRCODE = '55000';
END;
$$;

DROP TRIGGER IF EXISTS prevent_v1_operation_result_update ON steam.v1_operation_results;
CREATE TRIGGER prevent_v1_operation_result_update
    BEFORE UPDATE ON steam.v1_operation_results
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_update_except_redaction('result_payload');

DROP TRIGGER IF EXISTS prevent_v1_operation_result_delete ON steam.v1_operation_results;
CREATE TRIGGER prevent_v1_operation_result_delete
    BEFORE DELETE ON steam.v1_operation_results
    FOR EACH ROW
    EXECUTE FUNCTION steam.prevent_v1_operation_result_mutation();

DROP TRIGGER IF EXISTS prevent_v1_operation_result_truncate ON steam.v1_operation_results;
CREATE TRIGGER prevent_v1_operation_result_truncate
    BEFORE TRUNCATE ON steam.v1_operation_results
    FOR EACH STATEMENT
    EXECUTE FUNCTION steam.prevent_v1_operation_result_mutation();

DROP TRIGGER IF EXISTS prevent_v1_operation_event_update ON steam.v1_operation_events;
CREATE TRIGGER prevent_v1_operation_event_update
    BEFORE UPDATE ON steam.v1_operation_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_update_except_redaction('payload');

DROP TRIGGER IF EXISTS prevent_v1_operation_event_delete ON steam.v1_operation_events;
CREATE TRIGGER prevent_v1_operation_event_delete
    BEFORE DELETE ON steam.v1_operation_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_v1_operation_event_truncate ON steam.v1_operation_events;
CREATE TRIGGER prevent_v1_operation_event_truncate
    BEFORE TRUNCATE ON steam.v1_operation_events
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

RESET lock_timeout;
