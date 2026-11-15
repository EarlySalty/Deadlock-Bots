SET lock_timeout TO '2s';
ALTER TABLE public.llm_usage
    ADD COLUMN IF NOT EXISTS error_reason TEXT,
    ADD COLUMN IF NOT EXISTS cache_read_tokens BIGINT,
    ADD COLUMN IF NOT EXISTS cache_write_tokens BIGINT;
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'llm_usage_error_reason_shape' AND conrelid = 'public.llm_usage'::regclass) THEN
        ALTER TABLE public.llm_usage ADD CONSTRAINT llm_usage_error_reason_shape CHECK (error_reason ~ '^[a-z0-9_]{1,64}$') NOT VALID;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'llm_usage_cache_tokens_non_negative' AND conrelid = 'public.llm_usage'::regclass) THEN
        ALTER TABLE public.llm_usage ADD CONSTRAINT llm_usage_cache_tokens_non_negative CHECK (cache_read_tokens >= 0 AND cache_write_tokens >= 0) NOT VALID;
    END IF;
END
$$;
COMMENT ON COLUMN public.llm_usage.error_reason IS 'Upstream refusal reason of a failed call, for example billing, http_429 or http_5xx; NULL for successful calls and for rows written before this column existed.';
COMMENT ON COLUMN public.llm_usage.cache_read_tokens IS 'Input tokens served from the provider prompt cache, a subset of tokens_in; NULL means the provider reported nothing, 0 means it reported zero.';
COMMENT ON COLUMN public.llm_usage.cache_write_tokens IS 'Input tokens written to the provider prompt cache, a subset of tokens_in; NULL means the provider reported nothing, 0 means it reported zero.';
RESET lock_timeout;
