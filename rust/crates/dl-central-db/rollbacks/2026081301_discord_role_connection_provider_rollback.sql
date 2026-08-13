-- Rueckweg fuer Migration 2026081301 (Linked-Role-Provider-Dimension).
--
-- Liegt bewusst NICHT unter migrations/: sqlx wuerde die Datei sonst als
-- Migration anwenden. Sie wird von Hand gefahren und von
-- tests/fresh_migrations_schema.rs mit ausgefuehrt — ein Rueckweg, der nie
-- laeuft, ist nicht getestet, sondern geraten.
--
-- WANN: nur wenn auf ein Backend-Binary ohne Provider-Unterstuetzung
-- zurueckgerollt werden muss. Vorwaerts ist immer der bessere Weg.
--
-- DATENVERLUST: alle Creator-OAuth-Tokens sind danach aus der Live-Tabelle weg.
-- Jeder betroffene Nutzer muss die Creator-App neu autorisieren. Die Kopien
-- liegen in core.*_rollback_backup — sie enthalten verschluesselte Tokens und
-- gehoeren nach dem Vorfall gedroppt (siehe letzter Abschnitt), sonst hat ein
-- Loeschantrag eine Kopie, die dl-community/src/privacy.rs nicht kennt.
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
-- Der Constraint-Name wird gelesen, nicht geraten; dafuer stehen in der
-- Migration die DO-Bloecke.

BEGIN;

CREATE TABLE IF NOT EXISTS core.discord_role_connection_tokens_rollback_backup AS
    SELECT * FROM core.discord_role_connection_tokens WHERE provider <> 'steam';

CREATE TABLE IF NOT EXISTS core.discord_role_connection_sync_state_rollback_backup AS
    SELECT * FROM core.discord_role_connection_sync_state WHERE provider <> 'steam';

DO $rollback$
DECLARE
    tabelle TEXT;
    pk_name TEXT;
BEGIN
    FOREACH tabelle IN ARRAY ARRAY[
        'discord_role_connection_tokens',
        'discord_role_connection_sync_state'
    ] LOOP
        SELECT con.conname
          INTO pk_name
          FROM pg_constraint con
         WHERE con.conrelid = ('core.' || tabelle)::regclass
           AND con.contype = 'p';

        -- Zuerst der Unique-Ersatz, dann der PK-Tausch: zwischen DROP und ADD
        -- darf kein Moment ohne Index auf (discord_id, provider) liegen, sonst
        -- kann in dieser Transaktion kein Trigger mehr schreiben.
        EXECUTE format(
            'ALTER TABLE core.%I ADD CONSTRAINT %I UNIQUE (discord_id, provider)',
            tabelle, tabelle || '_discord_provider_key'
        );

        IF pk_name IS NOT NULL THEN
            EXECUTE format('ALTER TABLE core.%I DROP CONSTRAINT %I', tabelle, pk_name);
        END IF;

        EXECUTE format('DELETE FROM core.%I WHERE provider <> ''steam''', tabelle);
        EXECUTE format(
            'ALTER TABLE core.%I ALTER COLUMN provider SET DEFAULT ''steam''',
            tabelle
        );
        EXECUTE format(
            'ALTER TABLE core.%I ADD CONSTRAINT %I PRIMARY KEY (discord_id)',
            tabelle, tabelle || '_pkey'
        );
    END LOOP;
END
$rollback$;

DELETE FROM _sqlx_migrations WHERE version = 2026081301;

COMMIT;

-- Nach dem Vorfall, sobald klar ist, dass die Kopien nicht mehr gebraucht werden:
--   DROP TABLE core.discord_role_connection_tokens_rollback_backup;
--   DROP TABLE core.discord_role_connection_sync_state_rollback_backup;
