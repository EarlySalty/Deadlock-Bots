SET lock_timeout TO '2s';

COMMENT ON TABLE scrim.match_result_refs IS
    'Additive foundation deliberately does not build new non-concurrent indexes on this live table. After cutover, review partial one-selected-valid and validation lookup indexes.';

CREATE INDEX IF NOT EXISTS match_result_clarifications_ref_idx
    ON scrim.match_result_clarifications (result_ref_id, status, created_at DESC, id DESC);

RESET lock_timeout;
