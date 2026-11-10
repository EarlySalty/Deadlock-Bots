CREATE TABLE IF NOT EXISTS bot.access_gate_decisions (
    decision_id TEXT PRIMARY KEY,
    decided_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    caller TEXT NOT NULL,
    origin TEXT NOT NULL,
    user_id BIGINT,
    channel_id BIGINT,
    dm BOOLEAN NOT NULL,
    capability TEXT NOT NULL,
    class TEXT NOT NULL,
    internal BOOLEAN NOT NULL,
    target_user_id BIGINT,
    target_role_id BIGINT,
    args_hash TEXT NOT NULL,
    args_redacted JSONB,
    allowed BOOLEAN NOT NULL,
    reason TEXT NOT NULL,
    shadow BOOLEAN NOT NULL,
    confirm_required BOOLEAN NOT NULL,
    place_ok BOOLEAN NOT NULL,
    outcome TEXT,
    outcome_detail TEXT,
    outcome_at TIMESTAMPTZ,
    CONSTRAINT access_gate_decisions_id_format CHECK (decision_id ~ '^[0-9a-f]{32}$'),
    CONSTRAINT access_gate_decisions_origin_known CHECK (
        origin IN ('discord_message', 'discord_interaction', 'none')
    ),
    CONSTRAINT access_gate_decisions_class_known CHECK (
        class IN ('read', 'write', 'destructive')
    ),
    CONSTRAINT access_gate_decisions_args_hash_format CHECK (args_hash ~ '^[0-9a-f]{64}$'),
    CONSTRAINT access_gate_decisions_outcome_known CHECK (
        outcome IS NULL OR outcome IN ('ok', 'error')
    ),
    CONSTRAINT access_gate_decisions_outcome_complete CHECK (
        (outcome IS NULL AND outcome_at IS NULL AND outcome_detail IS NULL)
        OR (outcome IS NOT NULL AND outcome_at IS NOT NULL)
    ),
    CONSTRAINT access_gate_decisions_allow_reason CHECK (allowed = (reason = 'allowed')),
    CONSTRAINT access_gate_decisions_reason_known CHECK (
        reason IN (
            'allowed', 'no_identity', 'not_member', 'lookup_failed', 'access_file_unavailable',
            'unknown_caller', 'connector_disabled', 'capability_disabled', 'owner_only',
            'unmapped', 'missing_right', 'timed_out', 'hierarchy', 'target_lookup_failed'
        )
    ),
    CONSTRAINT access_gate_decisions_caller_format CHECK (caller ~ '^[a-z0-9._-]{1,128}$'),
    CONSTRAINT access_gate_decisions_capability_format CHECK (capability ~ '^[a-z0-9._-]{1,128}$'),
    CONSTRAINT access_gate_decisions_ids_positive CHECK (
        (user_id IS NULL OR user_id > 0)
        AND (channel_id IS NULL OR channel_id > 0)
        AND (target_user_id IS NULL OR target_user_id > 0)
        AND (target_role_id IS NULL OR target_role_id > 0)
    ),
    CONSTRAINT access_gate_decisions_args_object CHECK (
        args_redacted IS NULL OR jsonb_typeof(args_redacted) = 'object'
    ),
    CONSTRAINT access_gate_decisions_outcome_detail_bounded CHECK (
        outcome_detail IS NULL OR char_length(outcome_detail) BETWEEN 1 AND 1000
    )
);

CREATE INDEX IF NOT EXISTS access_gate_decisions_decided_at_idx
    ON bot.access_gate_decisions (decided_at);

COMMENT ON TABLE bot.access_gate_decisions IS
    'Decisions of the dl-bot access gate (POST /gate/v1/authorize). Every allow is inserted before the caller receives it; refusals are recorded best-effort and dropped with a warning log when the write queue is full or the insert fails. dl-bot deletes rows older than runtime.start.access_gate_retention_days every runtime.start.access_gate_purge_every_secs, whether the gate token is set or not.';
COMMENT ON COLUMN bot.access_gate_decisions.decision_id IS
    'Random 128-bit id in lower-case hex, returned to the caller as decision_id.';
COMMENT ON COLUMN bot.access_gate_decisions.user_id IS
    'Discord id of the asking user as sent by the caller; validated and stored even when origin = none, where the decision ignores it. NULL = the caller sent no user id.';
COMMENT ON COLUMN bot.access_gate_decisions.channel_id IS
    'Discord channel id of the request; NULL = none sent. With dm = true it is the DM channel with the asker.';
COMMENT ON COLUMN bot.access_gate_decisions.target_user_id IS
    'Discord id of the member the action targets; NULL = no member target.';
COMMENT ON COLUMN bot.access_gate_decisions.target_role_id IS
    'Discord id of the role the action targets; NULL = no role target.';
COMMENT ON COLUMN bot.access_gate_decisions.args_redacted IS
    'Tool arguments as a JSON object with secret-looking keys replaced and NUL characters removed from keys and strings; NULL = the caller sent none. Above 4096 bytes only {"truncated":true,"bytes":n} is stored.';
COMMENT ON COLUMN bot.access_gate_decisions.reason IS
    'Machine-readable reason from dl_access_gate::Reason; allowed is true exactly when reason = allowed. record_unavailable is never stored because no row could be written.';
COMMENT ON COLUMN bot.access_gate_decisions.outcome IS
    'Result the connector attached through POST /gate/v1/outcome; NULL = none attached yet.';
COMMENT ON COLUMN bot.access_gate_decisions.outcome_detail IS
    'Text attached with the outcome, NUL characters removed and cut to 1000 characters; NULL = none sent, empty after cleaning, or no outcome yet.';
COMMENT ON COLUMN bot.access_gate_decisions.outcome_at IS
    'Time the outcome was attached; NULL exactly when outcome is NULL.';

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON bot.access_gate_decisions TO deadlock;
    END IF;
END
$roles$;
