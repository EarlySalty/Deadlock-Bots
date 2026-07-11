-- Permanenter Spiegel des von Discord nur begrenzt vorgehaltenen Guild-Audit-Logs.
-- Diese Tabelle ist INSERT-only und hat bewusst keine Retention oder Cleanup-Funktion.
CREATE TABLE IF NOT EXISTS core.discord_audit_log (
    entry_id BIGINT PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    action_type INTEGER NOT NULL,
    user_id BIGINT NULL,
    target_id BIGINT NULL,
    changes JSONB NULL,
    options JSONB NULL,
    reason TEXT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    ingested_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
