CREATE TABLE brain.application_catalog (
    application_id TEXT NOT NULL,
    entity_type TEXT NOT NULL CHECK (entity_type IN ('hero', 'item', 'ability')),
    external_id BIGINT NOT NULL,
    snapshot_id BIGINT NOT NULL REFERENCES brain.entity_snapshots(id),
    PRIMARY KEY (application_id, entity_type, external_id)
);

CREATE TABLE brain.application_emojis (
    application_id TEXT NOT NULL,
    name TEXT NOT NULL,
    emoji_id TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (application_id, name),
    UNIQUE (application_id, emoji_id)
);
