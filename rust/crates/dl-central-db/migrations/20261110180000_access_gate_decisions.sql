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
    CONSTRAINT access_gate_decisions_allow_reason CHECK (allowed = (reason = 'allowed'))
);

CREATE INDEX IF NOT EXISTS access_gate_decisions_decided_at_idx
    ON bot.access_gate_decisions (decided_at);

COMMENT ON TABLE bot.access_gate_decisions IS
    'One row per decision of the dl-bot access gate (POST /gate/v1/authorize). Rows older than 180 days are purged by the gate.';
COMMENT ON COLUMN bot.access_gate_decisions.decision_id IS
    'Random 128-bit id in lower-case hex, returned to the caller as decision_id.';
COMMENT ON COLUMN bot.access_gate_decisions.user_id IS
    'Discord id of the asking user; NULL = the caller sent no user id.';
COMMENT ON COLUMN bot.access_gate_decisions.channel_id IS
    'Discord channel id of the request; NULL = none sent. With dm = true it is the DM channel with the asker.';
COMMENT ON COLUMN bot.access_gate_decisions.target_user_id IS
    'Discord id of the member the action targets; NULL = no member target.';
COMMENT ON COLUMN bot.access_gate_decisions.target_role_id IS
    'Discord id of the role the action targets; NULL = no role target.';
COMMENT ON COLUMN bot.access_gate_decisions.args_redacted IS
    'Tool arguments with secret-looking keys replaced; NULL = the caller sent none. Above 4096 bytes only {"truncated":true,"bytes":n} is stored.';
COMMENT ON COLUMN bot.access_gate_decisions.reason IS
    'Machine-readable reason; allowed is true exactly when reason = allowed.';
COMMENT ON COLUMN bot.access_gate_decisions.outcome IS
    'Result the connector attached through POST /gate/v1/outcome; NULL = none attached yet.';
COMMENT ON COLUMN bot.access_gate_decisions.outcome_detail IS
    'Bounded text (at most 1000 characters) attached with the outcome; NULL = none sent or no outcome yet.';
COMMENT ON COLUMN bot.access_gate_decisions.outcome_at IS
    'Time the outcome was attached; NULL exactly when outcome is NULL.';

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON bot.access_gate_decisions TO deadlock;
    END IF;
END
$roles$;
