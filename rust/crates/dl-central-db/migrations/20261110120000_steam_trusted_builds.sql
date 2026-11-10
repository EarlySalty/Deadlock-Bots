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

COMMENT ON COLUMN steam.trusted_build_curated_players.trusted IS
    'Manual trust override: true = hand-picked as trusted; false = never trusted, even when the account is in the high-MMR pool; NULL = no override.';
COMMENT ON COLUMN steam.trusted_build_curated_players.night_shift_titles IS
    'Number of Night Shift titles entered by hand; 0 = none, the account then earns no title points.';
COMMENT ON COLUMN steam.trusted_build_curated_players.night_shift_region IS
    'Region the titles were won in, used as home region when the account has titles and no pool region; NULL = not set.';
COMMENT ON COLUMN steam.trusted_build_curated_players.display_name IS
    'Name shown for the account instead of its Steam name; NULL or empty = use the Steam name.';
COMMENT ON COLUMN steam.trusted_build_curated_players.tag IS
    'Short label shown with the account; NULL or empty = none.';

CREATE TABLE steam.trusted_build_watch_regions (
    account_id BIGINT NOT NULL,
    server_region TEXT NOT NULL,
    match_day DATE NOT NULL,
    matches INTEGER NOT NULL DEFAULT 0,
    last_match_id BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, server_region, match_day),
    CONSTRAINT trusted_build_watch_matches_range CHECK (matches >= 0),
    CONSTRAINT trusted_build_watch_last_match_range CHECK (last_match_id >= 0),
    CONSTRAINT trusted_build_watch_server_region CHECK (server_region ~ '^[a-z0-9]+$')
);

COMMENT ON COLUMN steam.trusted_build_watch_regions.server_region IS
    'Server region of a sampled live match, reduced by the writer to lowercase ASCII letters and digits (for example europe, row, seasia); never empty.';
COMMENT ON COLUMN steam.trusted_build_watch_regions.matches IS
    'Number of sampled live matches of the account in this server region on this day; a match counts once because only a match id above last_match_id adds to it. The writer stores at least 1; the default 0 means nothing counted yet.';
COMMENT ON COLUMN steam.trusted_build_watch_regions.last_match_id IS
    'Highest match id counted in this row, kept so that every match counts once. The writer always stores a real match id; the default 0 means no match counted yet.';

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
    ),
    CONSTRAINT trusted_build_pool_non_negative CHECK (
        points >= 0
        AND badge >= 0
        AND ranked_games >= 0
        AND world_position >= 0
        AND region_position >= 0
    )
);

COMMENT ON COLUMN steam.trusted_build_pool.badge IS
    'Rank badge number reported with the pool row; 0 = the source reported no badge.';
COMMENT ON COLUMN steam.trusted_build_pool.ranked_games IS
    'Ranked games of the account in the ranked season the pool was read from; 0 = the source reported none.';
COMMENT ON COLUMN steam.trusted_build_pool.world_position IS
    'Rank by points among all pool players, starting at 1.';
COMMENT ON COLUMN steam.trusted_build_pool.region IS
    'Play region derived from the sampled live matches in trusted_build_watch_regions; NULL = the account was not seen in a sampled match of a known region, so it has no regional rank.';
COMMENT ON COLUMN steam.trusted_build_pool.region_position IS
    'Rank by points among the pool players of the same region, starting at 1; 0 = no region is known (region IS NULL).';

CREATE TABLE steam.trusted_build_pool_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT true,
    season TEXT NOT NULL,
    floor_points INTEGER NOT NULL,
    tier_points INTEGER NOT NULL,
    players INTEGER NOT NULL,
    refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT trusted_build_pool_state_singleton CHECK (singleton),
    CONSTRAINT trusted_build_pool_state_non_negative CHECK (
        floor_points >= 0 AND tier_points >= 0 AND players >= 0
    )
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
    refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT trusted_build_index_non_negative CHECK (
        version >= 0
        AND updated_ts >= 0
        AND published_ts >= 0
        AND favorites >= 0
        AND language >= 0
        AND last_used >= 0
    ),
    CONSTRAINT trusted_build_index_scores_finite CHECK (
        score > '-Infinity'::double precision
        AND score < 'Infinity'::double precision
        AND score_author > '-Infinity'::double precision
        AND score_author < 'Infinity'::double precision
        AND score_build > '-Infinity'::double precision
        AND score_build < 'Infinity'::double precision
    )
);

COMMENT ON COLUMN steam.trusted_build_index.name IS
    'Name of the build as reported by the builds source, trimmed; empty when the source reports none.';
COMMENT ON COLUMN steam.trusted_build_index.version IS
    'Version number of the build as reported by the builds source; 0 = not reported.';
COMMENT ON COLUMN steam.trusted_build_index.updated_ts IS
    'Unix seconds of the last update of the build as reported by the builds source; 0 = not reported.';
COMMENT ON COLUMN steam.trusted_build_index.published_ts IS
    'Unix seconds of the publication of the build as reported by the builds source; 0 = not reported.';
COMMENT ON COLUMN steam.trusted_build_index.favorites IS
    'Number of favourites of the build; 0 = none or not reported.';
COMMENT ON COLUMN steam.trusted_build_index.language IS
    'Language number of the build as reported by the builds source; 0 is also stored when the source reports no language.';
COMMENT ON COLUMN steam.trusted_build_index.score IS
    'Total trust score: score_author plus score_build plus a part for games run on the build. Finite; negative values are valid.';
COMMENT ON COLUMN steam.trusted_build_index.score_author IS
    'Part of the score earned by the standing of the author and by playing the hero. Finite.';
COMMENT ON COLUMN steam.trusted_build_index.score_build IS
    'Part of the score lost for items no longer in the game and for a build left untouched; zero or negative. Finite.';
COMMENT ON COLUMN steam.trusted_build_index.last_used IS
    'Unix seconds of the start of the most recent game in which a counted trusted player ran the build; 0 = no counted player ran it in the usage window.';
COMMENT ON COLUMN steam.trusted_build_index.author IS
    'JSON object {person, hero_games, main_hero}: the trust facts of the author at refresh time (account, name, badge, home region, pool position, points, titles, standing), the games of the author on the hero and whether those make it a main hero.';
COMMENT ON COLUMN steam.trusted_build_index.reasons IS
    'JSON array of score reasons, each an object with kind (trusted, ranked_points, night_shift_titles, plays_hero, items_gone, untouched), points and the facts of that kind; an empty array means no reason applied.';
COMMENT ON COLUMN steam.trusted_build_index.users IS
    'JSON array with one object per counted trusted player who ran the build (account, name, last, games, queue, weight, adherence, main, factor, hero_games, home, top, position, badge), heaviest first; an empty array means nobody ran it.';
COMMENT ON COLUMN steam.trusted_build_index.details IS
    'JSON object {description, tags, item_ids, phases, ability_order} with the items per phase and the ability order by name; NULL = the builds source returned no details for the build at refresh time.';
COMMENT ON COLUMN steam.trusted_build_index.changes IS
    'JSON object {patches, hero_lines, items, gone}: the patch notes after updated_ts that touch the hero or an item of the build, and the items of the build no longer in the game; NULL = no such patch was found (also when the patch notes could not be read) and no item of the build is gone.';

CREATE INDEX trusted_build_index_hero_score_idx
    ON steam.trusted_build_index (hero_id, score DESC);

CREATE TABLE steam.trusted_build_hero_state (
    hero_id INTEGER PRIMARY KEY,
    hero_name TEXT NOT NULL,
    builds INTEGER NOT NULL DEFAULT 0,
    refreshed_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    CONSTRAINT trusted_build_hero_state_builds_range CHECK (builds >= 0)
);

COMMENT ON COLUMN steam.trusted_build_hero_state.builds IS
    'Number of builds listed for the hero by the last successful refresh; 0 = none listed, or no refresh has succeeded yet (refreshed_at IS NULL).';

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT, UPDATE, DELETE ON
            steam.trusted_build_curated_players,
            steam.trusted_build_watch_regions,
            steam.trusted_build_pool,
            steam.trusted_build_pool_state,
            steam.trusted_build_index,
            steam.trusted_build_hero_state
        TO deadlock;
    END IF;
END
$roles$;
