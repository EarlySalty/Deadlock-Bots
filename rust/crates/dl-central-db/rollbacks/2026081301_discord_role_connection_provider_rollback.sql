-- Rueckweg fuer Migration 2026081301 (Linked-Role-Provider-Dimension).
--
-- Liegt bewusst NICHT unter migrations/: sqlx wuerde die Datei sonst als
-- Migration anwenden. Sie wird von Hand gefahren und von
-- tests/fresh_migrations_schema.rs mit ausgefuehrt — ein Rueckweg, der nie
-- laeuft, ist nicht getestet, sondern geraten. Er ist mehrfach ausfuehrbar.
--
-- WANN: nur wenn auf ein Backend-Binary ohne Provider-Unterstuetzung
-- zurueckgerollt werden muss. Vorwaerts ist immer der bessere Weg.
--
-- DATENVERLUST: alle Creator-OAuth-Tokens sind danach aus der Live-Tabelle weg.
-- Jeder betroffene Nutzer muss die Creator-App neu autorisieren. Die Kopien
-- liegen in core.*_rollback_backup; sie stehen in der Loeschregistratur
-- (dl-community/src/privacy.rs), gehoeren nach dem Vorfall aber gedroppt —
-- siehe letzter Abschnitt.
--
-- DIE ZEILE IN _sqlx_migrations BLEIBT STEHEN, und das ist der Kern dieses
-- Rueckwegs. Wuerde sie geloescht, wendet der naechste `dl-central-migrate`
-- (laeuft bei jedem Deploy dieses Repos) die Migration erneut an: der PK-Block
-- sieht wieder nur (discord_id), tauscht auf (discord_id, provider), und das
-- zurueckgerollte alte Binary ist ohne einen einzigen Logeintrag erneut tot.
-- Mit stehender Zeile bleibt die Datenbank stabil zurueckgerollt.
--
-- Drei Dinge muessen zurueck, nicht nur der Schluessel:
--   1. Der Default auf provider — sonst scheitert das alte Binary an 23502
--      (es nennt die Spalte in keinem INSERT) statt an 42P10. Genauso tot.
--   2. Ein Unique ueber (discord_id, provider) muss BLEIBEN. Die von 2026081301
--      installierte Trigger-Funktion nutzt ON CONFLICT (discord_id, provider),
--      und Postgres verlangt dafuer einen Index ueber exakt diese Spalten. Ohne
--      ihn wirft jeder Schreibvorgang auf core.steam_links 42P10, und der
--      Trigger laeuft in derselben Transaktion: Steam-Verknuepfung komplett tot,
--      schlimmer als der Zustand, aus dem man rollt.
--   3. Der Primaerschluessel auf (discord_id) — den braucht das alte Binary fuer
--      sein ON CONFLICT (discord_id).
--
-- NICHT zurueckgebaut, weil harmlos und fuer den Vorwaertsweg gebraucht: der
-- CHECK auf ('steam','creator') und die beiden Provider-Indizes bleiben stehen.
-- Das Unique heisst absichtlich *_discord_provider_key; die Migration droppt
-- genau diesen Namen, damit nach einem spaeteren Vorwaertsschritt keine zwei
-- identischen Unique-Indizes auf (discord_id, provider) uebrig bleiben.
--
-- Constraint-Namen werden gelesen, nicht geraten.

BEGIN;

-- Struktur und Inhalt getrennt: `CREATE TABLE IF NOT EXISTS … AS SELECT` wuerde
-- beim zweiten Lauf still nichts sichern (die Tabelle existiert schon), das
-- DELETE unten aber trotzdem loeschen. Wer zwischen zwei Rueckwegen wieder
-- vorwaerts migriert hat, verliert dann neue Creator-Tokens ohne Kopie.
CREATE TABLE IF NOT EXISTS core.discord_role_connection_tokens_rollback_backup
    (LIKE core.discord_role_connection_tokens);
CREATE TABLE IF NOT EXISTS core.discord_role_connection_sync_state_rollback_backup
    (LIKE core.discord_role_connection_sync_state);

-- Aufbewahrungsfrist wie bei server_config.rollback_exports (2026070240): die
-- Kopien tragen verschluesselte OAuth-Tokens und duerfen nicht unbefristet neben
-- der Live-Tabelle liegen. Der Retention-Job in dl-community/src/privacy.rs
-- raeumt sie ab; `ADD COLUMN IF NOT EXISTS`, damit ein zweiter Lauf durchlaeuft.
--
-- Der Name ist bewusst `rollback_expires_at` und nicht `expires_at`: die
-- Tokens-Tabelle hat schon ein `expires_at`, und das ist der OAuth-Ablauf des
-- Tokens (Tage, bei invalidierten Tokens Vergangenheit). Waere die Frist an
-- diesen Namen gehaengt, haette `ADD COLUMN IF NOT EXISTS` still nichts getan
-- und der Retention-Job die einzige Kopie beim naechsten Tick geloescht — das
-- Netz gegen Datenverlust haette sich selbst aufgeloest.
ALTER TABLE core.discord_role_connection_tokens_rollback_backup
    ADD COLUMN IF NOT EXISTS rollback_expires_at TIMESTAMPTZ NOT NULL
    DEFAULT (now() + INTERVAL '180 days');
ALTER TABLE core.discord_role_connection_sync_state_rollback_backup
    ADD COLUMN IF NOT EXISTS rollback_expires_at TIMESTAMPTZ NOT NULL
    DEFAULT (now() + INTERVAL '180 days');

-- Index auf die Frist wie bei server_config.rollback_exports: der Retention-Job
-- fragt genau diese Spalte ab. Bei ein paar hundert Zeilen egal, aber die
-- Begruendung "wie bei rollback_exports" soll ganz stimmen und nicht halb.
CREATE INDEX IF NOT EXISTS discord_role_connection_tokens_rollback_expires_idx
    ON core.discord_role_connection_tokens_rollback_backup (rollback_expires_at);
CREATE INDEX IF NOT EXISTS discord_role_connection_sync_rollback_expires_idx
    ON core.discord_role_connection_sync_state_rollback_backup (rollback_expires_at);

-- Spaltenliste aus dem Katalog, und zwar als Schnittmenge von Quelle und Kopie:
-- rollback_expires_at wird so per DEFAULT gefuellt, und eine Kopie aus einem
-- frueheren Lauf, der eine inzwischen dazugekommene Spalte fehlt, bricht nicht
-- an 42703 (was die ganze Transaktion mitnehmen wuerde). Die fehlende Spalte
-- fehlt dann in der Sicherung — sichtbar, aber nicht toedlich.
DO $sicherung$
DECLARE
    tabelle TEXT;
    spalten TEXT;
BEGIN
    FOREACH tabelle IN ARRAY ARRAY[
        'discord_role_connection_tokens',
        'discord_role_connection_sync_state'
    ] LOOP
        SELECT string_agg(quote_ident(quelle.attname), ', ' ORDER BY quelle.attnum)
          INTO spalten
          FROM pg_attribute quelle
         WHERE quelle.attrelid = ('core.' || tabelle)::regclass
           AND quelle.attnum > 0
           AND NOT quelle.attisdropped
           AND EXISTS (
               SELECT 1 FROM pg_attribute kopie
                WHERE kopie.attrelid = ('core.' || tabelle || '_rollback_backup')::regclass
                  AND kopie.attname = quelle.attname
                  AND kopie.attnum > 0
                  AND NOT kopie.attisdropped
           );

        -- IS DISTINCT FROM statt <>: auf einer handgepatchten nullable Spalte
        -- (den Vorzustand behandelt fresh_migrations_schema.rs als real) waere
        -- eine Creator-Zeile mit provider IS NULL sonst weder gesichert noch
        -- geloescht — sie ueberlebt und wird vom alten Binary per
        -- ON CONFLICT (discord_id) DO UPDATE mit Steam-Daten ueberschrieben.
        EXECUTE format(
            'INSERT INTO core.%I (%s) SELECT %s FROM core.%I '
            'WHERE provider IS DISTINCT FROM ''steam''',
            tabelle || '_rollback_backup', spalten, spalten, tabelle
        );
    END LOOP;
END
$sicherung$;

DO $rollback$
DECLARE
    tabelle TEXT;
    pk_name TEXT;
    pk_spalten TEXT[];
    unique_name TEXT;
BEGIN
    FOREACH tabelle IN ARRAY ARRAY[
        'discord_role_connection_tokens',
        'discord_role_connection_sync_state'
    ] LOOP
        unique_name := tabelle || '_discord_provider_key';

        -- Der Unique-Ersatz nur dort, wo er gebraucht wird: die Trigger-Funktion
        -- schreibt ausschliesslich sync_state mit
        -- ON CONFLICT (discord_id, provider). Auf tokens waere er ein dauerhaft
        -- redundanter Index — das alte Binary nutzt dort ON CONFLICT (discord_id)
        -- und kommt mit dem Primaerschluessel aus.
        --
        -- Zuerst der Ersatz, dann der PK-Tausch: zwischen DROP und ADD darf kein
        -- Moment ohne Index auf (discord_id, provider) liegen, sonst kann in
        -- dieser Transaktion kein Trigger mehr schreiben. Postgres kennt kein
        -- ADD CONSTRAINT IF NOT EXISTS, deshalb die Abfrage — ein zweiter Lauf
        -- soll nicht an 42P07 sterben und die Transaktion mitnehmen.
        IF tabelle = 'discord_role_connection_sync_state' AND NOT EXISTS (
            SELECT 1 FROM pg_constraint
             WHERE conrelid = ('core.' || tabelle)::regclass
               AND conname = unique_name
        ) THEN
            EXECUTE format(
                'ALTER TABLE core.%I ADD CONSTRAINT %I UNIQUE (discord_id, provider)',
                tabelle, unique_name
            );
        END IF;

        SELECT con.conname, array_agg(att.attname)
          INTO pk_name, pk_spalten
          FROM pg_constraint con
          JOIN pg_attribute att
            ON att.attrelid = con.conrelid
           AND att.attnum = ANY (con.conkey)
         WHERE con.conrelid = ('core.' || tabelle)::regclass
           AND con.contype = 'p'
         GROUP BY con.conname;

        -- Fremd-Provider-Zeilen fliegen immer raus, auch wenn schon
        -- zurueckgerollt wurde: das Unique (discord_id, provider) steht noch, ein
        -- provider-faehiges Binary kann also zwischen zwei Laeufen neue Zeilen
        -- geschrieben haben. Blieben sie liegen, wuerde das alte Binary sie per
        -- ON CONFLICT (discord_id) DO UPDATE mit Steam-Daten ueberschreiben.
        EXECUTE format(
            'DELETE FROM core.%I WHERE provider IS DISTINCT FROM ''steam''',
            tabelle
        );

        -- Primaerschluessel nur tauschen, wenn er noch provider traegt.
        IF pk_spalten IS NULL OR 'provider' = ANY (pk_spalten) THEN
            IF pk_name IS NOT NULL THEN
                EXECUTE format('ALTER TABLE core.%I DROP CONSTRAINT %I', tabelle, pk_name);
            END IF;
            EXECUTE format(
                'ALTER TABLE core.%I ADD CONSTRAINT %I PRIMARY KEY (discord_id)',
                tabelle, tabelle || '_pkey'
            );
        END IF;

        EXECUTE format(
            'ALTER TABLE core.%I ALTER COLUMN provider SET DEFAULT ''steam''',
            tabelle
        );
    END LOOP;
END
$rollback$;

COMMIT;

-- Die Kopien laufen nach 180 Tagen von selbst ab (rollback_expires_at, geraeumt vom
-- Retention-Job in dl-community/src/privacy.rs). Wer frueher fertig ist:
--   DROP TABLE core.discord_role_connection_tokens_rollback_backup;
--   DROP TABLE core.discord_role_connection_sync_state_rollback_backup;
