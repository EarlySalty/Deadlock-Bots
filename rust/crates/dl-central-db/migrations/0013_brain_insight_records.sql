CREATE TABLE IF NOT EXISTS brain.insight_records (
    id BIGSERIAL PRIMARY KEY,
    insight_hash TEXT NOT NULL UNIQUE,
    insight_type TEXT NOT NULL,
    entity_type TEXT,
    entity_name TEXT,
    subject TEXT,
    summary TEXT NOT NULL,
    reason TEXT,
    validity_status TEXT NOT NULL,
    currentness TEXT NOT NULL,
    trust_tier TEXT NOT NULL,
    confidence DOUBLE PRECISION NOT NULL DEFAULT 0.0,
    occurred_at TIMESTAMPTZ,
    observed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    source_patch_event_ids BIGINT[] NOT NULL DEFAULT '{}'::bigint[],
    source_urls TEXT[] NOT NULL DEFAULT '{}'::text[],
    source_references JSONB NOT NULL DEFAULT '[]'::jsonb,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS brain_insight_records_entity_time_idx
    ON brain.insight_records (entity_type, entity_name, COALESCE(occurred_at, observed_at) DESC);

CREATE INDEX IF NOT EXISTS brain_insight_records_currentness_idx
    ON brain.insight_records (currentness, validity_status, trust_tier);

CREATE INDEX IF NOT EXISTS brain_insight_records_type_time_idx
    ON brain.insight_records (insight_type, COALESCE(occurred_at, observed_at) DESC);
