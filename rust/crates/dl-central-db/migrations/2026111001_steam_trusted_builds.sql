CREATE TABLE steam.trusted_build_curated_players (
    account_id BIGINT PRIMARY KEY,
    trusted BOOLEAN,
    night_shift_titles INTEGER NOT NULL DEFAULT 0,
    night_shift_region TEXT,
    display_name TEXT,
    tag TEXT,
    note TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT trusted_build_curated_titles_range CHECK (night_shift_titles >= 0),
    CONSTRAINT trusted_build_curated_region CHECK (
        night_shift_region IS NULL
        OR night_shift_region IN ('NAmerica', 'Europe', 'Asia', 'SAmerica', 'Oceania')
    )
);

CREATE TABLE steam.trusted_build_watch_regions (
    account_id BIGINT NOT NULL,
    server_region TEXT NOT NULL,
    match_day DATE NOT NULL,
    matches INTEGER NOT NULL DEFAULT 0,
    last_match_id BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, server_region, match_day),
    CONSTRAINT trusted_build_watch_matches_range CHECK (matches >= 0)
);

CREATE INDEX trusted_build_watch_regions_day_idx
    ON steam.trusted_build_watch_regions (match_day);

CREATE TABLE steam.trusted_build_pool (
    account_id BIGINT PRIMARY KEY,
    points INTEGER NOT NULL,
    badge INTEGER NOT NULL DEFAULT 0,
    ranked_games INTEGER NOT NULL DEFAULT 0,
    world_position INTEGER NOT NULL,
    region TEXT,
    region_position INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT trusted_build_pool_region CHECK (
        region IS NULL OR region IN ('NAmerica', 'Europe', 'Asia', 'SAmerica', 'Oceania')
    )
);

CREATE TABLE steam.trusted_build_pool_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT true,
    season TEXT NOT NULL,
    floor_points INTEGER NOT NULL,
    tier_points INTEGER NOT NULL,
    players INTEGER NOT NULL,
    refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT trusted_build_pool_state_singleton CHECK (singleton)
);

CREATE TABLE steam.trusted_build_index (
    build_id BIGINT PRIMARY KEY,
    hero_id INTEGER NOT NULL,
    name TEXT NOT NULL,
    author_account_id BIGINT NOT NULL,
    version INTEGER NOT NULL DEFAULT 0,
    updated_ts BIGINT NOT NULL DEFAULT 0,
    published_ts BIGINT NOT NULL DEFAULT 0,
    favorites INTEGER NOT NULL DEFAULT 0,
    language INTEGER NOT NULL DEFAULT 0,
    score DOUBLE PRECISION NOT NULL,
    score_author DOUBLE PRECISION NOT NULL,
    score_build DOUBLE PRECISION NOT NULL,
    last_used BIGINT NOT NULL DEFAULT 0,
    author JSONB NOT NULL,
    reasons JSONB NOT NULL,
    users JSONB NOT NULL,
    details JSONB,
    changes JSONB,
    refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX trusted_build_index_hero_score_idx
    ON steam.trusted_build_index (hero_id, score DESC);

CREATE TABLE steam.trusted_build_hero_state (
    hero_id INTEGER PRIMARY KEY,
    hero_name TEXT NOT NULL,
    builds INTEGER NOT NULL DEFAULT 0,
    refreshed_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    last_error TEXT
);
