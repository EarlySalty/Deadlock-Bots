CREATE TABLE IF NOT EXISTS server_config.rollback_exports (
    rollback_export_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    snapshot_id BIGINT REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE SET NULL,
    created_by_user_id BIGINT,
    artifact_hash TEXT NOT NULL,
    artifact_json JSONB NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '180 days')
);

CREATE INDEX IF NOT EXISTS rollback_exports_guild_created_idx
    ON server_config.rollback_exports (guild_id, created_at DESC);

CREATE INDEX IF NOT EXISTS rollback_exports_expires_at_idx
    ON server_config.rollback_exports (expires_at);
