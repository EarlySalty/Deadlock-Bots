SET lock_timeout TO '2s';

CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE OR REPLACE FUNCTION scrim.is_machine_code(value TEXT)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT value IS NOT NULL AND value ~ '^[a-z][a-z0-9_]{2,63}$'
$$;

CREATE OR REPLACE FUNCTION scrim.is_domain_ref(value TEXT)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT value IS NOT NULL
       AND value ~ '^[a-z][a-z0-9_]{1,31}:[A-Za-z0-9][A-Za-z0-9_.:-]{0,95}$'
       AND value !~ '^[0-9]+$'
$$;

CREATE OR REPLACE FUNCTION scrim.is_actor_source(value TEXT)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT value IS NOT NULL
       AND value IN ('system', 'user', 'service', 'migration', 'turniere', 'dl_bots', 'steam_core')
$$;

CREATE OR REPLACE FUNCTION scrim.is_positive_discord_id(value TEXT)
RETURNS BOOLEAN
LANGUAGE sql
IMMUTABLE
AS $$
    SELECT CASE
        WHEN value IS NULL OR value !~ '^[1-9][0-9]{0,18}$' THEN false
        ELSE value::NUMERIC <= 9223372036854775807
    END
$$;

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

CREATE TABLE IF NOT EXISTS scrim.audit_actor_pseudonyms (
    actor_type TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'service')),
    actor_ref TEXT NOT NULL,
    actor_pseudonym TEXT NOT NULL UNIQUE CHECK (actor_pseudonym ~ '^act_[0-9a-f]{32}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT audit_actor_pseudonyms_actor_ref_format_check CHECK (
        CASE
            WHEN actor_type = 'user' THEN scrim.is_positive_discord_id(actor_ref)
            WHEN actor_type IN ('system', 'service') THEN scrim.is_machine_code(actor_ref) OR scrim.is_domain_ref(actor_ref)
            ELSE false
        END
    ),
    PRIMARY KEY (actor_type, actor_ref)
);

COMMENT ON TABLE scrim.audit_actor_pseudonyms IS
    'Private raw actor to random audit pseudonym mapping. Privacy erasure deletes this mapping while immutable audit rows keep only the opaque pseudonym.';

CALL scrim.ensure_check_constraint('scrim.audit_actor_pseudonyms'::regclass, 'audit_actor_pseudonyms_actor_ref_format_check', 'CASE WHEN actor_type = ''user'' THEN scrim.is_positive_discord_id(actor_ref) WHEN actor_type IN (''system'', ''service'') THEN scrim.is_machine_code(actor_ref) OR scrim.is_domain_ref(actor_ref) ELSE false END');
CALL scrim.validate_constraints('scrim.audit_actor_pseudonyms'::regclass, ARRAY[
    'audit_actor_pseudonyms_actor_ref_format_check'
]);

CREATE OR REPLACE FUNCTION scrim.random_audit_actor_pseudonym()
RETURNS TEXT
LANGUAGE sql
VOLATILE
AS $$
    SELECT 'act_' || encode(gen_random_bytes(16), 'hex')
$$;

CREATE OR REPLACE FUNCTION scrim.audit_actor_pseudonym(p_actor_type TEXT, p_actor_ref TEXT)
RETURNS TEXT
LANGUAGE plpgsql
VOLATILE
AS $$
DECLARE
    normalized_actor_type TEXT := COALESCE(NULLIF(btrim(p_actor_type), ''), 'service');
    normalized_actor_ref TEXT := NULLIF(btrim(p_actor_ref), '');
    pseudonym TEXT;
BEGIN
    IF normalized_actor_type NOT IN ('system', 'user', 'service') THEN
        RAISE EXCEPTION 'unsupported audit actor type %', normalized_actor_type
            USING ERRCODE = '23514';
    END IF;

    IF normalized_actor_ref IS NULL THEN
        RAISE EXCEPTION 'audit actor ref must be a bounded technical ref or positive Discord id'
            USING ERRCODE = '23514';
    END IF;

    IF normalized_actor_type = 'user' AND NOT scrim.is_positive_discord_id(normalized_actor_ref) THEN
        RAISE EXCEPTION 'user audit actor ref must be a positive numeric Discord id'
            USING ERRCODE = '23514';
    END IF;

    IF normalized_actor_type IN ('system', 'service')
       AND NOT (scrim.is_machine_code(normalized_actor_ref) OR scrim.is_domain_ref(normalized_actor_ref)) THEN
        RAISE EXCEPTION 'system/service audit actor ref must be a bounded machine or domain ref'
            USING ERRCODE = '23514';
    END IF;

    IF normalized_actor_type = 'user'
       AND EXISTS (
            SELECT 1
              FROM core.user_privacy
             WHERE user_id = normalized_actor_ref::BIGINT
               AND deleted_at IS NOT NULL
       ) THEN
        RETURN scrim.random_audit_actor_pseudonym();
    END IF;

    LOOP
        BEGIN
            WITH inserted AS (
                INSERT INTO scrim.audit_actor_pseudonyms(actor_type, actor_ref, actor_pseudonym)
                VALUES (normalized_actor_type, normalized_actor_ref, scrim.random_audit_actor_pseudonym())
                ON CONFLICT (actor_type, actor_ref) DO NOTHING
                RETURNING actor_pseudonym
            )
            SELECT actor_pseudonym INTO pseudonym
              FROM (
                    SELECT actor_pseudonym FROM inserted
                    UNION ALL
                    SELECT existing.actor_pseudonym
                      FROM scrim.audit_actor_pseudonyms AS existing
                     WHERE existing.actor_type = normalized_actor_type
                       AND existing.actor_ref = normalized_actor_ref
              ) AS candidate
             LIMIT 1;

            IF pseudonym IS NOT NULL THEN
                RETURN pseudonym;
            END IF;
        EXCEPTION WHEN unique_violation THEN
            pseudonym := NULL;
        END;
    END LOOP;
END;
$$;

CREATE TABLE IF NOT EXISTS scrim.audit_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_type TEXT NOT NULL,
    entity_type TEXT NOT NULL CHECK (
        entity_type IN (
            'runtime_control',
            'match',
            'match_request',
            'team',
            'announcement',
            'status_publication',
            'replacement_need',
            'replacement_candidate',
            'replacement_request',
            'match_result_ref',
            'match_result_selection',
            'ai_run',
            'ai_decision_ref',
            'discord_user',
            'steam_user'
        )
    ),
    entity_id TEXT,
    actor_type TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'service')),
    actor_pseudonym TEXT NOT NULL CHECK (actor_pseudonym ~ '^act_[0-9a-f]{32}$'),
    actor_source TEXT,
    request_id TEXT,
    correlation_id TEXT,
    before_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    after_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    decision_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT audit_events_entity_id_format_check CHECK (
        entity_id IS NULL
        OR (entity_type IN ('discord_user', 'steam_user') AND entity_id ~ '^act_[0-9a-f]{32}$')
        OR (entity_type = 'runtime_control' AND entity_id = 'scrim_runtime')
        OR (
            entity_type IN (
                'match',
                'match_request',
                'team',
                'announcement',
                'status_publication',
                'replacement_need',
                'replacement_candidate',
                'replacement_request',
                'match_result_ref',
                'match_result_selection',
                'ai_run',
                'ai_decision_ref'
            )
            AND entity_id ~ '^[1-9][0-9]{0,18}$'
        )
    )
);

CREATE INDEX IF NOT EXISTS audit_events_entity_idx
    ON scrim.audit_events (entity_type, entity_id, created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS audit_events_request_idx
    ON scrim.audit_events (request_id)
    WHERE request_id IS NOT NULL;

CALL scrim.ensure_check_constraint('scrim.audit_events'::regclass, 'audit_events_event_type_machine_code_check', 'scrim.is_machine_code(event_type)');
CALL scrim.ensure_check_constraint('scrim.audit_events'::regclass, 'audit_events_actor_source_enum_check', 'actor_source IS NULL OR scrim.is_actor_source(actor_source)');
CALL scrim.ensure_check_constraint('scrim.audit_events'::regclass, 'audit_events_request_id_domain_ref_check', 'request_id IS NULL OR scrim.is_domain_ref(request_id)');
CALL scrim.ensure_check_constraint('scrim.audit_events'::regclass, 'audit_events_correlation_id_domain_ref_check', 'correlation_id IS NULL OR scrim.is_domain_ref(correlation_id)');
CALL scrim.validate_constraints('scrim.audit_events'::regclass, ARRAY[
    'audit_events_event_type_machine_code_check',
    'audit_events_actor_source_enum_check',
    'audit_events_request_id_domain_ref_check',
    'audit_events_correlation_id_domain_ref_check'
]);

CREATE OR REPLACE FUNCTION scrim.prevent_audit_event_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'scrim audit events are immutable'
        USING ERRCODE = '55000';
END;
$$;

DROP TRIGGER IF EXISTS prevent_audit_event_update_delete ON scrim.audit_events;
CREATE TRIGGER prevent_audit_event_update_delete
    BEFORE UPDATE OR DELETE ON scrim.audit_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_audit_event_mutation();

DROP TRIGGER IF EXISTS prevent_audit_event_truncate ON scrim.audit_events;
CREATE TRIGGER prevent_audit_event_truncate
    BEFORE TRUNCATE ON scrim.audit_events
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_audit_event_mutation();

CREATE TABLE IF NOT EXISTS scrim.runtime_control_history (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    control_key TEXT NOT NULL DEFAULT 'scrim_runtime'
        CHECK (control_key = 'scrim_runtime'),
    mode TEXT NOT NULL DEFAULT 'legacy'
        CHECK (mode IN ('legacy', 'draining', 'turniere')),
    epoch BIGINT NOT NULL CHECK (epoch >= 0),
    operational_writer TEXT NOT NULL DEFAULT 'dl-bots'
        CHECK (operational_writer IN ('dl-bots', 'turniere')),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'service')),
    actor_pseudonym TEXT NOT NULL CHECK (actor_pseudonym ~ '^act_[0-9a-f]{32}$'),
    actor_source TEXT,
    request_id TEXT,
    correlation_id TEXT,
    decision_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT runtime_control_history_epoch_uidx UNIQUE (control_key, epoch),
    CONSTRAINT runtime_control_history_mode_writer_check CHECK (
        (mode = 'legacy' AND operational_writer = 'dl-bots')
        OR (mode IN ('draining', 'turniere') AND operational_writer = 'turniere')
    )
);

COMMENT ON TABLE scrim.runtime_control_history IS
    'Append-only runtime ownership transition history. The current state is exposed by the non-updatable scrim.runtime_control view; raw actor ids are kept only in the private audit pseudonym mapping.';

CALL scrim.ensure_check_constraint('scrim.runtime_control_history'::regclass, 'runtime_control_history_actor_source_enum_check', 'actor_source IS NULL OR scrim.is_actor_source(actor_source)');
CALL scrim.ensure_check_constraint('scrim.runtime_control_history'::regclass, 'runtime_control_history_request_id_domain_ref_check', 'request_id IS NULL OR scrim.is_domain_ref(request_id)');
CALL scrim.ensure_check_constraint('scrim.runtime_control_history'::regclass, 'runtime_control_history_correlation_id_domain_ref_check', 'correlation_id IS NULL OR scrim.is_domain_ref(correlation_id)');
CALL scrim.validate_constraints('scrim.runtime_control_history'::regclass, ARRAY[
    'runtime_control_history_actor_source_enum_check',
    'runtime_control_history_request_id_domain_ref_check',
    'runtime_control_history_correlation_id_domain_ref_check'
]);

CREATE OR REPLACE FUNCTION scrim.enforce_runtime_control_history_insert()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    previous_row scrim.runtime_control_history%ROWTYPE;
BEGIN
    PERFORM pg_advisory_xact_lock(724060001, 724060002);

    IF NEW.control_key <> 'scrim_runtime' THEN
        RAISE EXCEPTION 'scrim runtime control key must be scrim_runtime'
            USING ERRCODE = '23514';
    END IF;

    SELECT * INTO previous_row
      FROM scrim.runtime_control_history
     WHERE control_key = NEW.control_key
       AND epoch = NEW.epoch - 1
     FOR UPDATE;

    IF NEW.epoch = 0 THEN
        IF EXISTS (
            SELECT 1 FROM scrim.runtime_control_history WHERE control_key = NEW.control_key
        ) THEN
            RAISE EXCEPTION 'scrim runtime seed row already exists'
                USING ERRCODE = '23505';
        END IF;
        IF NEW.mode <> 'legacy' OR NEW.operational_writer <> 'dl-bots' THEN
            RAISE EXCEPTION 'scrim runtime seed must start in legacy/dl-bots'
                USING ERRCODE = '23514';
        END IF;
        RETURN NEW;
    END IF;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'scrim runtime epochs must be contiguous'
            USING ERRCODE = '23514';
    END IF;

    IF NOT (
        (previous_row.mode = 'legacy' AND NEW.mode = 'draining')
        OR (previous_row.mode = 'draining' AND NEW.mode IN ('legacy', 'turniere'))
        OR (previous_row.mode = 'turniere' AND NEW.mode = 'draining')
    ) THEN
        RAISE EXCEPTION 'scrim runtime transition % -> % is not allowed', previous_row.mode, NEW.mode
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS enforce_runtime_control_history_insert ON scrim.runtime_control_history;
CREATE TRIGGER enforce_runtime_control_history_insert
    BEFORE INSERT ON scrim.runtime_control_history
    FOR EACH ROW
    EXECUTE FUNCTION scrim.enforce_runtime_control_history_insert();

CREATE OR REPLACE FUNCTION scrim.prevent_runtime_control_history_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'scrim runtime control history is append-only'
        USING ERRCODE = '55000';
END;
$$;

DROP TRIGGER IF EXISTS prevent_runtime_control_history_update_delete ON scrim.runtime_control_history;
CREATE TRIGGER prevent_runtime_control_history_update_delete
    BEFORE UPDATE OR DELETE ON scrim.runtime_control_history
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_runtime_control_history_mutation();

DROP TRIGGER IF EXISTS prevent_runtime_control_history_truncate ON scrim.runtime_control_history;
CREATE TRIGGER prevent_runtime_control_history_truncate
    BEFORE TRUNCATE ON scrim.runtime_control_history
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_runtime_control_history_mutation();

CREATE OR REPLACE FUNCTION scrim.audit_runtime_control_history_insert()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    previous_row scrim.runtime_control_history%ROWTYPE;
BEGIN
    IF NEW.epoch = 0 THEN
        RETURN NEW;
    END IF;

    SELECT * INTO previous_row
      FROM scrim.runtime_control_history
     WHERE control_key = NEW.control_key
       AND epoch = NEW.epoch - 1;

    INSERT INTO scrim.audit_events(
        event_type,
        entity_type,
        entity_id,
        actor_type,
        actor_pseudonym,
        actor_source,
        request_id,
        correlation_id,
        before_data,
        after_data,
        decision_data
    )
    VALUES (
        'runtime_control_transition',
        'runtime_control',
        NEW.control_key,
        NEW.actor_type,
        NEW.actor_pseudonym,
        NEW.actor_source,
        NEW.request_id,
        NEW.correlation_id,
        to_jsonb(previous_row) - 'actor_pseudonym',
        to_jsonb(NEW) - 'actor_pseudonym',
        NEW.decision_data
    );

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS audit_runtime_control_history_insert ON scrim.runtime_control_history;
CREATE TRIGGER audit_runtime_control_history_insert
    AFTER INSERT ON scrim.runtime_control_history
    FOR EACH ROW
    EXECUTE FUNCTION scrim.audit_runtime_control_history_insert();

INSERT INTO scrim.runtime_control_history(
    control_key,
    mode,
    epoch,
    operational_writer,
    actor_type,
    actor_pseudonym,
    actor_source,
    request_id,
    correlation_id,
    decision_data
)
SELECT
    'scrim_runtime',
    'legacy',
    0,
    'dl-bots',
    'system',
    scrim.audit_actor_pseudonym('system', 'migration'),
    'migration',
    'migration:seed',
    'migration:seed',
    '{}'::jsonb
WHERE NOT EXISTS (
    SELECT 1 FROM scrim.runtime_control_history WHERE control_key = 'scrim_runtime'
);

CREATE OR REPLACE VIEW scrim.runtime_control AS
SELECT
    current_row.control_key,
    current_row.mode,
    current_row.epoch,
    current_row.operational_writer,
    current_row.actor_type,
    current_row.actor_pseudonym,
    current_row.actor_source,
    current_row.request_id,
    current_row.correlation_id,
    current_row.decision_data,
    current_row.created_at,
    current_row.created_at AS updated_at
FROM (
    SELECT h.*,
           row_number() OVER (PARTITION BY h.control_key ORDER BY h.epoch DESC, h.id DESC) AS row_num
      FROM scrim.runtime_control_history AS h
) AS current_row
WHERE current_row.row_num = 1;

COMMENT ON VIEW scrim.runtime_control IS
    'Non-updatable current runtime state derived from append-only scrim.runtime_control_history.';

CREATE OR REPLACE FUNCTION scrim.transition_runtime_control(
    expected_epoch BIGINT,
    new_mode TEXT,
    new_operational_writer TEXT,
    changed_by_user_id TEXT,
    changed_by_display_name TEXT,
    request_id TEXT DEFAULT NULL,
    correlation_id TEXT DEFAULT NULL,
    decision_data JSONB DEFAULT '{}'::jsonb
)
RETURNS TABLE(applied BOOLEAN, current_epoch BIGINT)
LANGUAGE plpgsql
AS $$
DECLARE
    current_row scrim.runtime_control_history%ROWTYPE;
    next_row scrim.runtime_control_history%ROWTYPE;
    updated_epoch BIGINT;
    actor_kind TEXT;
BEGIN
    PERFORM pg_advisory_xact_lock(724060001, 724060002);

    SELECT * INTO current_row
      FROM scrim.runtime_control_history
     WHERE control_key = 'scrim_runtime'
     ORDER BY epoch DESC, id DESC
     LIMIT 1
     FOR UPDATE;

    IF NOT FOUND THEN
        RETURN QUERY SELECT false, NULL::BIGINT;
        RETURN;
    END IF;

    IF current_row.epoch IS DISTINCT FROM expected_epoch THEN
        RETURN QUERY SELECT false, current_row.epoch;
        RETURN;
    END IF;

    IF NOT (
        (current_row.mode = 'legacy' AND new_mode = 'draining')
        OR (current_row.mode = 'draining' AND new_mode IN ('legacy', 'turniere'))
        OR (current_row.mode = 'turniere' AND new_mode = 'draining')
    ) THEN
        RAISE EXCEPTION 'scrim runtime transition % -> % is not allowed', current_row.mode, new_mode
            USING ERRCODE = '23514';
    END IF;

    IF NOT (
        (new_mode = 'legacy' AND new_operational_writer = 'dl-bots')
        OR (new_mode IN ('draining', 'turniere') AND new_operational_writer = 'turniere')
    ) THEN
        RAISE EXCEPTION 'scrim runtime mode/writer combination is not allowed'
            USING ERRCODE = '23514';
    END IF;

    actor_kind := CASE WHEN changed_by_user_id = 'migration' THEN 'system' ELSE 'user' END;

    INSERT INTO scrim.runtime_control_history(
        control_key,
        mode,
        epoch,
        operational_writer,
        actor_type,
        actor_pseudonym,
        actor_source,
        request_id,
        correlation_id,
        decision_data
    )
    VALUES (
        'scrim_runtime',
        new_mode,
        current_row.epoch + 1,
        new_operational_writer,
        actor_kind,
        scrim.audit_actor_pseudonym(actor_kind, changed_by_user_id),
        actor_kind,
        transition_runtime_control.request_id,
        transition_runtime_control.correlation_id,
        transition_runtime_control.decision_data
    )
    RETURNING * INTO next_row;

    updated_epoch := next_row.epoch;

    RETURN QUERY SELECT true, updated_epoch;
END;
$$;

RESET lock_timeout;
