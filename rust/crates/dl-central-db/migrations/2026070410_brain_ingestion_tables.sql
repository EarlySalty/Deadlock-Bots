-- Brain-Cutover Phase 0.5: die 26 fehlenden SQLite-Tabellen als sauberes brain.* PG-Schema.
--
-- Kontext: Deadlock-Brain zieht vollstaendig von SQLite auf die zentrale Postgres um
-- (docs/specs/2026-07-04-pg-cutover.md im Deadlock-Brain-Repo). 0012/0013 haben die ersten
-- 13 brain.*-Tabellen angelegt; diese Migration ergaenzt die restlichen 26 (Gap A) PG-idiomatisch,
-- NICHT als 1:1-SQLite-Kopie. Datenladung erfolgt separat in Phase 1 (One-Shot sqlx-Tool).
--
-- Konventionen (uebernommen aus 0012_brain_knowledge_timeline.sql):
--   * Autoincrement-PK-Tabellen bekommen `id BIGSERIAL PRIMARY KEY` + `legacy_sqlite_id BIGINT UNIQUE`
--     (Original-SQLite-rowid fuer das Phase-1-FK-Remap).
--   * Jede FK auf eine surrogat-id-Tabelle (BIGSERIAL) bekommt die echte Spalte
--     `<fk>_id BIGINT REFERENCES brain.<parent>(id)` PLUS `legacy_<fk>_id BIGINT` (Original-Wert,
--     wird in Phase 1 via `UPDATE ... FROM parent WHERE parent.legacy_sqlite_id = child.legacy_<fk>_id`
--     aufgeloest). ON DELETE-Semantik aus SQLite erhalten (CASCADE/SET NULL).
--   * Typmapping: `*_json` TEXT -> JSONB (Suffix faellt weg); INTEGER-Unix-Sekunden -> TIMESTAMPTZ;
--     TEXT-ISO-Zeit (youtube_videos.published_at) -> TIMESTAMPTZ; INTEGER -> BIGINT;
--     REAL -> DOUBLE PRECISION; TEXT -> TEXT.
--   * SQLite-UNIQUE-Constraints/-Indizes uebernommen (inkl. partieller COALESCE(model,'')-Unique-Indizes
--     der Note-Tabellen). Read-Pfad-Indizes (Namen/Slugs/entity_type/patch_tag/hero_id, FK-Spalten)
--     gleich mitangelegt.
--
-- Design-Entscheidungen (Abweichungen/Warzen-Fixes):
--   * NATUERLICHE / KOMPOSITE PKs erhalten, KEIN id-Remap, KEIN legacy_sqlite_id:
--       - hero_catalog (hero_id), item_catalog (item_id)   -> echte Deadlock-Ids
--       - hero_ability_orders / hero_item_stats / hero_item_synergies  -> komposite Natur-PKs
--       - youtube_feed_sources (feed_key TEXT), youtube_videos (video_id TEXT),
--         youtube_transcripts (video_id TEXT), youtube_transcript_claim_attempts (video_id,prompt_version)
--     FKs, die auf diese Natur-PKs zeigen, brauchen KEINE legacy-Spalte (Wert bleibt stabil).
--   * mechanic_notes.rowid: in SQLite eine ECHTE Spalte (INTEGER NOT NULL), keine System-rowid.
--     -> umbenannt in `sqlite_rowid` (BIGINT NOT NULL), damit nichts mit PG-Systemspalten kollidiert.
--   * learned_builds.language: in der Live-SQLite durchgaengig der INTEGER 0 (Deadlock-Builds-API-
--     Sprach-Enum, KEIN ISO-Code) -> bleibt BIGINT (kein TEXT).
--   * Boolean-artige 0/1-Flags (youtube_feed_sources.enabled, sheet_raw_heroes.disabled) bleiben
--     bewusst BIGINT statt BOOLEAN: die Spec fixiert INTEGER->BIGINT als deterministisches Phase-1-
--     Mapping; kein Sonderfall-Cast im Loader.
--   * sheet_tab_rows.row_json -> `row_data` (statt nacktem `row`; ROW ist in PG ein reserviertes Wort).
--   * KEINE synthetische `imported_at`-Provenienzspalte (anders als 0012): learned_builds und
--     youtube_transcripts fuehren `imported_at` bereits als ECHTE Datenspalte -> Kollision vermieden,
--     Schema bleibt spaltentreu zu SQLite (nur id/legacy_* kommen hinzu).
--   * sheet_items.item_id und sheet_raw_heroes.hero_id / learned_builds.hero_id haben in SQLite KEINE
--     FOREIGN KEY -> bleiben nackte BIGINT (kein neuer FK erfunden), nur Lookup-Indizes ergaenzt.

-- =====================================================================================
-- Gruppe a) Katalog + abgeleitete Katalog-Statistiken (natuerliche/komposite PKs)
-- =====================================================================================

CREATE TABLE IF NOT EXISTS brain.hero_catalog (
    hero_id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    base_health BIGINT NOT NULL,
    archetype TEXT NOT NULL,
    stats JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_hero_catalog_name_idx
    ON brain.hero_catalog (name);

CREATE TABLE IF NOT EXISTS brain.item_catalog (
    item_id BIGINT PRIMARY KEY,
    name TEXT NOT NULL,
    slot_type TEXT NOT NULL,
    tier BIGINT NOT NULL,
    defense_kind JSONB NOT NULL,
    damage_axis TEXT NOT NULL,
    properties JSONB NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_item_catalog_name_idx
    ON brain.item_catalog (name);

CREATE INDEX IF NOT EXISTS brain_item_catalog_slot_idx
    ON brain.item_catalog (slot_type);

CREATE TABLE IF NOT EXISTS brain.hero_ability_orders (
    hero_id BIGINT NOT NULL REFERENCES brain.hero_catalog(hero_id),
    bracket TEXT NOT NULL,
    abilities JSONB NOT NULL,
    wins BIGINT NOT NULL,
    losses BIGINT NOT NULL,
    matches BIGINT NOT NULL,
    players BIGINT NOT NULL,
    patch_tag TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (hero_id, bracket, patch_tag)
);

CREATE INDEX IF NOT EXISTS brain_hero_ability_orders_patch_idx
    ON brain.hero_ability_orders (patch_tag);

CREATE TABLE IF NOT EXISTS brain.hero_item_stats (
    hero_id BIGINT NOT NULL REFERENCES brain.hero_catalog(hero_id),
    item_id BIGINT NOT NULL REFERENCES brain.item_catalog(item_id),
    bracket TEXT NOT NULL,
    prevalence_builds BIGINT NOT NULL,
    wins BIGINT NOT NULL,
    losses BIGINT NOT NULL,
    matches BIGINT NOT NULL,
    players BIGINT NOT NULL,
    avg_buy_time_relative DOUBLE PRECISION,
    lift_pp DOUBLE PRECISION,
    patch_tag TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (hero_id, item_id, bracket, patch_tag)
);

CREATE INDEX IF NOT EXISTS brain_hero_item_stats_hero_idx
    ON brain.hero_item_stats (hero_id, bracket, patch_tag, prevalence_builds, matches);

CREATE INDEX IF NOT EXISTS brain_hero_item_stats_item_idx
    ON brain.hero_item_stats (item_id, patch_tag);

CREATE TABLE IF NOT EXISTS brain.hero_item_synergies (
    hero_id BIGINT NOT NULL REFERENCES brain.hero_catalog(hero_id),
    item_id BIGINT NOT NULL REFERENCES brain.item_catalog(item_id),
    with_item_id BIGINT NOT NULL REFERENCES brain.item_catalog(item_id),
    wins BIGINT NOT NULL,
    losses BIGINT NOT NULL,
    matches BIGINT NOT NULL,
    patch_tag TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (hero_id, item_id, with_item_id, patch_tag)
);

CREATE INDEX IF NOT EXISTS brain_hero_item_synergies_item_idx
    ON brain.hero_item_synergies (hero_id, item_id, patch_tag, matches);

CREATE INDEX IF NOT EXISTS brain_hero_item_synergies_with_item_idx
    ON brain.hero_item_synergies (with_item_id, patch_tag);

-- =====================================================================================
-- Gruppe b) Hero-Stat-Profile (Snapshot-abgeleitet, BIGSERIAL + legacy-Remap)
-- =====================================================================================

CREATE TABLE IF NOT EXISTS brain.hero_stat_profiles (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE SET NULL,
    legacy_entity_id BIGINT,
    hero_name TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    row_number BIGINT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_hero_stat_profiles_hero_idx
    ON brain.hero_stat_profiles (hero_name);

CREATE INDEX IF NOT EXISTS brain_hero_stat_profiles_entity_idx
    ON brain.hero_stat_profiles (entity_id);

CREATE TABLE IF NOT EXISTS brain.hero_stat_values (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    profile_id BIGINT NOT NULL REFERENCES brain.hero_stat_profiles(id) ON DELETE CASCADE,
    legacy_profile_id BIGINT NOT NULL,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE SET NULL,
    legacy_entity_id BIGINT,
    hero_name TEXT NOT NULL,
    stat_key TEXT NOT NULL,
    stat_label TEXT NOT NULL,
    numeric_value DOUBLE PRECISION,
    raw_value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (profile_id, stat_key)
);

CREATE INDEX IF NOT EXISTS brain_hero_stat_values_key_idx
    ON brain.hero_stat_values (stat_key);

CREATE INDEX IF NOT EXISTS brain_hero_stat_values_entity_key_idx
    ON brain.hero_stat_values (entity_id, stat_key);

-- =====================================================================================
-- Gruppe c) Community-Sheet-Rohdaten (alle Snapshot-abgeleitet, BIGSERIAL + legacy-Remap)
-- =====================================================================================

CREATE TABLE IF NOT EXISTS brain.sheet_boons_ap (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    souls BIGINT NOT NULL,
    boons BIGINT,
    ap BIGINT,
    note TEXT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_boons_ap_souls_idx
    ON brain.sheet_boons_ap (souls);

CREATE TABLE IF NOT EXISTS brain.sheet_hero_rankings (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE SET NULL,
    legacy_entity_id BIGINT,
    hero_name TEXT NOT NULL,
    carry DOUBLE PRECISION,
    crowd_control DOUBLE PRECISION,
    disengage DOUBLE PRECISION,
    early DOUBLE PRECISION,
    engage DOUBLE PRECISION,
    frontline DOUBLE PRECISION,
    late DOUBLE PRECISION,
    mid DOUBLE PRECISION,
    mid_contest DOUBLE PRECISION,
    mobility DOUBLE PRECISION,
    nuke_phys DOUBLE PRECISION,
    nuke_spirit DOUBLE PRECISION,
    pick DOUBLE PRECISION,
    poke DOUBLE PRECISION,
    support DOUBLE PRECISION,
    wave_clear DOUBLE PRECISION,
    sustain_dps_phys DOUBLE PRECISION,
    sustain_dps_spirit DOUBLE PRECISION,
    average_rank DOUBLE PRECISION,
    payload_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_hero_rankings_name_idx
    ON brain.sheet_hero_rankings (hero_name);

CREATE INDEX IF NOT EXISTS brain_sheet_hero_rankings_entity_idx
    ON brain.sheet_hero_rankings (entity_id);

CREATE TABLE IF NOT EXISTS brain.sheet_heroes_stats (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE SET NULL,
    legacy_entity_id BIGINT,
    hero_name TEXT NOT NULL,
    alt_fire_type TEXT,
    hero_labs TEXT,
    base_hp DOUBLE PRECISION,
    base_move_speed DOUBLE PRECISION,
    base_sprint DOUBLE PRECISION,
    base_stamina DOUBLE PRECISION,
    base_regen DOUBLE PRECISION,
    base_ammo DOUBLE PRECISION,
    pellets DOUBLE PRECISION,
    alt_fire_pellets DOUBLE PRECISION,
    base_bullet_dmg DOUBLE PRECISION,
    base_fire_rate DOUBLE PRECISION,
    base_dps DOUBLE PRECISION,
    max_gun_dps DOUBLE PRECISION,
    max_gun_damage DOUBLE PRECISION,
    dpm DOUBLE PRECISION,
    max_dpm DOUBLE PRECISION,
    falloff_range_min DOUBLE PRECISION,
    falloff_range_max DOUBLE PRECISION,
    hp_gain DOUBLE PRECISION,
    dmg_gain DOUBLE PRECISION,
    spirit_gain DOUBLE PRECISION,
    spirit_bonus DOUBLE PRECISION,
    spirit_bonus_2 DOUBLE PRECISION,
    spirit_ratio DOUBLE PRECISION,
    spirit_ratio_2 DOUBLE PRECISION,
    spirit_scaling DOUBLE PRECISION,
    spirit_scaling_2 DOUBLE PRECISION,
    aggregate_growth_pct DOUBLE PRECISION,
    dps_growth_pct DOUBLE PRECISION,
    hp_growth_pct DOUBLE PRECISION,
    melee_ratio DOUBLE PRECISION,
    total_bullet_ratio DOUBLE PRECISION,
    total_spirit_ratio DOUBLE PRECISION,
    max_level_hp DOUBLE PRECISION,
    payload_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_heroes_stats_name_idx
    ON brain.sheet_heroes_stats (hero_name);

CREATE INDEX IF NOT EXISTS brain_sheet_heroes_stats_entity_idx
    ON brain.sheet_heroes_stats (entity_id);

CREATE TABLE IF NOT EXISTS brain.sheet_items (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    item_id BIGINT,
    code_name TEXT NOT NULL,
    game_name TEXT NOT NULL,
    canonical_name TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_items_canonical_idx
    ON brain.sheet_items (canonical_name);

CREATE INDEX IF NOT EXISTS brain_sheet_items_game_idx
    ON brain.sheet_items (game_name);

CREATE INDEX IF NOT EXISTS brain_sheet_items_code_idx
    ON brain.sheet_items (code_name);

CREATE INDEX IF NOT EXISTS brain_sheet_items_item_idx
    ON brain.sheet_items (item_id);

CREATE TABLE IF NOT EXISTS brain.sheet_raw_heroes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE SET NULL,
    legacy_entity_id BIGINT,
    hero_name TEXT NOT NULL,
    hero_id BIGINT,
    disabled BIGINT NOT NULL DEFAULT 0,
    move_speed DOUBLE PRECISION,
    sprint_speed DOUBLE PRECISION,
    crouch_speed DOUBLE PRECISION,
    move_accel DOUBLE PRECISION,
    light_melee_dmg DOUBLE PRECISION,
    heavy_melee_dmg DOUBLE PRECISION,
    max_hp DOUBLE PRECISION,
    base_stamina DOUBLE PRECISION,
    stam_regen DOUBLE PRECISION,
    hp_regen DOUBLE PRECISION,
    base_health DOUBLE PRECISION,
    gun_growth DOUBLE PRECISION,
    alt_gun_growth DOUBLE PRECISION,
    hp_per_boon DOUBLE PRECISION,
    melee_gain DOUBLE PRECISION,
    spirit_per_boon DOUBLE PRECISION,
    payload_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_raw_heroes_name_idx
    ON brain.sheet_raw_heroes (hero_name);

CREATE INDEX IF NOT EXISTS brain_sheet_raw_heroes_entity_idx
    ON brain.sheet_raw_heroes (entity_id);

CREATE TABLE IF NOT EXISTS brain.sheet_shop_bonuses (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    souls_cost BIGINT NOT NULL,
    weapon BIGINT,
    spirit BIGINT,
    vitality BIGINT,
    inc_from_prev_pct DOUBLE PRECISION,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_shop_bonuses_souls_idx
    ON brain.sheet_shop_bonuses (souls_cost);

CREATE TABLE IF NOT EXISTS brain.sheet_tab_rows (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id) ON DELETE CASCADE,
    legacy_snapshot_id BIGINT NOT NULL,
    tab_name TEXT NOT NULL,
    gid TEXT NOT NULL,
    row_number BIGINT,
    canonical_name TEXT,
    row_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (snapshot_id)
);

CREATE INDEX IF NOT EXISTS brain_sheet_tab_rows_canonical_idx
    ON brain.sheet_tab_rows (canonical_name);

CREATE INDEX IF NOT EXISTS brain_sheet_tab_rows_tab_idx
    ON brain.sheet_tab_rows (tab_name);

-- =====================================================================================
-- Gruppe d) Gelernte Builds + Build-Lernnotizen
-- =====================================================================================

CREATE TABLE IF NOT EXISTS brain.learned_builds (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    source TEXT NOT NULL,
    source_build_id TEXT NOT NULL,
    hero_id BIGINT,
    hero_name TEXT,
    language BIGINT,
    source_rank BIGINT,
    quality_tier TEXT NOT NULL,
    quality_score DOUBLE PRECISION NOT NULL,
    name TEXT,
    author_account_id TEXT,
    description TEXT,
    tags JSONB NOT NULL DEFAULT '[]'::jsonb,
    details JSONB NOT NULL DEFAULT '{}'::jsonb,
    item_names JSONB NOT NULL DEFAULT '[]'::jsonb,
    ability_order JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    imported_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (source, source_build_id)
);

CREATE INDEX IF NOT EXISTS brain_learned_builds_quality_idx
    ON brain.learned_builds (quality_tier, quality_score);

CREATE INDEX IF NOT EXISTS brain_learned_builds_hero_idx
    ON brain.learned_builds (hero_name, quality_score);

CREATE INDEX IF NOT EXISTS brain_learned_builds_hero_id_idx
    ON brain.learned_builds (hero_id);

CREATE TABLE IF NOT EXISTS brain.build_learning_notes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    learned_build_id BIGINT REFERENCES brain.learned_builds(id) ON DELETE CASCADE,
    legacy_learned_build_id BIGINT,
    hero_name TEXT,
    source TEXT NOT NULL,
    context_hash TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    result_text TEXT,
    insights JSONB NOT NULL DEFAULT '{}'::jsonb,
    model TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_build_learning_notes_hero_idx
    ON brain.build_learning_notes (hero_name, updated_at DESC);

CREATE INDEX IF NOT EXISTS brain_build_learning_notes_build_idx
    ON brain.build_learning_notes (learned_build_id);

CREATE UNIQUE INDEX IF NOT EXISTS brain_build_learning_notes_unique_idx
    ON brain.build_learning_notes (learned_build_id, context_hash, prompt_version, COALESCE(model, ''), status);

-- =====================================================================================
-- Gruppe e) Analyse- / Impact- / Decision- / Trend- / Mechanik-Notizen (LLM-Ableitungen)
-- =====================================================================================

CREATE TABLE IF NOT EXISTS brain.analysis_notes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    query TEXT NOT NULL,
    entity_type TEXT,
    entity_name TEXT,
    context_kind TEXT NOT NULL,
    context_hash TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    result_text TEXT,
    model TEXT,
    confidence DOUBLE PRECISION,
    status TEXT NOT NULL,
    source_references JSONB NOT NULL DEFAULT '[]'::jsonb,
    context JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_analysis_notes_query_idx
    ON brain.analysis_notes (query, created_at);

CREATE INDEX IF NOT EXISTS brain_analysis_notes_entity_idx
    ON brain.analysis_notes (entity_type, entity_name);

CREATE UNIQUE INDEX IF NOT EXISTS brain_analysis_notes_unique_idx
    ON brain.analysis_notes (query, context_hash, prompt_version, COALESCE(model, ''), status);

CREATE TABLE IF NOT EXISTS brain.patch_impact_notes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    entity_type TEXT NOT NULL,
    entity_name TEXT NOT NULL,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE SET NULL,
    legacy_entity_id BIGINT,
    context_hash TEXT NOT NULL,
    prompt_version TEXT NOT NULL DEFAULT 'patch_impact_de_v1',
    prompt_text TEXT NOT NULL,
    result_text TEXT,
    insights JSONB NOT NULL DEFAULT '{}'::jsonb,
    model TEXT,
    status TEXT NOT NULL,
    patch_range_start TEXT,
    patch_range_end TEXT,
    event_count BIGINT,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_patch_impact_notes_status_idx
    ON brain.patch_impact_notes (status);

CREATE INDEX IF NOT EXISTS brain_patch_impact_notes_entity_name_idx
    ON brain.patch_impact_notes (entity_name);

CREATE INDEX IF NOT EXISTS brain_patch_impact_notes_entity_idx
    ON brain.patch_impact_notes (entity_id);

CREATE UNIQUE INDEX IF NOT EXISTS brain_patch_impact_notes_unique_idx
    ON brain.patch_impact_notes (entity_name, context_hash, prompt_version, COALESCE(model, ''));

CREATE TABLE IF NOT EXISTS brain.player_match_decision_notes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    account_id TEXT NOT NULL,
    match_id TEXT NOT NULL,
    hero_id TEXT,
    hero_name TEXT,
    context_hash TEXT NOT NULL,
    prompt_version TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    result_text TEXT,
    insights JSONB NOT NULL DEFAULT '{}'::jsonb,
    model TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_player_match_decision_notes_lookup_idx
    ON brain.player_match_decision_notes (account_id, match_id, updated_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS brain_player_match_decision_notes_unique_idx
    ON brain.player_match_decision_notes (account_id, match_id, context_hash, prompt_version, COALESCE(model, ''), status);

CREATE TABLE IF NOT EXISTS brain.meta_trend_notes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    entity_name TEXT NOT NULL,
    trend_direction TEXT NOT NULL,
    winrate_delta DOUBLE PRECISION NOT NULL,
    context JSONB NOT NULL DEFAULT '{}'::jsonb,
    result_text TEXT,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_meta_trend_notes_entity_name_idx
    ON brain.meta_trend_notes (entity_name);

-- mechanic_notes: SQLite `rowid` ist eine echte Datenspalte -> hier `sqlite_rowid`.
CREATE TABLE IF NOT EXISTS brain.mechanic_notes (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    title TEXT NOT NULL,
    content TEXT NOT NULL,
    source TEXT NOT NULL,
    category TEXT NOT NULL,
    sqlite_rowid BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (title)
);

CREATE INDEX IF NOT EXISTS brain_mechanic_notes_category_idx
    ON brain.mechanic_notes (category);

-- =====================================================================================
-- Gruppe f) YouTube-Lern-Layer (Feeds -> Videos -> Transkripte/Claims/Versuche)
-- =====================================================================================

CREATE TABLE IF NOT EXISTS brain.youtube_feed_sources (
    feed_key TEXT PRIMARY KEY,
    source_type TEXT NOT NULL,
    url TEXT NOT NULL,
    handle TEXT,
    playlist_id TEXT,
    channel_id TEXT,
    title TEXT,
    enabled BIGINT NOT NULL DEFAULT 1,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS brain.youtube_videos (
    video_id TEXT PRIMARY KEY,
    feed_key TEXT NOT NULL REFERENCES brain.youtube_feed_sources(feed_key),
    channel_id TEXT,
    channel_title TEXT,
    title TEXT NOT NULL,
    url TEXT NOT NULL,
    published_at TIMESTAMPTZ,
    description TEXT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    transcript_status TEXT NOT NULL DEFAULT 'missing',
    learning_status TEXT NOT NULL DEFAULT 'queued',
    discovered_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_youtube_videos_learning_idx
    ON brain.youtube_videos (learning_status, transcript_status, published_at);

CREATE INDEX IF NOT EXISTS brain_youtube_videos_feed_idx
    ON brain.youtube_videos (feed_key);

CREATE TABLE IF NOT EXISTS brain.youtube_transcripts (
    video_id TEXT PRIMARY KEY REFERENCES brain.youtube_videos(video_id),
    language TEXT,
    source_kind TEXT NOT NULL,
    transcript_text TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    source_document_id BIGINT REFERENCES brain.source_documents(id) ON DELETE SET NULL,
    legacy_source_document_id BIGINT,
    imported_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_youtube_transcripts_source_document_idx
    ON brain.youtube_transcripts (source_document_id);

CREATE TABLE IF NOT EXISTS brain.youtube_learning_claims (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    video_id TEXT NOT NULL REFERENCES brain.youtube_videos(video_id),
    claim_hash TEXT NOT NULL UNIQUE,
    claim_index BIGINT NOT NULL,
    entity_type TEXT,
    entity_name TEXT,
    claim_type TEXT NOT NULL,
    claim_text TEXT NOT NULL,
    evidence_quote TEXT NOT NULL,
    timestamp_seconds DOUBLE PRECISION,
    model_confidence DOUBLE PRECISION NOT NULL,
    verifier_confidence DOUBLE PRECISION NOT NULL,
    status TEXT NOT NULL,
    model TEXT,
    prompt_version TEXT NOT NULL,
    prompt_text TEXT NOT NULL,
    model_response_text TEXT NOT NULL,
    provider_metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    verifier JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_youtube_learning_claims_video_idx
    ON brain.youtube_learning_claims (video_id, status);

CREATE INDEX IF NOT EXISTS brain_youtube_learning_claims_entity_idx
    ON brain.youtube_learning_claims (entity_type, entity_name);

CREATE TABLE IF NOT EXISTS brain.youtube_transcript_claim_attempts (
    video_id TEXT NOT NULL REFERENCES brain.youtube_videos(video_id),
    prompt_version TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'normal',
    status TEXT NOT NULL,
    claim_count BIGINT NOT NULL DEFAULT 0,
    char_len BIGINT NOT NULL DEFAULT 0,
    note TEXT,
    attempted_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (video_id, prompt_version)
);

CREATE INDEX IF NOT EXISTS brain_youtube_transcript_claim_attempts_prompt_status_idx
    ON brain.youtube_transcript_claim_attempts (prompt_version, status);
