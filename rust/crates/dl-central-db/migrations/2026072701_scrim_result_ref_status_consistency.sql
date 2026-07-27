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

UPDATE scrim.match_result_refs
   SET fetch_status = CASE
           WHEN validation_status <> 'unvalidated' THEN 'fetched'
           ELSE fetch_status
       END,
       validation_status = CASE
           WHEN fetch_status = 'fetched' AND validation_status = 'unvalidated'
               THEN CASE WHEN winner_team_id IS NULL THEN 'ambiguous' ELSE 'valid' END
           ELSE validation_status
       END,
       updated_at = now()
 WHERE (fetch_status = 'fetched' AND validation_status = 'unvalidated')
    OR (fetch_status <> 'fetched' AND validation_status <> 'unvalidated');

ALTER TABLE scrim.match_result_refs
    VALIDATE CONSTRAINT match_result_refs_fetch_validation_status_check;

RESET lock_timeout;
