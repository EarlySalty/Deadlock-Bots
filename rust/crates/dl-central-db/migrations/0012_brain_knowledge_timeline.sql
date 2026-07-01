CREATE SCHEMA IF NOT EXISTS brain;

CREATE TABLE IF NOT EXISTS brain.source_runs (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    source TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL,
    finished_at TIMESTAMPTZ,
    summary JSONB NOT NULL DEFAULT '{}'::jsonb,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS brain_source_runs_source_started_idx
    ON brain.source_runs (source, started_at DESC);

CREATE TABLE IF NOT EXISTS brain.source_documents (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    source TEXT NOT NULL,
    external_id TEXT NOT NULL,
    title TEXT,
    url TEXT,
    content_type TEXT NOT NULL,
    raw_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    fetched_at TIMESTAMPTZ NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source, external_id, content_hash)
);

CREATE INDEX IF NOT EXISTS brain_source_documents_source_external_idx
    ON brain.source_documents (source, external_id);

CREATE INDEX IF NOT EXISTS brain_source_documents_fetched_idx
    ON brain.source_documents (fetched_at DESC);

CREATE TABLE IF NOT EXISTS brain.entity_snapshots (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    source TEXT NOT NULL,
    entity_type TEXT NOT NULL,
    external_id TEXT NOT NULL,
    canonical_name TEXT,
    payload_hash TEXT NOT NULL,
    payload JSONB NOT NULL,
    fetched_at TIMESTAMPTZ NOT NULL,
    source_document_id BIGINT REFERENCES brain.source_documents(id) ON DELETE SET NULL,
    legacy_source_document_id BIGINT,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (source, entity_type, external_id, payload_hash)
);

CREATE INDEX IF NOT EXISTS brain_entity_snapshots_entity_idx
    ON brain.entity_snapshots (entity_type, canonical_name);

CREATE INDEX IF NOT EXISTS brain_entity_snapshots_source_idx
    ON brain.entity_snapshots (source, entity_type, external_id);

CREATE TABLE IF NOT EXISTS brain.entities (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    entity_type TEXT NOT NULL,
    canonical_name TEXT NOT NULL,
    primary_external_id TEXT,
    source TEXT NOT NULL,
    first_snapshot_id BIGINT REFERENCES brain.entity_snapshots(id) ON DELETE SET NULL,
    legacy_first_snapshot_id BIGINT,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (entity_type, canonical_name)
);

CREATE INDEX IF NOT EXISTS brain_entities_type_name_idx
    ON brain.entities (entity_type, canonical_name);

CREATE TABLE IF NOT EXISTS brain.entity_aliases (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    entity_id BIGINT REFERENCES brain.entities(id) ON DELETE CASCADE,
    legacy_entity_id BIGINT,
    alias TEXT NOT NULL,
    alias_norm TEXT NOT NULL,
    alias_kind TEXT NOT NULL,
    source TEXT NOT NULL,
    external_id TEXT,
    snapshot_id BIGINT REFERENCES brain.entity_snapshots(id) ON DELETE SET NULL,
    legacy_snapshot_id BIGINT,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (entity_id, alias_norm, alias_kind)
);

CREATE INDEX IF NOT EXISTS brain_entity_aliases_norm_idx
    ON brain.entity_aliases (alias_norm);

CREATE TABLE IF NOT EXISTS brain.patch_events (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    patch_snapshot_id BIGINT REFERENCES brain.entity_snapshots(id) ON DELETE SET NULL,
    legacy_patch_snapshot_id BIGINT NOT NULL,
    patch_external_id TEXT NOT NULL,
    patch_title TEXT,
    patch_url TEXT,
    source_kind TEXT NOT NULL,
    posted_at TIMESTAMPTZ,
    line_index BIGINT NOT NULL,
    section TEXT,
    entity_type TEXT NOT NULL,
    entity_name TEXT,
    subject TEXT,
    change_type TEXT NOT NULL,
    raw_line TEXT NOT NULL,
    normalized_line TEXT NOT NULL,
    old_value TEXT,
    new_value TEXT,
    confidence DOUBLE PRECISION NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    event_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS brain_patch_events_entity_idx
    ON brain.patch_events (entity_type, entity_name, posted_at DESC);

CREATE INDEX IF NOT EXISTS brain_patch_events_patch_idx
    ON brain.patch_events (patch_external_id, line_index);

CREATE INDEX IF NOT EXISTS brain_patch_events_source_kind_idx
    ON brain.patch_events (source_kind);

CREATE TABLE IF NOT EXISTS brain.patch_event_enrichments (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    patch_event_id BIGINT REFERENCES brain.patch_events(id) ON DELETE CASCADE,
    legacy_patch_event_id BIGINT NOT NULL,
    stat_name TEXT,
    old_value TEXT,
    new_value TEXT,
    unit TEXT,
    ability_name TEXT,
    secondary_entity_name TEXT,
    confidence DOUBLE PRECISION NOT NULL,
    flags JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (patch_event_id)
);

CREATE INDEX IF NOT EXISTS brain_patch_event_enrichments_stat_idx
    ON brain.patch_event_enrichments (stat_name);

CREATE TABLE IF NOT EXISTS brain.entity_lineage (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    patch_event_id BIGINT REFERENCES brain.patch_events(id) ON DELETE CASCADE,
    legacy_patch_event_id BIGINT NOT NULL,
    relation_type TEXT NOT NULL,
    source_entity_type TEXT,
    source_name TEXT NOT NULL,
    source_name_norm TEXT NOT NULL,
    target_entity_type TEXT,
    target_name TEXT,
    target_name_norm TEXT,
    owner_entity_type TEXT,
    owner_name TEXT,
    owner_name_norm TEXT,
    confidence DOUBLE PRECISION NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS brain_entity_lineage_source_idx
    ON brain.entity_lineage (source_name_norm);

CREATE INDEX IF NOT EXISTS brain_entity_lineage_target_idx
    ON brain.entity_lineage (target_name_norm);

CREATE UNIQUE INDEX IF NOT EXISTS brain_entity_lineage_unique_idx
    ON brain.entity_lineage (
        patch_event_id,
        relation_type,
        source_name_norm,
        COALESCE(target_name_norm, ''),
        COALESCE(owner_name_norm, '')
    );

CREATE TABLE IF NOT EXISTS brain.legacy_entities (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    legacy_type TEXT NOT NULL,
    canonical_name TEXT NOT NULL,
    name_norm TEXT NOT NULL,
    observed_entity_type TEXT NOT NULL,
    first_patch_event_id BIGINT REFERENCES brain.patch_events(id) ON DELETE SET NULL,
    last_patch_event_id BIGINT REFERENCES brain.patch_events(id) ON DELETE SET NULL,
    legacy_first_patch_event_id BIGINT,
    legacy_last_patch_event_id BIGINT,
    first_seen_at TIMESTAMPTZ,
    last_seen_at TIMESTAMPTZ,
    event_count BIGINT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL,
    status TEXT NOT NULL,
    samples JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    UNIQUE (legacy_type, name_norm)
);

CREATE INDEX IF NOT EXISTS brain_legacy_entities_name_idx
    ON brain.legacy_entities (name_norm);

CREATE INDEX IF NOT EXISTS brain_legacy_entities_type_idx
    ON brain.legacy_entities (legacy_type, status);

CREATE TABLE IF NOT EXISTS brain.forum_claims (
    id BIGSERIAL PRIMARY KEY,
    legacy_sqlite_id BIGINT UNIQUE,
    post_snapshot_id BIGINT REFERENCES brain.entity_snapshots(id) ON DELETE SET NULL,
    legacy_post_snapshot_id BIGINT NOT NULL,
    thread_id TEXT NOT NULL,
    post_id TEXT NOT NULL,
    thread_title TEXT,
    source_url TEXT NOT NULL,
    posted_at TIMESTAMPTZ,
    author TEXT,
    author_role TEXT,
    claim_hash TEXT NOT NULL UNIQUE,
    claim_index BIGINT NOT NULL,
    claim_type TEXT NOT NULL,
    entity_type TEXT,
    entity_name TEXT,
    claim_text TEXT NOT NULL,
    evidence_quote TEXT NOT NULL,
    source_trust TEXT NOT NULL,
    validity_status TEXT NOT NULL,
    currentness TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL,
    safety_labels JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_references JSONB NOT NULL DEFAULT '[]'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    imported_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS brain_forum_claims_entity_idx
    ON brain.forum_claims (entity_type, entity_name);

CREATE INDEX IF NOT EXISTS brain_forum_claims_currentness_idx
    ON brain.forum_claims (currentness, validity_status);

CREATE INDEX IF NOT EXISTS brain_forum_claims_posted_idx
    ON brain.forum_claims (posted_at DESC);

CREATE TABLE IF NOT EXISTS brain.knowledge_events (
    id BIGSERIAL PRIMARY KEY,
    event_hash TEXT NOT NULL UNIQUE,
    event_source TEXT NOT NULL,
    source_table TEXT NOT NULL,
    source_legacy_id BIGINT,
    source_document_id BIGINT REFERENCES brain.source_documents(id) ON DELETE SET NULL,
    snapshot_id BIGINT REFERENCES brain.entity_snapshots(id) ON DELETE SET NULL,
    patch_event_id BIGINT REFERENCES brain.patch_events(id) ON DELETE SET NULL,
    forum_claim_id BIGINT REFERENCES brain.forum_claims(id) ON DELETE SET NULL,
    entity_type TEXT,
    entity_name TEXT,
    subject TEXT,
    event_type TEXT NOT NULL,
    validity_status TEXT NOT NULL,
    currentness TEXT NOT NULL,
    trust_tier TEXT NOT NULL,
    source_url TEXT,
    occurred_at TIMESTAMPTZ,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    effective_from TIMESTAMPTZ,
    effective_to TIMESTAMPTZ,
    raw_text TEXT,
    normalized_text TEXT,
    evidence_quote TEXT,
    old_value TEXT,
    new_value TEXT,
    confidence DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    safety_labels JSONB NOT NULL DEFAULT '[]'::jsonb,
    source_references JSONB NOT NULL DEFAULT '[]'::jsonb,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS brain_knowledge_events_source_legacy_idx
    ON brain.knowledge_events (event_source, source_table, source_legacy_id)
    WHERE source_legacy_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS brain_knowledge_events_entity_time_idx
    ON brain.knowledge_events (entity_type, entity_name, COALESCE(occurred_at, observed_at) DESC);

CREATE INDEX IF NOT EXISTS brain_knowledge_events_currentness_idx
    ON brain.knowledge_events (currentness, validity_status, trust_tier);

CREATE INDEX IF NOT EXISTS brain_knowledge_events_type_time_idx
    ON brain.knowledge_events (event_type, COALESCE(occurred_at, observed_at) DESC);

CREATE TABLE IF NOT EXISTS brain.current_entity_state (
    id BIGSERIAL PRIMARY KEY,
    entity_type TEXT NOT NULL,
    entity_name TEXT NOT NULL,
    state_kind TEXT NOT NULL,
    winning_event_id BIGINT REFERENCES brain.knowledge_events(id) ON DELETE SET NULL,
    winning_snapshot_id BIGINT REFERENCES brain.entity_snapshots(id) ON DELETE SET NULL,
    source TEXT NOT NULL,
    source_priority INTEGER NOT NULL,
    valid_from TIMESTAMPTZ,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    content_hash TEXT NOT NULL,
    payload JSONB NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (entity_type, entity_name, state_kind)
);

CREATE INDEX IF NOT EXISTS brain_current_entity_state_source_idx
    ON brain.current_entity_state (source, source_priority DESC);

CREATE INDEX IF NOT EXISTS brain_current_entity_state_observed_idx
    ON brain.current_entity_state (observed_at DESC);
