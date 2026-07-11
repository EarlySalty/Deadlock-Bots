-- Admin-getriebener Playtest-Invite: offene Freundschaftsanfragen, auf deren
-- Annahme der Invite-Poller wartet. Ersetzt den alten beta-invite-Funnel.
CREATE TABLE IF NOT EXISTS steam.invite_requests (
    steam_id64 BIGINT PRIMARY KEY,
    account_id BIGINT NOT NULL,
    admin_id BIGINT NOT NULL,
    target_discord_id BIGINT NULL,
    created_at BIGINT NOT NULL,
    last_check_at BIGINT NULL,
    attempts INT NOT NULL DEFAULT 0
);
