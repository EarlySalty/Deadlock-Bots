-- Dauerhafter Wiederholungsschutz, wenn ein Discord-Schritt im Patenablauf
-- möglicherweise erfolgt ist, aber keine sichere Bestätigung liefert.
ALTER TABLE bot.concierge_profiles
    ADD COLUMN IF NOT EXISTS pate_request_uncertain BOOLEAN NOT NULL DEFAULT FALSE;
