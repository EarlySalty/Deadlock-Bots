CREATE TABLE scrim.slot_presets (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name TEXT NOT NULL CHECK (btrim(name) <> '' AND char_length(name) <= 100),
    slots JSONB NOT NULL CHECK (
        jsonb_typeof(slots) = 'array'
        AND jsonb_array_length(slots) BETWEEN 2 AND 5
    ),
    created_by_user_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO scrim.slot_presets(name, slots, created_by_user_id)
VALUES (
    'Wochenende abends',
    '[
        {"day": "sat", "from": 1200, "to": 1320},
        {"day": "sun", "from": 1200, "to": 1320}
    ]'::jsonb,
    'system'
);
