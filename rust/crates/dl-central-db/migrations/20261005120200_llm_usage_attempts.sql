CREATE TABLE public.llm_usage (
    id BIGSERIAL PRIMARY KEY,
    ts TEXT NOT NULL,
    source TEXT NOT NULL,
    purpose TEXT NOT NULL,
    model TEXT NOT NULL,
    tokens_in BIGINT,
    tokens_out BIGINT,
    total BIGINT,
    success BIGINT NOT NULL DEFAULT 0,
    meta TEXT,
    project TEXT NOT NULL,
    service TEXT NOT NULL,
    provider TEXT NOT NULL,
    attempt_state TEXT NOT NULL CHECK (attempt_state IN ('legacy','started','succeeded','failed')),
    request_id TEXT,
    finished_at TIMESTAMPTZ,
    error_code TEXT,
    http_status INTEGER,
    latency_ms BIGINT
);
CREATE INDEX llm_usage_ts ON public.llm_usage(ts);
CREATE INDEX llm_usage_project_service ON public.llm_usage(project, service);
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='deadlock') THEN
        GRANT SELECT, INSERT, UPDATE ON public.llm_usage TO deadlock;
        GRANT USAGE, SELECT ON SEQUENCE public.llm_usage_id_seq TO deadlock;
    END IF;
END $$;
