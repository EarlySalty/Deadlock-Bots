ALTER TABLE community.team_applications
    DROP CONSTRAINT IF EXISTS team_applications_kind_check;

ALTER TABLE community.team_applications
    ADD CONSTRAINT team_applications_kind_check
    CHECK (kind IN ('moderation', 'coach', 'caster', 'turnier', 'coder', 'sonstiges', 'pate'));
