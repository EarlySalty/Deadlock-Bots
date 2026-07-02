-- Scrim-Management: strukturierte Wochen-Verfuegbarkeit + Coach-Notizen.
-- Rein additiv: bestehende Spalten und die compile-checked query!-Makros des
-- Bots (dl-squads / dl-community) bleiben unberuehrt. Kein .sqlx-Cache-Rebuild noetig.

-- Strukturierte Wochen-Verfuegbarkeit (Self-Service gepflegt).
-- Form: {"mon":{"status":"available","from":1140,"to":1200}, "tue":{"status":"available"}, ...}
--   status in ('available','unavailable','unknown'); from/to = Minuten seit Mitternacht (0..1440), optional.
-- NULL = noch keine strukturierten Daten -> Backend faellt auf den Legacy-Freitext (availability) zurueck.
ALTER TABLE scrim.participants
    ADD COLUMN IF NOT EXISTS availability_slots JSONB;

-- Freie Coach-Notiz zum Spieler (nur im Coach-Cockpit sichtbar).
ALTER TABLE scrim.participants
    ADD COLUMN IF NOT EXISTS notes TEXT;
