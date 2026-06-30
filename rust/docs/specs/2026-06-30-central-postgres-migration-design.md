# Zentrale Postgres-DB + sauberer Neuentwurf der Datenschicht — Design-Spec

Datum: 2026-06-30
Status: Design zur Freigabe (Brainstorm abgeschlossen, vor writing-plans)
Scope: Mehr-Repo-Umbau. Anker-Repo: `Deadlock-Bots`. Betrifft zusätzlich `Deadlock-Steam-Bot`, `Deadlock-Turniere`, `Deadlock--Patchnotes-Bot`, `Website` (Coaching).

## 1. Motivation / Wahres Ziel

Die Bots speichern heute in mehreren losen SQLite-Dateien mit teils gleichnamigen, aber inhaltlich verschiedenen Tabellen. Konkret bewiesen am 2026-06-30: Bot-DB (`data/deadlock.sqlite3`, inode 3671569) und Website-DB (`Website/builds/backend/deadlock.db`, inode 7127670) sind **verschiedene Dateien** mit zwei unvereinbaren `coaching_requests` (Bot: `id INTEGER` + Rollen/Voice/Reward-Spalten; Website: `id TEXT` + Plattform-Spalten). Ein Schema-Edit in einer Datei brach still die andere Domäne (Tests grün, Laufzeit kaputt).

Das eigentliche Ziel ist **nicht** „weniger Dateien", sondern **eine geteilte Wahrheit mit gemeinsamer Identität**: derselbe Discord-User taucht in Coaching, Scrim, Steam-Link und Turnier-Historie auf — heute 4× getrennt, künftig 1× in `core` und joinbar. Das beseitigt die ganze Bug-Klasse „falsche SQLite / Datei-Identität / Schema-Kollision" strukturell und macht den Coaching-Port (und alles danach) erst sauber möglich.

## 2. Scope

**In Scope (zentrale DB):** `Deadlock-Bots` (dl-bot/dl-community/dl-squads u.a.), `Website`-Coaching, `Deadlock-Steam-Bot`, `Deadlock-Turniere`, `Deadlock--Patchnotes-Bot` (wird dabei **nach Rust portiert** — letzter Python-Holdout).

**Out of Scope (bleiben separat):**
- `Deadlock-Twitch-Bot` — eigene TimescaleDB (`twitch-analytics-postgres`, :5433), bleibt unangetastet.
- `TradingBot` — andere Domäne (Finanzen, earlysalty.com), 113 MB, kein Daten-Synergie-Nutzen.
- `infisical-db` — Infisicals eigene Postgres, tabu.

## 3. Leitplanken (hart, nicht verhandelbar)

1. **Null Datenverlust.** Keine Zeile, keine Spalte, kein Datum geht verloren. Drops nur als bewusster, gelisteter Eintrag im Mapping-/Drop-Ledger (siehe §7).
2. **Sauber neu schreiben, nicht umbiegen.** Zielschema und Datenschicht werden neu/idiomatisch entworfen (nicht 1:1 vom alten SQLite kopiert, nicht rusqlite-Muster nachbauen). Das alte Chaos wird nicht mitgeschleppt.
3. **Verlustfreie Kompression, kein Delete.** TimescaleDB-Kompression ist spaltenweise + verlustfrei; keine Retention-Policy, nichts wird je automatisch gelöscht. Volle Historie für immer.
4. **Tests mit Zähnen.** Tests müssen echte Prod-Brüche fangen (§9), nicht grün-waschen. Die geänderten *Konsumenten* werden mitgetestet, nicht nur die geänderte Crate.
5. **Inkrementell + rückrollbar.** Pro Service harter, kurzer Cutover; die alte SQLite bleibt als Sofort-Rollback (§8).
6. **Quelle wird nie zerstört.** SQLite-Originale werden vor Migration gesnapshottet und nie überschrieben/gelöscht.

## 4. Entschiedene Parameter

| Parameter | Entscheidung | Begründung |
|---|---|---|
| DB-Engine | Postgres via **TimescaleDB-Image** (pg16-Familie), neue **dedizierte** Instanz | Superset von Postgres; Hypertable-Option für Aktivitäts-/Voice-Stats später, ohne heutige Festlegung. Eigener Blast-Radius getrennt von Twitch/Infisical. |
| Logische Struktur | **Eine Datenbank**, Postgres-**Schemas pro Domäne** | Löst Namens-Kollision sauber; ermöglicht Cross-Domänen-Joins über `core`. |
| Schemas | `core`, `coaching`, `scrim`, `steam`, `turnier`, `patchnotes`, `activity` | Domänen-Trennung mit schmaler geteilter `core`-Schnittstelle (tiefe Module). |
| Geteilte Spine | `core` mit **Discord-User-ID** als zentralem Join-Schlüssel | Ist in jeder heutigen DB faktisch schon der Schlüssel; wir formalisieren das Reale. |
| Rust-Stack | **sqlx + Postgres**, `query!`/`query_as!` (compile-geprüft), `cargo sqlx prepare` (offline-Cache committed) | Haus-Standard (Twitch/Turniere/TradingBot nutzen sqlx). Schema wird Compile-Zeit-Vertrag. |
| Migrations | `sqlx migrate`, **ein kanonischer Migrations-Owner** für die zentrale DB | Ein Schema-Owner statt Mehrfachquellen (vgl. alte Python-`init_schema`-Doppelung). |
| Kompression | An für alte Chunks, verlustfrei | Platz sparen ohne Datenverlust. |
| Retention | **Aus** | „kein Delete" — alles bleibt. |
| Daten-Reshape | Erlaubt (sauberes Modell), aber jeder Quell-Spalten-Verbleib im Ledger dokumentiert | „sauber neu" + „null Verlust" gleichzeitig erfüllbar. |
| Secrets | `DATABASE_URL` o.ä. via Infisical, nie im Klartext/Log | CLAUDE.md-Secret-Regeln. |
| Cutover | Pro Service harter Schnitt + SQLite-Rollback | Wenige Minuten Wartung ok; Zero-Downtime nicht nötig. |

## 5. Zielarchitektur

### 5.1 Postgres-Instanz
Neuer dedizierter TimescaleDB-Container (eigener Port, z.B. 5434), Datenbank z.B. `deadlock`, dedizierte Rolle(n). Volume mit Backups. Verbindung über Infisical-Secret. Erreichbar nur loopback/Docker-Netz, kein öffentlicher Port.

### 5.2 Schema-Layout (sauberer Neuentwurf, nicht Kopie)
- **`core`** — geteilte Identität: `core.users` (Discord-ID als PK, Username/Avatar/…), `core.steam_links` (discord→steam), ggf. `core.guild_members`. Genau das, was domänenübergreifend geteilt wird — nicht mehr.
- **`coaching`** — Coaching-Plattform (heutige Website-Domäne, sauber modelliert): Anfragen, Coaches, Coachees, Sessions, Reviews, Goals, Milestones, Appointments, Surveys, Notes. Bot-seitiger Rollen-/Voice-/Reward-State (heute in der Bot-DB) wird als **eigene, klar benannte Tabelle(n)** modelliert und über `core`-Identität + Session-Bezug verknüpft — **verknüpfen statt verschmelzen**.
- **`scrim`** — Scrim-Programm (dl-squads): Participants, Teams, Members, Matches.
- **`steam`** — Steam-Bot-Domäne.
- **`turnier`** — Turnier-Domäne (heute schon sqlx-sqlite → sqlx-pg).
- **`patchnotes`** — Patchnotes-Zustand (nach Inventar; minimal).
- **`activity`** — Aktivitäts-/Voice-/Message-Stats (Hypertable-Kandidaten; vorerst relational).

Jede Domäne „besitzt" ihr Schema; `core` ist die einzige geteilte Abhängigkeit.

### 5.3 Rust-Datenzugriff
- Pro Repo eine schlanke sqlx-Anbindung an die zentrale DB; jede Domäne definiert ihre eigenen Typen/Queries (inkl. der `core.*`, die sie liest). **Keine repo-übergreifende Zwangs-Crate** — geteilt wird auf DB-Ebene (Schema/`core`), nicht durch fragile Cross-Repo-Rust-Kopplung. Ein optionaler `core`-Typen-Crate kann später folgen.
- **Compile-Zeit-Verträge:** durchgehend `sqlx::query!`/`query_as!`. `cargo sqlx prepare` erzeugt den `.sqlx`-Offline-Cache (committed), damit Builds/CI keine Live-DB brauchen.
- **Migrations:** ein kanonischer Satz für die zentrale DB, von genau einem Migrator angewandt (eigener `migrate`-Schritt / Foundation-Dienst). Services nehmen das Schema als gegeben an und legen es nicht selbst an.

## 6. Sauberer Neuentwurf statt Umbiegen

Prinzip: Wir bauen das Modell, das wir *wollen*, und übersetzen die Altdaten hinein. Konkret:
- **Kein 1:1-Dump** der 119 Bot-Tabellen / der Website-DDL. Stattdessen pro Domäne ein durchdachtes, normalisiertes Schema mit klaren Namen (keine kryptischen Präfixe, vgl. [[feedback_clear_module_names]]).
- **Worked Example `coaching_requests`:** Die Website-Anfrage wird `coaching.requests` (saubere IDs). Der bot-seitige Rollen-/Voice-/Reward-Lifecycle wird z.B. `coaching.role_state`/`coaching.voice_sessions`, über `request_id`/`core.users` verknüpft. Beide Altdatensätze landen vollständig im neuen Modell, keiner wird verworfen.
- **Bewusstes Pruning (Drop-Ledger):** genuin tote Artefakte werden gestrichen, aber namentlich gelistet: 0-Byte-`data/bot.db`, `service/deadlock_bots.db`, `coaching_sessions_legacy` etc. Jeder Drop = ein Ledger-Eintrag mit Begründung, vom User abnickbar.
- **DB-nahe Domänenlogik** wird sauber neu auf sqlx geschrieben, nicht das alte rusqlite-Muster mechanisch übersetzt.

## 7. Datenmigration (verlustfrei ins saubere Modell)

1. **Snapshot:** jede Quell-SQLite wird vor ETL kopiert; Original bleibt unberührt.
2. **Mapping-Ledger (pro Quell-Tabelle/-Spalte):** explizite Zuordnung alt→neu, ODER Eintrag „bewusst gedroppt, weil …". Reviewbares Artefakt; kein stilles Weglassen.
3. **Pro-Tabelle-ETL mit Typ-Treue:** SQLite (locker typisiert) → Postgres (streng): Unix-INTEGER-Zeiten → `timestamptz`, TEXT-JSON → `jsonb`, BLOB, NULL-Semantik, Boolean (`0/1`→`bool`). Reshape erlaubt, Drop nur per Ledger.
4. **Verifikations-Gate (ersetzt naiven Tabellen-Hash, weil Schema reshaped ist):**
   - **Aggregat-Checks Quelle==Ziel:** Row-Counts (unter Berücksichtigung von Splits/Merges), Summen/Counts/distinct Keys der Kernfelder.
   - **Mapping-Vollständigkeit:** jede Quell-Spalte ist im Ledger entweder gemappt oder gedroppt — Lücke = Stopp.
   - **Stichproben-Round-Trip:** Zufallsdatensätze alt↔neu Feld-für-Feld verglichen.
   - Mismatch ⇒ Stopp, kein Cutover.

## 8. Cutover (pro Service, hart + rückrollbar)

Reihenfolge (Decomposition §10). Pro Service: Dienst stoppen → SQLite-Snapshot → ETL → Verifikations-Gate → Dienst auf Postgres starten → live verifizieren (Artefakt + Live-Zustand, nicht Erfolgs-Log; vgl. CLAUDE.md). **SQLite bleibt liegen als Sofort-Rollback.** Wenige Minuten Wartungsfenster sind akzeptabel. Falls ein Dienst partout nicht pausieren darf: Catch-up nur für den einen.

## 9. Test- & Vertragsstrategie (vier Säulen)

1. **Compile-Zeit-Queries:** `sqlx::query!`/`query_as!` gegen das echte Schema; String-`query()` in Prod-Pfaden verboten. „Spalte fehlt" = Build-Fehler.
2. **Cross-Boundary-Integrationstests:** echte Queries gegen ein echtes Test-Postgres (Testcontainers) mit dem echten Migrations-Schema; pro Domäne UND mit den Konsumenten (genau das fehlte beim S1-02-Crash).
3. **Schema-Drift-Gate:** CI prüft „frische DB aus Migrationen" == erwartetes Schema; kein Drift Bauplan↔Code↔Live.
4. **Migrations-Parität + Negativtests:** §7-Gate als automatisierter Test; mind. ein Test, der **rot** wird, wenn eine Pflicht-Spalte/Typ/NOT-NULL bricht (Beweis: die Tests haben Zähne).

Siehe [[feedback_tests_catch_prod_breakage]].

## 10. Decomposition / Sequencing

Jedes Sub-Projekt bekommt einen eigenen Plan (writing-plans), implementiert via Codex (gpt-5.5/xhigh), Loop Codex→frischer Codex-Kritiker→Rework. **Arbeitsteilung (Owner-Vorgabe 2026-06-30): Codex implementiert, verifiziert sich selbst (Gates/Tests selbst ausführen) UND reviewt selbst (frischer Codex-Kritiker). Claude organisiert nur (DAG + `commit`/`push`) und schreibt ausschließlich die user-sichtbaren Texte (UI/UX). Kein Claude-Code, kein Claude-Review, keine Claude-Verifikation.**

- **SP0 — Fundament:** Instanz hochziehen; `core`-Schema + Schema-Layout-Gerüst; kanonischer Migrations-Owner; sqlx-Anbindung + offline-prepare; ETL-/Verifikations-Harness (Ledger, Aggregat-Checks); Test-Harness (Testcontainers, Drift-Gate). Liefert die Schienen für alles Weitere.
- **SP1 — Deadlock-Bots:** größte DB (46 MB), hier sitzen Coaching-Bot-State + Scrim. Datenschicht sauber neu (sqlx statt dl-db/rusqlite), Schema-Design `core`/`scrim`/`activity`/coaching-bot-state, ETL + Cutover.
- **SP2 — Website-Coaching → `dl-etage` (Rust):** der eigentliche Coaching-Port, jetzt sauber auf der zentralen DB (`coaching`-Schema). Ersetzt das, was vorher „Slice 1 auf SQLite" werden sollte.
- **SP3 — Steam-Bot:** rusqlite → sqlx, `steam`-Schema, ETL + Cutover.
- **SP4 — Turniere:** sqlx-sqlite → sqlx-pg (kleinster Sprung), `turnier`-Schema, ETL + Cutover.
- **SP5 — Patchnotes → Rust:** Python→Rust-Port + (minimaler) `patchnotes`-State, Cutover. Schließt den Python-Ausstieg ab.
- **Danach:** Coaching-Slice-1-Features (Sichtbarkeit „Mein Team", Web-Anmeldung, Coach-Pool) auf der sauberen Foundation.

## 11. Risiken & offene Punkte

- **Größe.** „Sauber neu" über 5 Services ist viel; SP0 muss die Schienen wirklich tragfähig machen, sonst dupliziert sich Aufwand.
- **Repo-übergreifende Migrations-Koordination.** Wer ownt `core`-Migrations, wie ziehen separate Repos dasselbe Schema? In SP0 festzunageln.
- **Reshape-Risiko.** Reshape ist der Ort, wo Verlust sich versteckt → Mapping-Ledger + Aggregat-Checks sind Pflicht, nicht optional.
- **Patchnotes-Inventar.** Persistenz scheint minimal (kein .db/.json gefunden, `data/`-Inhalt noch zu prüfen) — in SP0/SP5 sauber inventarisieren.
- **sqlx-offline-Cache.** `.sqlx` muss gepflegt + committed sein, sonst brechen CI-Builds; Prozess festlegen.
- **Coaching-Slice-1 pausiert.** Der bereits gepushte `coaching-slice1`-Branch (S1-01 dl-etage-Scaffold auf rusqlite) wird durch SP2 ersetzt; S1-01-Code wird auf sqlx neu gemacht (kein Wegwerf-Verlust, das Konzept bleibt).

## 12. Definition of Done (gesamt)

- Eine zentrale TimescaleDB trägt alle In-Scope-Domänen; Twitch/TradingBot/Infisical separat.
- Alle Altdaten verlustfrei migriert; Mapping-/Drop-Ledger vollständig + reviewed; Verifikations-Gate grün pro Tabelle.
- Alle In-Scope-Dienste laufen auf der zentralen DB; SQLite-Originale als Rollback archiviert.
- Patchnotes läuft in Rust; kein Python-Bot mehr live.
- Vier Test-Säulen greifen in CI; ein bewusst eingebauter Bruch wird rot.
- Kompression an, Retention aus (alles bleibt).
