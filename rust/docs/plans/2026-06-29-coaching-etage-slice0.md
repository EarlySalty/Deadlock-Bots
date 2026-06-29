# Coaching-Etage Slice 0 — Auto-Sammeln + Roster-Import (Implementierungsplan)

> **Für Worker (Codex):** TDD pro Task (Red→Green→Refactor). Schritte als Checkboxen. Spec: `rust/docs/specs/2026-06-29-coaching-scrim-program-design.md`. Seed: `rust/docs/specs/seed-roster-2026-06-29.json`.

**Goal:** Der dl-bot sammelt Scrim-Anmeldungen automatisch (Rollen-Reaktion → Pool-Eintrag) und der aktuelle Roster/Team-Stand wird einmalig in die DB importiert. Rein Rust, kein Frontend.

**Architecture:** Neue Crate `dl-squads` (Datenmodell + DB-Zugriff für das Scrim-Programm). `dl-db`-Schema bekommt die Scrim-Tabellen (idempotentes CREATE-TABLE-on-Boot, wie bestehend). Im Reaction-Role-Handler (`dl-community/src/reaction_roles.rs::apply_mapping_add`) wird beim Vergeben der **Scrim-Signup-Rolle** ein Participant ge-upsertet. Ein einmaliger Seed-Importer liest die Seed-JSON.

**Tech Stack:** Rust, SQLite via bestehende `dl-db`-Fassade, `serde`/`serde_json`, `thiserror`/`anyhow`.

## Global Constraints

- **Quelle der Wahrheit = Rust**, alles in `Deadlock-Bots/rust`, geteilte `dl-db`.
- **Keine Funktionsänderung an bestehendem Verhalten** (Reaction-Roles, Coaching) außer den hier beschriebenen Ergänzungen.
- **Saubere, domänenklare Namen** (`dl-squads`, `scrim_*`-Tabellen). Keine kryptischen Präfixe.
- **`clippy --all-targets -- -D warnings` sauber**, `cargo fmt` gelaufen.
- Kein `.unwrap()`/`.expect()` in Produktionspfaden; Fehler via `thiserror`/`anyhow`.
- Keine ASCII-Schreibmaschinen-Quotes in deutschen Strings; benannte Konstanten statt Magic-Strings.
- Jeder Commit lauffähig + verifiziert, schrittweise; Branch `coaching-scrim-program`.

---

### Task 1: Scrim-Schema in `dl-db`

**Files:**
- Modify: `rust/crates/dl-db/docs/db-schema.sql` (bzw. die in `dl-db/src/lib.rs` referenzierte Schema-Quelle — den Pfad aus `include_str!` verifizieren)
- Test: `rust/crates/dl-db/tests/scrim_schema.rs`

**Interfaces — Produces:** Tabellen `scrim_participant`, `scrim_team`, `scrim_team_member`, `scrim_match` (für `dl-squads` + dl-bot).

DDL (SQLite, Stil wie bestehende `CREATE TABLE`-Blöcke — werden automatisch zu `IF NOT EXISTS`):

```sql
CREATE TABLE scrim_participant(
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  discord_id      INTEGER,                  -- NULL bei Seed-Namen ohne Verknuepfung
  display_name    TEXT NOT NULL,
  rank            TEXT,
  rank_source     TEXT NOT NULL DEFAULT 'self',   -- 'self' | 'steam'  (Steam-Naht)
  rank_verified   INTEGER NOT NULL DEFAULT 0,      -- bool
  roles           TEXT,                            -- freitext/CSV wie im Sheet
  availability    TEXT,                            -- JSON: {"Mo":"19-20",...}
  status          TEXT NOT NULL DEFAULT 'new',     -- new|profile_complete|admitted|assigned|bench|waitlist|left
  source          TEXT NOT NULL DEFAULT 'discord_reaction', -- discord_reaction|web_form|seed
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
);
CREATE TABLE scrim_team(
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  name               TEXT NOT NULL UNIQUE,
  coach              TEXT,                  -- 'Leo' | 'Deniz' (spaeter coach_id)
  discord_role_id    INTEGER,              -- bestehende Team-Rolle verlinken
  discord_channel_id INTEGER,             -- bestehender Team-Channel verlinken
  created_at         TEXT NOT NULL
);
CREATE TABLE scrim_team_member(
  team_id        INTEGER NOT NULL REFERENCES scrim_team(id),
  participant_id INTEGER NOT NULL REFERENCES scrim_participant(id),
  role           TEXT,
  is_captain     INTEGER NOT NULL DEFAULT 0,
  is_bench       INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(team_id, participant_id)
);
CREATE TABLE scrim_match(
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  team_a_id    INTEGER REFERENCES scrim_team(id),
  team_b_id    INTEGER REFERENCES scrim_team(id),
  when_text    TEXT,                      -- Slice 0/1: Freitext; Slice 2 strukturiert
  scheduled_at TEXT,
  status       TEXT NOT NULL DEFAULT 'planned',
  created_at   TEXT NOT NULL
);
```

- [ ] **Step 1:** Test schreiben: nach DB-Init existieren alle vier Tabellen (Query `SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'scrim_%'` liefert 4).
- [ ] **Step 2:** Test laufen → FAIL (Tabellen fehlen).
- [ ] **Step 3:** DDL ins Schema einfügen.
- [ ] **Step 4:** Test laufen → PASS.
- [ ] **Step 5:** Commit `feat(db): scrim_* Tabellen (participant/team/member/match)`.

---

### Task 2: Crate `dl-squads` — Modell + DB-Zugriff

**Files:**
- Create: `rust/crates/dl-squads/Cargo.toml`, `rust/crates/dl-squads/src/lib.rs`, `.../src/model.rs`, `.../src/store.rs`
- Modify: `rust/Cargo.toml` (Workspace-Member), abhängige Crates die `dl-squads` nutzen
- Test: `rust/crates/dl-squads/tests/store.rs`

**Interfaces — Produces:**
- `struct Participant { id, discord_id: Option<i64>, display_name, rank, rank_source, rank_verified, roles, availability, status, source, ... }`
- `async fn upsert_participant_by_discord(db, discord_id: i64, display_name: &str) -> Result<i64, SquadErr>` — legt bei neuem `discord_id` einen Eintrag an (`status=new`, `source=discord_reaction`); existiert er, no-op/Touch `updated_at`. Gibt participant-id zurück.
- `async fn upsert_participant_by_name(db, p: &SeedPlayer) -> Result<i64, SquadErr>` — name-gekeyt, für Seed (`source=seed`).
- `async fn create_team(db, name, coach, role_id: Option<i64>, channel_id: Option<i64>) -> Result<i64, SquadErr>`
- `async fn add_team_member(db, team_id, participant_id, role, is_captain, is_bench) -> Result<(), SquadErr>`
- `async fn list_pool(db, status: Option<&str>) -> Result<Vec<Participant>, SquadErr>`

- [ ] **Step 1:** Crate anlegen + ins Workspace `Cargo.toml`. Build leer → grün.
- [ ] **Step 2:** Test schreiben: `upsert_participant_by_discord` zweimal mit gleicher `discord_id` → genau **ein** Eintrag, `status='new'`.
- [ ] **Step 3:** Test laufen → FAIL.
- [ ] **Step 4:** `model.rs` + `store.rs` minimal implementieren.
- [ ] **Step 5:** Test laufen → PASS. Weitere Tests: `create_team`+`add_team_member`, `list_pool(Some("new"))` filtert korrekt.
- [ ] **Step 6:** `clippy --all-targets`, `fmt`, Commit `feat(squads): dl-squads Crate — Modell + Store`.

---

### Task 3: Auto-Sammeln im Reaction-Role-Hook

**Files:**
- Modify: `rust/crates/dl-community/src/reaction_roles.rs` (in/nach `apply_mapping_add`, ~Zeile 207-220)
- Modify: dl-bot-Verdrahtung, die den `ReactionRoleManager` baut (DB/`dl-squads` injizieren)
- Test: `rust/crates/dl-community/tests/scrim_signup_hook.rs` (oder in dl-squads mit Fake-Port)

**Interfaces — Consumes:** `dl_squads::upsert_participant_by_discord`. **Konstante:** `SCRIM_SIGNUP_ROLE_ID: u64` — die Rolle, die die Signup-Reaktion vergibt. **DEPENDENCY:** Wert aus `reaction_role_mappings` der Scrim-Signup-Nachricht ziehen (Claude/Codex ermittelt vor Implementierung; bis dahin als `const` mit Platzhalter + `// TODO Wert bestätigen` markiert und im Worker-Report Datei:Zeile melden).

**Verhalten:** In `apply_mapping_add`, **nachdem** `port.add_role` erfolgreich war: wenn `mapping.role_id == SCRIM_SIGNUP_ROLE_ID`, dann `dl_squads::upsert_participant_by_discord(db, user_id, &username)`. Fehler dabei dürfen die Rollenvergabe **nicht** kippen (loggen, weiterlaufen — fail-open für den Sammel-Nebeneffekt).

- [ ] **Step 1:** Test: Reaction-Add mit Scrim-Rolle → Participant existiert danach (Fake-Port + In-Memory-DB). Reaction-Add mit *anderer* Rolle → **kein** Participant.
- [ ] **Step 2:** Test laufen → FAIL.
- [ ] **Step 3:** Hook implementieren (fail-open, geloggt).
- [ ] **Step 4:** Test laufen → PASS. Regressions-Check: bestehende Reaction-Role-Tests grün.
- [ ] **Step 5:** `clippy`/`fmt`, Commit `feat(squads): Rollen-Reaktion sammelt Scrim-Pool-Eintrag (fail-open)`.

---

### Task 4: Einmaliger Roster-Seed-Import

**Files:**
- Create: `rust/bin/dl-bot/src/bin/seed_scrim.rs` **oder** ein `seed-scrim`-Subcommand am dl-bot (Pfad an bestehende Bin-Struktur anpassen)
- Test: `rust/crates/dl-squads/tests/seed_import.rs`

**Interfaces — Consumes:** `dl-squads`-Store-Funktionen; liest `rust/docs/specs/seed-roster-2026-06-29.json`.

**Verhalten:** JSON parsen → Spieler (`source=seed`, name-gekeyt, idempotent) + `pool_unassigned` (status `new`/`waitlist`) + Teams (mit `coach`, Mitglieder, `is_captain`, Bench) + Matches anlegen. **Discord-Rollen-/Channel-IDs der 4 bestehenden Teams** werden, falls bekannt, gesetzt (sonst NULL, später nachtragen). Zweiter Lauf ändert nichts (idempotent).

- [ ] **Step 1:** Test: Import gegen leere DB → 24 Spieler + 5 Pool + 4 Teams + 2 Matches; Team 1 hat Vicky als `is_captain`.
- [ ] **Step 2:** Test laufen → FAIL.
- [ ] **Step 3:** Importer implementieren (idempotent, name-gekeyt).
- [ ] **Step 4:** Test laufen → PASS; zweiter Lauf → keine Duplikate.
- [ ] **Step 5:** Commit `feat(squads): einmaliger Roster-Seed-Import`.

---

## Definition of Done (Slice 0)

1. `cargo build` + `clippy --all-targets -D warnings` + `fmt` sauber; alle Tests grün.
2. **Live-Verifikation:** dl-bot-Binary neu gebaut + Dienst neu gestartet; eine Test-Reaktion auf die Scrim-Signup-Nachricht erzeugt einen `scrim_participant` (DB-Gegenprüfung, Output-gemutet) — **Artefakt + Live-Zustand**, nicht dem Log trauen.
3. Seed-Import einmalig live gelaufen: 24 Spieler + 4 Teams + Bench + Matches in der DB.
4. Bestehendes Reaction-Role-/Coaching-Verhalten unverändert (Regressionstests grün).
5. Kein Frontend in diesem Slice.

## Self-Review (gegen Spec)

- Deckt §4 (Datenmodell scrim_*), §5 (Discord-Eingang Auto-Sammeln), §8 Slice 0 (Auto-Sammeln + Import). ✓
- Steam/Stripe-Nähte: `rank_source`/`rank_verified`/`status` vorhanden, kein Bau. ✓
- Offene Werte explizit als DEPENDENCY markiert (SCRIM_SIGNUP_ROLE_ID, Team-Rollen/Channel-IDs) statt stillem Platzhalter. ✓
