SET lock_timeout TO '2s';

ALTER TABLE scrim.match_result_refs
    ADD COLUMN IF NOT EXISTS external_result_ref TEXT,
    ADD COLUMN IF NOT EXISTS validation_status TEXT NOT NULL DEFAULT 'unvalidated',
    ADD COLUMN IF NOT EXISTS clarification_state TEXT NOT NULL DEFAULT 'none',
    ADD COLUMN IF NOT EXISTS clarification_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN IF NOT EXISTS is_selected BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS selected_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS selected_by_user_id TEXT,
    ADD COLUMN IF NOT EXISTS selection_reason TEXT,
    ADD COLUMN IF NOT EXISTS voided_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS void_reason TEXT,
    ADD COLUMN IF NOT EXISTS superseded_by_ref_id BIGINT;

ALTER TABLE scrim.match_result_refs
    DROP CONSTRAINT IF EXISTS match_result_refs_match_id_fkey;

ALTER TABLE scrim.match_result_refs
    DROP CONSTRAINT IF EXISTS match_result_refs_superseded_same_match_fkey;

ALTER TABLE scrim.match_result_refs
    DROP CONSTRAINT IF EXISTS match_result_refs_match_id_id_key;

ALTER TABLE scrim.match_result_refs
    ADD CONSTRAINT match_result_refs_match_id_fkey
        FOREIGN KEY (match_id)
        REFERENCES scrim.matches(id)
        ON DELETE RESTRICT
        NOT VALID;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_validation_status_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_validation_status_check
            CHECK (validation_status IN ('unvalidated', 'valid', 'ambiguous', 'rejected', 'void', 'superseded')) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_clarification_state_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_clarification_state_check
            CHECK (clarification_state IN ('none', 'needed', 'requested', 'resolved', 'cancelled')) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_selected_valid_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_selected_valid_check
            CHECK (
                NOT is_selected
                OR (
                    validation_status = 'valid'
                    AND fetch_status = 'fetched'
                    AND steam_match_id IS NOT NULL
                    AND winner_team_id IS NOT NULL
                    AND selected_at IS NOT NULL
                    AND selected_by_user_id IS NOT NULL
                    AND voided_at IS NULL
                    AND superseded_by_ref_id IS NULL
                )
            ) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_void_status_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_void_status_check
            CHECK (
                (voided_at IS NULL AND validation_status <> 'void')
                OR (voided_at IS NOT NULL AND validation_status = 'void')
            ) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_supersede_status_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_supersede_status_check
            CHECK (
                (superseded_by_ref_id IS NULL AND validation_status <> 'superseded')
                OR (superseded_by_ref_id IS NOT NULL AND validation_status = 'superseded')
            ) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_supersede_not_self_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_supersede_not_self_check
            CHECK (superseded_by_ref_id IS NULL OR superseded_by_ref_id <> id) NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_superseded_ref_fkey'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_superseded_ref_fkey
            FOREIGN KEY (superseded_by_ref_id)
            REFERENCES scrim.match_result_refs(id)
            ON DELETE RESTRICT
            NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_selection_reason_machine_code_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_selection_reason_machine_code_check
            CHECK (selection_reason IS NULL OR selection_reason ~ '^[a-z][a-z0-9_]{2,63}$') NOT VALID;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_void_reason_machine_code_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_void_reason_machine_code_check
            CHECK (void_reason IS NULL OR void_reason ~ '^[a-z][a-z0-9_]{2,63}$') NOT VALID;
    END IF;
END;
$$;

CALL scrim.ensure_check_constraint('scrim.match_result_refs'::regclass, 'match_result_refs_external_result_ref_domain_ref_check', 'external_result_ref IS NULL OR scrim.is_domain_ref(external_result_ref)');

CREATE OR REPLACE FUNCTION scrim.prevent_match_result_ref_identity_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.match_id IS DISTINCT FROM OLD.match_id
       OR NEW.steam_match_id IS DISTINCT FROM OLD.steam_match_id
       OR NEW.external_result_ref IS DISTINCT FROM OLD.external_result_ref THEN
        RAISE EXCEPTION 'scrim result ref match and source identity are immutable'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_match_result_ref_identity_mutation ON scrim.match_result_refs;
CREATE TRIGGER prevent_match_result_ref_identity_mutation
    BEFORE UPDATE ON scrim.match_result_refs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_match_result_ref_identity_mutation();

CREATE OR REPLACE FUNCTION scrim.enforce_match_result_ref_same_match_supersede()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    supersession_lock_namespace CONSTANT INTEGER := 2026072411;
    target_match_id INTEGER;
    creates_cycle BOOLEAN;
BEGIN
    IF TG_OP = 'INSERT' THEN
        IF NEW.superseded_by_ref_id IS NULL THEN
            RETURN NEW;
        END IF;
    ELSIF NEW.superseded_by_ref_id IS NULL
          AND OLD.superseded_by_ref_id IS NULL
          AND NEW.match_id IS NOT DISTINCT FROM OLD.match_id THEN
        RETURN NEW;
    END IF;

    -- Lock order for one match graph: transaction advisory lock first, then
    -- target lookup and recursive cycle check. This serializes A->B/B->A races
    -- without taking row locks across the whole result-ref table.
    PERFORM pg_advisory_xact_lock(supersession_lock_namespace, NEW.match_id);

    IF NEW.superseded_by_ref_id IS NULL THEN
        RETURN NEW;
    END IF;

    SELECT match_id INTO target_match_id
      FROM scrim.match_result_refs
     WHERE id = NEW.superseded_by_ref_id;

    IF NOT FOUND THEN
        RETURN NEW;
    END IF;

    IF target_match_id IS DISTINCT FROM NEW.match_id THEN
        RAISE EXCEPTION 'scrim result ref can only supersede another ref for the same match'
            USING ERRCODE = '23514';
    END IF;

    WITH RECURSIVE supersede_chain(id, superseded_by_ref_id) AS (
        SELECT id, superseded_by_ref_id
          FROM scrim.match_result_refs
         WHERE id = NEW.superseded_by_ref_id
        UNION
        SELECT next_ref.id, next_ref.superseded_by_ref_id
          FROM scrim.match_result_refs AS next_ref
          JOIN supersede_chain AS chain
            ON next_ref.id = chain.superseded_by_ref_id
         WHERE chain.superseded_by_ref_id IS NOT NULL
    )
    SELECT EXISTS(SELECT 1 FROM supersede_chain WHERE id = NEW.id)
      INTO creates_cycle;

    IF creates_cycle THEN
        RAISE EXCEPTION 'scrim result ref supersession cannot create cycles'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS enforce_match_result_ref_same_match_supersede ON scrim.match_result_refs;
CREATE TRIGGER enforce_match_result_ref_same_match_supersede
    BEFORE INSERT OR UPDATE OF match_id, superseded_by_ref_id ON scrim.match_result_refs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.enforce_match_result_ref_same_match_supersede();

CREATE OR REPLACE FUNCTION scrim.prevent_match_result_ref_delete_or_truncate()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    RAISE EXCEPTION 'scrim result refs are soft-voided or superseded, not deleted'
        USING ERRCODE = '55000';
END;
$$;

DROP TRIGGER IF EXISTS prevent_match_result_ref_delete ON scrim.match_result_refs;
CREATE TRIGGER prevent_match_result_ref_delete
    BEFORE DELETE ON scrim.match_result_refs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_match_result_ref_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_match_result_ref_truncate ON scrim.match_result_refs;
CREATE TRIGGER prevent_match_result_ref_truncate
    BEFORE TRUNCATE ON scrim.match_result_refs
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_match_result_ref_delete_or_truncate();

CREATE TABLE IF NOT EXISTS scrim.match_result_selections (
    match_id INTEGER PRIMARY KEY REFERENCES scrim.matches(id) ON DELETE RESTRICT,
    result_ref_id BIGINT NOT NULL UNIQUE REFERENCES scrim.match_result_refs(id) ON DELETE RESTRICT,
    selected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    selected_by_user_id TEXT NOT NULL,
    selected_by_display_name TEXT NOT NULL,
    selection_reason TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_selections'::regclass
           AND conname = 'match_result_selections_selection_reason_machine_code_check'
    ) THEN
        ALTER TABLE scrim.match_result_selections
            ADD CONSTRAINT match_result_selections_selection_reason_machine_code_check
            CHECK (selection_reason ~ '^[a-z][a-z0-9_]{2,63}$') NOT VALID;
    END IF;
END;
$$;

CREATE TABLE IF NOT EXISTS scrim.match_result_selection_events (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_type TEXT NOT NULL CHECK (event_type IN ('selected', 'reselected', 'redacted')),
    match_id INTEGER NOT NULL REFERENCES scrim.matches(id) ON DELETE RESTRICT,
    result_ref_id BIGINT NOT NULL REFERENCES scrim.match_result_refs(id) ON DELETE RESTRICT,
    old_result_ref_id BIGINT REFERENCES scrim.match_result_refs(id) ON DELETE RESTRICT,
    new_result_ref_id BIGINT REFERENCES scrim.match_result_refs(id) ON DELETE RESTRICT,
    actor_type TEXT NOT NULL CHECK (actor_type IN ('system', 'user', 'service')),
    actor_pseudonym TEXT NOT NULL CHECK (actor_pseudonym ~ '^act_[0-9a-f]{32}$'),
    actor_source TEXT,
    before_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    after_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS match_result_selection_events_match_idx
    ON scrim.match_result_selection_events (match_id, created_at DESC, id DESC);

CALL scrim.ensure_check_constraint('scrim.match_result_selection_events'::regclass, 'match_result_selection_events_actor_source_enum_check', 'actor_source IS NULL OR scrim.is_actor_source(actor_source)');

CREATE OR REPLACE FUNCTION scrim.record_match_result_selection_event()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    actor_ref TEXT;
    event_kind TEXT;
BEGIN
    IF TG_OP = 'INSERT' THEN
        actor_ref := NEW.selected_by_user_id;
        INSERT INTO scrim.match_result_selection_events(
            event_type,
            match_id,
            result_ref_id,
            old_result_ref_id,
            new_result_ref_id,
            actor_type,
            actor_pseudonym,
            actor_source,
            after_data
        )
        VALUES (
            'selected',
            NEW.match_id,
            NEW.result_ref_id,
            NULL,
            NEW.result_ref_id,
            'user',
            scrim.audit_actor_pseudonym('user', actor_ref),
            'user',
            to_jsonb(NEW) - 'selected_by_user_id' - 'selected_by_display_name'
        );
        RETURN NEW;
    END IF;

    IF NEW.result_ref_id IS DISTINCT FROM OLD.result_ref_id THEN
        event_kind := 'reselected';
        actor_ref := NEW.selected_by_user_id;
    ELSE
        event_kind := 'redacted';
        actor_ref := COALESCE(NULLIF(OLD.selected_by_user_id, 'redacted'), NEW.selected_by_user_id);
    END IF;

    INSERT INTO scrim.match_result_selection_events(
        event_type,
        match_id,
        result_ref_id,
        old_result_ref_id,
        new_result_ref_id,
        actor_type,
        actor_pseudonym,
        actor_source,
        before_data,
        after_data
    )
    VALUES (
        event_kind,
        NEW.match_id,
        NEW.result_ref_id,
        OLD.result_ref_id,
        NEW.result_ref_id,
        'user',
        scrim.audit_actor_pseudonym('user', actor_ref),
        'user',
        to_jsonb(OLD) - 'selected_by_user_id' - 'selected_by_display_name',
        to_jsonb(NEW) - 'selected_by_user_id' - 'selected_by_display_name'
    );
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS record_match_result_selection_insert ON scrim.match_result_selections;
CREATE TRIGGER record_match_result_selection_insert
    AFTER INSERT ON scrim.match_result_selections
    FOR EACH ROW
    EXECUTE FUNCTION scrim.record_match_result_selection_event();

DROP TRIGGER IF EXISTS record_match_result_selection_update ON scrim.match_result_selections;
CREATE TRIGGER record_match_result_selection_update
    AFTER UPDATE ON scrim.match_result_selections
    FOR EACH ROW
    WHEN (OLD.* IS DISTINCT FROM NEW.*)
    EXECUTE FUNCTION scrim.record_match_result_selection_event();

CREATE OR REPLACE FUNCTION scrim.enforce_match_result_selection_contract()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    selected_ref scrim.match_result_refs%ROWTYPE;
    privacy_target_ref TEXT;
    actor_redaction BOOLEAN;
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.match_id IS DISTINCT FROM OLD.match_id THEN
        RAISE EXCEPTION 'scrim result selection match is immutable'
            USING ERRCODE = '55000';
    END IF;

    IF TG_OP = 'UPDATE' THEN
        privacy_target_ref := scrim.current_privacy_erasure_target_ref();
        actor_redaction := NEW.result_ref_id IS NOT DISTINCT FROM OLD.result_ref_id
            AND NEW.selected_at IS NOT DISTINCT FROM OLD.selected_at
            AND NEW.selection_reason IS NOT DISTINCT FROM OLD.selection_reason
            AND NEW.selected_by_user_id IS DISTINCT FROM OLD.selected_by_user_id
            AND NEW.selected_by_display_name IS DISTINCT FROM OLD.selected_by_display_name
            AND privacy_target_ref IS NOT NULL
            AND scrim.privacy_erasure_authorized(privacy_target_ref)
            AND scrim.is_privacy_redaction(to_jsonb(OLD.selected_by_user_id), to_jsonb(NEW.selected_by_user_id))
            AND NEW.selected_by_display_name = 'redacted';

        IF actor_redaction THEN
            RETURN NEW;
        END IF;

        IF NEW.result_ref_id IS NOT DISTINCT FROM OLD.result_ref_id THEN
            RAISE EXCEPTION 'scrim result selection updates must be authorized privacy redaction or controlled reselection'
                USING ERRCODE = '55000';
        END IF;

        IF NEW.selected_at <= OLD.selected_at THEN
            RAISE EXCEPTION 'scrim result reselection must use a newer selected_at'
                USING ERRCODE = '23514';
        END IF;
    END IF;

    IF NULLIF(btrim(NEW.selected_by_user_id), '') IS NULL
       OR NEW.selected_by_user_id = 'redacted'
       OR NULLIF(btrim(NEW.selected_by_display_name), '') IS NULL
       OR NEW.selected_by_display_name = 'redacted'
       OR NULLIF(btrim(NEW.selection_reason), '') IS NULL THEN
        RAISE EXCEPTION 'scrim result selection requires actor, display label and reason'
            USING ERRCODE = '23514';
    END IF;

    -- Lock order: a selection first locks its result ref row, then writes the
    -- controlled/audited current pointer. Concurrent invalidation must wait
    -- and then re-check the committed selection in
    -- prevent_selected_match_result_ref_invalidation.
    SELECT * INTO selected_ref
      FROM scrim.match_result_refs
     WHERE id = NEW.result_ref_id
     FOR UPDATE;

    IF NOT FOUND THEN
        RAISE EXCEPTION 'scrim selected result ref does not exist'
            USING ERRCODE = '23503';
    END IF;

    IF selected_ref.match_id IS DISTINCT FROM NEW.match_id THEN
        RAISE EXCEPTION 'scrim selected result ref must belong to the selected match'
            USING ERRCODE = '23514';
    END IF;

    IF selected_ref.validation_status <> 'valid'
       OR selected_ref.fetch_status <> 'fetched'
       OR selected_ref.steam_match_id IS NULL
       OR selected_ref.winner_team_id IS NULL
       OR selected_ref.voided_at IS NOT NULL
       OR selected_ref.superseded_by_ref_id IS NOT NULL THEN
        RAISE EXCEPTION 'scrim selected result ref must be complete, valid, unvoided and current'
            USING ERRCODE = '23514';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS enforce_match_result_selection_contract ON scrim.match_result_selections;
CREATE TRIGGER enforce_match_result_selection_contract
    BEFORE INSERT ON scrim.match_result_selections
    FOR EACH ROW
    EXECUTE FUNCTION scrim.enforce_match_result_selection_contract();

DROP TRIGGER IF EXISTS prevent_match_result_selection_update ON scrim.match_result_selections;
CREATE TRIGGER prevent_match_result_selection_update
    BEFORE UPDATE ON scrim.match_result_selections
    FOR EACH ROW
    EXECUTE FUNCTION scrim.enforce_match_result_selection_contract();

DROP TRIGGER IF EXISTS prevent_match_result_selection_delete ON scrim.match_result_selections;
CREATE TRIGGER prevent_match_result_selection_delete
    BEFORE DELETE ON scrim.match_result_selections
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_match_result_selection_truncate ON scrim.match_result_selections;
CREATE TRIGGER prevent_match_result_selection_truncate
    BEFORE TRUNCATE ON scrim.match_result_selections
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_match_result_selection_event_update_delete ON scrim.match_result_selection_events;
CREATE TRIGGER prevent_match_result_selection_event_update_delete
    BEFORE UPDATE OR DELETE ON scrim.match_result_selection_events
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_match_result_selection_event_truncate ON scrim.match_result_selection_events;
CREATE TRIGGER prevent_match_result_selection_event_truncate
    BEFORE TRUNCATE ON scrim.match_result_selection_events
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

CREATE OR REPLACE FUNCTION scrim.prevent_selected_match_result_ref_invalidation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM scrim.match_result_selections AS selection
         WHERE selection.result_ref_id = OLD.id
    ) THEN
        RETURN NEW;
    END IF;

    IF NEW.fetch_status <> 'fetched'
       OR NEW.validation_status <> 'valid'
       OR NEW.winner_team_id IS NULL
       OR NEW.voided_at IS NOT NULL
       OR NEW.superseded_by_ref_id IS NOT NULL THEN
        RAISE EXCEPTION 'selected scrim result ref must remain fetched, valid, unvoided and current; change selection first'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_selected_match_result_ref_invalidation ON scrim.match_result_refs;
CREATE TRIGGER prevent_selected_match_result_ref_invalidation
    BEFORE UPDATE ON scrim.match_result_refs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_selected_match_result_ref_invalidation();

CREATE TABLE IF NOT EXISTS scrim.match_result_clarifications (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    result_ref_id BIGINT NOT NULL REFERENCES scrim.match_result_refs(id),
    clarification_kind TEXT NOT NULL CHECK (clarification_kind IN ('missing_result', 'ambiguous_winner', 'conflicting_ref', 'manual_review')),
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'requested', 'resolved', 'cancelled')),
    request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    response_payload JSONB,
    requested_by_user_id TEXT,
    requested_by_display_name TEXT,
    remote_system TEXT,
    remote_message_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    CONSTRAINT match_result_clarifications_resolved_time_check CHECK (
        (resolved_at IS NULL AND status IN ('open', 'requested'))
        OR (resolved_at IS NOT NULL AND status IN ('resolved', 'cancelled'))
    )
);

CALL scrim.ensure_check_constraint('scrim.match_result_clarifications'::regclass, 'match_result_clarifications_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.match_result_clarifications'::regclass, 'match_result_clarifications_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');

RESET lock_timeout;
