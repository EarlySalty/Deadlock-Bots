-- Persist steam-core task linkage for durable friend-request reconciliation
-- after steam-bot process restarts or OpenID fire-and-forget queueing.
ALTER TABLE steam.steam_friend_requests
    ADD COLUMN IF NOT EXISTS task_id BIGINT;

CREATE INDEX IF NOT EXISTS steam_friend_requests_pending_task_idx
    ON steam.steam_friend_requests (task_id)
    WHERE status = 'pending' AND task_id IS NOT NULL;
