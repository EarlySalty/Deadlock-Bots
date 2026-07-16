-- Scrim-Aushilfen: befristete Bank-Mitgliedschaft mit Ablauf.
-- Rein additiv: bestehende Spalten und die compile-checked query!-Makros des
-- Bots (dl-squads / dl-community) bleiben unberuehrt. Kein .sqlx-Cache-Rebuild noetig.

-- Ein Auswechselspieler, der fuer eine Session in einem Team aushilft, bekommt einen
-- team_members-Eintrag (is_bench = TRUE) und damit die Discord-Team-Rolle — sein
-- participants.status bleibt aber 'reserve', er verlaesst die Auswechselbank nicht.
--
-- substitute_until = Zeitpunkt, ab dem diese Aushilfe abgelaufen ist. Der Website-Backend-
-- Worker raeumt abgelaufene Eintraege weg und entzieht die Team-Rolle wieder.
--   NULL  = feste Mitgliedschaft, laeuft nie ab (alle Bestandszeilen, daher kein Backfill noetig).
--   Wert  = befristete Aushilfe (aktuell: Bestaetigungszeit + 24h).
ALTER TABLE scrim.team_members
    ADD COLUMN IF NOT EXISTS substitute_until TIMESTAMPTZ;

-- Der Aufraeum-Worker fragt ausschliesslich nach faelligen Aushilfen. Partiell, weil
-- die grosse Mehrheit der Zeilen feste Mitgliedschaften (NULL) sind und nie gesucht werden.
CREATE INDEX IF NOT EXISTS team_members_substitute_until_idx
    ON scrim.team_members (substitute_until)
    WHERE substitute_until IS NOT NULL;
