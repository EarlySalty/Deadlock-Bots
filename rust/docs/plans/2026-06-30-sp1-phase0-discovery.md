# SP1 Phase 0 — Daten-Landschaft-Discovery — Implementierungsplan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Für ausführende Worker (Codex):** Jedes Ticket = exakter Scope + Dateien + Gate + Definition of Done. **Codex implementiert, verifiziert sich SELBST (führt Gates/Tests selbst aus) und reviewt SELBST (frischer Codex-Kritiker → Rework). Claude organisiert nur den DAG, committet/pusht als Orchestrator und schreibt ausschließlich die user-sichtbaren Texte (UI/UX, deutsche Copy).** **WORKFLOW.md NIE anfassen.** User-sichtbare deutsche Texte = nur `"Platzhalter"` + Datei:Zeile melden, Claude finalisiert. Dieser Plan ist **read-only Inventarisierung** — er ändert KEINE Produktions-Schemas, migriert KEINE Daten, schaltet KEINEN Dienst um.

**Goal:** Ein konsolidiertes, gegen die lebenden DBs verifiziertes Daten-Landschaft-Dokument über alle 5 In-Scope-Projekte erzeugen (jede Tabelle × Spalten × Konsumenten × Zugriffsmuster + Vorschlag Ziel-Schema), als belastbare Eingabe für den Schema-Entwurf (SP1 Phase 1).

**Architecture:** Read-only Codex-Fan-out, **ein Worker pro Projekt** (kollisionsfrei, da nichts editiert wird). Jeder Worker schreibt ein strukturiertes Inventar (Mensch-`.md` + Maschine-`.tables.json`) seines Projekts. Ein wiederverwendbares Python-Gate vergleicht jedes JSON-Inventar gegen die **lebende** Projekt-DB und schlägt bei fehlender/zusätzlicher Tabelle oder Spalten-Abweichung fehl. Claude konsolidiert die fünf Inventare zu einem Daten-Landschaft-Doc + Cross-Projekt-Überschneidungs-Analyse + Tabelle→Schema-Vorschlag.

**Tech Stack:** Python 3 `sqlite3`-Modul (das `sqlite3`-CLI fehlt auf dem Host — immer `python3` nutzen), Markdown, JSON. Keine Rust-Builds in dieser Phase.

## Global Constraints

Wörtlich aus der freigegebenen Eltern-Spec `rust/docs/specs/2026-06-30-central-postgres-migration-design.md` (§3) und den Owner-Entscheidungen vom 2026-06-30:

1. **Null Datenverlust.** Keine Zeile/Spalte/Datum geht verloren; Drops nur als gelisteter Ledger-Eintrag. (Phase 0 mappt/droppt noch nichts — sie zählt nur vollständig.)
2. **Sauber neu schreiben, nicht umbiegen.** Inventar dient dem *neuen* Modell, nicht der 1:1-Kopie.
3. **Erst alles migrieren — auch tote Tabellen; Drop separat NACH Rebuild** (Issue #386). Inventar markiert 0-Konsumenten-Tabellen, droppt sie aber nicht.
4. **Behalten:** `changelog_entries`, `steam_rank_history` dauerhaft (Issue #385).
5. **Quelle wird nie zerstört.** Inventarisierung öffnet jede DB **read-only**; kein Schreibzugriff, kein Snapshot-Bedarf in Phase 0.
6. **Secrets nur via Infisical**, nie Klartext in Code/Log/Chat. Phase 0 braucht keine DSN (keine Postgres-Verbindung).
7. **Delegation (verbindlich, Owner-Vorgabe 2026-06-30):** ALLES geht an Codex (gpt-5.5/xhigh) — Implementierung, **Selbst-Verifikation** (Codex führt Gates/Tests selbst aus) UND **Review** (frischer Codex-Kritiker, Loop Codex→Kritiker→Rework). Claude **organisiert nur** (DAG, Dispatch, `git commit`/`push` als Orchestrator) und schreibt **ausschließlich** die user-sichtbaren Texte (UI/UX, deutsche Copy). **Kein Claude-Implementierungscode, kein Claude-Review, keine Claude-Verifikation.**
8. **Cross-SP-Eigentum festhalten** (Issue #387): `coaching_requests` Bot↔Website, `steam_links` DL-Bot↔Steam-Bot — in der Konsolidierung benennen.

---

## Arbeitsmethode (verbindlich — gilt für ALLE Tasks dieses Plans)

Owner-Vorgabe 2026-06-30: **Alles wird an Codex delegiert. Codex verifiziert und reviewt sich selbst. Claude organisiert nur und schreibt ausschließlich die Texte (UI/UX).**

| Rolle | Wer | Tut |
|---|---|---|
| Implementierung | **Codex** | Skripte, Inventare, Schema, ETL, Rewrite — der gesamte Code/Artefakt. |
| Selbst-Verifikation | **Codex** | Führt die Gates/Tests des Tickets SELBST aus, bis grün (z. B. `sp1_inventory_gate.py`). |
| Review | **Codex** | Frischer Codex-Kritiker prüft die Arbeit (Loop Codex→Kritiker→Rework), nicht Claude. |
| Orchestrierung | **Claude** | DAG/Sequencing, Worker-Dispatch, `git commit`/`push`, Statusrelais. |
| User-sichtbare Texte | **Claude** | UI/UX-Strings, deutsche Copy, Embeds, Fehlertexte. Codex setzt dort nur `"Platzhalter"` + Datei:Zeile. |

Claude schreibt **keinen** Implementierungscode, macht **kein** inhaltliches Review und führt **keine** eigene Verifikation aus — diese drei sind Codex-Pflicht. Der `git commit`/`push` bleibt beim Orchestrator (Claude prüft dabei nur die `changed_files`-Liste auf Plausibilität, kein Code-Review).

---

## SP1 Phasen-Übersicht (wohin dieser Plan gehört)

SP1 (Anker: `Deadlock-Bots`) folgt der Eltern-Spec §10 und wird als Phasen-DAG abgearbeitet; **jede Phase ein eigener writing-plans-Plan**:

- **Phase 0 — Discovery (DIESER Plan):** Daten-Landschaft aller 5 Projekte → 1 Doc. Liefert die Tabelle↔Schema- und Crate↔Tabelle-Karte.
- **Phase 1 — Schema + Ledger (Plan folgt nach P0):** zentrales Schema-Design pro Domäne (an Eltern-Spec §5.2) + vollständiges Mapping-Ledger (alles `mapped`).
- **Phase 2 — ETL (Plan folgt):** pro Tabelle/Domäne verlustfreie ETL auf `dl-central-etl` + Verify-Gates.
- **Phase 3 — Consumer-Rewrite (Plan folgt):** 1 Worker pro Crate, sqlx/PG statt inline-`rusqlite`.
- **Phase 4 — Cutover (Plan folgt):** ein atomarer Flip + SQLite-Rollback.

Phasen 1–4 werden erst nach P0 konkret geplant (sonst Schema-Raten). Dieser Plan endet mit dem freigegebenen Daten-Landschaft-Doc.

---

## File Structure (was Phase 0 erzeugt — alle im Anker-Repo `Deadlock-Bots`)

- `rust/scripts/sp1_inventory_gate.py` — wiederverwendbares Completeness-Gate (SQLite-Modus: Tabellen-/Spalten-Set Inventar==lebende DB).
- `rust/docs/_work/sp1/inventory/<projekt>.tables.json` — Maschinen-Inventar pro Projekt (gate-geprüft).
- `rust/docs/_work/sp1/inventory/<projekt>.md` — Mensch-Inventar pro Projekt (Konsumenten, Zugriffsmuster, Schema-Vorschlag, Notizen).
- `rust/docs/_work/sp1/data-landscape.md` — **das** konsolidierte Doc: Tabelle×Projekt×Schema-Matrix + Cross-Projekt-Überschneidungen + Tabelle→Schema-Vorschlag + #387-Stakes.

Projekt-DB-Pfade (Worker bestätigen den lebenden Pfad in ihrem Repo):
| Projekt | Repo | Erwartete DB | Tech |
|---|---|---|---|
| Deadlock-Bots | `/home/naniadm/Documents/Deadlock-Bots` | `data/deadlock.sqlite3` (124 Tab., 46M) | rusqlite |
| Steam-Bot | `/home/naniadm/Documents/Deadlock-Steam-Bot` | rusqlite-`.sqlite3`/`.db` (Pfad bestätigen) | rusqlite |
| Turniere | `/home/naniadm/Documents/Deadlock-Turniere` | `backend/data/tournament.db` (376K) | sqlx-sqlite |
| Website | `/home/naniadm/Documents/Website` | `builds/backend/deadlock.db` (260K, golden coaching) | aiosqlite |
| Patchnotes | `/home/naniadm/Documents/Deadlock--Patchnotes-Bot` | minimal/keine (Persistenz inventarisieren) | Python |

---

## Task 1: Inventar-Format + wiederverwendbares Completeness-Gate

**Files:**
- Create: `rust/scripts/sp1_inventory_gate.py`
- Create (Fixture): `rust/docs/_work/sp1/inventory/.gitkeep`

**Interfaces:**
- Produces: CLI `python3 rust/scripts/sp1_inventory_gate.py --db <pfad> --inventory <pfad.tables.json>`; Exit 0 = Inventar deckt die lebende DB exakt ab (Tabellen-Set + Spalten-Namen je Tabelle), Exit 1 = Lücke/Extra (mit konkreter Diff-Ausgabe). JSON-Schema pro Datei:
  ```json
  {
    "project": "<name>", "db_path": "<abs>", "db_tech": "<rusqlite|sqlx-sqlite|aiosqlite|python>",
    "tables": [
      {"name":"<t>","rows":<int>,
       "columns":[{"name":"<c>","type":"<sqlite-typ>","notnull":0,"pk":0}],
       "consumers":["<crate-oder-modul>"],
       "proposed_schema":"<core|coaching|scrim|steam|turnier|patchnotes|activity|OFFEN>",
       "notes":"<freitext>"}
    ]
  }
  ```
  Gate prüft NUR `name`+`columns[].name` gegen die lebende DB; `rows/consumers/proposed_schema/notes` sind Analyse-Felder (Review-geprüft, nicht DB-geprüft).

- [ ] **Step 1: Gate-Skript schreiben**

```python
#!/usr/bin/env python3
"""SP1 Inventar-Completeness-Gate: Inventar-JSON == lebende SQLite-DB (Tabellen + Spalten)."""
import argparse, json, sqlite3, sys

def live_schema(db_path):
    con = sqlite3.connect(f"file:{db_path}?mode=ro", uri=True)
    cur = con.cursor()
    cur.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
    tables = {r[0] for r in cur.fetchall()}
    cols = {}
    for t in tables:
        cur.execute(f'PRAGMA table_info("{t}")')
        cols[t] = {r[1] for r in cur.fetchall()}
    con.close()
    return tables, cols

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--db", required=True)
    ap.add_argument("--inventory", required=True)
    args = ap.parse_args()
    inv = json.load(open(args.inventory, encoding="utf-8"))
    inv_tables = {t["name"] for t in inv["tables"]}
    inv_cols = {t["name"]: {c["name"] for c in t["columns"]} for t in inv["tables"]}
    live_tables, live_cols = live_schema(args.db)

    problems = []
    missing = live_tables - inv_tables
    extra = inv_tables - live_tables
    if missing: problems.append(f"FEHLT im Inventar (in DB, nicht inventarisiert): {sorted(missing)}")
    if extra:   problems.append(f"EXTRA im Inventar (inventarisiert, nicht in DB): {sorted(extra)}")
    for t in sorted(live_tables & inv_tables):
        cmiss = live_cols[t] - inv_cols[t]
        cextra = inv_cols[t] - live_cols[t]
        if cmiss:  problems.append(f"  {t}: Spalten fehlen im Inventar: {sorted(cmiss)}")
        if cextra: problems.append(f"  {t}: Spalten extra im Inventar: {sorted(cextra)}")

    if problems:
        print(f"GATE FAIL ({inv['project']}): Inventar deckt die lebende DB NICHT exakt ab")
        for p in problems: print(p)
        sys.exit(1)
    print(f"GATE PASS ({inv['project']}): {len(live_tables)} Tabellen, Spalten vollständig abgedeckt")
    sys.exit(0)

if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Negativ-Beweis — Gate hat Zähne**

Künstliches Mini-Inventar mit absichtlich fehlender Tabelle gegen die Deadlock-Bots-DB laufen lassen; MUSS Exit 1 + die fehlende Tabelle nennen.

Run:
```bash
cd /home/naniadm/Documents/Deadlock-Bots
printf '{"project":"teeth-test","db_path":"x","db_tech":"rusqlite","tables":[{"name":"kv_store","rows":0,"columns":[{"name":"key","type":"TEXT","notnull":0,"pk":0}],"consumers":[],"proposed_schema":"OFFEN","notes":""}]}' > /tmp/teeth.json
python3 rust/scripts/sp1_inventory_gate.py --db data/deadlock.sqlite3 --inventory /tmp/teeth.json; echo "EXIT=$?"
```
Expected: `GATE FAIL`, listet u. a. `voice_session_log` als FEHLT, und `kv_store: Spalten fehlen im Inventar: ['value', ...]`; `EXIT=1`.

- [ ] **Step 3: Commit**

```bash
cd /home/naniadm/Documents/Deadlock-Bots
mkdir -p rust/docs/_work/sp1/inventory && touch rust/docs/_work/sp1/inventory/.gitkeep
git add rust/scripts/sp1_inventory_gate.py rust/docs/_work/sp1/inventory/.gitkeep
git commit -m "feat(sp1): Inventar-Completeness-Gate (Tabellen+Spalten gg. lebende DB)"
```

---

## Task 2: Inventar Deadlock-Bots (Anker-Projekt)

**Files:**
- Create: `rust/docs/_work/sp1/inventory/deadlock-bots.tables.json`
- Create: `rust/docs/_work/sp1/inventory/deadlock-bots.md`

**Interfaces:**
- Consumes: Gate aus Task 1; lebende DB `data/deadlock.sqlite3` (124 Tabellen).
- Produces: vollständiges Maschinen-+Mensch-Inventar; Konsumenten pro Tabelle (Crate-Ebene), Zugriffsmuster, `proposed_schema` je Tabelle.

**Codex-Worker-Briefing (read-only):** Inventarisiere **alle 124 Tabellen** von `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`. Für jede Tabelle: Name, Row-Count, Spalten (`PRAGMA table_info` → name/type/notnull/pk) via `python3 sqlite3`. Konsumenten: greppe `rust/crates/**` + `rust/bin/**` nach dem Tabellennamen (Wort-genau) und liste die besitzenden Crates; markiere `consumers: []` bei 0 Rust-Treffern. `proposed_schema` je Tabelle aus Eltern-Spec §5.2 vorschlagen (`core`/`coaching`/`scrim`/`steam`/`activity`; bei Unklarheit `OFFEN` + Notiz). Notizen: Hypertable-Kandidat (Zeit-Spalte nennen), tot (0 Konsumenten), Cross-SP-Verdacht (`steam_links`, `coaching_requests`). Schreibe Mensch-`.md` (Tabelle gruppiert nach `proposed_schema`) + Maschine-`.tables.json` im Schema aus Task 1.

- [ ] **Step 1: Inventar erzeugen** (Codex-Worker, read-only; Ausgabe in die zwei Dateien).

- [ ] **Step 2: Gate laufen lassen**

Run:
```bash
cd /home/naniadm/Documents/Deadlock-Bots
python3 rust/scripts/sp1_inventory_gate.py --db data/deadlock.sqlite3 --inventory rust/docs/_work/sp1/inventory/deadlock-bots.tables.json; echo "EXIT=$?"
```
Expected: `GATE PASS (Deadlock-Bots): 124 Tabellen, Spalten vollständig abgedeckt`; `EXIT=0`.

- [ ] **Step 3: Stichproben-Review (frischer Codex-Kritiker)** — 5 Tabellen quer (1 Schwergewicht `voice_session_log`, 1 tote `steam_launch_tokens`, 1 Cross-SP `steam_links`, 1 coaching `coaching_requests`, 1 scrim `scrim_match`): `proposed_schema` + `consumers` plausibel? Hypertable-Notiz vorhanden? (Codex prüft, nicht Claude.)

- [ ] **Step 4: Commit**

```bash
git add rust/docs/_work/sp1/inventory/deadlock-bots.tables.json rust/docs/_work/sp1/inventory/deadlock-bots.md
git commit -m "docs(sp1): Inventar Deadlock-Bots (124 Tabellen, gate-grün)"
```

---

## Task 3: Inventar Steam-Bot

**Files:**
- Create: `rust/docs/_work/sp1/inventory/steam-bot.tables.json`
- Create: `rust/docs/_work/sp1/inventory/steam-bot.md`

**Codex-Worker-Briefing (read-only):** Finde die **lebende** SQLite-DB im Repo `/home/naniadm/Documents/Deadlock-Steam-Bot` (suche `*.sqlite3`/`*.db`; bestätige über die laufende systemd-Unit/`ExecStart` bzw. `/proc/<pid>/exe`, welcher Pfad live ist — bei mehreren Kandidaten den live genutzten wählen, andere als Notiz). Inventarisiere alle Tabellen wie in Task 2. **Fokus #387:** Existiert hier ein `steam_links` (oder Äquivalent)? Spalten + Schlüssel exakt erfassen und gegen `core.steam_links` (SP0) notieren. `proposed_schema` i. d. R. `steam`/`core`.

- [ ] **Step 1: Live-DB-Pfad bestätigen + inventarisieren** (Codex-Worker).

- [ ] **Step 2: Gate laufen lassen**

Run:
```bash
cd /home/naniadm/Documents/Deadlock-Bots
python3 rust/scripts/sp1_inventory_gate.py --db "$(python3 -c "import json;print(json.load(open('rust/docs/_work/sp1/inventory/steam-bot.tables.json'))['db_path'])")" --inventory rust/docs/_work/sp1/inventory/steam-bot.tables.json; echo "EXIT=$?"
```
Expected: `GATE PASS (Steam-Bot): …`; `EXIT=0`.

- [ ] **Step 3: Commit**

```bash
git add rust/docs/_work/sp1/inventory/steam-bot.tables.json rust/docs/_work/sp1/inventory/steam-bot.md
git commit -m "docs(sp1): Inventar Steam-Bot (gate-grün, steam_links-Stake notiert)"
```

---

## Task 4: Inventar Turniere

**Files:**
- Create: `rust/docs/_work/sp1/inventory/turniere.tables.json`
- Create: `rust/docs/_work/sp1/inventory/turniere.md`

**Codex-Worker-Briefing (read-only):** DB `/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db` (Pfad bestätigen). Inventarisiere alle Tabellen wie in Task 2. Da Turniere bereits sqlx-sqlite nutzt: notiere je Tabelle, ob sie 1:1 nach `turnier`-Schema passt. `proposed_schema` i. d. R. `turnier`; Identitäts-/User-Bezug → `core`-Join notieren.

- [ ] **Step 1: Inventarisieren** (Codex-Worker).

- [ ] **Step 2: Gate laufen lassen**

Run:
```bash
cd /home/naniadm/Documents/Deadlock-Bots
python3 rust/scripts/sp1_inventory_gate.py --db /home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db --inventory rust/docs/_work/sp1/inventory/turniere.tables.json; echo "EXIT=$?"
```
Expected: `GATE PASS (Turniere): …`; `EXIT=0`.

- [ ] **Step 3: Commit**

```bash
git add rust/docs/_work/sp1/inventory/turniere.tables.json rust/docs/_work/sp1/inventory/turniere.md
git commit -m "docs(sp1): Inventar Turniere (gate-grün)"
```

---

## Task 5: Inventar Website-Coaching

**Files:**
- Create: `rust/docs/_work/sp1/inventory/website.tables.json`
- Create: `rust/docs/_work/sp1/inventory/website.md`

**Codex-Worker-Briefing (read-only):** DB `/home/naniadm/Documents/Website/builds/backend/deadlock.db` (Pfad bestätigen; das ist die *golden* Coaching-DB, eigene Datei/Inode — NICHT die Bot-DB). Inventarisiere alle Tabellen wie in Task 2. **Fokus #387 (Auslöser der Migration):** `coaching_requests` hier hat `id TEXT` + Plattform-Spalten — exakt erfassen und gegen die Bot-`coaching_requests` (`id INTEGER` + Rollen/Voice/Reward, aus Task 2) gegenüberstellen. `proposed_schema` i. d. R. `coaching`.

- [ ] **Step 1: Inventarisieren** (Codex-Worker).

- [ ] **Step 2: Gate laufen lassen**

Run:
```bash
cd /home/naniadm/Documents/Deadlock-Bots
python3 rust/scripts/sp1_inventory_gate.py --db /home/naniadm/Documents/Website/builds/backend/deadlock.db --inventory rust/docs/_work/sp1/inventory/website.tables.json; echo "EXIT=$?"
```
Expected: `GATE PASS (Website): …`; `EXIT=0`.

- [ ] **Step 3: Commit**

```bash
git add rust/docs/_work/sp1/inventory/website.tables.json rust/docs/_work/sp1/inventory/website.md
git commit -m "docs(sp1): Inventar Website-Coaching (gate-grün, coaching_requests-Konflikt erfasst)"
```

---

## Task 6: Inventar Patchnotes (Persistenz-Inventar)

**Files:**
- Create: `rust/docs/_work/sp1/inventory/patchnotes.md`
- Create (nur falls SQLite existiert): `rust/docs/_work/sp1/inventory/patchnotes.tables.json`

**Codex-Worker-Briefing (read-only):** Repo `/home/naniadm/Documents/Deadlock--Patchnotes-Bot`. Persistenz ist laut Eltern-Spec §11 minimal. Inventarisiere **alle Persistenz-Artefakte**: suche `*.db`/`*.sqlite3`/`*.json`/`*.yaml`/`*.csv` unter dem Repo (außer `node_modules`/`.git`/Caches) sowie alles unter `data/`. Pro Fund: Pfad, Typ, Zweck, ob Lauf-Zustand oder statisch. Falls eine SQLite existiert → zusätzlich `.tables.json` wie in Task 2 (gate-pflichtig). Falls KEINE DB → `patchnotes.md` dokumentiert, dass Persistenz datei-/JSON-basiert (oder leer) ist; das ist der vollständige Befund (Gate entfällt, weil keine SQLite).

- [ ] **Step 1: Persistenz inventarisieren** (Codex-Worker).

- [ ] **Step 2: Gate (nur falls SQLite gefunden)**

Run (überspringen, wenn keine `.tables.json` erzeugt wurde):
```bash
cd /home/naniadm/Documents/Deadlock-Bots
test -f rust/docs/_work/sp1/inventory/patchnotes.tables.json && \
python3 rust/scripts/sp1_inventory_gate.py --db "$(python3 -c "import json;print(json.load(open('rust/docs/_work/sp1/inventory/patchnotes.tables.json'))['db_path'])")" --inventory rust/docs/_work/sp1/inventory/patchnotes.tables.json; echo "EXIT=$?"
```
Expected: `GATE PASS (Patchnotes): …` falls DB existiert; sonst Skip (kein `.tables.json`).

- [ ] **Step 3: Commit**

```bash
git add rust/docs/_work/sp1/inventory/patchnotes.md rust/docs/_work/sp1/inventory/patchnotes.tables.json 2>/dev/null; git add rust/docs/_work/sp1/inventory/patchnotes.md
git commit -m "docs(sp1): Inventar Patchnotes (Persistenz-Befund)"
```

---

## Task 7: Konsolidierung → Daten-Landschaft-Doc

**Files:**
- Create: `rust/docs/_work/sp1/data-landscape.md`

**Interfaces:**
- Consumes: die 5 Projekt-Inventare aus Task 2–6 (alle gate-grün).
- Produces: das eine konsolidierte Doc, das Phase 1 (Schema-Design) als Eingabe nutzt.

**Inhalt (Codex-Worker synthetisiert aus den Inventaren; frischer Codex-Kritiker prüft Vollständigkeit; Claude finalisiert nur user-sichtbare Prosa):**
1. **Tabelle×Projekt×Schema-Matrix:** jede Quell-Tabelle aller Projekte, ihr Projekt, `proposed_schema`, Row-Count, Konsumenten-Crates, tot-Flag.
2. **Cross-Projekt-Überschneidungen / Zentralisierungs-Kandidaten:** gleichnamige oder semantisch gleiche Tabellen über Projekte (mind. `steam_links`, Identitäts-/User-Tabellen, `coaching_requests`); Vorschlag, was in `core` zentralisiert wird vs. domänen-lokal bleibt.
3. **#387-Stakes explizit:** (a) kanonische `coaching_requests`-Form, die Bot (`id INTEGER`+Rollen/Voice/Reward) UND Website (`id TEXT`+Plattform) verlustfrei vereint; (b) `core.steam_links` als einzige Wahrheit für DL-Bot (SP1) und Steam-Bot (SP3).
4. **Schema-Zuordnungs-Lücken:** Tabellen mit `proposed_schema: OFFEN` (z. B. TempVoice/Tierlist/Bot-State-Catch-all, die in Eltern-Spec §5.2 keinen expliziten Namespace haben) als Entscheidungsliste für Phase 1.
5. **Verweise:** Issues #385/#386/#387; Eltern-Spec §5.2/§7.

- [ ] **Step 1: Doc schreiben** (Codex-Worker; danach frischer Codex-Kritiker-Pass auf Vollständigkeit gegen die 5 JSON-Inventare. Claude schreibt nur etwaige user-sichtbare Prosa-Abschnitte final).

- [ ] **Step 2: Vollständigkeits-Gate (Konsolidierung deckt alle inventarisierten Tabellen ab)**

Run:
```bash
cd /home/naniadm/Documents/Deadlock-Bots
python3 - <<'PY'
import json, glob, re
inv_tables=set()
for f in glob.glob("rust/docs/_work/sp1/inventory/*.tables.json"):
    d=json.load(open(f,encoding="utf-8"))
    for t in d["tables"]: inv_tables.add((d["project"], t["name"]))
doc=open("rust/docs/_work/sp1/data-landscape.md",encoding="utf-8").read()
missing=[f"{p}.{t}" for (p,t) in sorted(inv_tables) if t not in doc]
print("INVENTARISIERTE TABELLEN:", len(inv_tables))
print("IM DOC FEHLEND:", len(missing))
if missing: print("FAIL — fehlen im Doc:", missing[:50]); raise SystemExit(1)
print("GATE PASS: Daten-Landschaft-Doc nennt jede inventarisierte Tabelle")
PY
echo "EXIT=$?"
```
Expected: `GATE PASS: …`; `EXIT=0`. (Jede inventarisierte Tabelle kommt im Doc namentlich vor — kein stilles Weglassen bei der Konsolidierung.)

- [ ] **Step 3: Commit**

```bash
git add rust/docs/_work/sp1/data-landscape.md
git commit -m "docs(sp1): Daten-Landschaft konsolidiert (5 Projekte, Zentralisierungs-Vorschlag, #387-Stakes)"
```

---

## Self-Review (gegen Eltern-Spec + Owner-Direktiven)

- **Spec-Abdeckung §7 (Mapping-Vorstufe):** Das Inventar liefert die „pro Quell-Tabelle/-Spalte"-Granularität, die das Ledger in Phase 1 braucht. ✓
- **Spec §11 (Patchnotes-Inventar offen):** Task 6 schließt das. ✓
- **Owner: alles inventarisieren, nichts droppen** — tote Tabellen werden markiert, nicht entfernt. ✓
- **Owner: parallel** — Task 2–6 sind unabhängig (read-only, getrennte Dateien) → parallel ausführbar; Task 7 ist der Barrier. ✓
- **#387** — in Task 3/5/7 explizit verankert. ✓
- **Placeholder-Scan:** Gate-Code vollständig; keine „TBD". Worker-Briefings nennen exakte Pfade + Felder. ✓
- **Typ-Konsistenz:** JSON-Schema in Task 1 definiert; Tasks 2–6 referenzieren genau dieses; Gate prüft genau `name`+`columns[].name`. ✓

## Execution Handoff

Reine Codex-Ausführung (siehe Arbeitsmethode): T1 Gate zuerst, dann **5 parallele Codex-Worker** für die Inventare (je Projekt, read-only, Selbst-Gate), T7 Konsolidierung als Barrier, danach **ein frischer Codex-Kritiker** über das gesamte Phase-0-Ergebnis (Gate-Zähne, alle Inventare gate-grün, Konsolidierungs-Vollständigkeit). Worker committen NICHT — Claude committet/pusht als Orchestrator nach grünem Kritiker. Reihenfolge: T1 → (T2‖T3‖T4‖T5‖T6) → T7 → Codex-Kritiker → Orchestrator-Commit.
