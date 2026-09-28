CREATE INDEX web_api_reservations_completed_window_idx
    ON steam.web_api_reservations (reserved_at)
    WHERE response_at IS NOT NULL;
