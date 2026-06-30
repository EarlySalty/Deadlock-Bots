# Central DB Architektur

## Instanz

Eine eigene TimescaleDB-Instanz, bewusst getrennt von der Twitch-Analytics-DB (`:5433`) und der TradingBot-DB. Sie lauscht nur auf dem Loopback-Interface — von außen ist nichts erreichbar, der Zugriff läuft ausschließlich lokal über die DSN aus Infisical. Hochgefahren wird sie über die Compose-Datei bzw. den `up.sh`-Wrapper, der das Passwort aus der DSN zieht, ohne es je zu loggen.

| Feld | Wert |
|---|---|
| Container | `deadlock-central-postgres` |
| Image | `timescale/timescaledb:2.17.2-pg16` |
| Host-Bind | `127.0.0.1:5434:5432` |
| Netzwerk-Exposition | loopback-only |
| Datenbank | `deadlock` |
| Rolle | `deadlock` |
| Persistenz | Docker-Volume `deadlock_central_pgdata` |
| Compose | [../../infra/central-db/docker-compose.yml](../../infra/central-db/docker-compose.yml) |
| Start-Wrapper | [../../infra/central-db/up.sh](../../infra/central-db/up.sh) |
| Stop-Wrapper | [../../infra/central-db/down.sh](../../infra/central-db/down.sh) |
| systemd-Unit-Vorlage | [../../infra/central-db/deadlock-central-db.service](../../infra/central-db/deadlock-central-db.service) |
| Infra-Runbook | [../../infra/central-db/README.md](../../infra/central-db/README.md) |

| Secret | Form |
|---|---|
| `DEADLOCK_CENTRAL_DSN` | `postgres://deadlock:<passwort>@127.0.0.1:5434/deadlock` |

## Schema-Layout

Eine Datenbank, ein Schema pro Domäne. `core` ist die geteilte Spine: Sie hält die Identitäten — die Discord-User-ID als BIGINT-Join-Key — und die Verknüpfungen, über die alle Domänen joinen. Dadurch bleiben die Domänen-Schemas entkoppelt: Eine Steam-Tabelle referenziert `core.users`, nicht direkt eine Coaching-Tabelle. SP0 legt nur die Spine an; die Domänen-Schemas werden ab SP1 befüllt. Retention ist aus, nichts wird je gelöscht.

| Schema | Zweck |
|---|---|
| `core` | Geteilte Spine fuer Identitaeten und Cross-Domain-Joins |
| `coaching` | Namespace fuer Coaching-Daten |
| `scrim` | Namespace fuer Scrim-Daten |
| `steam` | Namespace fuer Steam-Daten |
| `turnier` | Namespace fuer Turnier-Daten |
| `patchnotes` | Namespace fuer Patchnotes-Daten |
| `activity` | Namespace fuer Aktivitaetsdaten |

| SP0-Tabelle | Vertrag |
|---|---|
| `core.users` | `discord_id BIGINT PRIMARY KEY`; optionale Discord-Profilfelder; `first_seen TIMESTAMPTZ NOT NULL DEFAULT now()`; `last_seen TIMESTAMPTZ NOT NULL DEFAULT now()`; `raw JSONB` |
| `core.steam_links` | `discord_id BIGINT NOT NULL REFERENCES core.users(discord_id) ON DELETE CASCADE`; `steam_id64 BIGINT NOT NULL`; `verified BOOLEAN NOT NULL DEFAULT false`; `linked_at TIMESTAMPTZ NOT NULL DEFAULT now()`; `PRIMARY KEY (discord_id, steam_id64)` |
| `core.steam_links` Index | `steam_links_steam_id64_idx` auf `steam_id64` |

Quelle: [../../crates/dl-central-db/migrations/0001_core_and_schemas.sql](../../crates/dl-central-db/migrations/0001_core_and_schemas.sql)

## Migrations-Owner

Genau ein Crate besitzt die Migrationen: `dl-central-migrate` bettet sie zur Compile-Zeit per `sqlx::migrate!` ein und ist die einzige Stelle, die sie anwendet. Kein anderer Dienst migriert die zentrale DB — das verhindert konkurrierende oder doppelte Migrationsläufe und hält den Schema-Stand eindeutig. Wer das Schema ändert, legt eine neue Migration unter der Quelle an; der Migrator wendet sie idempotent an.

| Vertrag | Wert |
|---|---|
| Kanonischer Owner | [../../bin/dl-central-migrate](../../bin/dl-central-migrate) |
| Migrationsquelle | [../../crates/dl-central-db/migrations](../../crates/dl-central-db/migrations) |
| Einbettung | `sqlx::migrate!("../../crates/dl-central-db/migrations")` |
| Runtime-DSN | `DEADLOCK_CENTRAL_DSN` |

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust
cargo run -p dl-central-migrate
```

## Offline-Cache

Die `query!`/`query_as!`-Makros werden zur Compile-Zeit gegen den committeten `.sqlx`-Cache geprüft, nicht gegen eine Live-DB. So bleibt der Schema-Vertrag auch im CI- und Offline-Build erzwungen, ohne dass eine Datenbank erreichbar sein muss. Ändert sich eine Query oder das Schema, muss der Cache neu erzeugt und mitcommittet werden — sonst bricht der Offline-Build oder prüft gegen ein veraltetes Schema. Details im Pflege-Runbook.

| Feld | Wert |
|---|---|
| Cache-Pfad | [../../crates/dl-central-db/.sqlx](../../crates/dl-central-db/.sqlx) |
| Pflege-Runbook | [offline-prepare-runbook.md](offline-prepare-runbook.md) |
| Compile-Vertrag | `query!`/`query_as!` bauen mit `SQLX_OFFLINE=true` ohne Live-DB |

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust
env -u DATABASE_URL SQLX_OFFLINE=true cargo build --workspace
```

## ETL und Ledger

SP0 migriert noch keine Echtdaten — es baut nur das Framework für verlustfreie ETL, das ab SP1 die Alt-SQLite-Datenbanken überführt. Kernidee: Jede Quell-Spalte muss im Ledger bewusst als gemappt oder gedroppt erfasst sein, und die Verifikations-Gates brechen ab, sobald etwas still verloren ginge. Die Negativtests belegen, dass diese Gates wirklich greifen — ein grün-gewaschenes Gate fällt im Test durch.

| Feld | Wert |
|---|---|
| ETL-Crate | [../../crates/dl-central-etl](../../crates/dl-central-etl) |
| Ledger-Format | [ledger-format.md](ledger-format.md) |
| Negativtest | [../../crates/dl-central-etl/tests/verify_teeth.rs](../../crates/dl-central-etl/tests/verify_teeth.rs) |
| Fixture: vollstaendig | [../../crates/dl-central-etl/tests/fixtures/example-ledger.toml](../../crates/dl-central-etl/tests/fixtures/example-ledger.toml) |
| Fixture: fehlende Spalte | [../../crates/dl-central-etl/tests/fixtures/missing-column-ledger.toml](../../crates/dl-central-etl/tests/fixtures/missing-column-ledger.toml) |

## Vier-Säulen-CI

`central_ci.sh` bündelt die vier Verifikations-Säulen in einem Lauf: den Compile-Vertrag (Offline-Build gegen den `.sqlx`-Cache), das Schema-Drift-Gate und die Cross-Boundary-Integration gegen je eine frisch migrierte Wegwerf-DB, und die ETL-Negativtests. Die schnellen DB-losen Säulen laufen zuerst (fail-fast ohne Container-Kosten), die Container-Säulen danach. Erst wenn alle vier grün sind, meldet das Skript Erfolg.

| Säule | Mechanismus | Befehl/Script |
|---|---|---|
| 1 | Compile-Vertrag offline, ohne Container | `env -u DATABASE_URL SQLX_OFFLINE=true cargo build --workspace` |
| 1b | Lint offline, ohne Container | `env -u DATABASE_URL SQLX_OFFLINE=true cargo clippy -p dl-central-db -p dl-central-etl -p dl-central-migrate --all-targets -- -D warnings` |
| 4 | ETL-/Ledger-Negativtests, ohne DB | `env -u DATABASE_URL SQLX_OFFLINE=true cargo test -p dl-central-etl` |
| 3 | Schema-Drift-Gate auf frisch migrierter Wegwerf-DB | `./scripts/central_fresh_schema.sh` |
| 2 | Cross-Boundary-Integration auf eigener frisch migrierter Wegwerf-DB | `./scripts/central_test_db.sh cargo test -p dl-central-db --test integration_core -- --ignored` |

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust
./scripts/central_ci.sh
```

Skript: [../../scripts/central_ci.sh](../../scripts/central_ci.sh)
