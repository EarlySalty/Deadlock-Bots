-- Account-scope future rank-history reads. Existing rows stay NULL because
-- historical snapshots cannot be mapped back to a Steam account reliably.
ALTER TABLE steam.steam_rank_history
    ADD COLUMN IF NOT EXISTS steam_id BIGINT;

CREATE INDEX IF NOT EXISTS steam_rank_history_user_steam_captured_idx
    ON steam.steam_rank_history (user_id, steam_id, captured_at);
