# SP1 Phase 3 — Consumer-Query-Rewrites (rusqlite → zentrale Postgres) Implementation Plan

> **Für ausführende Worker:** Dieser Plan wird **vollständig an Codex (gpt-5.5 / xhigh) delegiert** — Codex implementiert, verifiziert sich selbst und reviewt sich selbst (frischer Kritiker). Claude orchestriert nur (DAG, commit/push, `changed_files`-Plausi) und schreibt user-sichtbare Texte. KEINE nativen Sub-Agents. Schritte mit Checkbox (`- [ ]`) für Fortschritt.

**Goal:** Alle 13 Consumer-Crates/Binaries von `dl-db`/rusqlite (synchron, SQLite) auf die zentrale Postgres umschreiben — compile-checked via `sqlx::query!`/`query_as!`, `.sqlx`-Offline-Cache, fully-qualified `schema.table`, pro Crate verifiziert. Danach ist der Code PG-only; der atomare Cutover (Dienste umschalten) ist **P4**, nicht Teil von P3.

**Architecture:** Die `dl-db::Db`-Abstraktion (synchrone rusqlite-`read`/`write`-Closures, die jedem Consumer ein rohes `&Connection` durchreichen) wird **ersetzt, nicht umgebogen** (Eltern-Spec-Leitplanke #2). Neue Wahrheit: ein async `sqlx::PgPool`, im Binary einmal via `dl_central_db::connect_pool(dsn_from_env()?)` gebaut und in jeden Service durchgereicht. Jede Methode ruft `sqlx::query!(…).fetch_*(&pool).await` direkt. Der Key-Value-Store wandert auf `bot.kv_store(ns, k, v)`. Tests laufen gegen eine Wegwerf-Postgres (dynamischer Port) statt gegen `Db::open_creating(tempfile.sqlite3)`.

**Tech Stack:** Rust 1.85, sqlx 0.8.6 (postgres/runtime-tokio/tls-rustls/macros/migrate/chrono), TimescaleDB pg16 @ `127.0.0.1:5434`, `DEADLOCK_CENTRAL_DSN` (Infisical, volle DSN-URL), chrono, tokio.

## Global Constraints

Jeder Ticket-Scope erbt diese Regeln implizit — sie werden NICHT pro Ticket wiederholt:

- **EINGEFROREN, nie anfassen:** Schema/Migrationen `0001`–`0011` (`crates/dl-central-db/migrations/`), das Zero-Loss-Ledger (`crates/dl-central-etl/ledger/**`), die ETL-Engine (`crates/dl-central-etl/src/**`). P3 ändert **keine** dieser Dateien.
- **`WORKFLOW.md` NIE anfassen.**
- **Ledger = einzige Wahrheit für Tabelle→Schema + Spalten.** Vor jedem Query: im passenden `ledger/{deadlock-sqlite3,website,tournament}/<domain>.toml` nachschlagen, in welchem PG-**Schema** die Tabelle liegt und wie Spalten **umbenannt/typisiert** wurden. Beispiele aus P2: `kv_store.key→k`, `kv_store.value→v`; bool-Spalten sind echte `BOOLEAN` (Vergleich `= TRUE`, **nicht** `= 1`); Zeitstempel sind `timestamptz` (chrono `DateTime<Utc>`, **kein** String); int-FKs sind `int4`/`int8` (kein stilles i32↔i64-Narrowing).
- **Fully-qualified `schema.table` in JEDEM Query** (z.B. `voice.voice_stats`, `moderation.ai_moderation_cases`, `bot.kv_store`). Kein `search_path`-Magie (verworfen: kollidiert bei gleichnamigen Tabellen über Schemas, fragil über Pool-Connections).
- **Compile-checked Pflicht:** Nur `sqlx::query!`/`query_as!` (beide gegen Live-Schema geprüft). Kein `sqlx::query(`-String-Builder, kein dynamisches SQL ohne Not. Wo dynamisches SQL unvermeidbar (z.B. variable IN-Listen), `QueryBuilder` mit explizitem Typ-Binding + Begründung im Code-Kommentar.
- **Null Datenverlust / treuer Mirror:** Verhalten der alten Queries bleibt erhalten (gleiche Filter, gleiche Upsert-Semantik, gleiche Default-Werte). Kein Feature erfinden, kein Filter ändern.
- **Discord/Steam-IDs:** Rust `u64` ↔ PG `BIGINT` (= i64). Snowflakes passen in i63 → Cast `as i64` beim Bind, `as u64` beim Read, an der Systemgrenze, konsistent (wie P2-ETL).
- **Tests mit Zähnen:** Jeder Consumer-Test läuft gegen echte Wegwerf-PG mit Schema `0001`–`0011`, prüft echtes Verhalten (Round-Trip, Upsert-Idempotenz, Filter-Korrektheit). Keine Fixture-Lügen ([[feedback_tests_catch_prod_breakage]]).
- **Branch-Disziplin:** Arbeit auf `central-postgres-sp1`. Mid-Stream ist `cargo build --workspace` **bewusst rot** (per-Crate-Cutover) — das ist auf einem Feature-Branch erlaubt. Verifikations-Vertrag: **pro Crate** `cargo build -p <crate>` + `cargo test -p <crate>` grün (Teeth pro Commit); **Workspace-grün** ist die Barriere T11 vor jedem Main-Merge/Cutover. `nichts Kaputtes pushen` gilt auf Workspace-Ebene erst ab T11.
- **`.unwrap()` raus aus Produktionspfaden**, Fehler via `thiserror`/`?`. Eigene ungenutzte Imports/`rusqlite`-Reste pro Crate entfernen.

## Arbeitsmethode (pro Ticket, hart)

1. **Codex implementiert** (gpt-5.5/xhigh) im Haupt-Tree, **sequentiell** (nie 2 Worker im selben Tree — Cargo-/`.sqlx`-Lock-Races).
2. **Codex verifiziert sich selbst:** die unten je Ticket gelisteten Verify-Kommandos laufen lassen, Output beweisen (nicht behaupten).
3. **Frischer Codex-Kritiker** (separater Worker, nicht der Implementierer) scannt mit der **Kritiker-Checkliste** (unten) — ALLE geänderten Dateien der Crate, jede Runde.
4. **Codex-Rework** bis Kritiker sauber meldet (W5-Lektion: 167-/143-Site-Crates brauchen mehrere Runden — einplanen).
5. **Claude** prüft `changed_files`-Plausi (Scope, kein Schema/Ledger/WORKFLOW.md angefasst), committet + pusht sofort. Ein Ticket = ein verifizierter Commit.

---

## Datei-/Architektur-Struktur (Entscheidungen, locked)

**`dl-central-db` (erweitert in T0 — die neue Wurzel):**
- `crates/dl-central-db/src/kv.rs` (NEU): async KV gegen `bot.kv_store(ns, k, v)`. Freie Funktionen im Stil von `core_users::{get_user,upsert_user}` (nehmen `&PgPool`).
- `crates/dl-central-db/src/testing.rs` (NEU, hinter Feature `testing`): `test_pool()` → Wegwerf-PG (dyn. Port) + Migrationen `0001`–`0011` + Teardown-Guard.
- `crates/dl-central-db/src/pool.rs` (bestehend): `connect_pool`, `dsn_from_env` — unverändert wiederverwendet.

**Pro Consumer-Crate (T1–T10):**
- `Cargo.toml`: `rusqlite` raus, `dl-db` raus, `dl-central-db.workspace = true` + `sqlx.workspace = true` rein; `[features] testing = ["dl-central-db/testing"]` + `[dev-dependencies]` für Tests.
- Service-Structs: Feld `db: Db` → `pool: sqlx::PgPool`; Konstruktor `new(db: Db)` → `new(pool: sqlx::PgPool)`.
- Methoden: `db.read(|conn| conn.query_row(…))` → `sqlx::query_as!(…).fetch_optional(&self.pool).await?`; `db.write(|conn| conn.execute(…))` → `sqlx::query!(…).execute(&self.pool).await?`. KV-Aufrufe → `dl_central_db::kv::*`.
- Tests: `Db::open_creating(tempfile)` → `dl_central_db::testing::test_pool()`; Test-Fn werden `async` (`#[tokio::test]`), `#[ignore]`-fähig wenn Test-DB-Env fehlt (Skip-if-absent), aber in `central_ci.sh` hart gefordert.

**Binaries (T11 — die Barriere):**
- `bin/dl-bot/src/main.rs:247`, `bin/dl-web/src/main.rs:17`, `bin/dl-twitch-invite-sync/src/main.rs:107`, `bin/dl-bot/src/bin/seed_scrim.rs:10`, `bin/dl-bot/src/onboardglue.rs:185` (Feld `db`), `bin/dl-bot/src/build_publisher.rs`: `dl_db::Db::open(&cfg.db_path)` → `dl_central_db::connect_pool(&dl_central_db::dsn_from_env()?).await?`; `PgPool` (Clone/Arc) in alle Service-Konstruktoren durchreichen.
- `crates/dl-db/**` löschen, aus `rust/Cargo.toml`-`members` entfernen, `rusqlite` aus `[workspace.dependencies]` entfernen.

---

## DAG (Wellen)

```
T0  Foundation (dl-central-db: KV + test_pool)         [serial, Wurzel]
        │
        ▼   Welle 1 (Blätter, Reihenfolge = depended-upon zuerst)
T1 dl-discord(3) → T2 dl-squads(13) → T3 dl-bridges(6) → T4 dl-stats(30)
   → T5 dl-tierlist(33) → T6 dl-moderation(18) → T7 dl-activity(74)
        │
        ▼   Welle 2
T8  dl-community(167)   [Tentpole — mehrere Kritiker-Runden einplanen]
        │
        ▼   Welle 3
T9 dl-dashboard(72) → T10 dl-voice(143)
        │
        ▼   Welle 4
T11 Binary-Wiring + cargo sqlx prepare --workspace + central_ci.sh
    + dl-db löschen + rusqlite raus                    [serial, Barriere]
```

Reihenfolge-Begründung: `dl-discord` zuerst (5 Crates hängen dran); innerhalb der Wellen Blatt-zuerst, damit `cargo build -p <crate>` bei Ticket-Abschluss grün ist (Geschwister-Konstruktoren bereits konvertiert). `dl-community`/`dl-voice` sind je EIN atomares Ticket (eine Crate kann nicht halb-konvertiert bauen).

---

## Voraussetzungs-Gate (vor T0, Codex prüft)

- [ ] **Live-Oracle prüfen:** `DEADLOCK_CENTRAL_DSN` (Infisical) zeigt auf eine PG mit **allen** Schemas + Tabellen aus `0001`–`0011` (compile-time-Oracle für `query!`). Prüfen via Count-Query (Anzahl Schemas / Tabellen), **niemals Secret-Komponenten ausgeben** ([[feedback_no_secret_component_echo]] — nur Booleans/Zahlen printen). Fehlt etwas → `cargo run -p dl-central-migrate` gegen :5434, dann re-prüfen.
- [ ] **`.sqlx`-Cache-Ort bestätigen:** wo liegt der Offline-Cache (`rust/.sqlx/` vs. per-Crate), wie regeneriert `cargo sqlx prepare --workspace`? (Runbook: `docs/central-db/offline-prepare-runbook.md`.) `sqlx-cli` installiert? Falls nicht: installieren.

---

## Task T0: Foundation — async KV + Test-Harness in `dl-central-db`

**Files:**
- Create: `crates/dl-central-db/src/kv.rs`
- Create: `crates/dl-central-db/src/testing.rs`
- Modify: `crates/dl-central-db/src/lib.rs` (Module + Re-Exports + `CentralDbError`-Varianten)
- Modify: `crates/dl-central-db/Cargo.toml` (`[features] testing = []`; Test-Deps)
- Test: `crates/dl-central-db/tests/kv_roundtrip.rs`

**Interfaces — Produces (von allen späteren Tickets genutzt):**
- `dl_central_db::kv::get(pool: &PgPool, ns: &str, key: &str) -> Result<Option<String>, CentralDbError>`
- `dl_central_db::kv::set(pool: &PgPool, ns: &str, key: &str, value: &str) -> Result<(), CentralDbError>` (Upsert auf PK `(ns,k)`)
- `dl_central_db::kv::delete(pool: &PgPool, ns: &str, key: &str) -> Result<(), CentralDbError>`
- `#[cfg(feature = "testing")] dl_central_db::testing::test_pool() -> Result<TestDb, CentralDbError>` mit `TestDb { pool: PgPool }` (Deref zu `PgPool`) + `Drop` der den Wegwerf-Container/das Test-Schema abräumt.

**Consumes:** `pool.rs` (`connect_pool`), `migrations/` (via `sqlx::migrate!("./migrations")`, nur für Test-DBs — `dl-central-migrate` bleibt einziger Prod-Migrator).

- [ ] **Step 1: KV-Round-Trip-Test schreiben (failing).** `tests/kv_roundtrip.rs`: `test_pool()` holen, `kv::get` (None) → `kv::set("voice","x","1")` → `kv::get` (Some("1")) → `kv::set` überschreibt → `kv::delete` → `kv::get` (None). `#[tokio::test]`, `#[ignore]`-Skip wenn Test-DB-Env fehlt.
- [ ] **Step 2: Test laufen, Fehlschlag beweisen** (`kv`-Modul existiert nicht). Run: `cargo test -p dl-central-db --features testing kv_roundtrip -- --include-ignored`. Erwartet: Compile-Fehler / FAIL.
- [ ] **Step 3: `kv.rs` + `testing.rs` implementieren.** KV gegen `bot.kv_store(ns,k,v)` mit `query!`/`query_as!`; `set` = `INSERT … ON CONFLICT (ns,k) DO UPDATE SET v = EXCLUDED.v`. `test_pool` ruft `central_test_db.sh` (dyn. Port) oder erzeugt ein eindeutiges Test-Schema, dann `sqlx::migrate!("./migrations").run(&pool)`.
- [ ] **Step 4: Test grün.** Run: `DATABASE_URL=$DEADLOCK_CENTRAL_DSN cargo test -p dl-central-db --features testing -- --include-ignored`. Erwartet: PASS.
- [ ] **Step 5: prepare + clippy + fmt.** `cargo sqlx prepare -p dl-central-db` (DATABASE_URL gesetzt) → `.sqlx` aktualisiert; `cargo clippy -p dl-central-db --all-features -- -D warnings`; `cargo fmt --check`.
- [ ] **Step 6: Kritiker + Commit.** Frischer Codex-Kritiker (Checkliste). Dann Claude: `feat(sp1): T0 — async KV (bot.kv_store) + test_pool-Harness in dl-central-db`.

---

## Tasks T1–T10: Consumer-Crate-Rewrites

**Einheitliches Muster** (gilt für jedes Crate-Ticket; Ledger-Domain je Crate unten):

**Files (pro Crate):** alle `src/*.rs` mit `rusqlite`/SQL-Literalen (siehe Site-Zahlen), `Cargo.toml`, vorhandene `tests/` + `#[cfg(test)]`-Module.

**Interfaces — Produces:** Service-Konstruktoren `new(pool: sqlx::PgPool, …)` (statt `Db`). **Consumes:** `dl_central_db::{connect_pool, kv, testing}`, T0.

- [ ] **Step 1 — Ledger lesen:** für jede Tabelle der Crate das Ziel-Schema + Spalten-Mapping aus `ledger/<source>/<domain>.toml` ziehen. Mapping-Tabelle (alt-SQLite-Spalte → PG-Schema.Tabelle.Spalte+Typ) als Arbeitsnotiz.
- [ ] **Step 2 — Tests zuerst auf PG umstellen (Red):** bestehende Crate-Tests von `Db::open_creating` auf `dl_central_db::testing::test_pool()` + `#[tokio::test]` umschreiben; sie schlagen fehl (Service nimmt noch `Db`).
- [ ] **Step 3 — Queries umschreiben (Green):** `read/write`-Closures → `sqlx::query!/query_as!`-Aufrufe (fully-qualified, Typ-treu nach Ledger, bool=`TRUE`, timestamptz=`DateTime<Utc>`, id-Casts). `rusqlite`/`dl-db`-Imports raus.
- [ ] **Step 4 — Verifizieren (Teeth):**
  - `DATABASE_URL=$DEADLOCK_CENTRAL_DSN cargo build -p <crate>` (query! gegen Live-Schema)
  - `cargo sqlx prepare -p <crate>` (Offline-Cache)
  - `DATABASE_URL=$DEADLOCK_CENTRAL_DSN cargo test -p <crate> --features testing -- --include-ignored` (Verhalten gegen echte PG)
  - `cargo clippy -p <crate> --all-features -- -D warnings` + `cargo fmt --check`
- [ ] **Step 5 — Kritiker-Runden** (Checkliste, frischer Worker, ALLE geänderten Dateien) bis sauber.
- [ ] **Step 6 — Commit/Push:** `feat(sp1): T<n> — <crate> rusqlite→zentrale Postgres`.

| Ticket | Crate | SQL-Sites | Ledger-Domain(s) | KV? | Notiz |
|---|---|---|---|---|---|
| T1 | dl-discord | 3 | bot/content | – | zuerst (5 Crates hängen dran) |
| T2 | dl-squads | 13 | scrim | – | Blatt |
| T3 | dl-bridges | 6 | steam/core | ja (`steam.rs`) | Blatt |
| T4 | dl-stats | 30 | activity/voice/core | – | Blatt |
| T5 | dl-tierlist | 33 | tierlist | – | Blatt; Quelle deadlock **und** website (Union beachten) |
| T6 | dl-moderation | 18 | moderation | – | `ai_moderation_cases` |
| T7 | dl-activity | 74 | activity | – | `lfg`/`text_stats`; CSV→Array-Spalten (P2-Befund `co_participant_ids`) |
| T8 | dl-community | 167 | coaching/core/content/bot | ja (5 Dateien) | **Tentpole**, mehrere Runden |
| T9 | dl-dashboard | 72 | content/bot/activity | ja (`deadlock.rs`) | `deadlock_hero_builds` Replace-Semantik |
| T10 | dl-voice | 143 | voice/bot | ja (3 Dateien) | zweitgrößte; `tempvoice`-Submodul |

> Coaching-Tabellen (T8) liegen als **Union** in beiden Quellen (deadlock-sqlite3 + website) — Ledger beider Quell-Verzeichnisse prüfen; Spaltenvertrag = zentrales Schema `0004`. Tierlist (T5) analog.

---

## Task T11: Barriere — Binary-Wiring, Workspace-Grün, dl-db-Abriss

**Files:** `bin/dl-bot/src/{main.rs,onboardglue.rs,build_publisher.rs,bin/seed_scrim.rs}`, `bin/dl-web/src/main.rs`, `bin/dl-twitch-invite-sync/src/main.rs`, `rust/Cargo.toml` (`members` − `dl-db`, `[workspace.dependencies]` − `rusqlite`), löschen: `crates/dl-db/**`.

- [ ] **Step 1:** Binaries: `Db::open` → `connect_pool(dsn_from_env())`; `PgPool` in alle Service-`new(...)` durchreichen. `db`-Felder/Typen anpassen.
- [ ] **Step 2:** `cargo sqlx prepare --workspace` (DATABASE_URL=$DEADLOCK_CENTRAL_DSN) → vollständiger `.sqlx`-Cache; committen.
- [ ] **Step 3 — Workspace-Grün (Teeth):**
  - `SQLX_OFFLINE=true cargo build --workspace` (offline, ohne Live-DB — beweist Cache vollständig)
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `bash scripts/central_ci.sh` (alle vier Säulen grün)
  - `cargo fmt --check`
- [ ] **Step 4 — Abriss:** `crates/dl-db/` löschen, aus `members` raus, `rusqlite` aus `[workspace.dependencies]` raus; `grep -r rusqlite crates/ bin/` == leer. Erneut `SQLX_OFFLINE=true cargo build --workspace` grün.
- [ ] **Step 5 — Kritiker + Commit:** `feat(sp1): T11 — P3 abgeschlossen; alle Consumer auf zentrale Postgres, dl-db/rusqlite entfernt`. Memory-Update + Übergabe an P4.

---

## Kritiker-Checkliste (jede Runde, ALLE geänderten Dateien der Crate)

1. **Schema-Qualifikation:** Jede Tabelle `schema.table` und das Schema stimmt mit Ledger/Migration. Kein unqualifizierter Tabellenname.
2. **Spalten-Treue:** Spaltennamen = zentrales Schema (umbenannte Spalten berücksichtigt, z.B. `k`/`v`). Keine alt-SQLite-Namen.
3. **Typ-Treue (P2-Bug-Klasse):** bool → `= TRUE`/`BOOLEAN` (nicht `=1`); int-Breite (i32 vs i64, kein stilles Narrowing an int4/int8-FK → `try_from` + `?`); `timestamptz` → `DateTime<Utc>` (kein String-Bind in dyn-SQL); nullable → `Option<T>` + `query!`-Override `col!`/`col?` korrekt; id-Cast `u64↔i64` konsistent.
4. **Verhaltens-Treue:** gleiche WHERE/ORDER/LIMIT, gleiche Upsert/Replace-Semantik (ON CONFLICT-Ziel = echter PK/Unique), gleiche Defaults; keine Zeile mehr/weniger betroffen als vorher.
5. **Compile-checked:** nur `query!`/`query_as!`; dyn-SQL nur mit Begründung + getypten Binds.
6. **Tests mit Zähnen:** laufen gegen echte PG, prüfen Round-Trip/Idempotenz/Filter — keine Fixture, die das echte Schema nicht trifft; Negativ-Pfad mind. 1×.
7. **Sauberkeit:** kein `rusqlite`/`dl-db`-Rest-Import, kein `.unwrap()` im Prod-Pfad, kein toter Code; nichts Eingefrorenes (Schema/Ledger/`0001`–`0011`/WORKFLOW.md) berührt.
8. **`.sqlx` aktualisiert:** Cache für die neuen Queries vorhanden (sonst bricht offline-Build der Barriere).

---

## Verifikations-Vertrag (Zusammenfassung)

- **Pro Crate (T1–T10):** `build -p` + `sqlx prepare -p` + `test -p --features testing` + `clippy -p -D warnings` + `fmt --check` — ALLE grün, Output bewiesen, bevor commit.
- **Barriere (T11):** `SQLX_OFFLINE=true cargo build --workspace` + `clippy --workspace --all-targets -D warnings` + `central_ci.sh` (4 Säulen) + `rusqlite`-grep leer. Erst danach ist der Workspace wieder als Ganzes grün und P4-fähig.
- **Geheimnisse:** DSN nie ausgeben; Diagnose nur Booleans/Counts.
- **Rollback (für P4, nicht P3):** vorheriges Release-Binary + unangetastete SQLite-Snapshots = operativer Rollback; KEIN Code-Dual-Pfad im neuen Build.

---

## Self-Review (gegen Eltern-Spec + SP1-Entscheidungen)

- **Leitplanke #1 (null Datenverlust):** P3 schreibt nur Queries um, migriert keine Daten (das war P2); Verhaltens-Treue durch Kritiker-Punkt 4 + Verhaltens-Tests. ✓
- **Leitplanke #2 (neu schreiben, nicht umbiegen):** `Db`-Closure-Abstraktion ersetzt durch direkten `PgPool` + `query!`; kein rusqlite-Muster nachgebaut. ✓
- **Leitplanke #4 (Tests mit Zähnen):** Wegwerf-PG + echtes Schema, Round-Trip-Asserts. ✓
- **Leitplanke #5 (Cutover + Rollback):** P3 produziert PG-only-Code; Cutover/Rollback explizit P4. ✓
- **„pro Crate verifiziert":** je Ticket eigene `-p`-Teeth + ein Commit. ✓
- **Eingefroren:** Schema/Ledger/Migrationen/WORKFLOW.md unangetastet (Kritiker-Punkt 7 + Claude-`changed_files`-Plausi). ✓
- **Delegation:** alles Codex, Claude orchestriert/committet. ✓
- **Offene Annahme (in T0-Gate aufgelöst):** Live-:5434 trägt `0001`–`0011` als Compile-Oracle; `.sqlx`-Cache-Ort bestätigt.

## Execution Handoff

Plan gespeichert: `rust/docs/plans/2026-06-30-sp1-phase3-consumer-rewrites.md`. Ausführung = **Codex-Delegation** (Hausmethode, nicht native Subagents): T0 zuerst (Wurzel, serial), dann Wellen 1–3 sequentiell pro Crate (impl → frischer Kritiker → rework → Claude verify+commit+push), T11 als Barriere. Start: Voraussetzungs-Gate + T0.
