-- Die bereits angewendete Migration 2026071602 liess wegen SQL-NULL-Logik
-- halb gesetzte Stammzeiten durch. Historische Migrationen bleiben unveraendert;
-- diese Folgemigration bereinigt moegliche Halbwerte und schaerft den Vertrag.
UPDATE scrim.teams
SET default_from = NULL,
    default_to = NULL
WHERE (default_from IS NULL) <> (default_to IS NULL);

ALTER TABLE scrim.teams
    DROP CONSTRAINT IF EXISTS teams_default_window_sane;

ALTER TABLE scrim.teams
    ADD CONSTRAINT teams_default_window_sane CHECK (
        (default_from IS NULL AND default_to IS NULL)
        OR (
            default_from IS NOT NULL
            AND default_to IS NOT NULL
            AND default_from >= 0
            AND default_to <= 1440
            AND default_from < default_to
        )
    );
