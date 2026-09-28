CREATE TABLE steam.web_api_budget (
    id BOOLEAN PRIMARY KEY DEFAULT true CHECK (id),
    cooldown_until TIMESTAMPTZ,
    consecutive_429 INTEGER NOT NULL DEFAULT 0 CHECK (consecutive_429 >= 0),
    last_pruned_at TIMESTAMPTZ
);

INSERT INTO steam.web_api_budget (id) VALUES (true);

CREATE TABLE steam.web_api_reservations (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    caller TEXT NOT NULL CHECK (caller ~ '^[a-z][a-z0-9_-]{0,63}$'),
    caller_class TEXT NOT NULL CHECK (caller_class IN ('optional_patch', 'standard')),
    reserved_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    response_at TIMESTAMPTZ,
    http_status SMALLINT CHECK (http_status BETWEEN 100 AND 599),
    retry_after TEXT,
    CONSTRAINT web_api_report_requires_response CHECK (
        response_at IS NOT NULL OR (http_status IS NULL AND retry_after IS NULL)
    )
);

CREATE INDEX web_api_reservations_window_idx
    ON steam.web_api_reservations (reserved_at);

CREATE INDEX web_api_reservations_optional_window_idx
    ON steam.web_api_reservations (reserved_at)
    WHERE caller_class = 'optional_patch';
