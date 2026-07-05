ALTER TABLE voice.tempvoice_presets
    ADD COLUMN IF NOT EXISTS mode TEXT NOT NULL DEFAULT 'casual'
        CHECK (mode IN ('casual', 'ranked', 'street_brawl'));

CREATE INDEX IF NOT EXISTS tempvoice_presets_standard_idx
    ON voice.tempvoice_presets (user_id)
    WHERE category_id = 0 AND name = 'standard';
