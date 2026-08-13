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

CREATE TABLE IF NOT EXISTS core.discord_role_connection_tokens_rollback_backup AS
    SELECT * FROM core.discord_role_connection_tokens WHERE provider <> 'steam';

CREATE TABLE IF NOT EXISTS core.discord_role_connection_sync_state_rollback_backup AS
    SELECT * FROM core.discord_role_connection_sync_state WHERE provider <> 'steam';

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

        -- Zuerst der Unique-Ersatz, dann der PK-Tausch: zwischen DROP und ADD
        -- darf kein Moment ohne Index auf (discord_id, provider) liegen, sonst
        -- kann in dieser Transaktion kein Trigger mehr schreiben. Postgres kennt
        -- kein ADD CONSTRAINT IF NOT EXISTS, deshalb die Abfrage — ein zweiter
        -- Lauf soll nicht an 42P07 sterben und die ganze Transaktion mitnehmen.
        IF NOT EXISTS (
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

        -- Schon zurueckgerollt? Dann nur die Zeilen und den Default pruefen.
        IF pk_spalten IS NOT NULL AND 'provider' = ANY (pk_spalten) THEN
            EXECUTE format('ALTER TABLE core.%I DROP CONSTRAINT %I', tabelle, pk_name);
            EXECUTE format('DELETE FROM core.%I WHERE provider <> ''steam''', tabelle);
            EXECUTE format(
                'ALTER TABLE core.%I ADD CONSTRAINT %I PRIMARY KEY (discord_id)',
                tabelle, tabelle || '_pkey'
            );
        ELSIF pk_spalten IS NULL THEN
            EXECUTE format('DELETE FROM core.%I WHERE provider <> ''steam''', tabelle);
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

-- Nach dem Vorfall, sobald klar ist, dass die Kopien nicht mehr gebraucht werden:
--   DROP TABLE core.discord_role_connection_tokens_rollback_backup;
--   DROP TABLE core.discord_role_connection_sync_state_rollback_backup;
