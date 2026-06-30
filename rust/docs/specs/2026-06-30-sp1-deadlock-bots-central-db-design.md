# SP1 — Deadlock-Bots auf die zentrale Postgres/TimescaleDB — Design-Spec

Datum: 2026-06-30
Status: Design zur Freigabe (Brainstorm abgeschlossen, vor writing-plans)
Anker-Repo: `Deadlock-Bots`
Voraussetzung: SP0 KOMPLETT + gemergt (main `85278da`) — Instanz `:5434`, `core`-Spine + 7 Schemas, kanonischer Migrator `dl-central-migrate`, `.sqlx`-Offline-Cache, ETL-/Ledger-Framework `dl-central-etl`, Vier-Säulen-CI.
Quelle/Eltern-Spec: `rust/docs/specs/2026-06-30-central-postgres-migration-design.md`

---

## 1. Motivation / Wahres Ziel

Die Deadlock-Bots-SQLite (`data/deadlock.sqlite3`, 46 MB, **124 Tabellen / 159k Zeilen**) ist die größte und am stärksten verflochtene Quell-DB des Gesamtumbaus. Hier sitzen Identität, Aktivität/Voice-Statistik, Scrim, Bot-seitiges Coaching, TempVoice, Tierlist/Builds und viel Bot-State. SP1 überführt diese DB **verlustfrei** in die zentrale DB und schreibt die Datenschicht des Bots **sauber neu** (sqlx/Postgres statt inline-`rusqlite`), mit **einem harten, rückrollbaren Cutover**.

SP1 ist gleichzeitig der **Beweis des ETL-/Cutover-Musters** für SP2–SP5. Deshalb wird die Discovery-Phase global angelegt (alle In-Scope-Projekte), damit das zentrale Schema **einmal richtig** entsteht und in späteren SPs nicht nachgebaut werden muss.

## 2. Scope

**Rebuild (Code + ETL + Cutover): NUR `Deadlock-Bots`.** Die 13 Rust-Crates, die heute `rusqlite` gegen `data/deadlock.sqlite3` fahren, werden auf sqlx/Postgres umgestellt; die Daten werden migriert; ein atomarer Cutover schaltet den Live-Bot um.

**Discovery (nur read-only Inventarisierung): alle 5 In-Scope-Projekte** — `Deadlock-Bots`, `Deadlock-Steam-Bot`, `Deadlock-Turniere`, `Website` (Coaching), `Deadlock--Patchnotes-Bot`. Zweck: das zentrale Schema (v. a. `core` + geteilte Tabellen wie `steam_links`) so entwerfen, dass SP2–SP5 ohne `core`-Umbau andocken.

**Out of Scope (unverändert):** Twitch-Bot (eigene TimescaleDB `:5433`), TradingBot, Infisical-DB. Kein Rebuild von Steam-Bot/Turniere/Website/Patchnotes in SP1 — die kommen in SP2–SP5.

## 3. Leitplanken (hart, nicht verhandelbar)

Erbt §3 der Eltern-Spec, plus SP1-spezifische Owner-Entscheidungen vom 2026-06-30:

1. **Null Datenverlust.** Jede Quell-Spalte ist im Mapping-/Drop-Ledger entweder `mapped` oder `dropped` — kein stilles Weglassen.
2. **Erst alles migrieren — auch tote Tabellen.** **Kein Drop in SP1.** Auch die ~35 referenzlosen/Legacy-Tabellen werden migriert. → Das Ledger ist in SP1 faktisch **vollständig `mapped`**. Das Droppen der bestätigt-toten Tabellen passiert in einem **separaten, sauberen Re-Verify-Pass NACH dem Rebuild** (Issue #386).
3. **Behalten trotz 0 Konsumenten:** `changelog_entries`, `steam_rank_history` werden dauerhaft migriert (Feature-Idee offen, Issue #385).
4. **Sauber neu schreiben, nicht umbiegen.** Kein 1:1-Dump der 124 SQLite-Tabellen; kein mechanisches Übersetzen von `rusqlite`-Mustern. Idiomatisches sqlx/Postgres-Modell, klare Namen, schema-qualifiziert.
5. **Ein atomarer Cutover.** Wegen Cross-Domain-Joins über `core` wird das **gesamte** Deadlock-Bots-Repo in **einem** Schnitt umgeschaltet — nicht pro Domäne live. Rollback über SQLite-Snapshot + Vor-Cutover-Binary.
6. **Verlustfreie Kompression, keine Retention.** Aktivitäts-/Voice-Tabellen werden TimescaleDB-Hypertables, Kompression an für alte Chunks, Retention AUS.
7. **Quelle wird nie zerstört.** SQLite wird vor jedem ETL-Lauf gesnapshottet; Original unberührt.
8. **Tests mit Zähnen.** Vier Säulen (§10), Konsumenten mittesten, ein bewusster Bruch wird rot.
9. **Secrets nur via Infisical.** `DEADLOCK_CENTRAL_DSN` als volle DSN-URL; nie Klartext in Code/Log/Chat.

## 4. Befunde aus der Recon (warum SP1 die Form unten hat)

- **124 Tabellen, 159k Zeilen.** Schwergewichte: `voice_session_log` (63.844), `user_co_players` (25.178), `rename_requests` (17.447), `steam_launch_tokens` (13.292), `changelog_entries` (6.290), `text_conversation_log` (4.078), `steam_rank_history` (3.948).
- **89 Tabellen von Rust referenziert, 35 nicht** (tot oder nur Python-Legacy). Python ist für Deadlock-Bots **disabled** (Rust-Bot live); die `py=`-Referenzen sind totes Gewicht, **kein** Live-Konsument — muss vor Cutover bestätigt werden (§9).
- **Keine Fassade.** `dl-db` ist nur ein dünner Handle (`read`/`write`-Closures auf rohe `rusqlite::Connection` + `kv`). Jede Consumer-Crate trägt **eigenes inline-SQL**. → Es gibt keinen zentralen Swap-Punkt; der Rewrite ist über die Crates verteilt. → **Parallelität nach Crate, nicht nach Domäne** (§6).
- **ETL-Framework steht** (`dl-central-etl`): `SourceSqlite` (read-only, JSON-Werte), `TargetWriter` (PgPool), `convert::*` (Unix→`timestamptz`, `0/1`→`bool`, TEXT-JSON→`jsonb`), `verify::*` (`check_mapping_completeness`, `check_row_counts[_with_factor]`, `check_sample_covers_mapped_columns`, `sample_round_trip`), `Ledger` (TOML, `deny_unknown_fields`). SP1 schreibt die **bespoke Insert-/Mapping-Logik pro Tabelle** auf diesem Fundament.

## 5. Architektur: SP1 in 4 Phasen

SP1 ist kein einzelner Plan, sondern ein Programm aus vier Phasen. Jede Phase liefert ein review- und freigebbares Artefakt; jede Phase wird per Codex-Loop (implementieren → frischer Codex-Kritiker → Rework → Claude-Externverifikation → commit+push) abgearbeitet.

### SP1.0 — Discovery (parallel, read-only)

Codex-Fan-out, **ein Worker pro In-Scope-Projekt** (5 Worker, read-only, kein Edit → kollisionsfrei). Jeder Worker inventarisiert für sein Projekt: genutzte Tabellen + Spalten + Zugriffsmuster (read/write, Häufigkeit, Schlüssel/Joins), und ob die Tabelle in der zentralen DB schon eine Heimat hat.

**Ausgabe:** EIN konsolidiertes Daten-Landschafts-Doc `rust/docs/_work/sp1/data-landscape.md` mit (a) vollständiger Tabellen×Projekt×Crate-Matrix, (b) Cross-Projekt-Überschneidungen (Zentralisierungs-Kandidaten: Identität, `steam_links`, …), (c) Vorschlag, welche Tabelle in welches Schema (`core` vs. Domäne) gehört. Claude synthetisiert die Worker-Ausgaben zu diesem einen Doc.

### SP1.1 — Unified Schema-Design (Synthese, bewusst NICHT parallel)

Aus dem Discovery-Doc ein **kohärenter** Gesamtentwurf des zentralen Schemas für die Deadlock-Bots-Domänen + die in SP1 zu zentralisierenden geteilten Tabellen. Ein Designer (Claude + ein Codex-Schema-Worker als Kritiker), nicht 8 parallele — sonst widersprechen sich die `core`-Entscheidungen.

**Ausgabe:** (a) Migrations-SQL-Entwürfe pro Domäne (`coaching`, `scrim`, `activity`, `voice`, `tierlist`, `steam`-Discord-Seite, `bot`-State; `core`-Erweiterungen), inkl. Hypertable-/Kompressions-Festlegung für `activity`; (b) das **vollständige Mapping-Ledger** (`rust/etl/sp1/ledger/*.toml`, eine Datei pro Quell-Tabelle oder pro Domäne) — in SP1 alles `mapped`; (c) Schema-Eigentum an den Cross-SP-Stellen geklärt (Issue #387: kanonische `coaching_requests`-Form, `core.steam_links` als einzige Wahrheit).

### SP1.2 — Rebuild (parallel nach korrekter Achse)

Pro Arbeitspaket ein Codex-Loop. Drei parallelisierbare Achsen (siehe §6), aufeinander aufbauend pro Domäne: **Schema-Migration → ETL-Modul → Consumer-Rewrite**. Der **Live-Bot bleibt die ganze Zeit auf SQLite** — der neue PG-Code wächst daneben und wird gegen eine echte PG-Instanz mit migrierten Daten getestet, geht aber erst beim Cutover live.

**Ausgabe:** angewandte Migrationen (über `dl-central-migrate`), ein ETL-Binary/-Modul, das die ganze DB verlustfrei nach PG überführt (gated durch die Verify-Säulen), und die auf sqlx/PG umgeschriebenen Consumer-Crates (compile-checked `query!`/`query_as!` + committeter `.sqlx`-Cache).

### SP1.3 — Cutover (ein atomarer Flip)

Ablauf: (1) bestätigen, dass **nur** der Rust-Bot in die SQLite schreibt (kein Python/kein Fremdprozess); (2) Bot stoppen (Wartungsfenster, wenige Minuten); (3) **finaler ETL-Lauf** auf frischem Snapshot → Verify-Gate grün (Row-Counts, Mapping-Vollständigkeit, Stichproben-Round-Trip); (4) DSN/Config umstellen → Bot-Binary (jetzt sqlx/PG) starten; (5) Live-Verifikation (Boot gesund, Kernpfade lesen/schreiben PG). **Rollback:** Vor-Cutover-Binary redeployen + auf SQLite-Snapshot zeigen (Minuten). SQLite-Original wird archiviert, nie zerstört.

## 6. Parallelisierungs-Achsen (Owner-Wunsch „so parallel wie möglich")

Parallel JA — aber nach der Achse, die nicht kollidiert. Naives „ein Worker pro Domäne" scheitert, weil `dl-community` ~50 Tabellen über 6 Domänen anfasst (sechs Worker im selben File = Index-Klau + Merge-Hölle).

| Arbeit | Parallel-Achse | Kollisionsfrei, weil |
|---|---|---|
| Discovery | 1 Worker / Projekt | read-only, nichts wird editiert |
| Schema-DDL | 1 Migration / Domäne | getrennte `.sql`-Files, additiv |
| ETL | 1 Modul / Quell-Tabelle (oder Domäne) | unabhängige Insert-/Mapping-Logik |
| Consumer-Rewrite | **1 Worker / Crate** | jede Crate hat genau einen Besitzer → nie zwei Worker im selben File |

Das Discovery-Doc liefert die exakte Crate↔Tabelle-Karte, die die Crate-Achse erst sauber schneidbar macht. Wenn eine Crate (z. B. `dl-community`) zu groß für einen Worker ist, wird sie in SP1.1 entlang Domänen-Submodulen vorstrukturiert, damit Domänen-Worker disjunkte Files besitzen.

## 7. Schema-Layout (Prinzipien — final in SP1.1)

- **`core` erweitern** um die wirklich geteilten Identitäts-/Verknüpfungs-Tabellen (Kandidaten aus Discovery: `user_privacy`, `user_tags`, `user_data`, `user_mod_tags`; `steam_links` ist bereits `core.steam_links`).
- **Domänen-Schemas** für den Rest: `scrim`, `activity` (Hypertables), `coaching` (Bot-State), `voice` (TempVoice/Config), `tierlist`/builds, `steam` (Discord-seitige Steam-Tabellen), plus ein `bot`-/`activity`-State-Namespace für Catch-all (kv_store, persistent_views, reaction_roles, faq, moderation, notifications, clips, changelog, …). Exakte Schema-Namen werden in SP1.1 festgelegt (SP0 legte `core/coaching/scrim/steam/turnier/patchnotes/activity` an; ggf. ein zusätzliches Namespace für Voice/Tierlist/Bot-State, falls sinnvoll — Entscheidung in SP1.1).
- **Reshape erlaubt, Drop nur per Ledger** (in SP1: keiner). Typ-Treue: Unix-INTEGER→`timestamptz`, `0/1`→`bool`, TEXT-JSON→`jsonb`, BLOB erhalten, NULL-Semantik erhalten.
- **TimescaleDB:** `activity`-Tabellen als Hypertables auf der Zeitspalte; Kompression an für alte Chunks; Retention AUS.

## 8. Mapping-/Drop-Ledger (Strategie)

- Format wie `dl-central-etl` vorgibt: `[tables.<tabelle>.columns.<spalte>]` mit `status = "mapped"` + `to = "schema.tabelle.spalte"` (in SP1 durchgängig) bzw. `status = "dropped"` + `reason` (erst im späteren Drop-Pass, #386).
- Eine Ledger-Datei pro Domäne oder pro Quell-Tabelle (review-freundlich), unter `rust/etl/sp1/ledger/`.
- **Gate (Säule 4):** `check_mapping_completeness` über die echten Quell-Spalten jeder Tabelle — eine ungemappte Spalte = Stopp, kein Cutover.

## 9. Cutover & Rollback (Details)

- **Pre-Flight-Gate:** Beweisen, dass kein Python-/Fremdprozess in die SQLite schreibt (sonst Post-Cutover-Divergenz). Quelle: laufende systemd-Units + offene Writer auf die DB-Datei prüfen.
- **Rollback-Modell (empfohlen, bestätigbar in SP1.3):** Vor-Cutover-Binary aus Git (sauberer rusqlite-Stand) + SQLite-Snapshot = Rollback in Minuten. Damit bleibt das neue Binary **rein sqlx/PG** (kein Dual-Impl-Ballast — entspricht „sauber neu, nicht umbiegen"). Alternative (Config-Flag mit zwei Impls in einem Binary) wird nur gewählt, falls Sub-Minuten-Rollback gefordert ist.
- **Verify am Schnitt:** finaler ETL-Lauf → `check_row_counts` (mit Split/Merge-Faktor wo reshaped), `sample_round_trip` auf Zufallsstichprobe je Tabelle, Mapping-Vollständigkeit. Mismatch ⇒ Abbruch, kein Flip.

## 10. Test- & Vertragsstrategie (vier Säulen, auf SP1 angewandt)

1. **Compile-Vertrag:** alle neuen Consumer-Queries als `query!`/`query_as!`, committeter `.sqlx`-Cache, `SQLX_OFFLINE=true`-Build grün.
2. **Cross-Boundary-Integration:** Tests gegen eine **echte** PG-Instanz mit migrierten Daten (Wegwerf-Container `central_test_db.sh`), inkl. der Cross-Domain-Joins über `core`. Konsumenten mittesten, nicht nur die geänderte Crate.
3. **Schema-Drift-Gate:** frische DB aus Migrationen == Erwartungs-Snapshot; bewusster Drift wird rot.
4. **ETL-Verify mit Zähnen:** Row-Counts + Mapping-Vollständigkeit + Stichproben-Round-Trip pro Tabelle; ungemappte Spalte / Count-Mismatch / Round-Trip-Mismatch ⇒ rot.

## 11. Risiken & offene Punkte

- **`dl-community` ist riesig** (~50 Tabellen / 6 Domänen). Mitigation: in SP1.1 entlang Domänen-Submodulen vorstrukturieren, damit SP1.2 disjunkt parallelisiert.
- **Cross-SP-Schema-Eigentum** (#387): `coaching_requests` Bot↔Website, `steam_links` DL-Bot↔Steam-Bot — in SP1.1 kanonisch festnageln.
- **Python-Legacy-Referenzen** verschleiern „lebt vs. tot". Wahrheit ist der Live-Rust-Bot; vor Cutover bestätigen, dass Python wirklich nicht schreibt.
- **Lockere SQLite-Typisierung → strenges PG.** Einzelne Spalten mischen evtl. Typen (TEXT/INT in derselben Spalte). ETL-Konverter müssen das pro Spalte sauber entscheiden (Framework liefert die Bausteine; bespoke Mapping pro Tabelle).
- **Tote Tabellen mit Masse** (`steam_launch_tokens` 13k) werden in SP1 mitmigriert (Owner-Entscheidung) — Drop später (#386).

## 12. Decomposition in writing-plans-Pläne

Aus diesem Spec entstehen separate Implementierungspläne (jeder eigener writing-plans → Codex-Loop):

- **Plan A — SP1.0 Discovery:** 5-Projekt-Fan-out + Konsolidierung zum Daten-Landschafts-Doc.
- **Plan B — SP1.1 Schema + Ledger:** zentrales Schema-Design (Migrationen-Entwürfe) + vollständiges Mapping-Ledger + Cross-SP-Eigentum.
- **Plan C…N — SP1.2 Rebuild:** pro Domäne/Crate-Paket (Schema anwenden → ETL → Consumer-Rewrite), parallel nach §6.
- **Plan Z — SP1.3 Cutover:** Pre-Flight, finaler ETL, Flip, Live-Verify, Rollback-Probe.

Reihenfolge: A → B → (C…N parallel) → Z. Plan A wird zuerst per writing-plans ausgearbeitet; die folgenden Pläne nutzen die Discovery-Ergebnisse.

## 13. Definition of Done (SP1 gesamt)

- Daten-Landschafts-Doc (alle 5 Projekte) konsolidiert + reviewed.
- Zentrales Schema für Deadlock-Bots-Domänen live über den kanonischen Migrator; Drift-Gate grün.
- Mapping-Ledger vollständig (`check_mapping_completeness` grün über alle 124 Quell-Tabellen).
- ETL überführt die gesamte DB verlustfrei (Row-Counts + Round-Trip + Mapping-Gate grün); SQLite-Snapshot archiviert.
- Alle 13 Consumer-Crates auf sqlx/PG (compile-checked + `.sqlx`-Cache); Integrationstests inkl. Cross-Domain-Joins grün.
- Atomarer Cutover vollzogen + live verifiziert; Rollback-Pfad nachweislich vorhanden.
- Vier Test-Säulen in CI grün; ein bewusster Bruch wird rot.
- Offene Folgearbeiten als Issues: #385 (changelog/steam_rank Feature), #386 (Drop-Pass), #387 (Cross-SP-Eigentum).

## 14. Verweise

- Eltern-Spec: `rust/docs/specs/2026-06-30-central-postgres-migration-design.md`
- SP0-Plan: `rust/docs/plans/2026-06-30-sp0-central-db-foundation.md`
- Architektur: `rust/docs/central-db/architecture.md`
- Ledger-Format: `rust/docs/central-db/ledger-format.md`
- ETL-Framework: `rust/crates/dl-central-etl/`
- Issues: #385, #386, #387
