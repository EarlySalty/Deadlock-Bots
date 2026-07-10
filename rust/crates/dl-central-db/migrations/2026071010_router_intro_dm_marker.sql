-- Router-Onboarding: markiert, welchen Usern schon einmal die Erst-DM
-- geschickt wurde (Erst-Join in den Router ohne gespeicherten Standard).
-- Existenz der Zeile = DM wurde versucht; es gibt keinen zweiten Versuch.
CREATE TABLE IF NOT EXISTS voice.router_intro_dm (
    user_id BIGINT PRIMARY KEY,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
