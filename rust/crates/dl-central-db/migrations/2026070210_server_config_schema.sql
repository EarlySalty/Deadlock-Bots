CREATE SCHEMA IF NOT EXISTS server_config;

CREATE TABLE IF NOT EXISTS server_config.desired_categories (
    guild_id BIGINT NOT NULL,
    category_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, category_id)
);

CREATE TABLE IF NOT EXISTS server_config.desired_channels (
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    topic TEXT,
    position INTEGER NOT NULL,
    parent_category_id BIGINT,
    nsfw BOOLEAN NOT NULL DEFAULT false,
    bitrate INTEGER,
    user_limit INTEGER,
    rate_limit_per_user INTEGER,
    status TEXT,
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, channel_id)
);

CREATE INDEX IF NOT EXISTS desired_channels_parent_idx
    ON server_config.desired_channels (guild_id, parent_category_id);

CREATE TABLE IF NOT EXISTS server_config.desired_roles (
    guild_id BIGINT NOT NULL,
    role_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    color INTEGER NOT NULL DEFAULT 0,
    hoist BOOLEAN NOT NULL DEFAULT false,
    mentionable BOOLEAN NOT NULL DEFAULT false,
    managed BOOLEAN NOT NULL DEFAULT false,
    permissions_bitmask BIGINT NOT NULL CHECK (permissions_bitmask >= 0),
    position INTEGER NOT NULL,
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, role_id)
);

CREATE TABLE IF NOT EXISTS server_config.desired_permission_overwrites (
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('role', 'member')),
    target_id BIGINT NOT NULL,
    allow_bits BIGINT NOT NULL CHECK (allow_bits >= 0),
    deny_bits BIGINT NOT NULL CHECK (deny_bits >= 0),
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, channel_id, target_type, target_id)
);

CREATE INDEX IF NOT EXISTS desired_permission_overwrites_target_idx
    ON server_config.desired_permission_overwrites (guild_id, target_type, target_id);

CREATE TABLE IF NOT EXISTS server_config.desired_bot_messages (
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    message_key TEXT NOT NULL,
    message_kind TEXT NOT NULL CHECK (message_kind IN ('panel', 'pinned')),
    message_id BIGINT,
    expected_hash TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, channel_id, message_key)
);

CREATE TABLE IF NOT EXISTS server_config.live_snapshots (
    snapshot_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    source TEXT NOT NULL DEFAULT 'serenity_http',
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS live_snapshots_guild_captured_idx
    ON server_config.live_snapshots (guild_id, captured_at DESC);

CREATE TABLE IF NOT EXISTS server_config.live_snapshot_categories (
    snapshot_id BIGINT NOT NULL REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL,
    guild_id BIGINT NOT NULL,
    category_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    position INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, category_id)
);

CREATE TABLE IF NOT EXISTS server_config.live_snapshot_channels (
    snapshot_id BIGINT NOT NULL REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    channel_type TEXT NOT NULL,
    topic TEXT,
    position INTEGER NOT NULL,
    parent_category_id BIGINT,
    nsfw BOOLEAN NOT NULL DEFAULT false,
    bitrate INTEGER,
    user_limit INTEGER,
    rate_limit_per_user INTEGER,
    status TEXT,
    PRIMARY KEY (snapshot_id, channel_id)
);

CREATE INDEX IF NOT EXISTS live_snapshot_channels_parent_idx
    ON server_config.live_snapshot_channels (snapshot_id, parent_category_id);

CREATE TABLE IF NOT EXISTS server_config.live_snapshot_roles (
    snapshot_id BIGINT NOT NULL REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL,
    guild_id BIGINT NOT NULL,
    role_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    color INTEGER NOT NULL DEFAULT 0,
    hoist BOOLEAN NOT NULL DEFAULT false,
    mentionable BOOLEAN NOT NULL DEFAULT false,
    managed BOOLEAN NOT NULL DEFAULT false,
    permissions_bitmask BIGINT NOT NULL CHECK (permissions_bitmask >= 0),
    position INTEGER NOT NULL,
    PRIMARY KEY (snapshot_id, role_id)
);

CREATE TABLE IF NOT EXISTS server_config.live_snapshot_permission_overwrites (
    snapshot_id BIGINT NOT NULL REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    target_type TEXT NOT NULL CHECK (target_type IN ('role', 'member')),
    target_id BIGINT NOT NULL,
    allow_bits BIGINT NOT NULL CHECK (allow_bits >= 0),
    deny_bits BIGINT NOT NULL CHECK (deny_bits >= 0),
    PRIMARY KEY (snapshot_id, channel_id, target_type, target_id)
);

CREATE INDEX IF NOT EXISTS live_snapshot_permission_overwrites_target_idx
    ON server_config.live_snapshot_permission_overwrites (snapshot_id, target_type, target_id);

CREATE TABLE IF NOT EXISTS server_config.live_snapshot_bot_messages (
    snapshot_id BIGINT NOT NULL REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE CASCADE,
    captured_at TIMESTAMPTZ NOT NULL,
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    message_key TEXT NOT NULL,
    message_kind TEXT NOT NULL CHECK (message_kind IN ('panel', 'pinned')),
    message_id BIGINT,
    observed_hash TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    PRIMARY KEY (snapshot_id, channel_id, message_key)
);

CREATE TABLE IF NOT EXISTS server_config.dynamic_namespaces (
    namespace_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    namespace_key TEXT NOT NULL,
    system_name TEXT NOT NULL,
    object_kind TEXT NOT NULL CHECK (object_kind IN ('channel', 'permission_overwrite', 'bot_message')),
    match_rule_type TEXT NOT NULL CHECK (match_rule_type IN ('parent_category', 'name_prefix', 'name_pattern', 'channel_id')),
    match_rule JSONB NOT NULL DEFAULT '{}'::jsonb,
    foreign_writer TEXT,
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (guild_id, namespace_key)
);

CREATE INDEX IF NOT EXISTS dynamic_namespaces_guild_active_idx
    ON server_config.dynamic_namespaces (guild_id, active);

CREATE TABLE IF NOT EXISTS server_config.documented_exceptions (
    exception_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    exception_key TEXT NOT NULL,
    exception_type TEXT NOT NULL,
    object_kind TEXT NOT NULL CHECK (object_kind IN ('category', 'channel', 'role', 'permission_overwrite', 'bot_message')),
    object_id BIGINT,
    channel_id BIGINT,
    target_type TEXT CHECK (target_type IN ('role', 'member')),
    target_id BIGINT,
    allow_bits BIGINT CHECK (allow_bits IS NULL OR allow_bits >= 0),
    deny_bits BIGINT CHECK (deny_bits IS NULL OR deny_bits >= 0),
    reason TEXT NOT NULL,
    review_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (guild_id, exception_key)
);

CREATE INDEX IF NOT EXISTS documented_exceptions_target_idx
    ON server_config.documented_exceptions (guild_id, object_kind, channel_id, target_type, target_id)
    WHERE active;

CREATE TABLE IF NOT EXISTS server_config.diff_previews (
    preview_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    snapshot_id BIGINT REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE SET NULL,
    diff_hash TEXT NOT NULL,
    diff_json JSONB NOT NULL,
    human_summary TEXT NOT NULL,
    created_by_user_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    applied_at TIMESTAMPTZ,
    UNIQUE (guild_id, diff_hash)
);

CREATE INDEX IF NOT EXISTS diff_previews_snapshot_idx
    ON server_config.diff_previews (snapshot_id);

CREATE TABLE IF NOT EXISTS server_config.apply_runs (
    apply_run_id BIGSERIAL PRIMARY KEY,
    preview_id BIGINT NOT NULL REFERENCES server_config.diff_previews (preview_id) ON DELETE CASCADE,
    guild_id BIGINT NOT NULL,
    confirmed_diff_hash TEXT NOT NULL,
    requested_by_user_id BIGINT,
    dry_run BOOLEAN NOT NULL DEFAULT true,
    status TEXT NOT NULL CHECK (status IN ('planned', 'running', 'applied', 'dry_run', 'failed', 'hash_mismatch')),
    result_json JSONB NOT NULL DEFAULT '{}'::jsonb,
    error_text TEXT,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS apply_runs_preview_idx
    ON server_config.apply_runs (preview_id, started_at DESC);

CREATE TABLE IF NOT EXISTS server_config.auto_revert_whitelist (
    whitelist_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    object_kind TEXT NOT NULL CHECK (object_kind IN ('role', 'permission_overwrite')),
    object_id BIGINT,
    target_type TEXT CHECK (target_type IN ('role', 'member')),
    target_id BIGINT,
    permission_bits BIGINT NOT NULL CHECK (permission_bits >= 0),
    action TEXT NOT NULL CHECK (action IN ('allow_added', 'deny_removed', 'permission_added')),
    reason TEXT NOT NULL,
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS auto_revert_whitelist_guild_active_idx
    ON server_config.auto_revert_whitelist (guild_id, active);

CREATE TABLE IF NOT EXISTS server_config.drift_events (
    drift_event_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    snapshot_id BIGINT REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE SET NULL,
    preview_id BIGINT REFERENCES server_config.diff_previews (preview_id) ON DELETE SET NULL,
    detected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    fingerprint TEXT NOT NULL,
    event_kind TEXT NOT NULL,
    severity TEXT NOT NULL CHECK (severity IN ('info', 'warning', 'critical')),
    object_kind TEXT NOT NULL,
    object_id BIGINT,
    action TEXT NOT NULL,
    diff_json JSONB NOT NULL,
    filtered_dynamic BOOLEAN NOT NULL DEFAULT false,
    documented_exception_id BIGINT REFERENCES server_config.documented_exceptions (exception_id) ON DELETE SET NULL,
    auto_revert_eligible BOOLEAN NOT NULL DEFAULT false,
    status TEXT NOT NULL DEFAULT 'open'
);

CREATE INDEX IF NOT EXISTS drift_events_guild_detected_idx
    ON server_config.drift_events (guild_id, detected_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS drift_events_open_fingerprint_idx
    ON server_config.drift_events (guild_id, fingerprint)
    WHERE resolved_at IS NULL;

CREATE TABLE IF NOT EXISTS server_config.adoption_events (
    adoption_event_id BIGSERIAL PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    snapshot_id BIGINT NOT NULL REFERENCES server_config.live_snapshots (snapshot_id) ON DELETE CASCADE,
    adopted_by_user_id BIGINT,
    object_kind TEXT NOT NULL,
    object_id BIGINT,
    adopted_diff JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS adoption_events_guild_created_idx
    ON server_config.adoption_events (guild_id, created_at DESC);
