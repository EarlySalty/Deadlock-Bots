# SP1 Phase 0 - Kritikerbefund

Stand: 2026-06-30. Modus: read-only gegen Code/DB/Services; einzige Aenderung ist dieser Bericht. Kein Git. `WORKFLOW.md` nicht angefasst.

## 1) GATE-ZAEHNE: PASS

Ich habe Mini-Inventare direkt aus `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` erzeugt und danach gezielt manipuliert.

- Fehlende Tabelle: entfernte `ai_moderation_cases`; Gate-Exit `1`; Ausgabe: `FEHLT im Inventar (in DB, nicht inventarisiert): ['ai_moderation_cases']`.
- Fehlende Spalte: entfernte `ai_moderation_cases.case_id`; Gate-Exit `1`; Ausgabe: `ai_moderation_cases: Spalten fehlen im Inventar: ['case_id']`.
- Korrektes Mini-Inventar aus Live-Schema: Gate-Exit `0`; Ausgabe: `GATE PASS (mini-deadlock-bots): 124 Tabellen, Spalten vollständig abgedeckt`.

Damit hat `rust/scripts/sp1_inventory_gate.py` echte Zaehne fuer fehlende Tabellen und Spalten.

## 2) INVENTAR-GATE GRUEN: PASS

Alle vier `.tables.json` laufen gegen ihren jeweiligen `db_path` mit Exit `0`:

| Inventar | db_path | Beleg |
|---|---|---|
| `deadlock-bots.tables.json` | `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` | `GATE PASS (Deadlock-Bots): 124 Tabellen, Spalten vollständig abgedeckt` |
| `steam-bot.tables.json` | `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` | `GATE PASS (Steam-Bot): 124 Tabellen, Spalten vollständig abgedeckt` |
| `turniere.tables.json` | `/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db` | `GATE PASS (Turniere): 38 Tabellen, Spalten vollständig abgedeckt` |
| `website.tables.json` | `/home/naniadm/Documents/Website/builds/backend/deadlock.db` | `GATE PASS (Website): 21 Tabellen, Spalten vollständig abgedeckt` |

`db_path` zeigt jeweils auf gelebte DBs:

- `deadlock-bot-rust.service`: `/proc/1404746/exe -> .../Deadlock-Bots/rust/target/release/dl-bot`; offene FDs auf `data/deadlock.sqlite3`, `-wal`, `-shm`.
- `steam-bot.service`: `/proc/1471159/exe -> .../Deadlock-Steam-Bot/rust/target/release/steam-bot`; offene FDs auf `data/deadlock.sqlite3`, `-wal`, `-shm`.
- `deadlock-turniere.service`: `/proc/1478659/exe -> .../Deadlock-Turniere/rust/target/release/turnier-bot`; offene FDs auf `backend/data/tournament.db` und zusaetzlich Steam-Bridge-FDs auf `Deadlock-Bots/data/deadlock.sqlite3`.
- `deadlock-website-backend.service`: `/proc/2674415/cmdline` ist `uvicorn app.main:app`; offene FDs auf `/home/naniadm/Documents/Website/builds/backend/deadlock.db`, `-wal`, `-shm`.

## 3) STEAM SHARED-DB UNABHAENGIG: PASS / BESTAETIGT

`systemctl --user list-units --all | grep -i steam` zeigt `steam-bot.service` aktiv. `systemctl --user show steam-bot.service`:

- `FragmentPath=/home/naniadm/.config/systemd/user/steam-bot.service`
- `ExecStart=/usr/bin/bash -lc /home/naniadm/Documents/Deadlock-Steam-Bot/rust/deploy/run-steam-bot.sh`
- `Environment=DEADLOCK_DB_PATH=/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`
- `MainPID=1471159`

Laufzeitbelege:

- `/proc/1471159/exe -> /home/naniadm/Documents/Deadlock-Steam-Bot/rust/target/release/steam-bot`; das ist nicht das Deadlock-Bots-Binary.
- `/proc/1471159/cmdline = ['/home/naniadm/Documents/Deadlock-Steam-Bot/rust/target/release/steam-bot']`
- offene FDs: `fd 9 .../Deadlock-Bots/data/deadlock.sqlite3`, `fd 10 ...deadlock.sqlite3-wal`, `fd 11 ...deadlock.sqlite3-shm`.
- Repo-Scan unter `/home/naniadm/Documents/Deadlock-Steam-Bot` fand keine eigene `*.db`, `*.sqlite`, `*.sqlite3`.
- Codebeleg: `rust/crates/steam-bot/src/main.rs` liest `DEADLOCK_DB_PATH` und defaultet sonst auf `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`; `Db::open(&config.db_path)` oeffnet diesen Pfad.

Verdikt: Steam-Bot nutzt die geteilte `deadlock.sqlite3` und hat keine eigene SQLite-DB.

## 4) KONSOLIDIERUNGS-VOLLSTAENDIGKEIT: PASS

Vorgegebener Check:

```text
UNIQUE: 178 MISSING: 0 []
```

Damit ist jede inventarisierte Tabelle namentlich in `rust/docs/_work/sp1/data-landscape.md` enthalten.

## 5) KONSOLIDIERUNGS-KORREKTHEIT: FAIL

Die geforderten Stichproben sind groesstenteils korrekt, aber es gibt einen echten Konsolidierungsfehler bei Patchnotes-Consumern der geteilten DB.

### PASS: geteilte `deadlock.sqlite3` nicht doppelt gezaehlt, Consumer gemerged

`data-landscape.md` zaehlt `deadlock.sqlite3` physisch einmal: 124 Tabellen in `deadlock.sqlite3`, 38 in `tournament.db`, 21 in `deadlock.db`, zusammen 183 physische Tabellenzeilen. Beispiele:

- `steam_links`: JSON Deadlock-Bots-Consumer `dl-activity, dl-bot, dl-bridges, dl-central-db, dl-community, dl-db, dl-stats, dl-voice`; JSON Steam-Consumer u.a. `steam-core::task::handlers::friends`, `steam-flows::*`, `steam-persistence::*`, `steam-web::*`. `data-landscape.md:128` enthaelt beide Sets.
- `voice_session_log`: Deadlock-Bots-Consumer `dl-activity, dl-community, dl-dashboard, dl-db, dl-stats, dl-voice`; Steam-Consumer-Set im JSON ist leer. `data-landscape.md:172` enthaelt die Deadlock-Bots-Consumer und markiert die Sicht als `Deadlock-Bots + Steam-Bot`.
- `kv_store`: Deadlock-Bots-Consumer plus Steam-Consumer `steam-persistence::builds`; `data-landscape.md:97` enthaelt beide Sets.

### PASS: `coaching_requests` Cross-DB-Konflikt korrekt

Live-PRAGMA:

- Bot-DB `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`: `id INTEGER pk=1`, plus `message_id`, `channel_id`, `role_assigned_at`, `role_expires_at`, `role_removed_at`, `scheduled_slot`, `website_request_id`, `coachee_id`.
- Website-DB `/home/naniadm/Documents/Website/builds/backend/deadlock.db`: `id TEXT pk=1`, plus `assigned_coach_username`, `bot_request_id`, `preferred_coach_id`, `notify_discord_at`.

`data-landscape.md:255-259` beschreibt diese Differenz und die Spaltenlisten korrekt.

### PASS: `steam_links` kanonische Form verliert keine gelebte Spalte

Live-PRAGMA `steam_links`: `user_id INTEGER`, `steam_id TEXT`, `name TEXT`, `verified INTEGER`, `primary_account INTEGER`, `created_at DATETIME`, `updated_at DATETIME`, `legacy_ref TEXT`, `migrated_at INTEGER`, `deadlock_rank INTEGER`, `deadlock_rank_name TEXT`, `deadlock_subrank INTEGER`, `deadlock_badge_level INTEGER`, `deadlock_rank_updated_at INTEGER`, `is_steam_friend INTEGER`.

`data-landscape.md:249-251` nennt alle Live-Spalten. Der kanonische Vorschlag bildet Umbenennungen explizit ab: `user_id -> discord_id`, `name -> steam_display_name`, `created_at -> linked_at`. Fehlende semantische Spalten: keine.

### PASS: OFFEN-Abdeckung und Namespace-Vorschlaege

Maschineller Abgleich der physisch deduplizierten `proposed_schema=OFFEN`-Menge gegen die Entscheidungsliste:

```text
OFFEN physical count= 82
/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3 expected= 71 listed= 71
/home/naniadm/Documents/Deadlock-Turniere/backend/data/tournament.db expected= 1 listed= 1
/home/naniadm/Documents/Website/builds/backend/deadlock.db expected= 10 listed= 10
MISSING= []
EXTRA= []
CLUSTER_MISSING= []
```

Die Cluster `voice`, `tierlist`, `moderation`, `bot`, `clips`, `content`, `core`, `activity`, `turnier_meta` sind plausibel und decken die OFFEN-Liste ab.

### FAIL: Patchnotes-Consumer der geteilten DB fehlen in `data-landscape.md`

Das Patchnotes-Inventar selbst sagt, dass der Bot keine eigene SQLite-Datei hat, aber zur Laufzeit die geteilte Deadlock-DB nutzt:

- `rust/docs/_work/sp1/inventory/patchnotes.md:21-31`: Patchnotes nutzt zusaetzlich den geteilten Deadlock-DB-Layer unter `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`.
- `patchnotes.md:28`: Patchnotes liest/schreibt dort `changelog_posts`, `deadlock_changelogs` und `kv_store` im Namespace `patchnotes_bot`.
- Codebeleg `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/main.py:37`: `from service import db as deadlock_db`.
- Codebeleg `main.py:1668-1754`: `CREATE TABLE IF NOT EXISTS changelog_posts`, `CREATE TABLE IF NOT EXISTS deadlock_changelogs`, danach `UPDATE/INSERT` in beide Tabellen.
- Codebeleg `main.py:1871-1933`: `deadlock_db.get_kv/set_kv(KV_NAMESPACE, ...)`, `KV_NAMESPACE = "patchnotes_bot"` in `main.py:22`.
- Laufzeitbeleg `deadlock-patchnotes.service`: `MainPID=5049`, `/proc/5049/cmdline = ['.../Deadlock-Bots/.venv/bin/python', 'main.py']`, `DEADLOCK_HOME=/home/naniadm/Documents/Deadlock-Bots`, offene FDs `.../Deadlock-Bots/data/deadlock.sqlite3`, `-wal`, `-shm`.

Die Konsolidierung verliert diese aktiven Consumer:

- `rust/docs/_work/sp1/data-landscape.md:62` listet `changelog_posts` nur mit `dl-dashboard`, ohne Patchnotes.
- `data-landscape.md:83` und `data-landscape.md:313` markieren `deadlock_changelogs` mit Consumer `-` und `tot=ja`, obwohl der laufende Patchnotes-Code schreibt.
- `data-landscape.md:97` und `data-landscape.md:327` listet `kv_store` ohne `patchnotes_bot`.
- `data-landscape.md:400` formuliert Patchnotes als rein file-basiert/nicht SQLite und uebernimmt die Laufzeit-Persistenz aus `patchnotes.md:21-31` nicht sichtbar in die Konsolidierung.

Impact: `deadlock_changelogs` kann faelschlich als tot/drop-kandidat erscheinen; `changelog_posts` und `kv_store` verlieren einen aktiven Writer/Owner. Das ist keine separate Patchnotes-DB, aber ein Konsolidierungsfehler der geteilten `deadlock.sqlite3`.

## 6) PATCHNOTES: PASS mit Laufzeit-Hinweis

Eigener Repo-Scan:

```text
find /home/naniadm/Documents/Deadlock--Patchnotes-Bot ... \( -name '*.db' -o -name '*.sqlite' -o -name '*.sqlite3' -o -name '*.db3' \) -print
```

Ausgabe war leer. Im Patchnotes-Repo existiert keine eigene SQLite-Datei. Der laufende Patchnotes-Service nutzt aber die externe, geteilte `Deadlock-Bots/data/deadlock.sqlite3`; dieser Consumer-Verlust ist oben unter Punkt 5 als FAIL bewertet.

## Gesamt-Verdikt: NACHARBEIT NOETIG

Konkrete Mangelliste:

1. `rust/docs/_work/sp1/data-landscape.md:62`, `:83`, `:97`, `:300`, `:313`, `:327`: Patchnotes als aktiven Consumer/Writer fuer `changelog_posts`, `deadlock_changelogs`, `kv_store` aufnehmen; `deadlock_changelogs` nicht als `tot=ja`/consumerlos fuehren.
2. `rust/docs/_work/sp1/data-landscape.md:281`, `:284`, `:400`: Namespace/Domain fuer Patchnotes-Tabellen in der geteilten DB klaeren. Mindestens `changelog_posts` und `deadlock_changelogs` gehoeren nicht blind in `bot`/tot, sondern brauchen Patchnotes-Zuordnung oder explizite Migrationsentscheidung.
3. `rust/docs/_work/sp1/data-landscape.md:400-402`: Patchnotes-Abschnitt um die Laufzeit-Persistenz aus `inventory/patchnotes.md:21-31` ergaenzen, damit klar bleibt: keine eigene SQLite-Datei, aber aktive Nutzung der geteilten `deadlock.sqlite3`.

Alle anderen geforderten Pruefpunkte sind PASS.
