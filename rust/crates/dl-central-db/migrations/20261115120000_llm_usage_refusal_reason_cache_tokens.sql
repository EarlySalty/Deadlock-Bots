SET lock_timeout TO '2s';
ALTER TABLE public.llm_usage
    ADD COLUMN IF NOT EXISTS error_reason TEXT,
    ADD COLUMN IF NOT EXISTS cache_read_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS cache_write_tokens BIGINT;
RESET lock_timeout;
