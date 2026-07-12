ALTER TABLE core.steam_links
    ADD COLUMN IF NOT EXISTS unlink_reason TEXT;

ALTER TABLE core.steam_links
    ADD COLUMN IF NOT EXISTS refriend_attempted_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS steam_links_refriend_candidate_idx
    ON core.steam_links (discord_id)
    WHERE is_steam_friend = false
      AND unlink_reason = 'inactive_purge';
