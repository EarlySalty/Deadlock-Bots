# Central DB sqlx Offline Prepare

## Zweck

Der `.sqlx`-Offline-Cache erlaubt es, die `query!`/`query_as!`-Makros zu kompilieren, ohne dass beim Build eine Live-Datenbank erreichbar ist. sqlx prüft die Queries zur Compile-Zeit gegen den committeten Cache (`crates/dl-central-db/.sqlx/*.json`) statt gegen `DATABASE_URL`. So bleiben CI- und Offline-Builds reproduzierbar und trotzdem schema-geprüft — der Schema-Vertrag wird zur Compile-Zeit erzwungen, nicht erst zur Laufzeit.

## Wann laufen lassen

Immer wenn sich eine `query!`/`query_as!`-Query **oder** das Schema (eine Migration) ändert. Der Cache muss dann neu erzeugt und mitcommittet werden, sonst schlägt der Offline-Build fehl oder prüft gegen ein veraltetes Schema. Faustregel: Query oder Migration angefasst → `cargo sqlx prepare` → `.sqlx` mitcommitten (per `git add -f`, siehe unten).

## Voraussetzungen

```bash
cargo install sqlx-cli --version ^0.8 --no-default-features --features postgres,rustls
```

```bash
docker run --rm --name dl-central-sqlx-prepare \
  -e POSTGRES_DB=deadlock \
  -e POSTGRES_USER=deadlock \
  -e POSTGRES_PASSWORD=deadlock_prepare_pw \
  -p 127.0.0.1:5505:5432 \
  -d timescale/timescaledb:2.17.2-pg16
```

```bash
until pg_isready -h 127.0.0.1 -p 5505 -U deadlock -d deadlock; do sleep 1; done
```

## Migration anwenden

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust
sqlx migrate run \
  --source crates/dl-central-db/migrations \
  --database-url postgres://deadlock:deadlock_prepare_pw@127.0.0.1:5505/deadlock
```

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust
DATABASE_URL=postgres://deadlock:deadlock_prepare_pw@127.0.0.1:5505/deadlock \
DEADLOCK_CENTRAL_DSN=postgres://deadlock:deadlock_prepare_pw@127.0.0.1:5505/deadlock \
  cargo run -p dl-central-migrate
```

## Offline-Cache erzeugen

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-central-db
DATABASE_URL=postgres://deadlock:deadlock_prepare_pw@127.0.0.1:5505/deadlock \
  cargo sqlx prepare -- --lib --tests
```

```bash
git status --short .sqlx
```

```bash
git status --short --ignored .sqlx
git add -f .sqlx/*.json
```

## Offline-Build prüfen

```bash
cd /home/naniadm/Documents/Deadlock-Bots/rust
SQLX_OFFLINE=true cargo build -p dl-central-db
```

## CI-Vertrag

CI baut ausschließlich mit `SQLX_OFFLINE=true` und erwartet einen aktuellen, committeten `.sqlx`-Cache. Ein veralteter Cache wird mit `cargo sqlx prepare --check` erkannt (Teil der Pipeline) und bricht den Build, statt still gegen ein falsches Schema zu prüfen. Der Cache liegt unter `crates/dl-central-db/.sqlx/` und ist trotz des globalen `*.json`-Ignores per Negationsregel in `.gitignore` eingecheckt — bei jeder Query-/Schema-Änderung also `prepare` laufen lassen und die aktualisierten `.sqlx/*.json` mitcommitten.

## Aufräumen

```bash
docker rm -f dl-central-sqlx-prepare
```
