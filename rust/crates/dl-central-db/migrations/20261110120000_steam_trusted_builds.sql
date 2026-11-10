CREATE TABLE steam.trusted_build_curated_players (
    account_id BIGINT PRIMARY KEY,
    trusted BOOLEAN,
    night_shift_titles INTEGER NOT NULL DEFAULT 0,
    night_shift_region TEXT,
    display_name TEXT,
    tag TEXT,
    note TEXT,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT trusted_build_curated_account_range CHECK (account_id BETWEEN 1 AND 4294967295),
    CONSTRAINT trusted_build_curated_titles_range CHECK (night_shift_titles >= 0),
    CONSTRAINT trusted_build_curated_region CHECK (
        night_shift_region IS NULL
        OR night_shift_region IN ('NAmerica', 'Europe', 'Asia', 'SAmerica', 'Oceania')
    ),
    CONSTRAINT trusted_build_curated_display_name_filled CHECK (
        display_name IS NULL OR display_name ~ '[^[:space:]]'
    ),
    CONSTRAINT trusted_build_curated_tag_filled CHECK (tag IS NULL OR tag ~ '[^[:space:]]'),
    CONSTRAINT trusted_build_curated_note_filled CHECK (note IS NULL OR note ~ '[^[:space:]]'),
    CONSTRAINT trusted_build_curated_updated_finite CHECK (isfinite(updated_at))
);

COMMENT ON COLUMN steam.trusted_build_curated_players.account_id IS
    'Public Steam account id of the player in its 32 bit form, 1 to 4294967295; the 17 digit 64 bit Steam id is out of range and rejected.';
COMMENT ON COLUMN steam.trusted_build_curated_players.trusted IS
    'Manual trust override: true = hand-picked as trusted; false = never trusted, even when the account is in the high-MMR pool; NULL = no override.';
COMMENT ON COLUMN steam.trusted_build_curated_players.night_shift_titles IS
    'Number of Night Shift titles entered by hand; 0 = none, the account then earns no title points.';
COMMENT ON COLUMN steam.trusted_build_curated_players.night_shift_region IS
    'Region the titles were won in, one of NAmerica, Europe, Asia, SAmerica, Oceania; used as home region when the account has titles and no pool region, and ignored while night_shift_titles is 0. NULL = not set.';
COMMENT ON COLUMN steam.trusted_build_curated_players.display_name IS
    'Name shown for the account instead of its Steam name. NULL = use the Steam name; a text that is empty or only whitespace is rejected.';
COMMENT ON COLUMN steam.trusted_build_curated_players.tag IS
    'Short label shown with the account. NULL = none; a text that is empty or only whitespace is rejected.';
COMMENT ON COLUMN steam.trusted_build_curated_players.note IS
    'Free remark for maintainers; the Steam bot does not read it and it is not shown. NULL = none; a text that is empty or only whitespace is rejected.';
COMMENT ON COLUMN steam.trusted_build_curated_players.updated_at IS
    'Time of the last change of the row. Set when the row is inserted; there is no trigger, so a hand edit must set it. Finite.';

CREATE TABLE steam.trusted_build_watch_regions (
    account_id BIGINT NOT NULL,
    server_region TEXT NOT NULL,
    match_day DATE NOT NULL,
    matches INTEGER NOT NULL DEFAULT 0,
    last_match_id BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, server_region, match_day),
    CONSTRAINT trusted_build_watch_account_range CHECK (account_id BETWEEN 1 AND 4294967295),
    CONSTRAINT trusted_build_watch_matches_range CHECK (matches >= 0),
    CONSTRAINT trusted_build_watch_last_match_range CHECK (last_match_id >= 0),
    CONSTRAINT trusted_build_watch_server_region CHECK (server_region ~ '^[a-z0-9]+$'),
    CONSTRAINT trusted_build_watch_day_finite CHECK (isfinite(match_day))
);

COMMENT ON COLUMN steam.trusted_build_watch_regions.account_id IS
    'Public Steam account id of a player seen in a sampled live match, 32 bit form, 1 to 4294967295.';
COMMENT ON COLUMN steam.trusted_build_watch_regions.server_region IS
    'Server region of the sampled live match as the live match source names it, reduced by the writer to lowercase ASCII letters and digits (for example row, europe, seasia); never empty.';
COMMENT ON COLUMN steam.trusted_build_watch_regions.match_day IS
    'UTC calendar day of the match start; when the source reports no start time, the UTC day of the sampling. Finite.';
COMMENT ON COLUMN steam.trusted_build_watch_regions.matches IS
    'Matches of the account in this server region on this day, each match counted once: only a match id above last_match_id adds to it. The writer stores at least 1; the default 0 means nothing counted yet.';
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
    CONSTRAINT trusted_build_pool_account_range CHECK (account_id BETWEEN 1 AND 4294967295),
    CONSTRAINT trusted_build_pool_region CHECK (
        region IS NULL OR region IN ('NAmerica', 'Europe', 'Asia', 'SAmerica', 'Oceania')
    ),
    CONSTRAINT trusted_build_pool_non_negative CHECK (
        points >= 0
        AND badge >= 0
        AND ranked_games >= 0
        AND region_position >= 0
    ),
    CONSTRAINT trusted_build_pool_world_position CHECK (world_position >= 1),
    CONSTRAINT trusted_build_pool_region_pair CHECK ((region IS NULL) = (region_position = 0))
);

COMMENT ON COLUMN steam.trusted_build_pool.account_id IS
    'Public Steam account id of the pool member, 32 bit form, 1 to 4294967295.';
COMMENT ON COLUMN steam.trusted_build_pool.points IS
    'Ranked points of the account in the ranked season the pool was read from. The writer stores only accounts at or above floor_points of trusted_build_pool_state; 0 = the points are not known, the reader then scores the account by its regional rank alone.';
COMMENT ON COLUMN steam.trusted_build_pool.badge IS
    'Rank badge number reported with the pool row; 0 = the source reported no badge.';
COMMENT ON COLUMN steam.trusted_build_pool.ranked_games IS
    'Ranked games of the account in the ranked season the pool was read from; 0 = the source reported none.';
COMMENT ON COLUMN steam.trusted_build_pool.world_position IS
    'Rank by points among all pool players, starting at 1; 0 and negative values are rejected.';
COMMENT ON COLUMN steam.trusted_build_pool.region IS
    'Play region derived from the sampled live matches in trusted_build_watch_regions, one of NAmerica, Europe, Asia, SAmerica, Oceania; NULL = the account was not seen in a sampled match of a known region, so it has no regional rank.';
COMMENT ON COLUMN steam.trusted_build_pool.region_position IS
    'Rank by points among the pool players of the same region, starting at 1; 0 = no region is known. The schema keeps both in step: region is NULL exactly when region_position is 0.';

CREATE TABLE steam.trusted_build_pool_state (
    singleton BOOLEAN PRIMARY KEY DEFAULT true,
    season TEXT NOT NULL,
    floor_points INTEGER NOT NULL,
    tier_points INTEGER NOT NULL,
    players INTEGER NOT NULL,
    refreshed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT trusted_build_pool_state_singleton CHECK (singleton),
    CONSTRAINT trusted_build_pool_state_ranges CHECK (
        floor_points >= 0 AND tier_points > 0 AND players >= 0
    ),
    CONSTRAINT trusted_build_pool_state_refreshed_finite CHECK (isfinite(refreshed_at))
);

COMMENT ON COLUMN steam.trusted_build_pool_state.singleton IS
    'Always true; false is rejected. As primary key it keeps the table at one row, the state of the last pool read.';
COMMENT ON COLUMN steam.trusted_build_pool_state.season IS
    'Name of the ranked season the pool was read from, as the source reports it; empty only when the source reports an empty name.';
COMMENT ON COLUMN steam.trusted_build_pool_state.floor_points IS
    'Lowest ranked points of a pool member: nine times tier_points, the writer keeps only accounts at or above it. 0 = no lower bound; the writer does not store it because tier_points is positive.';
COMMENT ON COLUMN steam.trusted_build_pool_state.tier_points IS
    'Ranked points one rank tier spans in that season. Positive: the writer skips a season that reports no tier span, and the reader divides by this value.';
COMMENT ON COLUMN steam.trusted_build_pool_state.players IS
    'Number of rows in trusted_build_pool after the last pool read; 0 = the source listed no account at or above floor_points.';
COMMENT ON COLUMN steam.trusted_build_pool_state.refreshed_at IS
    'Time the last pool read was stored. Finite.';

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
    CONSTRAINT trusted_build_index_ids CHECK (
        build_id > 0
        AND hero_id > 0
        AND author_account_id BETWEEN 0 AND 4294967295
    ),
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
    ),
    CONSTRAINT trusted_build_index_score_signs CHECK (score_author >= 0 AND score_build <= 0),
    CONSTRAINT trusted_build_index_author_shape CHECK (
        jsonb_typeof(author) = 'object'
        AND jsonb_typeof(author -> 'person') IS NOT DISTINCT FROM 'object'
        AND jsonb_typeof(author -> 'hero_games') IS NOT DISTINCT FROM 'number'
        AND jsonb_typeof(author -> 'main_hero') IS NOT DISTINCT FROM 'boolean'
    ),
    CONSTRAINT trusted_build_index_reasons_array CHECK (jsonb_typeof(reasons) = 'array'),
    CONSTRAINT trusted_build_index_users_array CHECK (jsonb_typeof(users) = 'array'),
    CONSTRAINT trusted_build_index_details_object CHECK (
        details IS NULL OR jsonb_typeof(details) = 'object'
    ),
    CONSTRAINT trusted_build_index_changes_object CHECK (
        changes IS NULL OR jsonb_typeof(changes) = 'object'
    ),
    CONSTRAINT trusted_build_index_refreshed_finite CHECK (isfinite(refreshed_at))
);

COMMENT ON COLUMN steam.trusted_build_index.build_id IS
    'Id of the build as the builds source reports it; positive.';
COMMENT ON COLUMN steam.trusted_build_index.hero_id IS
    'Id of the hero the build belongs to; positive. The same id as trusted_build_hero_state.hero_id.';
COMMENT ON COLUMN steam.trusted_build_index.name IS
    'Name of the build as reported by the builds source, trimmed; empty when the source reports none.';
COMMENT ON COLUMN steam.trusted_build_index.author_account_id IS
    'Public Steam account id of the author as the builds source reports it, 32 bit form, at most 4294967295; 0 = the source reported no author, no account has that id.';
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
    'Part of the score earned by the standing of the author and by playing the hero: the sum of the trusted, ranked_points, night_shift_titles and plays_hero points in reasons. Finite, zero or positive; 0 = the author earns no points.';
COMMENT ON COLUMN steam.trusted_build_index.score_build IS
    'Part of the score lost for items no longer in the game and for a build left untouched: the sum of the items_gone and untouched points in reasons. Finite, zero or negative; 0 = nothing is deducted.';
COMMENT ON COLUMN steam.trusted_build_index.last_used IS
    'Unix seconds of the start of the most recent game in which a counted trusted player ran the build; 0 = no counted player ran it in the usage window.';
COMMENT ON COLUMN steam.trusted_build_index.author IS
    'JSON object with the required keys person (object), hero_games (number) and main_hero (true or false): the trust facts of the author at refresh time (account, name, badge, home region, pool position, points, titles, standing), the games of the author on the hero and whether those make it a main hero. The schema checks the object and these three keys; the content of person is written and read by the Steam bot.';
COMMENT ON COLUMN steam.trusted_build_index.reasons IS
    'JSON array of score reasons, each written as an object with kind (trusted, ranked_points, night_shift_titles, plays_hero, items_gone, untouched), points and the facts of that kind; an empty array means no reason applied. The schema checks that it is an array; the entries are written and read by the Steam bot.';
COMMENT ON COLUMN steam.trusted_build_index.users IS
    'JSON array with one entry per counted trusted player who ran the build, each written as an object (account, name, last, games, queue, weight, adherence, main, factor, hero_games, home, top, position, badge), heaviest first; an empty array means nobody ran it. The schema checks that it is an array; the entries are written and read by the Steam bot.';
COMMENT ON COLUMN steam.trusted_build_index.details IS
    'JSON object {description, tags, item_ids, phases, ability_order} with the items per phase and the ability order by name; a key missing from the object reads as not reported. NULL = the builds source returned no details for the build at refresh time. JSON null and every other JSON type are rejected.';
COMMENT ON COLUMN steam.trusted_build_index.changes IS
    'JSON object {patches, hero_lines, items, gone}: the patch notes after updated_ts that touch the hero or an item of the build, and the items of the build no longer in the game; a key missing from the object reads as nothing found. NULL = no such patch was found and no item of the build is gone. NULL does not tell whether the patch notes were read: when they could not be read at refresh time the writer finds no patch and stores NULL as well, or an object with patches 0 when an item is gone. JSON null and every other JSON type are rejected.';
COMMENT ON COLUMN steam.trusted_build_index.refreshed_at IS
    'Time the row was last written by a refresh of its hero. Finite.';

CREATE INDEX trusted_build_index_hero_score_idx
    ON steam.trusted_build_index (hero_id, score DESC);

CREATE TABLE steam.trusted_build_hero_state (
    hero_id INTEGER PRIMARY KEY,
    hero_name TEXT NOT NULL,
    builds INTEGER NOT NULL DEFAULT 0,
    refreshed_at TIMESTAMPTZ,
    last_attempt_at TIMESTAMPTZ,
    last_error TEXT,
    CONSTRAINT trusted_build_hero_state_hero_range CHECK (hero_id > 0),
    CONSTRAINT trusted_build_hero_state_builds_range CHECK (builds >= 0),
    CONSTRAINT trusted_build_hero_state_times_finite CHECK (
        (refreshed_at IS NULL OR isfinite(refreshed_at))
        AND (last_attempt_at IS NULL OR isfinite(last_attempt_at))
    ),
    CONSTRAINT trusted_build_hero_state_last_error_filled CHECK (
        last_error IS NULL OR last_error <> ''
    )
);

COMMENT ON COLUMN steam.trusted_build_hero_state.hero_id IS
    'Id of the hero as the refresh task carried it; positive. The cycle task queues only heroes with a positive id; a task queued by hand with id 0 fails on this check.';
COMMENT ON COLUMN steam.trusted_build_hero_state.hero_name IS
    'Name of the hero as the refresh task carried it; empty when the task was queued without a name. A failed attempt does not change the name of an existing row.';
COMMENT ON COLUMN steam.trusted_build_hero_state.builds IS
    'Number of builds listed for the hero by the last successful refresh; 0 = none listed, or no refresh has succeeded yet (refreshed_at IS NULL).';
COMMENT ON COLUMN steam.trusted_build_hero_state.refreshed_at IS
    'Time of the last successful refresh of the hero; NULL = none has succeeded yet. Finite.';
COMMENT ON COLUMN steam.trusted_build_hero_state.last_attempt_at IS
    'Time of the last refresh attempt, successful or not; NULL = never. Finite.';
COMMENT ON COLUMN steam.trusted_build_hero_state.last_error IS
    'Error text of the last refresh attempt; NULL = the last attempt succeeded or none was made. Never empty: the writer stores a non-empty text for a failed attempt.';

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
