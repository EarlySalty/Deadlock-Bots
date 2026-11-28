SET LOCAL lock_timeout TO '5s';

ALTER TABLE turnier.tournaments
    ADD COLUMN IF NOT EXISTS bench_slots BIGINT NOT NULL DEFAULT 0;

ALTER TABLE turnier.tournaments
    DROP CONSTRAINT IF EXISTS tournaments_bench_slots_range;

ALTER TABLE turnier.tournaments
    ADD CONSTRAINT tournaments_bench_slots_range CHECK (bench_slots BETWEEN 0 AND 5);

ALTER TABLE turnier.team_members
    ADD COLUMN IF NOT EXISTS is_bench BOOLEAN NOT NULL DEFAULT false;

ALTER TABLE turnier.team_applications
    ADD COLUMN IF NOT EXISTS as_bench BOOLEAN NOT NULL DEFAULT false;

COMMENT ON COLUMN turnier.tournaments.bench_slots IS
    'Maximum bench (substitute) players per team on top of team_size starters. 0 means bench is off; rows before 20261128120000 default to 0 and keep their behaviour.';

COMMENT ON COLUMN turnier.team_members.is_bench IS
    'true for a bench (substitute) member. Starters are is_bench = false and alone count against team_size for completeness and recruitment.';

COMMENT ON COLUMN turnier.team_applications.as_bench IS
    'true when the applicant asked for a bench seat; accepting such an application places the player on the bench or fails when no bench seat is free.';
