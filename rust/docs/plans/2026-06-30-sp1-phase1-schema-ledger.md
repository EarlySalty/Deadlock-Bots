# SP1 Phase 1 — Unified Schema + Ledger (Implementierungsplan)

> **Für Codex-Worker:** Dies ist der Orchestrierungs-Plan. Jedes Ticket wird von **einem Codex-Worker** implementiert, der seine DoD **selbst verifiziert** (Gates/Tests selbst ausführen) und **selbst reviewt** (frischer Codex-Kritiker im Anschluss). Claude organisiert nur (DAG, Datei-Ownership, commit/push, `changed_files`-Plausibilität) und schreibt ausschließlich user-sichtbare Texte. Kein Claude-Implementierungscode, kein Claude-Review, keine Claude-Verifikation.

**Goal:** Ein einziges kanonisches Postgres-Schema, das **alle 178 unique Quell-Tabellen** (deadlock.sqlite3 124 / tournament.db 38 / Website deadlock.db 21, physisch dedupliziert) abdeckt, plus ein **vollständiges TOML-Ledger** (jede Quellspalte `mapped{to}` oder `dropped{reason}`) als Null-Datenverlust-Vertrag.

**Architektur:** Migrations als kanonische Kette unter `rust/crates/dl-central-db/migrations/` (Owner: `dl-central-migrate`). Ledger als TOML unter `rust/crates/dl-central-etl/` (bestehender `Ledger`-Loader, `deny_unknown_fields`). Verifikation über den bestehenden SP0-ETL-Harness (`check_mapping_completeness`, `sample_round_trip`, Drift-Gate `central_fresh_schema.sh`). **Phase 1 erzeugt NUR Schema + Ledger — KEINE Datenbewegung (das ist P2 ETL) und KEINE Query-Rewrites (das ist P3).**

**Tech-Stack:** Postgres/TimescaleDB :5434, sqlx 0.8.6 (migrate!), Rust ETL-Crate `dl-central-etl`, TOML-Ledger, Bash-Gates.

**Quell-Wahrheit für Design:** `rust/docs/_work/sp1/data-landscape.md` (Codex-erstellt, Kritiker-freigegeben) — §2 Tabelle×Projekt×proposed_schema-Matrix, §3 Cross-DB-Konflikte (#387), §4 Namespace-Zuordnung der OFFEN-Tabellen. **Codex implementiert gegen die eigene, freigegebene Discovery — nicht gegen neu erfundene Annahmen.**

---

## Global Constraints

Jedes Ticket erbt diese Regeln implizit:

- **Erst ALLES migrieren, inkl. toter Tabellen** (kein Drop in SP1). Tote Tabellen (`tot=ja`) bekommen Schema + Ledger wie alle anderen; der Drop passiert separat NACH dem Rebuild (#386).
- **`changelog_entries` + `steam_rank_history` dauerhaft behalten** (#385) — als reguläre Tabellen modellieren, nicht als tot behandeln.
- **Null Datenverlust:** Jede Spalte jeder Quell-Tabelle ist im Ledger entweder `status="mapped" to="<schema>.<table>.<column>"` oder `status="dropped" reason="<konkrete Begründung>"`. `check_mapping_completeness` muss für jede Quell-DB grün sein (0 unaccounted columns).
- **Sauberer Neuentwurf:** Postgres-idiomatische Typen (TIMESTAMPTZ statt Unix-INTEGER, BOOLEAN statt 0/1, JSONB statt Text-JSON, BIGINT für Discord-IDs/Snowflakes). Quell-SQLite-Schema NICHT 1:1 nachbiegen; die Konvertierung deckt der bestehende `convert::*`-Layer in P2 ab.
- **Compile-checked Queries sind P3, nicht P1.** Phase 1 = reine DDL-Migrations + Ledger + Schema-Tests. Keine `query!`-Rewrites der Consumer-Crates.
- **Namespace-Set (Eltern-Spec §5.2 + Discovery-Erweiterung §4):** Basis `core`, `coaching`, `scrim`, `steam`, `turnier`, `patchnotes`, `activity` + neu `voice`, `tierlist`, `moderation`, `bot`, `clips`, `content`. Die §5.2-Erweiterung ist als GH-Issue dokumentiert (Entscheidungs-Record).
- **`deny_unknown_fields`** im Ledger bleibt aktiv. Drop-Begründungen müssen konkret sein (kein leerer/generischer Text — der Loader lehnt leere ab).
- **`WORKFLOW.md` NIEMALS anfassen.** User-sichtbare Strings: nur `"Platzhalter"` + Datei:Zeile zurückmelden, Claude finalisiert. (In reinen DDL/Ledger-Tickets entstehen keine.)
- **Datei-Ownership ist verbindlich:** Jedes Ticket fasst NUR die ihm zugewiesenen Dateien an. Kein zwei parallele Tickets schreiben dieselbe Datei (Migrations-Nummern + Ledger-Fragmente sind vorab vergeben).

---

## Namespace → Tabellen (aus data-landscape.md §2/§4)

Maßgeblich ist data-landscape.md. Übersicht der Cluster (Tabellenzahl ≈):

| Namespace | Quelle(n) | ~Tabellen | Kern-Inhalt |
|---|---|---|---|
| `core` | sqlite3, website | users (SP0) + `steam_links` (#387-Union), `meta_users`, `user_privacy` | Geteilter Spine, Identität, Privacy |
| `steam` | sqlite3 (shared bot+steam) | 25 | Steam-Links/Rank/Builds/Presence/Party (Steam-Bot-Hälfte) |
| `coaching` | sqlite3 + website | 8 + 11 + #387-Union | Coaching-Plattform; `requests`/`coaches`/`coach_applications` kanonisch vereinheitlicht |
| `scrim` | sqlite3 | 4 | Scrim-Programm |
| `voice` | sqlite3 | 18 | TempVoice/Voice-State/Rename-Queue/Voice-Stats |
| `tierlist` | sqlite3 + website | 11 + 6 (`meta_*`) | Hero/Build/Tierlist-Content + Website-Meta-Builds |
| `bot` | sqlite3 | 26 | Discord-/FAQ-/OAuth-/Invite-/Notification-Infra, `kv_store`, `changelog_entries` |
| `clips` | sqlite3 | 9 | Clip-Contests/Templates/Submissions |
| `turnier` | tournament.db | 37 | Turnier-Engine |
| `activity` | sqlite3 | 11 + `member_events` | Event-/Zeitreihen |
| `moderation` | sqlite3 | 3 | AI-Moderation, SecurityGuard |
| `patchnotes` | sqlite3 + website | `changelog_posts`, `deadlock_changelogs`, `meta_patch_notes` | Patchnotes-Runtime (geteilte DB!) |
| `content` | website | `meta_announcements`, `meta_reports` | Website-Admin/Reports |

`_sqlx_migrations` (tournament.db) = Tooling-Bookkeeping, KEINE Fachtabelle → im Ledger als `dropped{reason="sqlx-Tooling-Bookkeeping, keine Fachdaten"}`, nicht ins Zielschema.

---

## Zwei Design-Knoten (in T1 zu lösen, gelten für alle Tickets)

**K1 — Ledger-Keying über Quell-DBs.** Der `Ledger`-Loader keyt flach `tables.<name>`. Aber `coaching_requests`, `coaches`, `coach_applications` existieren physisch in `deadlock.sqlite3` UND Website `deadlock.db` mit unterschiedlichen Spaltensätzen; `steam_links` ist EINE physisch geteilte Tabelle (kein Konflikt, ein Eintrag). Codex muss in T1 ein **quell-qualifiziertes Keying** etablieren (z. B. ein Ledger-Fragment **pro Quell-DB** — `ledger/deadlock-sqlite3/…`, `ledger/tournament/…`, `ledger/website/…` — wo Tabellennamen je DB unique sind; cross-DB-Gleichnamige landen in getrennten Dateien), und den Completeness-Gate **pro Quell-DB** gegen das jeweils zuständige Ledger laufen lassen. Mechanik wählt Codex; DoD: kein gleichnamiges Tabellenpaar aus zwei Quell-DBs überschreibt sich, beide werden vollständig verifiziert.

**K2 — Parallelität vs. geordnete Migrations-Kette.** Migrations werden numerisch geordnet angewendet. Jedes Ticket besitzt **eine feste Migrations-Nummer** (unten zugewiesen) und **eigene Ledger-Fragment-Dateien** — so schreiben parallele Worker nie dieselbe Datei. Cross-Namespace-FKs (selten) werden NICHT in den Namespace-Migrations gesetzt, sondern gesammelt im Barrier-Ticket T10 nach Existenz aller Schemas. Namespace-Tabellen dürfen auf `core` FK-en (core wird in 0002 zuerst angewendet).

---

## DAG

```
T1 (Foundation, 0002) ──┬─> T2 steam      (0003)
                        ├─> T3 coaching+scrim (0004)
                        ├─> T4 voice       (0005)
                        ├─> T5 tierlist     (0006)
                        ├─> T6 bot          (0007)
                        ├─> T7 clips        (0008)
                        ├─> T8 turnier      (0009)
                        └─> T9 activity+moderation+content+patchnotes (0010)
                                   │
   (alle T2..T9 fertig) ──────────┴─> T10 Barrier (0011 cross-FK + Voll-Completeness + Drift)
```

T2..T9 laufen **parallel** nach T1 (Owner-Wunsch: nicht warten). T10 ist die Barriere.

---

## T1 — Foundation: Schemas + core-Erweiterung + Ledger/Gate-Infrastruktur

**Blockt:** T2..T10.

**Files (Ownership):**
- Create: `rust/crates/dl-central-db/migrations/0002_sp1_schemas_and_core.sql`
- Modify: `rust/crates/dl-central-etl/src/ledger.rs` (falls Keying-Erweiterung nötig — Codex entscheidet), `rust/crates/dl-central-etl/src/verify.rs` (per-Quell-DB-Completeness)
- Create: `rust/crates/dl-central-etl/ledger/` (Verzeichnisstruktur pro Quell-DB, K1)
- Create/Modify: Gate-Wiring (z. B. `rust/scripts/central_*` oder ETL-Test), das pro Quell-DB das zuständige Ledger gegen das echte Quell-Schema completeness-prüft.

**Scope:**
1. Migration `0002`: `CREATE SCHEMA IF NOT EXISTS` für `voice`, `tierlist`, `moderation`, `bot`, `clips`, `content`. (Die 7 Basis-Schemas existieren aus SP0-0001.)
2. `core.steam_links` auf die kanonische Union-Form (#387, data-landscape.md §3) erweitern: `discord_id BIGINT` (Alias `user_id`), `steam_id TEXT NOT NULL`, optional `steam_id64 BIGINT` (nur wenn parsebar), `steam_display_name`, `verified BOOL`, `primary_account BOOL`, `linked_at`/`updated_at`/`migrated_at TIMESTAMPTZ`, `legacy_ref TEXT`, `deadlock_rank/_subrank/_badge_level INTEGER`, `deadlock_rank_name TEXT`, `deadlock_rank_updated_at TIMESTAMPTZ`, `is_steam_friend BOOL`. PK/Unique/Owner-Guard-Semantik aus §3 erhalten.
3. `core.meta_users` (Website-Auth-Identität, §4) + `core.user_privacy` (querschnittliche Privacy/Deletion-Präferenz, §4) anlegen.
4. K1 lösen: quell-qualifiziertes Ledger-Keying + per-Quell-DB-Completeness-Gate.

**Definition of Done (Codex verifiziert selbst):**
- `0001`+`0002` wenden idempotent auf frische :5434-Instanz an (Wegwerf-Container via `central_test_db.sh`); 13 Schemas + erweiterte `core.steam_links` + `core.meta_users` + `core.user_privacy` live.
- Ledger-Loader lädt die Verzeichnisstruktur, `validate()` grün; ein Negativ-Test beweist, dass zwei gleichnamige Tabellen aus zwei Quell-DBs getrennt erfasst werden (Zähne: künstlich eine Quellspalte aus dem Ledger entfernen → Gate FAIL).
- `cargo build`/Tests des ETL-Crates grün; `SQLX_OFFLINE=true`-Build unberührt.
- Frischer Codex-Kritiker: FREIGABE.

---

## T2 — `steam` (0003)

**Files:** Create `migrations/0003_steam.sql` + Ledger-Fragmente der `steam`-Tabellen (Quell-DB `deadlock.sqlite3`).

**Scope:** Die 25 `steam`-Tabellen aus §2 (Steam-Bot-Hälfte der geteilten DB: Links/Rank/Builds/Presence/Party/Heartbeat-State). `CREATE TABLE` in Schema `steam`, idiomatische PG-Typen. `steam_rank_history` behalten (#385). Builds-/Hero-Tabellen, die §4 dem `tierlist`-Namespace zuordnet (`deadlock_hero_builds`, `deadlock_heroes`, `hero_build_*`, `watched_build_authors`), gehören NICHT hierher — die macht T5. Jede Quellspalte → Ledger `mapped`/`dropped`.

**DoD:** `0003` wendet auf frische :5434 (nach 0001/0002) an; `check_mapping_completeness` für die `steam`-Slice grün; Round-Trip-Sample-Test über ≥1 Tabelle; Codex-Kritiker FREIGABE.

---

## T3 — `coaching` (+ `scrim`) (0004) — enthält #387-Kern

**Files:** Create `migrations/0004_coaching_scrim.sql` + Ledger-Fragmente (Quell-DBs `deadlock.sqlite3` UND Website `deadlock.db`).

**Scope:**
- **#387-Vereinheitlichung** (data-landscape.md §3) — der schwierigste Teil:
  - `coaching.requests`: kanonischer `request_uid TEXT PRIMARY KEY`; `bot_request_id INTEGER UNIQUE NULL` + `website_request_id TEXT UNIQUE NULL`; **Spalten-Union** aus Bot- und Website-`coaching_requests`. Beide Quell-`coaching_requests` (sqlite3 + website) mappen ihre Spalten in diese Zieltabelle (K1: getrennte Quell-Ledger-Fragmente, gleiches Ziel).
  - `coaching.coaches`: Spalten-Union (Website = operative Wahrheit, §3); `discord_user_id BIGINT UNIQUE`.
  - `coaching.coach_applications`: Spalten-Union; `reviewed_by TEXT` kanonisch.
- Restliche `coaching`-Tabellen (Bot 8 + Website 11 minus die obigen) anlegen.
- `scrim` (4 Tabellen aus §2) in Schema `scrim` anlegen.

**DoD:** Migration wendet an; Completeness für ALLE coaching/scrim-Quelltabellen aus BEIDEN Quell-DBs grün (K1-Beweis: Bot- und Website-`coaching_requests` beide vollständig gemappt, keine Spalte verloren); Round-Trip-Sample; Codex-Kritiker FREIGABE.

---

## T4 — `voice` (0005)

**Files:** Create `migrations/0005_voice.sql` + Ledger-Fragmente (`deadlock.sqlite3`).

**Scope:** Die 18 `voice`-Tabellen aus §4 (TempVoice, Rename-Queue, Voice-Settings/Stats/Watch, Party-Members). PG-idiomatisch. Jede Spalte → Ledger.

**DoD:** Migration wendet an; Completeness der voice-Slice grün; Round-Trip-Sample; Codex-Kritiker FREIGABE.

---

## T5 — `tierlist` (+ Builds) (0006)

**Files:** Create `migrations/0006_tierlist.sql` + Ledger-Fragmente (`deadlock.sqlite3` UND Website `deadlock.db`).

**Scope:** 11 Bot-Tabellen (`deadlock_hero_builds`, `deadlock_heroes`, `hero_build_clones`, `hero_build_sources`, `tierlist_*`, `watched_build_authors`) + 6 Website-`meta_*` (`meta_builds`, `meta_heroes`, `meta_items`, `meta_tier_history`, `meta_tier_lists`, `meta_votes`) — alle in Schema `tierlist` (§4: Bot- und Website-Tierlist im selben Namespace). Beachte: `deadlock_hero_builds`/`hero_build_*` werden auch von Steam-Bot-Crates konsumiert (shared) — eine Zieltabelle, ein Ledger-Eintrag (Quell-DB sqlite3).

**DoD:** Migration wendet an; Completeness grün für beide Quell-DBs; Round-Trip-Sample; Codex-Kritiker FREIGABE.

---

## T6 — `bot` (0007)

**Files:** Create `migrations/0007_bot.sql` + Ledger-Fragmente (`deadlock.sqlite3`).

**Scope:** Die 26 `bot`-Infra-Tabellen aus §4 (Discord-/FAQ-/OAuth-/Invite-/Notification-State, `kv_store`, `changelog_entries` [#385 behalten], `persistent_views`, `reaction_role_*`, `standalone_*`, `schema_version`, etc.). `kv_store` bleibt generisch (enthält u. a. Patchnotes-Keys ns `patchnotes_bot`) → in `bot.kv_store`, NICHT splitten. PG-idiomatisch.

**DoD:** Migration wendet an; Completeness der bot-Slice grün; Round-Trip-Sample über `kv_store`; Codex-Kritiker FREIGABE.

---

## T7 — `clips` (0008)

**Files:** Create `migrations/0008_clips.sql` + Ledger-Fragmente (`deadlock.sqlite3`).

**Scope:** Die 9 `clips`-Tabellen aus §4 (Contests, Fetch-Historie, Templates global/streamer, Submissions, Windows).

**DoD:** Migration wendet an; Completeness grün; Round-Trip-Sample; Codex-Kritiker FREIGABE.

---

## T8 — `turnier` (0009)

**Files:** Create `migrations/0009_turnier.sql` + Ledger-Fragmente (Quell-DB `tournament.db`).

**Scope:** Die 37 `turnier`-Tabellen aus tournament.db (§2). `_sqlx_migrations` NICHT als Fachtabelle anlegen → Ledger `dropped{reason="sqlx-Tooling-Bookkeeping"}`. Hinweis: tournament.db ist bereits sqlx-Postgres-fähig (kleinster Typ-Sprung) — trotzdem ins zentrale Schema modellieren.

**DoD:** Migration wendet an; Completeness aller 38 tournament.db-Tabellen (37 mapped + 1 dropped) grün; Round-Trip-Sample; Codex-Kritiker FREIGABE.

---

## T9 — `activity` + `moderation` + `content` + `patchnotes`-Runtime (0010)

**Files:** Create `migrations/0010_activity_moderation_content_patchnotes.sql` + Ledger-Fragmente (`deadlock.sqlite3` UND Website `deadlock.db`).

**Scope:**
- `activity`: 11 Tabellen aus §2 + `member_events` (§4).
- `moderation`: `ai_moderation_cases`, `ai_moderation_ragebait_hits`, `security_guard_incidents`.
- `content`: `meta_announcements`, `meta_reports` (Website).
- `patchnotes`-Runtime: `changelog_posts`, `deadlock_changelogs` (geteilte sqlite3, Patchnotes-Service-Consumer) + `meta_patch_notes` (Website). Diese Tabellen sind aktiv vom `deadlock-patchnotes.service` belegt — exakt modellieren, nichts als tot droppen.

**DoD:** Migration wendet an; Completeness aller vier Slices über beide Quell-DBs grün; Round-Trip-Sample je Slice; Codex-Kritiker FREIGABE.

---

## T10 — Barrier: Cross-FK + Voll-Completeness + Drift-Gate (0011)

**Voraussetzung:** T1..T9 fertig + committed.

**Files:** Create `migrations/0011_cross_schema_fks.sql` (nur falls echte Cross-Namespace-FKs existieren — sonst leer/entfällt) + ggf. Anpassung des Gesamt-Gate-Scripts.

**Scope:**
1. Etwaige Cross-Namespace-FKs (Tabellen, die auf andere Namespaces als `core` verweisen) hier nachziehen, nachdem alle Zielschemas existieren.
2. **Voll-Completeness:** `check_mapping_completeness` über **alle drei Quell-DBs** gegen das vollständige Ledger — 0 unaccounted columns, jede der 178 unique Tabellen erfasst (mapped oder bewusst dropped).
3. **Drift-Gate:** `central_fresh_schema.sh` (frische Instanz → alle Migrations 0001..0011 → erwartetes Schema == frisch gebautes Schema) grün.
4. **Migrate-on-fresh:** `dl-central-migrate` läuft auf frischer :5434 sauber durch alle 11 Migrations.

**DoD (Codex verifiziert selbst):**
- Voll-Completeness-Gate Exit 0 über alle 3 Quell-DBs (Zähne: künstlich eine beliebige Quellspalte aus einem Ledger entfernen → Gate FAIL, dann revert).
- Drift-Gate grün; `migrate`-on-fresh grün.
- `SQLX_OFFLINE=true`-Workspace-Build grün.
- Frischer Codex-Kritiker über das Gesamt-Ergebnis: FREIGABE (prüft insb. K1 — kein cross-DB-Gleichnamiger verloren — und dass tote Tabellen mitmigriert sind, #386/#385 eingehalten).

---

## Arbeitsmethode (verbindlich)

| Rolle | Wer |
|---|---|
| Implementierung (DDL/Ledger/Tests) | **Codex** |
| Selbst-Verifikation (Gates/Tests ausführen) | **Codex** |
| Review (frischer Kritiker pro Ticket) | **Codex** |
| Orchestrierung (DAG, Datei-Ownership, commit/push, `changed_files`-Plausi) | **Claude** |
| User-sichtbare Texte | **Claude** (in P1 i. d. R. keine) |

Ablauf pro Ticket: Codex implementiert → Codex führt eigene DoD-Gates/Tests aus → frischer Codex-Kritiker reviewt → Codex-Rework bis FREIGABE → Claude plausibilisiert `changed_files`, committet + pusht auf `central-postgres-sp1`. Default-Modell `gpt-5.5`, effort `xhigh`.

SP1 = interne Infra/Migration, NICHT user-sichtbar → KEIN CHANGELOG/Discord (wie SP0 und P0).

## Self-Review (Claude, vor Übergabe)

- **Spec-Abdeckung:** Alle 178 unique Tabellen sind genau einem Ticket zugeordnet (T2..T9 Namespaces + core in T1); 3 Quell-DBs abgedeckt; #385/#386/#387 als Global Constraints + in T3/T1 verankert.
- **Keine Platzhalter:** DoD je Ticket konkret (welche Gates, welche Zähne). SQL/Ledger-Inhalt bewusst NICHT vorgeschrieben — das ist Codex-Implementierung (Delegationsmodell); Quelle ist data-landscape.md.
- **Datei-Ownership konsistent:** Migrations 0002..0011 disjunkt pro Ticket; Ledger pro Quell-DB fragmentiert (K1) → parallele Tickets kollidieren nicht.
