CREATE EXTENSION IF NOT EXISTS vector;

CREATE SCHEMA IF NOT EXISTS knowledge;
REVOKE ALL ON SCHEMA knowledge FROM PUBLIC;

CREATE TABLE knowledge.index_generations (
    index_generation bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    model_fingerprint text NOT NULL CHECK (model_fingerprint ~ '^[a-f0-9]{64}$'),
    state text NOT NULL DEFAULT 'building' CHECK (state IN ('building', 'ready')),
    chunk_count bigint NOT NULL DEFAULT 0 CHECK (chunk_count >= 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    completed_at timestamptz,
    UNIQUE (index_generation, state),
    CHECK (state <> 'ready' OR (chunk_count > 0 AND completed_at IS NOT NULL))
);

CREATE TABLE knowledge.chunk_embeddings (
    index_generation bigint NOT NULL REFERENCES knowledge.index_generations (index_generation) ON DELETE CASCADE,
    chunk_id text NOT NULL CHECK (chunk_id ~ '^[a-f0-9]{64}$'),
    content_hash text NOT NULL CHECK (content_hash ~ '^[a-f0-9]{64}$'),
    embedding vector(384) NOT NULL CHECK (vector_norm(embedding) > 0),
    stand text NOT NULL CHECK (btrim(stand) <> ''),
    quelle text NOT NULL CHECK (btrim(quelle) <> ''),
    doc_path text NOT NULL CHECK (doc_path <> ''),
    title text NOT NULL,
    section text NOT NULL,
    chunk_text text NOT NULL CHECK (btrim(chunk_text) <> ''),
    PRIMARY KEY (index_generation, chunk_id)
);

CREATE INDEX knowledge_embeddings_content_hash_idx
    ON knowledge.chunk_embeddings (content_hash, index_generation);

CREATE TABLE knowledge.active_index (
    singleton boolean PRIMARY KEY CHECK (singleton),
    index_generation bigint,
    state text NOT NULL DEFAULT 'ready' CHECK (state = 'ready'),
    activated_at timestamptz,
    FOREIGN KEY (index_generation, state)
        REFERENCES knowledge.index_generations (index_generation, state)
);

INSERT INTO knowledge.active_index (singleton) VALUES (true);

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'dl_knowledge_dml') THEN
        BEGIN
            CREATE ROLE dl_knowledge_dml NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN
            NULL;
        END;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA knowledge TO dl_knowledge_dml;
GRANT SELECT, INSERT, UPDATE, DELETE
    ON knowledge.index_generations, knowledge.chunk_embeddings TO dl_knowledge_dml;
GRANT SELECT, UPDATE ON knowledge.active_index TO dl_knowledge_dml;
GRANT USAGE, SELECT ON SEQUENCE knowledge.index_generations_index_generation_seq
    TO dl_knowledge_dml;
