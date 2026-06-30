CREATE SCHEMA IF NOT EXISTS voice;
CREATE SCHEMA IF NOT EXISTS tierlist;
CREATE SCHEMA IF NOT EXISTS moderation;
CREATE SCHEMA IF NOT EXISTS bot;
CREATE SCHEMA IF NOT EXISTS clips;
CREATE SCHEMA IF NOT EXISTS content;

DROP INDEX IF EXISTS core.steam_links_steam_id64_idx;

ALTER TABLE core.steam_links DROP CONSTRAINT IF EXISTS steam_links_pkey;
ALTER TABLE core.steam_links DROP CONSTRAINT IF EXISTS steam_links_discord_id_fkey;

ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS steam_id TEXT;
UPDATE core.steam_links
   SET steam_id = steam_id64::text
 WHERE steam_id IS NULL
   AND steam_id64 IS NOT NULL;

ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS steam_display_name TEXT;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS primary_account BOOLEAN;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS updated_at TIMESTAMPTZ;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS legacy_ref TEXT;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS migrated_at TIMESTAMPTZ;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS deadlock_rank INTEGER;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS deadlock_subrank INTEGER;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS deadlock_badge_level INTEGER;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS deadlock_rank_name TEXT;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS deadlock_rank_updated_at TIMESTAMPTZ;
ALTER TABLE core.steam_links ADD COLUMN IF NOT EXISTS is_steam_friend BOOLEAN;

UPDATE core.steam_links
   SET steam_id64 = steam_id::bigint
 WHERE steam_id64 IS NULL
   AND steam_id ~ '^[0-9]+$'
   AND (
        length(steam_id) < 19
        OR (length(steam_id) = 19 AND steam_id <= '9223372036854775807')
   );

UPDATE core.steam_links SET verified = false WHERE verified IS NULL;
UPDATE core.steam_links SET primary_account = false WHERE primary_account IS NULL;
UPDATE core.steam_links SET is_steam_friend = false WHERE is_steam_friend IS NULL;

ALTER TABLE core.steam_links ALTER COLUMN steam_id SET NOT NULL;
ALTER TABLE core.steam_links ALTER COLUMN steam_id64 DROP NOT NULL;
ALTER TABLE core.steam_links ALTER COLUMN verified SET DEFAULT false;
ALTER TABLE core.steam_links ALTER COLUMN verified SET NOT NULL;
ALTER TABLE core.steam_links ALTER COLUMN primary_account SET DEFAULT false;
ALTER TABLE core.steam_links ALTER COLUMN primary_account SET NOT NULL;
ALTER TABLE core.steam_links ALTER COLUMN linked_at DROP DEFAULT;
ALTER TABLE core.steam_links ALTER COLUMN linked_at DROP NOT NULL;
ALTER TABLE core.steam_links ALTER COLUMN is_steam_friend SET DEFAULT false;
ALTER TABLE core.steam_links ALTER COLUMN is_steam_friend SET NOT NULL;

ALTER TABLE core.steam_links
    ADD CONSTRAINT steam_links_pkey PRIMARY KEY (discord_id, steam_id);

CREATE INDEX IF NOT EXISTS steam_links_discord_id_idx
    ON core.steam_links (discord_id);

CREATE INDEX IF NOT EXISTS steam_links_steam_id64_idx
    ON core.steam_links (steam_id64)
    WHERE steam_id64 IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS uq_steam_links_steam_owner
    ON core.steam_links (steam_id)
    WHERE discord_id != 0;

CREATE OR REPLACE FUNCTION core.steam_links_owner_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    has_conflict BOOLEAN;
BEGIN
    IF NEW.discord_id = 0 THEN
        RETURN NEW;
    END IF;

    IF TG_OP = 'UPDATE'
       AND NEW.discord_id = OLD.discord_id
       AND NEW.steam_id = OLD.steam_id THEN
        RETURN NEW;
    END IF;

    IF TG_OP = 'INSERT' THEN
        SELECT EXISTS (
            SELECT 1
              FROM core.steam_links
             WHERE steam_id = NEW.steam_id
               AND discord_id NOT IN (NEW.discord_id, 0)
        )
          INTO has_conflict;
    ELSE
        SELECT EXISTS (
            SELECT 1
              FROM core.steam_links
             WHERE steam_id = NEW.steam_id
               AND discord_id NOT IN (NEW.discord_id, 0)
               AND NOT (discord_id = OLD.discord_id AND steam_id = OLD.steam_id)
        )
          INTO has_conflict;
    END IF;

    IF has_conflict THEN
        RAISE EXCEPTION 'steam_links ownership conflict'
            USING ERRCODE = '23505';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_steam_links_owner_guard_insert ON core.steam_links;
CREATE TRIGGER trg_steam_links_owner_guard_insert
    BEFORE INSERT ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.steam_links_owner_guard();

DROP TRIGGER IF EXISTS trg_steam_links_owner_guard_update ON core.steam_links;
CREATE TRIGGER trg_steam_links_owner_guard_update
    BEFORE UPDATE OF discord_id, steam_id ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.steam_links_owner_guard();

CREATE OR REPLACE FUNCTION core.steam_links_user_guard()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.discord_id <> 0
       AND NOT EXISTS (
           SELECT 1
             FROM core.users
            WHERE discord_id = NEW.discord_id
       ) THEN
        RAISE EXCEPTION 'insert or update on table "steam_links" violates user guard'
            USING ERRCODE = '23503';
    END IF;

    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_steam_links_user_guard_insert ON core.steam_links;
CREATE TRIGGER trg_steam_links_user_guard_insert
    BEFORE INSERT ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.steam_links_user_guard();

DROP TRIGGER IF EXISTS trg_steam_links_user_guard_update ON core.steam_links;
CREATE TRIGGER trg_steam_links_user_guard_update
    BEFORE UPDATE OF discord_id ON core.steam_links
    FOR EACH ROW
    EXECUTE FUNCTION core.steam_links_user_guard();

CREATE OR REPLACE FUNCTION core.users_steam_links_delete_cascade()
RETURNS trigger
LANGUAGE plpgsql
AS $$
BEGIN
    DELETE FROM core.steam_links
     WHERE discord_id = OLD.discord_id;

    RETURN OLD;
END;
$$;

DROP TRIGGER IF EXISTS trg_core_users_steam_links_delete_cascade ON core.users;
CREATE TRIGGER trg_core_users_steam_links_delete_cascade
    AFTER DELETE ON core.users
    FOR EACH ROW
    EXECUTE FUNCTION core.users_steam_links_delete_cascade();

CREATE TABLE IF NOT EXISTS core.meta_users (
    id BIGINT PRIMARY KEY,
    username TEXT,
    display_name TEXT,
    avatar_url TEXT,
    role TEXT NOT NULL DEFAULT 'user',
    created_at TIMESTAMPTZ DEFAULT now()
);

CREATE TABLE IF NOT EXISTS core.user_privacy (
    user_id BIGINT PRIMARY KEY,
    opted_out BOOLEAN NOT NULL DEFAULT false,
    deleted_at TIMESTAMPTZ,
    reason TEXT,
    updated_at TIMESTAMPTZ DEFAULT now()
);

CREATE INDEX IF NOT EXISTS user_privacy_opted_out_idx
    ON core.user_privacy (opted_out);
