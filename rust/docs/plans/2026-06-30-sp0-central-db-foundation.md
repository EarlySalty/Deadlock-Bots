# SP0 — Fundament der zentralen Postgres/TimescaleDB — Implementierungsplan

> **Für ausführende Worker (Codex):** Dieser Plan ist ein Ticket-DAG. Jedes Ticket = exakter Scope + Dateien + Test-Vertrag + Definition of Done. **Codex implementiert den Code; Claude orchestriert, verifiziert extern, committet.** Schritte tragen `- [ ]` zum Abhaken. **WORKFLOW.md NIE anfassen.** User-sichtbare deutsche Texte (Fehlertexte, Doku-Prosa fürs UI) = nur `"Platzhalter"` setzen + Datei:Zeile melden, Claude finalisiert.

**Ziel:** Die tragfähigen „Schienen" für die zentrale DB bauen — dedizierte TimescaleDB-Instanz, kanonischer Migrations-Owner mit `core`-Schema + 7 Domänen-Namespaces, sqlx-Anbindung mit compile-checked Queries + committetem Offline-Cache, sowie ein ETL-/Verifikations-Harness und ein Vier-Säulen-Test-Harness mit echten Zähnen. **SP0 migriert KEINE Echtdaten und schneidet KEINEN Dienst um** — null Datenrisiko; reale ETL beginnt erst SP1.

**Architektur:** Eine Postgres-DB (TimescaleDB-Image, pg16), Postgres-Schemas pro Domäne (`core`/`coaching`/`scrim`/`steam`/`turnier`/`patchnotes`/`activity`), `core` als schmale geteilte Spine mit Discord-User-ID als Join-Key. Genau ein kanonischer Migrator (`dl-central-migrate`) wendet das Schema an; Dienste nehmen das Schema als gegeben. Datenzugriff über `sqlx` mit `query!`/`query_as!` (compile-geprüft) + committetem `.sqlx`-Cache. ETL liest Quell-SQLite via `rusqlite`, schreibt Postgres via `sqlx`, abgesichert durch Mapping-/Drop-Ledger + Aggregat-Gates.

**Tech-Stack:** Rust (Workspace `Deadlock-Bots/rust`), `sqlx 0.8.6` (postgres, runtime-tokio, tls-rustls, macros, migrate, chrono), `rusqlite 0.32` (bereits Workspace-Dep, für ETL-Lesen), TimescaleDB `timescale/timescaledb:2.17.2-pg16`, Docker Compose + systemd, Infisical für DSN.

---

## Global Constraints (gelten implizit für JEDES Ticket)

Wörtlich aus der freigegebenen Spec (`rust/docs/specs/2026-06-30-central-postgres-migration-design.md`):

1. **Null Datenverlust.** Keine Zeile/Spalte/Datum geht verloren. Drops nur als gelisteter Eintrag im Mapping-/Drop-Ledger. (In SP0 fällt noch keine Echtdaten-Migration an — der Ledger-Mechanismus wird hier nur gebaut + negativ getestet.)
2. **Sauber neu schreiben, nicht umbiegen.** Kein 1:1-Dump der 119 SQLite-Tabellen, kein rusqlite-Muster mechanisch übersetzen. Idiomatisches sqlx/Postgres-Modell, klare Namen (keine kryptischen Präfixe).
3. **Verlustfreie Kompression, kein Delete.** TimescaleDB-Kompression spaltenweise + verlustfrei; **keine** Retention-Policy. (In SP0 noch keine Hypertables/Compression-Policies — nur die Extension verfügbar machen; Aktivierung erst, wenn `activity`-Tabellen in SP1 entstehen.)
4. **Tests mit Zähnen.** Tests fangen echte Prod-Brüche (vier Säulen, §9 der Spec), waschen nicht grün. Konsumenten mittesten, nicht nur die geänderte Crate.
5. **Inkrementell + rückrollbar.** (Greift ab SP1 beim Cutover; SP0 schneidet nichts um.)
6. **Quelle wird nie zerstört.** SQLite-Originale werden vor jeder ETL gesnapshottet, nie überschrieben.
7. **Secrets:** DSN ausschließlich via Infisical, nie Klartext in Code/Log/Chat. Keine `*.env`/`service_token.json`/`/proc/environ` lesen.

**Feste Parameter (in SP0 nageln):**

| Parameter | Wert |
|---|---|
| Image | `timescale/timescaledb:2.17.2-pg16` (identisch zum laufenden `twitch-analytics-postgres`) |
| Host-Port | `127.0.0.1:5434->5432` (loopback-only; 5433 ist Twitch) |
| DB-Name | `deadlock` |
| Container/Volume | Container `deadlock-central-postgres`, Volume `deadlock_central_pgdata` |
| Rolle | Owner-Rolle `deadlock` (Least-Privilege-Split runtime/migrator als spätere Option notiert, in SP0 bewusst eine Rolle) |
| Infisical-Secret | `DEADLOCK_CENTRAL_DSN` (vollständige DSN; **vom User anzulegen**) |
| sqlx | `0.8` (Lock 0.8.6), Features exakt wie Twitch: `postgres, runtime-tokio, tls-rustls, macros, migrate, chrono` |
| Foundation-Crate | `crates/dl-central-db` |
| Migrator-Binary | `bin/dl-central-migrate` |
| ETL-Crate | `crates/dl-central-etl` |
| Migrations-Dir | `crates/dl-central-db/migrations/` (eingebettet via `sqlx::migrate!`) |
| Offline-Cache | `crates/dl-central-db/.sqlx/` (committed), Builds `SQLX_OFFLINE=true` |

---

## Zielbild Schema-Layout (Design — Codex setzt final um)

SP0 legt **nur** die 7 Schema-Namespaces + die `core`-Tabellen an. Die Domänen-Tabellen kommen in ihren jeweiligen SPs.

```sql
-- Migration 0001: Extension + Schemas + core-Spine
CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE SCHEMA IF NOT EXISTS core;
CREATE SCHEMA IF NOT EXISTS coaching;
CREATE SCHEMA IF NOT EXISTS scrim;
CREATE SCHEMA IF NOT EXISTS steam;
CREATE SCHEMA IF NOT EXISTS turnier;
CREATE SCHEMA IF NOT EXISTS patchnotes;
CREATE SCHEMA IF NOT EXISTS activity;

-- core.users: die eine geteilte Identität (Discord-User-ID = Join-Key, BIGINT, KEIN uuid)
CREATE TABLE core.users (
    discord_id   BIGINT PRIMARY KEY,
    username     TEXT,                       -- aktueller Discord-Username (handle)
    global_name  TEXT,                       -- Anzeigename
    avatar       TEXT,
    first_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen    TIMESTAMPTZ NOT NULL DEFAULT now(),
    raw          JSONB                        -- Roh-Discord-Payload-Reste, verlustfrei
);

-- core.steam_links: discord -> steam (heute in deadlock.sqlite3 verstreut)
CREATE TABLE core.steam_links (
    discord_id   BIGINT NOT NULL REFERENCES core.users(discord_id) ON DELETE CASCADE,
    steam_id64   BIGINT NOT NULL,
    verified     BOOLEAN NOT NULL DEFAULT false,
    linked_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (discord_id, steam_id64)
);
CREATE INDEX ON core.steam_links (steam_id64);
```

**Bewusst aufgeschoben:** `core.guild_members` (Spec: „ggf.") — Membership-Semantik gehört in die Bot-Domäne; in SP1 entscheiden, wenn die reale Form bekannt ist. In diesem Plan markiert, NICHT in SP0 angelegt.

---

## Offene Architektur-Entscheidungen (meine Empfehlung — bei Plan-Freigabe abnicken/ändern)

- **E1 — Test-Postgres: Haus-Skript-Muster statt `testcontainers-rs`.** *Empfehlung:* das bewährte Wegwerf-`docker run`-Muster (wie Twitch `test_db.sh`/`test-fresh-schema.sh`) übernehmen — keine neue Crate-Dependency, schon im Haus erprobt, erfüllt §9.2 („echte Queries gegen echtes Test-Postgres mit echtem Migrations-Schema"). Alternative `testcontainers-rs` (in-Prozess-Teardown, schöner für CI) verworfen wegen neuer Lernkurve/Dependency ohne Mehrwert hier.
- **E2 — Instanz-Verwaltung: Docker-Compose + systemd-Unit (System-Ebene), wie `infisical-compose.service`.** *Empfehlung:* reproduzierbarer als der bare `docker run` der Twitch-Instanz (die hat keine Compose-/Unit-Datei). Healthcheck (`pg_isready`) ergänzen, den Twitch fehlt. Alternative: systemd-**user**-Unit (konsistent mit den Bots) — leichter, aber DB ist geteilte Infra → System-Ebene robuster.
- **E3 — Eine Rolle `deadlock` statt runtime/migrator-Split.** *Empfehlung:* in SP0 Einfachheit; Least-Privilege-Split als spätere Härtung notiert (Spec verlangt ihn nicht).
- **E4 — Compile-checked `query!`/`query_as!` ab sofort** (Spec-Pflicht §9.1), obwohl Haus-Bestand runtime-checked ist. Bringt den `.sqlx`-Offline-Cache-Workflow neu ins Haus. Kein Spielraum (Spec), aber bewusst genannt wegen Pflegeaufwand (`cargo sqlx prepare` + commit).

---

## Datei-Struktur (was SP0 erzeugt/ändert)

**Neu (Codex):**
- `rust/infra/central-db/docker-compose.yml` — dedizierte TimescaleDB-Instanz
- `rust/infra/central-db/README.md` — Hochziehen/Backup/Verifikation (Prosa = Claude finalisiert)
- `rust/infra/central-db/deadlock-central-db.service` — systemd-Unit-Vorlage (Compose-Wrapper)
- `rust/scripts/central_test_db.sh` — Wegwerf-Test-Postgres (dynamischer Port → `CENTRAL_TEST_DSN`)
- `rust/scripts/central_fresh_schema.sh` — Fresh-DB-aus-Migrationen für Drift-Gate
- `rust/scripts/central_ci.sh` — Vier-Säulen-CI-Bündel (offline-build + drift + integration + etl-negativ)
- `rust/crates/dl-central-db/{Cargo.toml,src/lib.rs,src/pool.rs,src/core_users.rs,migrations/0001_core_and_schemas.sql,.sqlx/}`
- `rust/crates/dl-central-db/tests/{integration_core.rs,fresh_migrations_schema.rs}`
- `rust/crates/dl-central-etl/{Cargo.toml,src/lib.rs,src/ledger.rs,src/verify.rs,tests/verify_teeth.rs,tests/fixtures/*}`
- `rust/bin/dl-central-migrate/{Cargo.toml,src/main.rs}`
- `rust/docs/central-db/{architecture.md,offline-prepare-runbook.md,ledger-format.md}` (Prosa = Claude finalisiert)

**Geändert (Codex):**
- `rust/Cargo.toml` — `[workspace].members` += die 3 neuen Crates/Bins; `[workspace.dependencies]` += `sqlx`, `uuid`(nur falls nötig — voraussichtlich NICHT)
- `Deadlock-Bots/CHANGELOG.md` — neuer Eintrag (nur falls user-sichtbar relevant; SP0 ist Infra → ggf. nur interner Vermerk)

**Human-Schritte (du / Claude mit Freigabe — NICHT Codex):**
- Infisical-Secret `DEADLOCK_CENTRAL_DSN` anlegen (DSN inkl. Passwort — gehört dir).
- Container live hochziehen + Live-Verifikation (Artefakt + Live-Zustand).

---

## Ticket-DAG (Abhängigkeiten)

```
T0 (Infra+Secret) ──┐
                    ├─> T5 (Integration) ─┐
T1 (Crate+Pool) ────┼─> T2 (Schema) ─> T3 (Migrator) ─┼─> T6 (Drift-Gate) ─┐
                    │                  └─> T4 (query!+.sqlx) ───────────────┼─> T8 (CI+Docs+Merge)
                    └─> T7 (ETL-Harness+Negativtest) ──────────────────────┘
```
T1/T2/T3/T4/T5/T6/T7 entwickeln gegen **Wegwerf-Test-Postgres** (T0 nicht blockierend für Rust-Dev). T0s Live-Verifikation gated nur das finale „echte Instanz steht".

---

## T0 — Dedizierte TimescaleDB-Instanz + Infisical-Secret

**Zweck:** Die echte zentrale Instanz auf `:5434` + das Secret. Kein Rust.

**Dateien:** `rust/infra/central-db/docker-compose.yml`, `.../README.md`, `.../deadlock-central-db.service`.

**Codex-Scope:**
- Compose: Image `timescale/timescaledb:2.17.2-pg16`, `ports: ["127.0.0.1:5434:5432"]`, named volume `deadlock_central_pgdata:/var/lib/postgresql/data`, `restart: unless-stopped`, **Healthcheck** `pg_isready -U deadlock -d deadlock`, `POSTGRES_DB=deadlock`, `POSTGRES_USER=deadlock`, `POSTGRES_PASSWORD` aus Env (via Infisical zur `up`-Zeit geladen — **nie** Klartext in der Datei).
- systemd-Unit-Vorlage analog `/etc/systemd/system/infisical-compose.service` (Compose-Wrapper, `WantedBy=multi-user.target`).
- README: Hochziehen, Backup-Hinweis, Verifikations-Checkliste (Prosa Platzhalter → Claude).

**Human-Schritte (Rücksprache):**
- Du legst `DEADLOCK_CENTRAL_DSN` in Infisical an (Format `postgres://deadlock:<pw>@127.0.0.1:5434/deadlock`).
- Claude (mit deiner Freigabe) zieht den Container hoch: Secret via `export_claude_secret.py` in die Env, `docker compose up -d`.

**Definition of Done:**
- [ ] `docker ps` zeigt `deadlock-central-postgres` auf `127.0.0.1:5434`, Healthcheck `healthy`.
- [ ] `pg_isready` grün; `\dn` (psql) zeigt noch **keine** Domänen-Schemas (leer/Default) — Beweis: Migrator legt sie an, nicht der Init.
- [ ] Secret existiert (`export_claude_secret.py --list` zeigt `DEADLOCK_CENTRAL_DSN`); **Wert nie ausgegeben**.
- [ ] Compose-Datei enthält **kein** Klartext-Passwort.

---

## T1 — Foundation-Crate `dl-central-db` + sqlx-Workspace-Dep + Pool

**Zweck:** Schlanke sqlx-Anbindung an die zentrale DB (Pool, Config, DSN aus Env).

**Dateien:** `rust/Cargo.toml` (Workspace), `rust/crates/dl-central-db/{Cargo.toml,src/lib.rs,src/pool.rs}`.

**Interfaces (Produziert — spätere Tickets bauen darauf):**
- `pub async fn connect_pool(dsn: &str) -> Result<sqlx::PgPool, CentralDbError>` (via `PgPoolOptions`, Muster `tb-db/src/pool.rs`).
- `pub fn dsn_from_env() -> Result<String, CentralDbError>` liest `DEADLOCK_CENTRAL_DSN` (Runtime, nicht Build-Zeit).
- Fehler-Typ `CentralDbError` (thiserror), kapselt `sqlx::Error`.

**Codex-Scope:**
- `sqlx` in `[workspace.dependencies]` mit exakt den Twitch-Features; `dl-central-db` in `[workspace].members`.
- Pool + Config; **kein** Schema-Anlegen hier (Dienste legen kein Schema an).

**Test-Vertrag:**
- Integrationstest (gated hinter `CENTRAL_TEST_DSN`, sonst `#[ignore]`): `connect_pool` öffnet, `SELECT 1` läuft.
- `cargo build -p dl-central-db` grün.

**Definition of Done:**
- [ ] `cargo build -p dl-central-db` grün; `cargo clippy -p dl-central-db -- -D warnings` sauber.
- [ ] Connection-Smoke-Test grün gegen Wegwerf-Test-Postgres.
- [ ] **Konsumenten-Check:** `cargo build` über den ganzen Workspace bleibt grün (neue Crate bricht nichts).

---

## T2 — `core`-Schema + 7 Domänen-Namespaces (Migration 0001)

**Zweck:** Das kanonische Anfangsschema (siehe Zielbild oben). TDD: Test zuerst.

**Dateien:** `rust/crates/dl-central-db/migrations/0001_core_and_schemas.sql`, `rust/crates/dl-central-db/tests/fresh_migrations_schema.rs` (Erstanlage).

**Test-Vertrag (TDD, Red zuerst):**
- Test bringt frische DB hoch, wendet `sqlx::migrate!` an, dann Assertions gegen `information_schema`:
  - alle 7 Schemas existieren (`core,coaching,scrim,steam,turnier,patchnotes,activity`).
  - `core.users` hat Spalten `discord_id BIGINT PK NOT NULL`, `first_seen/last_seen timestamptz NOT NULL`, `raw jsonb`.
  - `core.steam_links` hat FK auf `core.users`, PK `(discord_id, steam_id64)`, Index auf `steam_id64`.
  - `timescaledb`-Extension ist installiert.
- **Negativ-Anker (Zähne):** der Test prüft konkrete NOT-NULL/Typ/PK — ein späteres Entfernen einer Pflicht-Spalte/eines Typs MUSS ihn rot machen.

**Definition of Done:**
- [ ] Test zuerst rot (Migration fehlt), dann grün (Migration da).
- [ ] `cargo build -p dl-central-db` grün.
- [ ] Schema-Namen exakt wie Parameter-Tabelle.

---

## T3 — Kanonischer Migrator `bin/dl-central-migrate`

**Zweck:** Genau **ein** Owner wendet Migrationen an (Spec: „eigener migrate-Schritt"). Dienste migrieren nicht selbst.

**Dateien:** `rust/bin/dl-central-migrate/{Cargo.toml,src/main.rs}`, `rust/Cargo.toml` (Member).

**Interfaces (Consumes):** `dl-central-db::{connect_pool, dsn_from_env}`; `sqlx::migrate!("crates/dl-central-db/migrations")` (eingebettet, Muster `tb-db/migrate.rs`).

**Codex-Scope:** Binary: DSN aus Env → Pool → `migrate!().run()`. Idempotent. Klare Exit-Codes/Logs (Log-Prosa = Platzhalter → Claude).

**Test-Vertrag:**
- Integrationstest: Migrator zweimal gegen frische DB → 2. Lauf no-op, kein Fehler; `_sqlx_migrations` enthält 0001.
- **Artefakt-Beweis (CLAUDE.md-Deploy-Regel):** nach Build im Binary nach dem eingebetteten Migrations-Namen greppen (kein stale Embed).

**Definition of Done:**
- [ ] `cargo build -p dl-central-migrate` grün.
- [ ] Doppellauf-Test grün; `_sqlx_migrations`-Zeile live gegengeprüft.
- [ ] Migrator ist die **einzige** Stelle mit `migrate!` (grep: kein zweiter Migrations-Owner).

---

## T4 — Compile-checked `query_as!` + `.sqlx`-Offline-Cache + Runbook

**Zweck:** Den Spec-Pflicht-Vertrag (§9.1) etablieren: Schema = Compile-Zeit-Vertrag; Builds ohne Live-DB via `.sqlx`.

**Dateien:** `rust/crates/dl-central-db/src/core_users.rs`, `rust/crates/dl-central-db/.sqlx/` (committed), `rust/docs/central-db/offline-prepare-runbook.md`.

**Interfaces (Produziert):**
- `pub async fn upsert_user(pool, discord_id: i64, username: Option<&str>, global_name: Option<&str>, avatar: Option<&str>) -> Result<(), CentralDbError>` — `INSERT ... ON CONFLICT (discord_id) DO UPDATE`, `query!`.
- `pub async fn get_user(pool, discord_id: i64) -> Result<Option<CoreUser>, CentralDbError>` — `query_as!`.
- `pub struct CoreUser { discord_id: i64, username: Option<String>, global_name: Option<String>, avatar: Option<String>, first_seen: DateTime<Utc>, last_seen: DateTime<Utc> }`.

**Codex-Scope:**
- Beide Funktionen mit **compile-checked** Makros gegen das reale Schema (aus T2).
- `.sqlx`-Cache via `cargo sqlx prepare` gegen ein migriertes Test-Postgres erzeugen + committen.
- Runbook: wann/wie `cargo sqlx prepare` laufen muss, dass `.sqlx` committet wird, `SQLX_OFFLINE=true` für Builds/CI (Prosa Platzhalter → Claude).

**Test-Vertrag:**
- `SQLX_OFFLINE=true cargo build -p dl-central-db` grün **ohne** laufende DB (beweist Offline-Cache greift).
- Integrationstest: `upsert_user` dann `get_user` → Round-Trip-Werte gleich (gegen Test-Postgres).
- **Zähne:** ein bewusst falscher Spaltenname im Makro ⇒ **Compile-Fehler** (kurz im Runbook als Beleg dokumentieren, dann zurücknehmen).

**Definition of Done:**
- [ ] Offline-Build grün ohne DB.
- [ ] `.sqlx/` committed + im Runbook erklärt.
- [ ] Round-Trip-Test grün.

---

## T5 — Cross-Boundary-Integrationstest-Harness (Säule 2)

**Zweck:** Echte Queries gegen echtes Test-Postgres mit echtem Migrations-Schema (genau das, was beim S1-02-Crash fehlte).

**Dateien:** `rust/scripts/central_test_db.sh`, `rust/crates/dl-central-db/tests/integration_core.rs`.

**Codex-Scope (E1: Haus-Skript-Muster):**
- `central_test_db.sh`: Wegwerf-`timescale/timescaledb:2.17.2-pg16` auf dynamischem Port, exportiert `CENTRAL_TEST_DSN`, wartet via `pg_isready`, Teardown am Ende (Muster Twitch `test_db.sh`/`test-fresh-schema.sh`).
- `integration_core.rs`: startet von Migrationen, übt `upsert_user`/`get_user`/`steam_links`-Insert+Join `core.users ⨝ core.steam_links`.

**Test-Vertrag:**
- Skript hoch → `cargo test -p dl-central-db --test integration_core` grün → Teardown.
- Join-Test beweist die `core`-Spine ist über Discord-ID joinbar.

**Definition of Done:**
- [ ] Skript ist hermetisch (eigener Container/Port, räumt auf, kollidiert nicht mit :5433/:5434).
- [ ] Integrationstest grün gegen frisch migriertes Schema.

---

## T6 — Schema-Drift-Gate (Säule 3)

**Zweck:** CI beweist „frische DB aus Migrationen == erwartetes Schema" — kein Drift Bauplan↔Code↔Live.

**Dateien:** `rust/scripts/central_fresh_schema.sh`, Erweiterung `rust/crates/dl-central-db/tests/fresh_migrations_schema.rs`, `rust/crates/dl-central-db/tests/expected_schema.sql` (oder serialisierter Erwartungs-Snapshot).

**Codex-Scope:**
- Fresh-DB-aus-Migrationen, Introspektion `information_schema.columns`/`table_constraints` über alle Schemas, Vergleich gegen einen **committeten Erwartungs-Snapshot**.
- Skript-Variante (`central_fresh_schema.sh`) für CI (Muster Twitch `test-fresh-schema.sh` + `fresh_migrations_schema`).

**Test-Vertrag (Zähne):**
- Gate grün bei Übereinstimmung.
- **Bewusster Drift** (Erwartungs-Snapshot temporär verfälschen) ⇒ Gate **rot**. Im PR/Runbook als Beleg zeigen, dann zurücknehmen.

**Definition of Done:**
- [ ] Drift-Gate grün auf echtem Migrations-Schema.
- [ ] Bewusst eingebauter Drift wird rot (dokumentiert).

---

## T7 — ETL-/Verifikations-Harness-Framework + Mapping/Drop-Ledger + Negativtests (Säule 4-Vorstufe)

**Zweck:** Das **Framework** für verlustfreie ETL bauen (SP1+ nutzt es). SP0 migriert keine Echtdaten — beweist aber, dass das Gate Zähne hat.

**Dateien:** `rust/crates/dl-central-etl/{Cargo.toml,src/lib.rs,src/ledger.rs,src/verify.rs}`, `tests/verify_teeth.rs`, `tests/fixtures/*`, `rust/docs/central-db/ledger-format.md`.

**Interfaces (Produziert):**
- Quell-Reader: `rusqlite`-basiert (Workspace-Dep), öffnet **read-only** Snapshot-Kopie (nie Original).
- Ziel-Writer: `sqlx::PgPool`.
- `Ledger` (aus TOML/Markdown geparst): pro Quell-Tabelle/-Spalte Status `Mapped{to}` | `Dropped{reason}`.
- `pub fn check_mapping_completeness(source_columns, ledger) -> Result<(), VerifyError>` — jede Quell-Spalte gemappt **oder** gedroppt, sonst `Err` (Lücke = Stopp).
- `pub fn check_row_counts(expected, actual) -> Result<(), VerifyError>` (mit Split/Merge-Faktor).
- `pub fn sample_round_trip(...) -> Result<(), VerifyError>` (Feld-für-Feld auf Stichprobe).

**Ledger-Format (Design):** Markdown/TOML mit `quelle.tabelle.spalte = "ziel.schema.tabelle.spalte"` oder `= "DROP: <Begründung>"`. Reviewbar, vom User abnickbar.

**Test-Vertrag (Zähne — Kern dieses Tickets):**
- Fixture mit einer Quell-Spalte, die **weder** gemappt **noch** gedroppt ist ⇒ `check_mapping_completeness` MUSS `Err` liefern (Test rot bei „grün-waschen").
- Row-Count-Mismatch ⇒ `check_row_counts` `Err`.
- Korrekt gemappte Fixture ⇒ alle Gates grün.
- Typ-Treue-Smoke: Unix-INTEGER→`timestamptz`, `0/1`→`bool`, TEXT-JSON→`jsonb` an Mini-Fixtures.

**Definition of Done:**
- [ ] `cargo build -p dl-central-etl` + clippy sauber.
- [ ] Negativtests beweisen: unmapped Spalte UND Count-Mismatch werden **rot**.
- [ ] Ledger-Format dokumentiert.

---

## T8 — Vier-Säulen-CI-Bündel + Docs + Merge

**Zweck:** Alles verdrahten, dokumentieren, sauber nach `main` bringen.

**Dateien:** `rust/scripts/central_ci.sh`, `rust/docs/central-db/architecture.md`, `Deadlock-Bots/CHANGELOG.md` (falls user-sichtbar).

**Codex-Scope:**
- `central_ci.sh` bündelt: (1) `SQLX_OFFLINE=true cargo build --workspace`, (2) Drift-Gate (T6), (3) Integration (T5), (4) ETL-Negativtest (T7) — alle vier Säulen in einem Lauf.
- `architecture.md`: Instanz, Schema-Layout, Migrations-Owner, Offline-Cache-Pflege, ETL-/Ledger-Workflow (Prosa Platzhalter → Claude finalisiert).

**Definition of Done:**
- [ ] `central_ci.sh` grün end-to-end.
- [ ] **Claude-Externverifikation:** voller Workspace-Build + clippy + alle vier Säulen grün; Konsumenten-Crates mitgebaut.
- [ ] Docs vollständig; CHANGELOG-Eintrag (falls user-sichtbar).
- [ ] Branch `central-postgres-migration` → nach Verifikation Merge nach `main`, push.

---

## Ausführungs-Protokoll (pro Ticket, verbindlich)

1. **Codex baut** (gpt-5.5, xhigh) — exakter Ticket-Scope, „WORKFLOW.md NIE anfassen", deutsche UI-Texte = `"Platzhalter"` + Datei:Zeile.
2. **Frischer Codex-Kritiker** (neuer Kontext) reviewt gegen Test-Vertrag + Global Constraints.
3. **Codex-Rework** bis sauber.
4. **Claude verifiziert extern:** `cargo build`/`clippy` scoped **+ Konsumenten**, Test-Verträge real ausgeführt, Live-/Artefakt-Beweis statt Erfolgs-Log.
5. **Claude committt + pusht** (eigenes Review der `changed_files`), Branch sofort auf `origin`.

## Self-Review (Spec-Abdeckung §9 — vier Säulen)

- Säule 1 (compile-checked Queries) → **T4**.
- Säule 2 (Cross-Boundary-Integration gg. echtes Test-PG) → **T5**.
- Säule 3 (Schema-Drift-Gate) → **T6**.
- Säule 4 (Migrations-Parität + Negativtest mit Zähnen) → **T7** (Ledger/Gates) + die Negativ-Anker in **T2/T4/T6**.
- Spec §5 (Instanz/Schema/Migrations-Owner/sqlx) → **T0/T1/T2/T3/T4**.
- Spec §7 (ETL verlustfrei, Ledger, Aggregat-Gates) → **T7** (Framework; reale ETL ab SP1).
- Spec §3 Leitplanken → Global Constraints + „SP0 ohne Echtdaten/Cutover" (Datenrisiko = null).

## Notizen fürs Sequencing (aus Recon, betrifft spätere SPs — nicht SP0)

- **Steam-Bot teilt `data/deadlock.sqlite3`** mit den Deadlock-Bots (gleiche Datei). SP1 (`steam`-Schema-Quelle) + SP3 überlappen in der Quelle → in SP1-Planung berücksichtigen.
- **Patchnotes-Persistenz minimal:** nur `data/patch_signal_history.ndjson` (6,9 KB) + geteilter DB-Zugriff → SP5 vor allem Code-Port.
- **Website-Coaching-DB:** `Website/builds/backend/deadlock.db` (256 KB); exakter Coaching-DB-Pfad in SP2 final verifizieren (Router nutzt evtl. separate Datei).
