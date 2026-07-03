CREATE TABLE IF NOT EXISTS core.discord_role_connection_tokens (
    discord_id BIGINT PRIMARY KEY REFERENCES core.meta_users(id) ON DELETE CASCADE,
    access_token BYTEA NOT NULL,
    refresh_token BYTEA NOT NULL,
    token_type TEXT NOT NULL DEFAULT 'Bearer',
    scope TEXT NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    token_version INTEGER NOT NULL DEFAULT 1,
    active BOOLEAN NOT NULL DEFAULT true,
    invalidated_at TIMESTAMPTZ,
    invalidation_reason TEXT,
    last_refresh_at TIMESTAMPTZ,
    last_push_at TIMESTAMPTZ,
    last_push_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS discord_role_connection_tokens_active_idx
    ON core.discord_role_connection_tokens (active, expires_at);

CREATE TABLE IF NOT EXISTS core.discord_role_connection_sync_state (
    discord_id BIGINT PRIMARY KEY,
    pending BOOLEAN NOT NULL DEFAULT true,
    reason TEXT NOT NULL DEFAULT 'manual',
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    locked_at TIMESTAMPTZ,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS discord_role_connection_sync_due_idx
    ON core.discord_role_connection_sync_state (pending, next_attempt_at, updated_at);

CREATE OR REPLACE FUNCTION core.enqueue_discord_role_connection_sync()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    target_discord_id BIGINT;
    sync_reason TEXT := 'steam_link_changed';
BEGIN
    IF TG_OP = 'DELETE' THEN
        target_discord_id := OLD.discord_id;
        sync_reason := 'steam_link_removed';
    ELSIF TG_OP = 'UPDATE' AND NEW.discord_id IS DISTINCT FROM OLD.discord_id THEN
        INSERT INTO core.discord_role_connection_sync_state (
            discord_id, pending, reason, attempts, next_attempt_at, locked_at, last_error, updated_at
        )
        SELECT target.target_discord_id, TRUE, target.sync_reason, 0, now(), NULL, NULL, now()
          FROM (
              VALUES
                  (OLD.discord_id, 'steam_link_removed'::TEXT),
                  (NEW.discord_id, 'steam_link_changed'::TEXT)
          ) AS target(target_discord_id, sync_reason)
         WHERE target.target_discord_id IS NOT NULL
           AND target.target_discord_id <> 0
        ON CONFLICT (discord_id) DO UPDATE SET
            pending = TRUE,
            reason = EXCLUDED.reason,
            attempts = 0,
            next_attempt_at = now(),
            locked_at = NULL,
            last_error = NULL,
            updated_at = now();

        RETURN NEW;
    ELSE
        target_discord_id := NEW.discord_id;
        IF TG_OP = 'UPDATE'
           AND (
               COALESCE(NEW.deadlock_badge_level, NEW.deadlock_rank, 0)
                   IS DISTINCT FROM COALESCE(OLD.deadlock_badge_level, OLD.deadlock_rank, 0)
               OR NEW.deadlock_rank_name IS DISTINCT FROM OLD.deadlock_rank_name
               OR NEW.deadlock_subrank IS DISTINCT FROM OLD.deadlock_subrank
               OR NEW.deadlock_rank_updated_at IS DISTINCT FROM OLD.deadlock_rank_updated_at
           ) THEN
            sync_reason := 'rank_changed';
        END IF;
    END IF;

    IF target_discord_id IS NULL OR target_discord_id = 0 THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;

    INSERT INTO core.discord_role_connection_sync_state (
        discord_id, pending, reason, attempts, next_attempt_at, locked_at, last_error, updated_at
    )
    VALUES (
        target_discord_id, TRUE, sync_reason, 0, now(), NULL, NULL, now()
    )
    ON CONFLICT (discord_id) DO UPDATE SET
        pending = TRUE,
        reason = EXCLUDED.reason,
        attempts = 0,
        next_attempt_at = now(),
        locked_at = NULL,
        last_error = NULL,
        updated_at = now();

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_steam_links_role_connection_sync_insert ON core.steam_links;
CREATE TRIGGER trg_steam_links_role_connection_sync_insert
    AFTER INSERT ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.enqueue_discord_role_connection_sync();

DROP TRIGGER IF EXISTS trg_steam_links_role_connection_sync_update ON core.steam_links;
CREATE TRIGGER trg_steam_links_role_connection_sync_update
    AFTER UPDATE OF discord_id, verified, primary_account, deadlock_rank, deadlock_subrank,
                    deadlock_badge_level, deadlock_rank_name, deadlock_rank_updated_at
    ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.enqueue_discord_role_connection_sync();

DROP TRIGGER IF EXISTS trg_steam_links_role_connection_sync_delete ON core.steam_links;
CREATE TRIGGER trg_steam_links_role_connection_sync_delete
    AFTER DELETE ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.enqueue_discord_role_connection_sync();
