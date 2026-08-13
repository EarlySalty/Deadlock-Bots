-- Mitspieler-Bewertungen aus der Voice-Umfrage ("Wuerdest du wieder mit XY
-- spielen?"). Zweck ist das Matching: der Router und der Pairing-Vorschlag
-- sollen Leute zusammenbringen, die schon gut miteinander konnten, und Paare
-- meiden, die es ausdruecklich nicht wollen.
--
-- Bewusst getrennt von activity.user_co_players: dort steht, WIE LANGE zwei
-- zusammen im Voice waren (Verhaltensdaten, automatisch erhoben). Hier steht,
-- WIE es war (eine bewusste Aussage einer Person ueber eine andere). Beides in
-- einer Tabelle zu fuehren wuerde das Loeschen und das Auswerten vermischen:
-- Verhaltensdaten verfallen mit der Retention, eine Aussage nicht.
--
-- rater_user_id ist der Fragende, mate_user_id der Bewertete. Die Zeile ist damit
-- personenbezogen fuer ZWEI Menschen; der Privacy-Erase in
-- dl-community/src/privacy.rs loescht deshalb ueber beide Spalten. Die Endung
-- _user_id ist Absicht: der Privacy-Vertragstest erkennt user-id-artige
-- Spalten am Namen und erzwingt genau diese Abdeckung.
--
-- Kein UNIQUE ueber (rater_user_id, mate_user_id): dieselbe Paarung darf spaeter erneut
-- bewertet werden, die Historie ist das Interessante. Die Frequenz begrenzt
-- der Bot ueber Cooldowns in bot.kv_store, nicht die Datenbank.
--
-- rating ist bewusst TEXT mit CHECK statt ENUM: eine vierte Stufe soll man
-- ohne Typ-Migration nachziehen koennen.

CREATE TABLE IF NOT EXISTS activity.voice_mate_ratings (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    rater_user_id BIGINT NOT NULL,
    mate_user_id BIGINT NOT NULL,
    rating TEXT NOT NULL CHECK (rating IN ('again', 'ok', 'rather_not')),
    comment TEXT,
    channel_id BIGINT,
    session_seconds BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (rater_user_id <> mate_user_id)
);

-- Lesepfad des Matchings: "wie hat A ueber B geurteilt, zuletzt?"
CREATE INDEX IF NOT EXISTS voice_mate_ratings_pair_idx
    ON activity.voice_mate_ratings (rater_user_id, mate_user_id, created_at DESC);

-- Gegenrichtung fuer den Privacy-Erase und fuer "wie kommt B bei anderen an".
CREATE INDEX IF NOT EXISTS voice_mate_ratings_mate_idx
    ON activity.voice_mate_ratings (mate_user_id, created_at DESC);
