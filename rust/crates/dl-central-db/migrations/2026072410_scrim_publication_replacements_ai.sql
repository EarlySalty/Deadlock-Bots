SET lock_timeout TO '2s';

CREATE OR REPLACE FUNCTION scrim.announcement_effect_hash(
    scope TEXT,
    block_key TEXT,
    title TEXT,
    body TEXT,
    payload JSONB
)
RETURNS BYTEA
LANGUAGE sql
IMMUTABLE
AS $$
    WITH effect AS (
        SELECT
            COALESCE(scope, '')
            || E'\x1f'
            || COALESCE(block_key, '')
            || E'\x1f'
            || COALESCE(title, '')
            || E'\x1f'
            || COALESCE(body, '')
            || E'\x1f'
            || COALESCE(payload, 'null'::jsonb)::text AS content
    )
    SELECT decode(md5(content) || md5('scrim-announcement:v1:' || content), 'hex')
      FROM effect
$$;

CREATE OR REPLACE FUNCTION scrim.status_publication_effect_hash(
    target_kind TEXT,
    target_id TEXT,
    status_kind TEXT,
    payload JSONB
)
RETURNS BYTEA
LANGUAGE sql
IMMUTABLE
AS $$
    WITH effect AS (
        SELECT
            COALESCE(target_kind, '')
            || E'\x1f'
            || COALESCE(target_id, '')
            || E'\x1f'
            || COALESCE(status_kind, '')
            || E'\x1f'
            || COALESCE(payload, 'null'::jsonb)::text AS content
    )
    SELECT decode(md5(content) || md5('scrim-status-publication:v1:' || content), 'hex')
      FROM effect
$$;

CREATE TABLE IF NOT EXISTS scrim.announcement_drafts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    scope TEXT NOT NULL CHECK (scope IN ('block', 'global')),
    block_key TEXT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    status TEXT NOT NULL DEFAULT 'draft'
        CHECK (status IN ('draft', 'pending_approval', 'approved', 'publishing', 'published', 'rejected', 'cancelled', 'failed')),
    idempotency_key TEXT NOT NULL,
    idempotency_generation INTEGER NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    created_by_user_id TEXT NOT NULL,
    created_by_display_name TEXT NOT NULL,
    approved_by_user_id TEXT,
    approved_by_display_name TEXT,
    approved_at TIMESTAMPTZ,
    published_at TIMESTAMPTZ,
    remote_system TEXT,
    remote_message_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT announcement_drafts_block_key_check CHECK (scope <> 'block' OR block_key IS NOT NULL),
    CONSTRAINT announcement_drafts_approval_time_check CHECK (
        (approved_at IS NULL AND status NOT IN ('approved', 'publishing', 'published'))
        OR (approved_at IS NOT NULL AND status IN ('approved', 'publishing', 'published'))
    ),
    CONSTRAINT announcement_drafts_publish_time_check CHECK (
        (published_at IS NULL AND status <> 'published')
        OR (published_at IS NOT NULL AND status = 'published')
    ),
    CONSTRAINT announcement_drafts_payload_hash_matches_content_check CHECK (
        payload_hash = scrim.announcement_effect_hash(scope, block_key, title, body, payload)
    )
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.announcement_drafts'::regclass
           AND conname = 'announcement_drafts_payload_hash_matches_content_check'
    ) THEN
        ALTER TABLE scrim.announcement_drafts
            ADD CONSTRAINT announcement_drafts_payload_hash_matches_content_check
            CHECK (payload_hash = scrim.announcement_effect_hash(scope, block_key, title, body, payload)) NOT VALID;
    END IF;
END;
$$;

CREATE UNIQUE INDEX IF NOT EXISTS announcement_drafts_idempotency_generation_uidx
    ON scrim.announcement_drafts (idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS announcement_drafts_one_active_idempotency_uidx
    ON scrim.announcement_drafts (idempotency_key)
    WHERE status IN ('draft', 'pending_approval', 'approved', 'publishing');

CALL scrim.ensure_check_constraint('scrim.announcement_drafts'::regclass, 'announcement_drafts_block_key_domain_ref_check', 'block_key IS NULL OR scrim.is_domain_ref(block_key)');
CALL scrim.ensure_check_constraint('scrim.announcement_drafts'::regclass, 'announcement_drafts_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('scrim.announcement_drafts'::regclass, 'announcement_drafts_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.announcement_drafts'::regclass, 'announcement_drafts_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.validate_constraints('scrim.announcement_drafts'::regclass, ARRAY[
    'announcement_drafts_payload_hash_matches_content_check',
    'announcement_drafts_block_key_domain_ref_check',
    'announcement_drafts_idempotency_key_domain_ref_check',
    'announcement_drafts_remote_system_machine_code_check',
    'announcement_drafts_remote_message_id_domain_ref_check'
]);

DROP TRIGGER IF EXISTS prevent_announcement_draft_idempotency_mutation ON scrim.announcement_drafts;
CREATE TRIGGER prevent_announcement_draft_idempotency_mutation
    BEFORE UPDATE ON scrim.announcement_drafts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_logical_identity_mutation(
        'scope',
        'block_key',
        'idempotency_key',
        'idempotency_generation'
    );

CREATE OR REPLACE FUNCTION scrim.prevent_announcement_content_mutation_after_lock()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    locked_states CONSTANT TEXT[] := ARRAY['approved', 'publishing', 'published', 'rejected', 'cancelled', 'failed'];
BEGIN
    IF (OLD.status = ANY(locked_states) OR NEW.status = ANY(locked_states))
       AND (
            NEW.title IS DISTINCT FROM OLD.title
            OR NEW.body IS DISTINCT FROM OLD.body
            OR NEW.payload IS DISTINCT FROM OLD.payload
            OR NEW.payload_hash IS DISTINCT FROM OLD.payload_hash
       ) THEN
        RAISE EXCEPTION 'announcement content is immutable once approval or publication starts'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_announcement_content_mutation_after_lock ON scrim.announcement_drafts;
CREATE TRIGGER prevent_announcement_content_mutation_after_lock
    BEFORE UPDATE ON scrim.announcement_drafts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_announcement_content_mutation_after_lock();

DROP TRIGGER IF EXISTS prevent_announcement_terminal_reopen ON scrim.announcement_drafts;
CREATE TRIGGER prevent_announcement_terminal_reopen
    BEFORE UPDATE ON scrim.announcement_drafts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'status',
        'published',
        'rejected',
        'cancelled',
        'failed'
    );

DROP TRIGGER IF EXISTS prevent_announcement_delete ON scrim.announcement_drafts;
CREATE TRIGGER prevent_announcement_delete
    BEFORE DELETE ON scrim.announcement_drafts
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_announcement_truncate ON scrim.announcement_drafts;
CREATE TRIGGER prevent_announcement_truncate
    BEFORE TRUNCATE ON scrim.announcement_drafts
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE INDEX IF NOT EXISTS announcement_drafts_status_idx
    ON scrim.announcement_drafts (status, created_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS scrim.announcement_approvals (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    draft_id BIGINT NOT NULL REFERENCES scrim.announcement_drafts(id),
    decision TEXT NOT NULL CHECK (decision IN ('approved', 'rejected', 'cancelled')),
    decided_by_user_id TEXT NOT NULL,
    decided_by_display_name TEXT NOT NULL,
    decision_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS announcement_approvals_draft_idx
    ON scrim.announcement_approvals (draft_id, decided_at DESC, id DESC);

DROP TRIGGER IF EXISTS prevent_announcement_approval_update ON scrim.announcement_approvals;
CREATE TRIGGER prevent_announcement_approval_update
    BEFORE UPDATE ON scrim.announcement_approvals
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_update_except_redaction(
        'decided_by_user_id',
        'decided_by_display_name'
    );

DROP TRIGGER IF EXISTS prevent_announcement_approval_delete ON scrim.announcement_approvals;
CREATE TRIGGER prevent_announcement_approval_delete
    BEFORE DELETE ON scrim.announcement_approvals
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_announcement_approval_truncate ON scrim.announcement_approvals;
CREATE TRIGGER prevent_announcement_approval_truncate
    BEFORE TRUNCATE ON scrim.announcement_approvals
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

CREATE TABLE IF NOT EXISTS scrim.status_publication_approvals (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('match', 'match_request', 'team', 'replacement')),
    target_id TEXT NOT NULL,
    status_kind TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    payload_hash BYTEA NOT NULL CHECK (octet_length(payload_hash) = 32),
    decision TEXT NOT NULL DEFAULT 'pending'
        CHECK (decision IN ('pending', 'approved', 'rejected', 'cancelled', 'published')),
    decided_by_user_id TEXT,
    decided_by_display_name TEXT,
    decision_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    remote_system TEXT,
    remote_message_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    decided_at TIMESTAMPTZ,
    CONSTRAINT status_publication_decision_time_check CHECK (
        (decided_at IS NULL AND decision = 'pending')
        OR (decided_at IS NOT NULL AND decision <> 'pending')
    ),
    CONSTRAINT status_publication_payload_hash_matches_content_check CHECK (
        payload_hash = scrim.status_publication_effect_hash(target_kind, target_id, status_kind, payload)
    )
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
         WHERE conrelid = 'scrim.status_publication_approvals'::regclass
           AND conname = 'status_publication_payload_hash_matches_content_check'
    ) THEN
        ALTER TABLE scrim.status_publication_approvals
            ADD CONSTRAINT status_publication_payload_hash_matches_content_check
            CHECK (payload_hash = scrim.status_publication_effect_hash(target_kind, target_id, status_kind, payload)) NOT VALID;
    END IF;
END;
$$;

CREATE UNIQUE INDEX IF NOT EXISTS status_publication_pending_uidx
    ON scrim.status_publication_approvals (target_kind, target_id, status_kind)
    WHERE decision IN ('pending', 'approved');

CALL scrim.ensure_check_constraint('scrim.status_publication_approvals'::regclass, 'status_publication_target_id_typed_numeric_check', 'target_id ~ ''^[1-9][0-9]{0,18}$''');
CALL scrim.ensure_check_constraint('scrim.status_publication_approvals'::regclass, 'status_publication_status_kind_machine_code_check', 'scrim.is_machine_code(status_kind)');
CALL scrim.ensure_check_constraint('scrim.status_publication_approvals'::regclass, 'status_publication_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.status_publication_approvals'::regclass, 'status_publication_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.validate_constraints('scrim.status_publication_approvals'::regclass, ARRAY[
    'status_publication_payload_hash_matches_content_check',
    'status_publication_target_id_typed_numeric_check',
    'status_publication_status_kind_machine_code_check',
    'status_publication_remote_system_machine_code_check',
    'status_publication_remote_message_id_domain_ref_check'
]);

CREATE OR REPLACE FUNCTION scrim.prevent_status_publication_content_mutation_after_lock()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    locked_states CONSTANT TEXT[] := ARRAY['approved', 'published', 'rejected', 'cancelled'];
BEGIN
    IF (OLD.decision = ANY(locked_states) OR NEW.decision = ANY(locked_states))
       AND (
            NEW.target_kind IS DISTINCT FROM OLD.target_kind
            OR NEW.target_id IS DISTINCT FROM OLD.target_id
            OR NEW.status_kind IS DISTINCT FROM OLD.status_kind
            OR NEW.payload IS DISTINCT FROM OLD.payload
            OR NEW.payload_hash IS DISTINCT FROM OLD.payload_hash
       ) THEN
        RAISE EXCEPTION 'status publication content is immutable once approval starts'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_status_publication_content_mutation_after_lock ON scrim.status_publication_approvals;
CREATE TRIGGER prevent_status_publication_content_mutation_after_lock
    BEFORE UPDATE ON scrim.status_publication_approvals
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_status_publication_content_mutation_after_lock();

DROP TRIGGER IF EXISTS prevent_status_publication_terminal_reopen ON scrim.status_publication_approvals;
CREATE TRIGGER prevent_status_publication_terminal_reopen
    BEFORE UPDATE ON scrim.status_publication_approvals
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'decision',
        'published',
        'rejected',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_status_publication_delete ON scrim.status_publication_approvals;
CREATE TRIGGER prevent_status_publication_delete
    BEFORE DELETE ON scrim.status_publication_approvals
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_status_publication_truncate ON scrim.status_publication_approvals;
CREATE TRIGGER prevent_status_publication_truncate
    BEFORE TRUNCATE ON scrim.status_publication_approvals
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE TABLE IF NOT EXISTS scrim.replacement_needs (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    match_id INTEGER REFERENCES scrim.matches(id),
    team_id INTEGER REFERENCES scrim.teams(id),
    participant_id INTEGER REFERENCES scrim.participants(id),
    needed_role TEXT,
    reason TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open'
        CHECK (status IN ('open', 'contacting', 'filled', 'cancelled', 'expired')),
    request_id TEXT,
    correlation_id TEXT,
    created_by_user_id TEXT NOT NULL,
    created_by_display_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at TIMESTAMPTZ,
    CONSTRAINT replacement_needs_closed_time_check CHECK (
        (closed_at IS NULL AND status IN ('open', 'contacting'))
        OR (closed_at IS NOT NULL AND status IN ('filled', 'cancelled', 'expired'))
    )
);

CREATE INDEX IF NOT EXISTS replacement_needs_status_idx
    ON scrim.replacement_needs (status, created_at, id)
    WHERE status IN ('open', 'contacting');

CREATE INDEX IF NOT EXISTS replacement_needs_match_team_idx
    ON scrim.replacement_needs (match_id, team_id);

CALL scrim.ensure_check_constraint('scrim.replacement_needs'::regclass, 'replacement_needs_needed_role_machine_code_check', 'needed_role IS NULL OR scrim.is_machine_code(needed_role)');
CALL scrim.ensure_check_constraint('scrim.replacement_needs'::regclass, 'replacement_needs_reason_machine_code_check', 'scrim.is_machine_code(reason)');
CALL scrim.ensure_check_constraint('scrim.replacement_needs'::regclass, 'replacement_needs_request_id_domain_ref_check', 'request_id IS NULL OR scrim.is_domain_ref(request_id)');
CALL scrim.ensure_check_constraint('scrim.replacement_needs'::regclass, 'replacement_needs_correlation_id_domain_ref_check', 'correlation_id IS NULL OR scrim.is_domain_ref(correlation_id)');
CALL scrim.validate_constraints('scrim.replacement_needs'::regclass, ARRAY[
    'replacement_needs_needed_role_machine_code_check',
    'replacement_needs_reason_machine_code_check',
    'replacement_needs_request_id_domain_ref_check',
    'replacement_needs_correlation_id_domain_ref_check'
]);

CREATE TABLE IF NOT EXISTS scrim.replacement_candidates (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    need_id BIGINT NOT NULL REFERENCES scrim.replacement_needs(id),
    participant_id INTEGER REFERENCES scrim.participants(id),
    discord_user_id BIGINT,
    candidate_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    score_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    status TEXT NOT NULL DEFAULT 'candidate'
        CHECK (status IN ('candidate', 'shortlisted', 'requested', 'accepted', 'declined', 'expired', 'rejected')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT replacement_candidates_identity_check CHECK (participant_id IS NOT NULL OR discord_user_id IS NOT NULL),
    CONSTRAINT replacement_candidates_need_id_id_key UNIQUE (need_id, id)
);

CREATE UNIQUE INDEX IF NOT EXISTS replacement_candidates_participant_uidx
    ON scrim.replacement_candidates (need_id, participant_id)
    WHERE participant_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS replacement_candidates_discord_uidx
    ON scrim.replacement_candidates (need_id, discord_user_id)
    WHERE discord_user_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS replacement_candidates_need_status_idx
    ON scrim.replacement_candidates (need_id, status, id);

CALL scrim.ensure_check_constraint('scrim.replacement_candidates'::regclass, 'replacement_candidates_status_machine_code_check', 'scrim.is_machine_code(status)');
CALL scrim.validate_constraints('scrim.replacement_candidates'::regclass, ARRAY[
    'replacement_candidates_status_machine_code_check'
]);

CREATE TABLE IF NOT EXISTS scrim.replacement_requests (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    need_id BIGINT NOT NULL REFERENCES scrim.replacement_needs(id),
    candidate_id BIGINT,
    participant_id INTEGER REFERENCES scrim.participants(id),
    discord_user_id BIGINT,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'sent', 'accepted', 'declined', 'expired', 'cancelled', 'failed')),
    request_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    remote_system TEXT,
    remote_message_id TEXT,
    requested_by_user_id TEXT NOT NULL,
    requested_by_display_name TEXT NOT NULL,
    requested_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    responded_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT replacement_requests_identity_check CHECK (candidate_id IS NOT NULL OR participant_id IS NOT NULL OR discord_user_id IS NOT NULL),
    CONSTRAINT replacement_requests_response_time_check CHECK (
        (responded_at IS NULL AND status IN ('pending', 'sent', 'cancelled', 'failed'))
        OR (responded_at IS NOT NULL AND status IN ('accepted', 'declined', 'expired'))
    ),
    CONSTRAINT replacement_requests_candidate_need_fkey
        FOREIGN KEY (need_id, candidate_id)
        REFERENCES scrim.replacement_candidates(need_id, id)
);

CREATE INDEX IF NOT EXISTS replacement_requests_need_status_idx
    ON scrim.replacement_requests (need_id, status, requested_at DESC, id DESC);

CREATE UNIQUE INDEX IF NOT EXISTS replacement_requests_remote_message_uidx
    ON scrim.replacement_requests (remote_system, remote_message_id)
    WHERE remote_message_id IS NOT NULL;

CALL scrim.ensure_check_constraint('scrim.replacement_requests'::regclass, 'replacement_requests_status_machine_code_check', 'scrim.is_machine_code(status)');
CALL scrim.ensure_check_constraint('scrim.replacement_requests'::regclass, 'replacement_requests_remote_system_machine_code_check', 'remote_system IS NULL OR scrim.is_machine_code(remote_system)');
CALL scrim.ensure_check_constraint('scrim.replacement_requests'::regclass, 'replacement_requests_remote_message_id_domain_ref_check', 'remote_message_id IS NULL OR scrim.is_domain_ref(remote_message_id)');
CALL scrim.validate_constraints('scrim.replacement_requests'::regclass, ARRAY[
    'replacement_requests_status_machine_code_check',
    'replacement_requests_remote_system_machine_code_check',
    'replacement_requests_remote_message_id_domain_ref_check'
]);

CREATE TABLE IF NOT EXISTS scrim.match_lineup_snapshots (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    match_id INTEGER NOT NULL REFERENCES scrim.matches(id),
    snapshot_kind TEXT NOT NULL CHECK (snapshot_kind IN ('planned', 'confirmed', 'actual', 'replacement')),
    lineup_payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    source TEXT NOT NULL CHECK (source IN ('legacy', 'turniere', 'dl-bots', 'steam-core')),
    request_id TEXT,
    correlation_id TEXT,
    created_by_user_id TEXT,
    created_by_display_name TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS match_lineup_snapshots_match_idx
    ON scrim.match_lineup_snapshots (match_id, created_at DESC, id DESC);

CALL scrim.ensure_check_constraint('scrim.match_lineup_snapshots'::regclass, 'match_lineup_snapshots_request_id_domain_ref_check', 'request_id IS NULL OR scrim.is_domain_ref(request_id)');
CALL scrim.ensure_check_constraint('scrim.match_lineup_snapshots'::regclass, 'match_lineup_snapshots_correlation_id_domain_ref_check', 'correlation_id IS NULL OR scrim.is_domain_ref(correlation_id)');
CALL scrim.validate_constraints('scrim.match_lineup_snapshots'::regclass, ARRAY[
    'match_lineup_snapshots_request_id_domain_ref_check',
    'match_lineup_snapshots_correlation_id_domain_ref_check'
]);

DROP TRIGGER IF EXISTS prevent_match_lineup_snapshot_update ON scrim.match_lineup_snapshots;
CREATE TRIGGER prevent_match_lineup_snapshot_update
    BEFORE UPDATE ON scrim.match_lineup_snapshots
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_update_except_redaction(
        'lineup_payload',
        'created_by_user_id',
        'created_by_display_name'
    );

DROP TRIGGER IF EXISTS prevent_match_lineup_snapshot_delete ON scrim.match_lineup_snapshots;
CREATE TRIGGER prevent_match_lineup_snapshot_delete
    BEFORE DELETE ON scrim.match_lineup_snapshots
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_match_lineup_snapshot_truncate ON scrim.match_lineup_snapshots;
CREATE TRIGGER prevent_match_lineup_snapshot_truncate
    BEFORE TRUNCATE ON scrim.match_lineup_snapshots
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

CREATE TABLE IF NOT EXISTS scrim.ai_runs (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_kind TEXT NOT NULL,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    idempotency_generation INTEGER NOT NULL DEFAULT 0 CHECK (idempotency_generation >= 0),
    input_hash BYTEA NOT NULL CHECK (octet_length(input_hash) = 32),
    state TEXT NOT NULL DEFAULT 'queued'
        CHECK (state IN ('queued', 'running', 'succeeded', 'failed', 'retry', 'uncertain', 'dead', 'cancelled')),
    provider TEXT,
    model TEXT,
    lease_owner TEXT,
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TIMESTAMPTZ,
    decision_ref_kind TEXT,
    decision_ref_id TEXT,
    lagebild_snapshot_id BIGINT REFERENCES scrim.lagebild_snapshots(id) ON DELETE SET NULL,
    request_id TEXT,
    correlation_id TEXT,
    last_error_code TEXT CHECK (last_error_code IS NULL OR last_error_code ~ '^err_[a-z0-9_]{2,64}$'),
    last_error_hash BYTEA CHECK (last_error_hash IS NULL OR octet_length(last_error_hash) = 32),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ,
    CONSTRAINT ai_runs_subject_kind_check CHECK (
        subject_kind IN ('match', 'match_request', 'team', 'discord_user', 'steam_user')
    ),
    CONSTRAINT ai_runs_decision_ref_pair_check CHECK (
        (decision_ref_kind IS NULL AND decision_ref_id IS NULL)
        OR (decision_ref_kind IS NOT NULL AND decision_ref_id IS NOT NULL)
    ),
    CONSTRAINT ai_runs_decision_ref_kind_check CHECK (
        decision_ref_kind IS NULL
        OR decision_ref_kind IN (
            'runtime_control_history',
            'announcement_approval',
            'status_publication_approval',
            'match_lineup_snapshot',
            'match_result_ref',
            'match_result_clarification'
        )
    ),
    CONSTRAINT ai_runs_decision_ref_id_check CHECK (
        decision_ref_id IS NULL OR decision_ref_id ~ '^[1-9][0-9]{0,17}$'
    ),
    CONSTRAINT ai_runs_lease_pair_check CHECK (
        (lease_owner IS NULL AND lease_until IS NULL)
        OR (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT ai_runs_running_lease_check CHECK (
        (state = 'running') = (lease_owner IS NOT NULL AND lease_until IS NOT NULL)
    ),
    CONSTRAINT ai_runs_retry_time_check CHECK (state <> 'retry' OR next_attempt_at IS NOT NULL),
    CONSTRAINT ai_runs_start_time_check CHECK (started_at IS NULL OR state <> 'queued'),
    CONSTRAINT ai_runs_finish_time_check CHECK (
        (finished_at IS NULL AND state NOT IN ('succeeded', 'failed', 'uncertain', 'dead', 'cancelled'))
        OR (finished_at IS NOT NULL AND state IN ('succeeded', 'failed', 'uncertain', 'dead', 'cancelled'))
    )
);

CREATE UNIQUE INDEX IF NOT EXISTS ai_runs_idempotency_generation_uidx
    ON scrim.ai_runs (run_kind, idempotency_key, idempotency_generation);

CREATE UNIQUE INDEX IF NOT EXISTS ai_runs_one_active_idempotency_uidx
    ON scrim.ai_runs (run_kind, idempotency_key)
    WHERE state IN ('queued', 'running', 'retry');

CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_run_kind_machine_code_check', 'scrim.is_machine_code(run_kind)');
CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_idempotency_key_domain_ref_check', 'scrim.is_domain_ref(idempotency_key)');
CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_provider_machine_code_check', 'provider IS NULL OR scrim.is_machine_code(provider)');
CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_model_machine_code_check', 'model IS NULL OR scrim.is_machine_code(model)');
CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_lease_owner_domain_ref_check', 'lease_owner IS NULL OR scrim.is_domain_ref(lease_owner)');
CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_request_id_domain_ref_check', 'request_id IS NULL OR scrim.is_domain_ref(request_id)');
CALL scrim.ensure_check_constraint('scrim.ai_runs'::regclass, 'ai_runs_correlation_id_domain_ref_check', 'correlation_id IS NULL OR scrim.is_domain_ref(correlation_id)');
CALL scrim.validate_constraints('scrim.ai_runs'::regclass, ARRAY[
    'ai_runs_run_kind_machine_code_check',
    'ai_runs_idempotency_key_domain_ref_check',
    'ai_runs_provider_machine_code_check',
    'ai_runs_model_machine_code_check',
    'ai_runs_lease_owner_domain_ref_check',
    'ai_runs_request_id_domain_ref_check',
    'ai_runs_correlation_id_domain_ref_check'
]);

CREATE OR REPLACE FUNCTION scrim.ai_run_subject_redaction_allowed(
    old_subject_kind TEXT,
    old_subject_id TEXT,
    new_subject_id TEXT
)
RETURNS BOOLEAN
LANGUAGE plpgsql
STABLE
AS $$
DECLARE
    target_ref TEXT := scrim.current_privacy_erasure_target_ref();
    target_namespace TEXT;
BEGIN
    IF old_subject_id IS NULL
       OR new_subject_id IS DISTINCT FROM 'redacted'
       OR target_ref IS NULL
       OR old_subject_id IS DISTINCT FROM target_ref THEN
        RETURN false;
    END IF;

    target_namespace := scrim.privacy_erasure_target_namespace(target_ref);

    RETURN (old_subject_kind = 'discord_user' AND target_namespace = 'discord_user')
        OR (old_subject_kind = 'steam_user' AND target_namespace = 'steam_user');
END;
$$;

CREATE OR REPLACE FUNCTION scrim.prevent_ai_run_identity_mutation()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.run_kind IS DISTINCT FROM OLD.run_kind
       OR NEW.subject_kind IS DISTINCT FROM OLD.subject_kind
       OR NEW.idempotency_key IS DISTINCT FROM OLD.idempotency_key
       OR NEW.idempotency_generation IS DISTINCT FROM OLD.idempotency_generation
       OR NEW.input_hash IS DISTINCT FROM OLD.input_hash THEN
        RAISE EXCEPTION 'scrim ai run identity fields are immutable'
            USING ERRCODE = '55000';
    END IF;

    IF NEW.subject_id IS DISTINCT FROM OLD.subject_id
       AND NOT scrim.ai_run_subject_redaction_allowed(OLD.subject_kind, OLD.subject_id, NEW.subject_id) THEN
        RAISE EXCEPTION 'scrim ai run subject identity is immutable'
            USING ERRCODE = '55000';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS prevent_ai_run_idempotency_mutation ON scrim.ai_runs;
CREATE TRIGGER prevent_ai_run_idempotency_mutation
    BEFORE UPDATE ON scrim.ai_runs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_ai_run_identity_mutation();

DROP TRIGGER IF EXISTS prevent_ai_run_terminal_reopen ON scrim.ai_runs;
CREATE TRIGGER prevent_ai_run_terminal_reopen
    BEFORE UPDATE ON scrim.ai_runs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_terminal_state_reopen(
        'state',
        'succeeded',
        'failed',
        'uncertain',
        'dead',
        'cancelled'
    );

DROP TRIGGER IF EXISTS prevent_ai_run_delete ON scrim.ai_runs;
CREATE TRIGGER prevent_ai_run_delete
    BEFORE DELETE ON scrim.ai_runs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

DROP TRIGGER IF EXISTS prevent_ai_run_truncate ON scrim.ai_runs;
CREATE TRIGGER prevent_ai_run_truncate
    BEFORE TRUNCATE ON scrim.ai_runs
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_journal_delete_or_truncate();

CREATE INDEX IF NOT EXISTS ai_runs_subject_idx
    ON scrim.ai_runs (subject_kind, subject_id, created_at DESC, id DESC);

CREATE TABLE IF NOT EXISTS scrim.ai_decision_refs (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    run_id BIGINT NOT NULL REFERENCES scrim.ai_runs(id),
    decision_kind TEXT NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('match', 'match_request', 'team', 'ai_run')),
    target_id TEXT NOT NULL,
    decision_data JSONB NOT NULL DEFAULT '{}'::jsonb,
    confidence NUMERIC(5, 4) CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT ai_decision_refs_target_id_check CHECK (
        target_id ~ '^[1-9][0-9]{0,18}$'
    )
);

CREATE INDEX IF NOT EXISTS ai_decision_refs_target_idx
    ON scrim.ai_decision_refs (target_kind, target_id, created_at DESC, id DESC);

CALL scrim.ensure_check_constraint('scrim.ai_decision_refs'::regclass, 'ai_decision_refs_decision_kind_machine_code_check', 'scrim.is_machine_code(decision_kind)');
CALL scrim.validate_constraints('scrim.ai_decision_refs'::regclass, ARRAY[
    'ai_decision_refs_decision_kind_machine_code_check'
]);

DROP TRIGGER IF EXISTS prevent_ai_decision_ref_update_delete ON scrim.ai_decision_refs;
CREATE TRIGGER prevent_ai_decision_ref_update_delete
    BEFORE UPDATE OR DELETE ON scrim.ai_decision_refs
    FOR EACH ROW
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

DROP TRIGGER IF EXISTS prevent_ai_decision_ref_truncate ON scrim.ai_decision_refs;
CREATE TRIGGER prevent_ai_decision_ref_truncate
    BEFORE TRUNCATE ON scrim.ai_decision_refs
    FOR EACH STATEMENT
    EXECUTE FUNCTION scrim.prevent_append_only_mutation();

RESET lock_timeout;
