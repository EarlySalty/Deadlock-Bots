CREATE SCHEMA pool;

CREATE TABLE pool.profiles (
    guild_id BIGINT NOT NULL CHECK (guild_id > 0),
    discord_id BIGINT NOT NULL REFERENCES core.users(discord_id) ON DELETE CASCADE CHECK (discord_id > 0),
    modes TEXT[] NOT NULL DEFAULT '{}' CHECK (modes <@ ARRAY['casual', 'ranked', 'street_brawl']::text[]),
    group_size_min SMALLINT NOT NULL DEFAULT 2 CHECK (group_size_min BETWEEN 2 AND 6),
    group_size_max SMALLINT NOT NULL DEFAULT 6 CHECK (group_size_max BETWEEN group_size_min AND 6),
    play_style TEXT NOT NULL DEFAULT 'any' CHECK (play_style IN ('any', 'relaxed', 'competitive', 'learning')),
    voice_preference TEXT NOT NULL DEFAULT 'any' CHECK (voice_preference IN ('any', 'with_voice', 'without_voice')),
    languages TEXT[] NOT NULL DEFAULT ARRAY['de']::text[],
    timezone TEXT NOT NULL DEFAULT 'Europe/Berlin',
    dm_opt_in BOOLEAN NOT NULL DEFAULT false,
    published BOOLEAN NOT NULL DEFAULT false,
    interview_completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, discord_id)
);

CREATE INDEX profiles_discord_idx ON pool.profiles (discord_id);
CREATE INDEX profiles_published_idx ON pool.profiles (guild_id, discord_id) WHERE published;
CREATE INDEX profiles_modes_idx ON pool.profiles USING gin (modes);

CREATE TABLE pool.availability (
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    weekday SMALLINT NOT NULL CHECK (weekday BETWEEN 0 AND 6),
    start_minute SMALLINT NOT NULL CHECK (start_minute BETWEEN 0 AND 1439),
    end_minute SMALLINT NOT NULL CHECK (end_minute BETWEEN 1 AND 1440 AND end_minute > start_minute),
    PRIMARY KEY (guild_id, discord_id, weekday, start_minute, end_minute),
    FOREIGN KEY (guild_id, discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE
);

CREATE TABLE pool.preferred_players (
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    target_discord_id BIGINT NOT NULL CHECK (target_discord_id <> discord_id),
    PRIMARY KEY (guild_id, discord_id, target_discord_id),
    FOREIGN KEY (guild_id, discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE,
    FOREIGN KEY (guild_id, target_discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE
);
CREATE INDEX preferred_players_target_idx ON pool.preferred_players (target_discord_id);

CREATE TABLE pool.api_snapshots (
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    steam_id TEXT NOT NULL,
    rank_tier INTEGER CHECK (rank_tier >= 0),
    rank_subtier INTEGER CHECK (rank_subtier >= 0),
    games_played BIGINT CHECK (games_played >= 0),
    total_play_seconds BIGINT CHECK (total_play_seconds >= 0),
    observed_games BIGINT NOT NULL CHECK (observed_games >= 0),
    observed_play_seconds BIGINT NOT NULL CHECK (observed_play_seconds >= 0),
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL CHECK (window_end >= window_start),
    fetched_at TIMESTAMPTZ NOT NULL,
    heatmap INTEGER[] NOT NULL CHECK (array_ndims(heatmap) = 1 AND array_lower(heatmap, 1) = 1 AND cardinality(heatmap) = 168 AND 0 <= ALL(heatmap) AND array_position(heatmap, NULL) IS NULL),
    PRIMARY KEY (guild_id, discord_id),
    FOREIGN KEY (guild_id, discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE,
    FOREIGN KEY (discord_id, steam_id) REFERENCES core.steam_links(discord_id, steam_id) ON DELETE CASCADE
);

CREATE TABLE pool.matches (
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    steam_id TEXT NOT NULL,
    match_id BIGINT NOT NULL CHECK (match_id > 0),
    started_at TIMESTAMPTZ NOT NULL,
    duration_seconds INTEGER CHECK (duration_seconds >= 0),
    mode TEXT CHECK (mode IN ('casual', 'ranked', 'street_brawl')),
    PRIMARY KEY (guild_id, discord_id, match_id),
    FOREIGN KEY (guild_id, discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE,
    FOREIGN KEY (discord_id, steam_id) REFERENCES core.steam_links(discord_id, steam_id) ON DELETE CASCADE
);
CREATE INDEX matches_started_idx ON pool.matches (guild_id, discord_id, started_at DESC);

CREATE TABLE pool.co_players (
    guild_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    target_discord_id BIGINT NOT NULL CHECK (discord_id < target_discord_id),
    games_together BIGINT NOT NULL CHECK (games_together > 0),
    last_played_at TIMESTAMPTZ NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL CHECK (window_end >= window_start),
    fetched_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (guild_id, discord_id, target_discord_id),
    FOREIGN KEY (guild_id, discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE,
    FOREIGN KEY (guild_id, target_discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE
);
CREATE INDEX co_players_target_idx ON pool.co_players (guild_id, target_discord_id, last_played_at DESC);

CREATE TABLE pool.sessions (
    session_id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    guild_id BIGINT NOT NULL,
    initiator_discord_id BIGINT NOT NULL,
    mode TEXT NOT NULL CHECK (mode IN ('casual', 'ranked', 'street_brawl')),
    channel_id BIGINT UNIQUE CHECK (channel_id > 0),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'open', 'active', 'ended', 'expired', 'failed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT (now() + INTERVAL '1 hour'),
    started_at TIMESTAMPTZ,
    ended_at TIMESTAMPTZ,
    CHECK (expires_at = created_at + INTERVAL '1 hour'),
    CHECK (started_at IS NULL OR started_at >= created_at),
    CHECK (ended_at IS NULL OR ended_at >= COALESCE(started_at, created_at)),
    CHECK (status NOT IN ('open', 'active', 'ended') OR channel_id IS NOT NULL),
    CHECK (status NOT IN ('active', 'ended') OR started_at IS NOT NULL),
    CHECK ((status IN ('ended', 'expired', 'failed')) = (ended_at IS NOT NULL)),
    UNIQUE (guild_id, session_id),
    FOREIGN KEY (guild_id, initiator_discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE
);
CREATE INDEX sessions_due_idx ON pool.sessions (expires_at) WHERE status IN ('pending', 'open', 'active');

CREATE TABLE pool.session_participants (
    guild_id BIGINT NOT NULL,
    session_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    joined_at TIMESTAMPTZ,
    PRIMARY KEY (guild_id, session_id, discord_id),
    FOREIGN KEY (guild_id, session_id) REFERENCES pool.sessions(guild_id, session_id) ON DELETE CASCADE,
    FOREIGN KEY (guild_id, discord_id) REFERENCES pool.profiles(guild_id, discord_id) ON DELETE CASCADE
);
CREATE INDEX session_participants_user_idx ON pool.session_participants (discord_id, guild_id, session_id);

CREATE TABLE pool.feedback (
    guild_id BIGINT NOT NULL,
    session_id BIGINT NOT NULL,
    discord_id BIGINT NOT NULL,
    play_again BOOLEAN NOT NULL DEFAULT false,
    friendly BOOLEAN NOT NULL DEFAULT false,
    good_communication BOOLEAN NOT NULL DEFAULT false,
    balanced_match BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (guild_id, session_id, discord_id),
    FOREIGN KEY (guild_id, session_id, discord_id) REFERENCES pool.session_participants(guild_id, session_id, discord_id) ON DELETE CASCADE
);

CREATE FUNCTION pool.erase_profile_sessions() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM pool.sessions s
     WHERE s.guild_id = OLD.guild_id
       AND (s.initiator_discord_id = OLD.discord_id OR EXISTS (
           SELECT 1 FROM pool.session_participants p
            WHERE p.guild_id = s.guild_id AND p.session_id = s.session_id AND p.discord_id = OLD.discord_id
       ));
    RETURN OLD;
END
$$;
CREATE TRIGGER erase_profile_sessions BEFORE DELETE ON pool.profiles
    FOR EACH ROW EXECUTE FUNCTION pool.erase_profile_sessions();

CREATE FUNCTION pool.invalidate_steam_data() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.verified = OLD.verified
       AND NEW.primary_account = OLD.primary_account AND NEW.steam_id = OLD.steam_id THEN
        RETURN NEW;
    END IF;
    DELETE FROM pool.co_players WHERE discord_id = OLD.discord_id OR target_discord_id = OLD.discord_id;
    DELETE FROM pool.api_snapshots WHERE discord_id = OLD.discord_id;
    DELETE FROM pool.matches WHERE discord_id = OLD.discord_id;
    RETURN NULL;
END
$$;
CREATE TRIGGER pool_invalidate_steam_data AFTER DELETE OR UPDATE OF verified, primary_account, steam_id ON core.steam_links
    FOR EACH ROW EXECUTE FUNCTION pool.invalidate_steam_data();

REVOKE ALL ON SCHEMA pool FROM PUBLIC;
REVOKE ALL ON ALL TABLES IN SCHEMA pool FROM PUBLIC;
GRANT USAGE ON SCHEMA pool TO deadlock;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pool TO deadlock;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pool TO deadlock;
