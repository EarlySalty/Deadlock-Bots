-- Scrim-Teams: uebliche Spielzeit ("Stammzeit").
-- Rein additiv: bestehende Spalten und die compile-checked query!-Makros des
-- Bots (dl-squads / dl-community) bleiben unberuehrt. Kein .sqlx-Cache-Rebuild noetig.

-- Jedes Team hat eine feste Uhrzeit, an der es ueblicherweise spielt ("Team 2 und 4 spielen
-- um 20 Uhr immer", "Team 3 ab 16 Uhr"). Bisher lebte die nur im Kopf des Coaches und wurde
-- in jeden Such-Aufruf neu getippt.
--
-- BEWUSST nur Uhrzeit, KEIN Wochentag: In den 742 Nachrichten des Scrim-Kanals nennt niemand
-- feste Spieltage — die Uhrzeit liegt fest, der Tag wird pro Woche ausgehandelt. Ein Modell mit
-- Wochentagen wuerde eine Regelmaessigkeit erfinden, die es nicht gibt.
--
-- Minuten seit Mitternacht (0..1440), gleiche Einheit wie scrim.participants.availability_slots.
-- NULL = keine Stammzeit hinterlegt; dann bleibt es beim bisherigen Verhalten (Coach tippt die Zeit).
ALTER TABLE scrim.teams
    ADD COLUMN IF NOT EXISTS default_from INTEGER,
    ADD COLUMN IF NOT EXISTS default_to INTEGER;

-- Unsinnige Zeiten gar nicht erst zulassen: Start vor Ende, beide im Tagesraster.
-- NOT VALID: gilt fuer neue/geaenderte Zeilen, prueft den Bestand nicht nach (der ist NULL).
ALTER TABLE scrim.teams
    DROP CONSTRAINT IF EXISTS teams_default_window_sane;
ALTER TABLE scrim.teams
    ADD CONSTRAINT teams_default_window_sane CHECK (
        (default_from IS NULL AND default_to IS NULL)
        OR (default_from >= 0 AND default_to <= 1440 AND default_from < default_to)
    ) NOT VALID;
