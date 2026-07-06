CREATE TABLE IF NOT EXISTS activity.presence_daily_seen (
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    day DATE NOT NULL,
    PRIMARY KEY (guild_id, user_id, day)
);

CREATE INDEX IF NOT EXISTS presence_daily_seen_day_guild_idx
    ON activity.presence_daily_seen (day, guild_id);

CREATE TABLE IF NOT EXISTS activity.presence_daily_aggregates (
    day DATE NOT NULL,
    guild_id BIGINT NOT NULL,
    distinct_user_count BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (day, guild_id)
);

CREATE TABLE IF NOT EXISTS activity.vanity_uses_snapshots (
    guild_id BIGINT NOT NULL,
    uses INTEGER NOT NULL,
    code TEXT,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS vanity_uses_snapshots_guild_captured_idx
    ON activity.vanity_uses_snapshots (guild_id, captured_at DESC);

CREATE TABLE IF NOT EXISTS activity.guild_member_directory (
    guild_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    joined_at TIMESTAMPTZ,
    account_created_at TIMESTAMPTZ,
    is_bot BOOLEAN NOT NULL DEFAULT FALSE,
    present BOOLEAN NOT NULL DEFAULT TRUE,
    synced_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (guild_id, user_id)
);

CREATE TABLE IF NOT EXISTS activity.insights_imports (
    import_kind TEXT NOT NULL,
    period_start DATE NOT NULL,
    dimension TEXT NOT NULL,
    value NUMERIC NOT NULL,
    guild_id BIGINT NOT NULL,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (guild_id, import_kind, period_start, dimension)
);

CREATE INDEX IF NOT EXISTS insights_imports_guild_kind_period_idx
    ON activity.insights_imports (guild_id, import_kind, period_start);
