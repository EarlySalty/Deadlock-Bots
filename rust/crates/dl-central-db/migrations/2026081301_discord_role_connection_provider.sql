-- Linked-Role-Provider-Dimension.
--
-- Bisher gab es genau eine Discord-Application als Linked-Role-Provider, also
-- genau eine Token- und eine Sync-Zeile pro Discord-User. Ab jetzt tragen beide
-- Tabellen zusaetzlich den Provider ('steam' = Steam-/Deadlock-App,
-- 'creator' = Twitch-Creator-App). Bestehende Zeilen sind Steam-Zeilen.
--
-- REIHENFOLGE, nicht optional: Diese Migration ist fuer den *alten*
-- Website-Backend-Build nicht abwaertskompatibel. Dessen
-- `INSERT … ON CONFLICT (discord_id)` findet nach dem PK-Wechsel keinen
-- passenden Unique-Index mehr und scheitert mit SQLSTATE 42P10 — die bereits
-- live laufende Steam-Verknuepfung waere in diesem Fenster tot. Deshalb gilt:
-- Backend-Binary mit Provider-Unterstuetzung bereitlegen, dann migrieren, dann
-- sofort neu starten. Achtung: `dl-central-migrate` laeuft bei jedem Deploy
-- DIESES Repos, der Consumer liegt aber im Repo Website. Ein Deadlock-Bots-Deploy
-- zieht diese Migration also mit, ohne das Website-Binary zu tauschen — deshalb
-- gehoert beides in denselben Deploy-Zug. Ein Rollback auf das alte Binary ist ohne DB-Rueckbau
-- kaputt; sqlx hat keine Down-Migration.
--
-- RUECKWEG: rollbacks/2026081301_discord_role_connection_provider_rollback.sql
-- im selben Crate. Er liegt als ausfuehrbare Datei dort und nicht als Kommentar
-- hier, weil tests/fresh_migrations_schema.rs ihn mitfaehrt — ein Rueckweg, der
-- nie laeuft, ist nicht getestet, sondern geraten. Er loescht Daten (alle
-- Creator-OAuth-Tokens) und sichert sie vorher; Einzelheiten stehen im Kopf der
-- Datei.

ALTER TABLE core.discord_role_connection_tokens
    ADD COLUMN IF NOT EXISTS provider TEXT NOT NULL DEFAULT 'steam';

ALTER TABLE core.discord_role_connection_sync_state
    ADD COLUMN IF NOT EXISTS provider TEXT NOT NULL DEFAULT 'steam';

-- ADD COLUMN IF NOT EXISTS ueberspringt still, wenn die Spalte schon existiert —
-- in einer handgepatchten Umgebung womoeglich als nullable. Dann waere der ganze
-- Pflichtfeld-Vertrag dort nicht gueltig, und der Start-Guard des Backends prueft
-- nur Spalte und Unique-Key. Deshalb Nullability ausdruecklich nachziehen; die
-- Backfill-Zuweisung davor stellt sicher, dass das gelingt.
UPDATE core.discord_role_connection_tokens SET provider = 'steam' WHERE provider IS NULL;
UPDATE core.discord_role_connection_sync_state SET provider = 'steam' WHERE provider IS NULL;

ALTER TABLE core.discord_role_connection_tokens
    ALTER COLUMN provider SET NOT NULL;

ALTER TABLE core.discord_role_connection_sync_state
    ALTER COLUMN provider SET NOT NULL;

-- Der Default hat nur die bestehenden Zeilen als Steam-Zeilen markiert. Bleibt er
-- stehen, erzeugt jedes INSERT, das provider vergisst, still eine Steam-Zeile —
-- ueber ON CONFLICT … DO UPDATE koennte das ein echtes Steam-Token mit
-- Creator-Zugangsdaten ueberschreiben. Ab hier setzt jeder Schreiber den
-- Provider ausdruecklich.
ALTER TABLE core.discord_role_connection_tokens
    ALTER COLUMN provider DROP DEFAULT;

ALTER TABLE core.discord_role_connection_sync_state
    ALTER COLUMN provider DROP DEFAULT;

ALTER TABLE core.discord_role_connection_tokens
    DROP CONSTRAINT IF EXISTS discord_role_connection_tokens_provider_check;
ALTER TABLE core.discord_role_connection_tokens
    ADD CONSTRAINT discord_role_connection_tokens_provider_check
    CHECK (provider IN ('steam', 'creator'));

ALTER TABLE core.discord_role_connection_sync_state
    DROP CONSTRAINT IF EXISTS discord_role_connection_sync_state_provider_check;
ALTER TABLE core.discord_role_connection_sync_state
    ADD CONSTRAINT discord_role_connection_sync_state_provider_check
    CHECK (provider IN ('steam', 'creator'));

-- Falls diese Migration nach einem Rollback erneut laeuft: das Unique, das der
-- Rueckweg als Ersatz angelegt hat, wird hier weggeraeumt. Sonst traegt die
-- Tabelle dauerhaft zwei identische Unique-Indizes auf (discord_id, provider).
ALTER TABLE core.discord_role_connection_tokens
    DROP CONSTRAINT IF EXISTS discord_role_connection_tokens_discord_provider_key;
ALTER TABLE core.discord_role_connection_sync_state
    DROP CONSTRAINT IF EXISTS discord_role_connection_sync_state_discord_provider_key;

-- Primaerschluessel auf (discord_id, provider) umstellen, sofern noch nicht getan.
DO $$
DECLARE
    pk_name TEXT;
    pk_columns TEXT[];
BEGIN
    -- Den Namen mitlesen statt den Default zu raten: heisst der Schluessel in
    -- einer Umgebung anders, greift DROP CONSTRAINT IF EXISTS still nicht und
    -- das folgende ADD CONSTRAINT kippt mit "multiple primary keys".
    SELECT con.conname, array_agg(att.attname)
      INTO pk_name, pk_columns
      FROM pg_constraint con
      JOIN pg_attribute att
        ON att.attrelid = con.conrelid
       AND att.attnum = ANY (con.conkey)
     WHERE con.conrelid = 'core.discord_role_connection_tokens'::regclass
       AND con.contype = 'p'
     GROUP BY con.conname;

    IF pk_columns IS NULL OR NOT ('provider' = ANY (pk_columns)) THEN
        IF pk_name IS NOT NULL THEN
            EXECUTE format('ALTER TABLE core.discord_role_connection_tokens DROP CONSTRAINT %I', pk_name);
        END IF;
        ALTER TABLE core.discord_role_connection_tokens
            ADD CONSTRAINT discord_role_connection_tokens_pkey
            PRIMARY KEY (discord_id, provider);
    END IF;
END
$$;

DO $$
DECLARE
    pk_name TEXT;
    pk_columns TEXT[];
BEGIN
    -- Den Namen mitlesen statt den Default zu raten: heisst der Schluessel in
    -- einer Umgebung anders, greift DROP CONSTRAINT IF EXISTS still nicht und
    -- das folgende ADD CONSTRAINT kippt mit "multiple primary keys".
    SELECT con.conname, array_agg(att.attname)
      INTO pk_name, pk_columns
      FROM pg_constraint con
      JOIN pg_attribute att
        ON att.attrelid = con.conrelid
       AND att.attnum = ANY (con.conkey)
     WHERE con.conrelid = 'core.discord_role_connection_sync_state'::regclass
       AND con.contype = 'p'
     GROUP BY con.conname;

    IF pk_columns IS NULL OR NOT ('provider' = ANY (pk_columns)) THEN
        IF pk_name IS NOT NULL THEN
            EXECUTE format('ALTER TABLE core.discord_role_connection_sync_state DROP CONSTRAINT %I', pk_name);
        END IF;
        ALTER TABLE core.discord_role_connection_sync_state
            ADD CONSTRAINT discord_role_connection_sync_state_pkey
            PRIMARY KEY (discord_id, provider);
    END IF;
END
$$;

-- Der stuendliche Creator-Abgleich sucht genau diese Zeilen und liest daraus nur
-- die discord_id; der neue Primaerschluessel hat provider an zweiter Stelle und
-- hilft dabei nicht. Beide Spalten im Index, damit der Sweep index-only laufen
-- kann statt die Zeilen anzufassen.
CREATE INDEX IF NOT EXISTS discord_role_connection_tokens_provider_active_idx
    ON core.discord_role_connection_tokens (provider, discord_id)
 WHERE active;

-- Die Sync-Tabelle traegt ab jetzt bis zu zwei Zeilen pro User, und jeder
-- Update-Pfad des Backends filtert zusaetzlich auf provider. Der bestehende
-- Faelligkeits-Index (pending, next_attempt_at, updated_at) kennt die Spalte
-- nicht.
CREATE INDEX IF NOT EXISTS discord_role_connection_sync_provider_pending_idx
    ON core.discord_role_connection_sync_state (provider, next_attempt_at)
 WHERE pending;

-- Der Steam-Link-Trigger schreibt ab jetzt ausdruecklich Steam-Sync-Zeilen.
-- Fuer den Creator-Provider liegen die Quelldaten in der Twitch-Datenbank; dort
-- gibt es keinen Trigger. Diese Zeilen stellt der Sync-Worker des Website-Backends
-- selbst ein (Reason 'creator_reconcile', Abstand
-- DISCORD_ROLE_CONNECTION_CREATOR_RECONCILE_SECONDS, Standard eine Stunde).
-- Zusaetzlich kann jeder Dienst POST /api/internal/discord-role-connections/sync
-- mit provider=creator und enqueue=true aufrufen, um sofort abzugleichen.
CREATE OR REPLACE FUNCTION core.enqueue_discord_role_connection_sync()
RETURNS trigger
LANGUAGE plpgsql
AS $$
DECLARE
    target_discord_id BIGINT;
    sync_reason TEXT := 'steam_link_changed';
BEGIN
    IF TG_OP = 'DELETE' THEN
        target_discord_id := OLD.discord_id;
        sync_reason := 'steam_link_removed';
    ELSIF TG_OP = 'UPDATE' AND NEW.discord_id IS DISTINCT FROM OLD.discord_id THEN
        INSERT INTO core.discord_role_connection_sync_state (
            discord_id, provider, pending, reason, attempts, next_attempt_at,
            locked_at, last_error, updated_at
        )
        SELECT target.target_discord_id, 'steam', TRUE, target.sync_reason, 0, now(),
               NULL, NULL, now()
          FROM (
              VALUES
                  (OLD.discord_id, 'steam_link_removed'::TEXT),
                  (NEW.discord_id, 'steam_link_changed'::TEXT)
          ) AS target(target_discord_id, sync_reason)
         WHERE target.target_discord_id IS NOT NULL
           AND target.target_discord_id <> 0
        ON CONFLICT (discord_id, provider) DO UPDATE SET
            pending = TRUE,
            reason = EXCLUDED.reason,
            attempts = 0,
            next_attempt_at = now(),
            locked_at = NULL,
            last_error = NULL,
            updated_at = now();

        RETURN NEW;
    ELSE
        target_discord_id := NEW.discord_id;
        IF TG_OP = 'UPDATE'
           AND (
               COALESCE(NEW.deadlock_badge_level, NEW.deadlock_rank, 0)
                   IS DISTINCT FROM COALESCE(OLD.deadlock_badge_level, OLD.deadlock_rank, 0)
               OR NEW.deadlock_rank_name IS DISTINCT FROM OLD.deadlock_rank_name
               OR NEW.deadlock_subrank IS DISTINCT FROM OLD.deadlock_subrank
               OR NEW.deadlock_rank_updated_at IS DISTINCT FROM OLD.deadlock_rank_updated_at
           ) THEN
            sync_reason := 'rank_changed';
        END IF;
    END IF;

    IF target_discord_id IS NULL OR target_discord_id = 0 THEN
        IF TG_OP = 'DELETE' THEN
            RETURN OLD;
        END IF;
        RETURN NEW;
    END IF;

    INSERT INTO core.discord_role_connection_sync_state (
        discord_id, provider, pending, reason, attempts, next_attempt_at,
        locked_at, last_error, updated_at
    )
    VALUES (
        target_discord_id, 'steam', TRUE, sync_reason, 0, now(), NULL, NULL, now()
    )
    ON CONFLICT (discord_id, provider) DO UPDATE SET
        pending = TRUE,
        reason = EXCLUDED.reason,
        attempts = 0,
        next_attempt_at = now(),
        locked_at = NULL,
        last_error = NULL,
        updated_at = now();

    IF TG_OP = 'DELETE' THEN
        RETURN OLD;
    END IF;
    RETURN NEW;
END;
$$;
