-- Ein abweichender Producer-Schlüssel für einen schon eingereichten Clip
-- bleibt auch nach Wochenwechsel ein Duplikat. Die Zuordnung enthält keine
-- Discord-ID und verhindert, dass ein verlorener HTTP-Erfolg neu einreicht.
CREATE TABLE clips.clip_submission_replays (
    idempotency_key TEXT PRIMARY KEY,
    clip_key TEXT NOT NULL,
    submission_id BIGINT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

DO $roles$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'deadlock') THEN
        GRANT SELECT, INSERT ON clips.clip_submission_replays TO deadlock;
    END IF;
END
$roles$;
