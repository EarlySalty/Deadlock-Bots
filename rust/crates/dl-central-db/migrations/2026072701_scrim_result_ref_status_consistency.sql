SET lock_timeout TO '2s';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM pg_constraint
         WHERE conrelid = 'scrim.match_result_refs'::regclass
           AND conname = 'match_result_refs_fetch_validation_status_check'
    ) THEN
        ALTER TABLE scrim.match_result_refs
            ADD CONSTRAINT match_result_refs_fetch_validation_status_check
            CHECK (
                (fetch_status, validation_status) IN (
                    ('pending', 'unvalidated'),
                    ('fetching', 'unvalidated'),
                    ('failed', 'unvalidated'),
                    ('fetched', 'ambiguous'),
                    ('fetched', 'valid'),
                    ('fetched', 'rejected'),
                    ('fetched', 'void'),
                    ('fetched', 'superseded')
                )
            ) NOT VALID;
    END IF;
END;
$$;

ALTER TABLE scrim.match_result_refs
    VALIDATE CONSTRAINT match_result_refs_fetch_validation_status_check;

RESET lock_timeout;
