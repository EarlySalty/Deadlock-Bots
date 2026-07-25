ALTER TABLE scrim.match_request_reminders
    DROP CONSTRAINT IF EXISTS match_request_reminders_template_check;

ALTER TABLE scrim.match_request_reminders
    ALTER COLUMN template SET DEFAULT 'antwort_fehlt',
    ADD CONSTRAINT match_request_reminders_template_check
        CHECK (template IN ('antwort_fehlt', 'frist_bald', 'bestaetigung_offen'));
