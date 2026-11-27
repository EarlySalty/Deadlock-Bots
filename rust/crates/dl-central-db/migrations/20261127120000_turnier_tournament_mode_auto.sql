SET LOCAL lock_timeout TO '5s';

ALTER TABLE turnier.tournaments
    ADD COLUMN IF NOT EXISTS tournament_mode_auto BOOLEAN NOT NULL DEFAULT false;

COMMENT ON COLUMN turnier.tournaments.tournament_mode_auto IS
    'true when the admin chose "Automatisch": tournament_mode holds the provisional bracket_only until finalize-checkin confirms the teams, then the final team count decides (16 or more teams: group_stage, fewer: bracket_only). An explicit mode sets it to false and is never overwritten. Rows before 20261127120000 default to false and keep their stored mode.';
