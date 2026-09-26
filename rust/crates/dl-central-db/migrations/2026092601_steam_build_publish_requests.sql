-- Persistent idempotency key for brain.build_publish.v1. A request and its
-- Steam task are committed in the same transaction by the Steam provider.
CREATE TABLE steam.build_publish_requests (
    request_id TEXT PRIMARY KEY,
    request_sha256 CHAR(64) NOT NULL,
    task_id BIGINT NOT NULL UNIQUE REFERENCES steam.steam_tasks(id) ON DELETE RESTRICT,
    submitted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT build_publish_request_id_length CHECK (length(request_id) BETWEEN 1 AND 128),
    CONSTRAINT build_publish_request_hash_format CHECK (request_sha256 ~ '^[0-9a-f]{64}$')
);
