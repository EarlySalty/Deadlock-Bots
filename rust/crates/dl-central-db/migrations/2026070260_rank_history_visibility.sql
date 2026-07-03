CREATE TABLE IF NOT EXISTS steam.rank_history_visibility (
    user_id BIGINT PRIMARY KEY,
    visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (visibility IN ('private', 'members', 'public')),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS rank_history_visibility_visibility_idx
    ON steam.rank_history_visibility (visibility);

CREATE INDEX IF NOT EXISTS steam_rank_history_user_captured_visible_cover_idx
    ON steam.steam_rank_history (user_id, captured_at DESC)
    INCLUDE (badge_level, rank_name)
    WHERE badge_level IS NOT NULL AND rank_name IS NOT NULL;
