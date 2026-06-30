# Inventar - Patchnotes

Scope: `/home/naniadm/Documents/Deadlock--Patchnotes-Bot`

Scan: `*.db`, `*.sqlite`, `*.sqlite3`, `*.db3`, `*.json`, `*.yaml`, `*.yml`, `*.csv` unter dem Repo plus alle Dateien unter `data/`; ausgeschlossen wurden `.git`, `node_modules`, `__pycache__`, `venv`, `.venv` und Cache-Verzeichnisse.

Ergebnis: Im Patchnotes-Repo existiert keine eigene SQLite-Datei. Deshalb wurde keine `patchnotes.tables.json` erzeugt; das SP1-Gate fuer eine separate Patchnotes-DB entfaellt. Das heisst aber nicht, dass Patchnotes keine SQLite nutzt: der laufende `deadlock-patchnotes.service` verwendet die geteilte `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`.

## Persistenz-Artefakte im Repo

| Pfad | Typ | Zweck | Einordnung |
|---|---|---|---|
| `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/.github/dependabot.yml` | YAML | GitHub Dependabot-Konfiguration. | statisch/Config |
| `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/.github/workflows/codeql.yml` | YAML | GitHub CodeQL-Workflow. | statisch/Config |
| `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/.github/workflows/secret-scanning.yml` | YAML | GitHub Secret-Scanning-Workflow. | statisch/Config |
| `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/.github/workflows/security.yml` | YAML | GitHub Security-Workflow. | statisch/Config |
| `/home/naniadm/Documents/Deadlock--Patchnotes-Bot/data/patch_signal_history.ndjson` | NDJSON | Patch-Signal- und Verarbeitungs-History. `main.py` haengt Events an und rotiert die Datei ueber `PATCH_SIGNAL_HISTORY_MAX_ENTRIES` (Default: 500). Aktueller Befund: 22 Zeilen, 6900 Bytes. | Lauf-Zustand |

Nicht gefunden im Patchnotes-Repo: `*.db`, `*.sqlite`, `*.sqlite3`, `*.db3`, `*.json`, `*.yaml`, `*.csv`. Die gefundenen YAML-Dateien nutzen die `.yml`-Endung.

## Laufzeit-Persistenz ausserhalb des Repo-Fundsets

Der lokale Repo-Fund ist minimal und datei-/NDJSON-basiert. Der laufende Bot nutzt aber zusaetzlich den geteilten Deadlock-DB-Layer; das ist eine fremde DB-Datei, keine eigene Patchnotes-DB:

- Unit: `deadlock-patchnotes.service`, FragmentPath `/home/naniadm/.config/systemd/user/deadlock-patchnotes.service`.
- ExecStart: `/usr/bin/bash -lc /home/naniadm/Documents/Deadlock--Patchnotes-Bot/scripts/run_patchnotes_bot.sh`.
- WorkingDirectory: `/home/naniadm/Documents/Deadlock--Patchnotes-Bot`; MainPID `5049`.
- `/proc/5049/exe -> /usr/bin/python3.12`; Cmdline `/home/naniadm/Documents/Deadlock-Bots/.venv/bin/python main.py`.
- `/proc/5049/environ` enthaelt `DEADLOCK_HOME=/home/naniadm/Documents/Deadlock-Bots`; kein separates Patchnotes-DB-File.
- `/proc/5049/fd`: offene FDs auf `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3`, `deadlock.sqlite3-wal`, `deadlock.sqlite3-shm`.
- `scripts/run_patchnotes_bot.sh` setzt `DEADLOCK_HOME` standardmaessig auf `/home/naniadm/Documents/Deadlock-Bots`.
- `main.py` fuegt `DEADLOCK_HOME` in `sys.path` ein und importiert `service.db` als `deadlock_db`.
- `service/db.py` loest die DB ueber `DEADLOCK_DB_PATH`, sonst `DEADLOCK_DB_DIR`, sonst den Repo-Default `data/deadlock.sqlite3` auf.

Patchnotes-Consumer der geteilten `deadlock.sqlite3`:

| Tabelle | Nutzung | Evidenz |
|---|---|---|
| `changelog_posts` | read/write/DDL | `main.py` liest per `SELECT`, erstellt per `CREATE TABLE IF NOT EXISTS`, erweitert per `ALTER TABLE`, schreibt per `UPDATE`/`INSERT`. |
| `deadlock_changelogs` | read/write/DDL, Legacy-Backfill | `main.py` erstellt die Legacy-Tabelle bei Bedarf, liest bestehende Rows und schreibt `UPDATE`/`INSERT`. |
| `kv_store` | read/write indirekt ueber `deadlock_db.get_kv/set_kv`, Namespace `patchnotes_bot` | `main.py` nutzt `KV_NAMESPACE = "patchnotes_bot"` mit Keys `last_forum_url`, `last_test_post_url`, `last_prepared_patch_signature`, `dispatch_content_sha256:*`; `service/db.py` implementiert `kv_store(ns,k,v)`. |

Vollscan gegen alle 124 Tabellennamen aus `deadlock-bots.tables.json`: direkte Patchnotes-Code-Treffer nur fuer `changelog_posts` und `deadlock_changelogs`; `kv_store` ist der einzige indirekte Treffer ueber den gemeinsamen KV-Adapter. Plausibilisierung gegen die lebende DB: `changelog_posts` 117 Rows, `deadlock_changelogs` 117 Rows, `kv_store` mit `ns='patchnotes_bot'` 9 Rows.

Diese geteilte SQLite ist bereits Teil des Deadlock-Bots-Inventars und wurde hier nicht als Patchnotes-Repo-SQLite gezaehlt; deshalb keine separate `patchnotes.tables.json`.

## Weitere Pfade aus dem Code

- `PATCH_SIGNAL_HISTORY_FILE` kann die NDJSON-History per Env auf einen anderen Pfad verschieben; ohne Env zeigt sie auf `data/patch_signal_history.ndjson`.
- `PATCH_OUTPUT_DIR` kann uebersetzte Patches als `patch*.txt` in ein konfiguriertes Ausgabeverzeichnis schreiben. Im Repo wurde kein solcher Output-Ordner gefunden.
- `PATCH_PREPARED_FILE` liest optionale vorbereitete Patch-Dateien; der Helper `scripts/run_prepared_patch_once.sh` nutzt standardmaessig `prepared_patches/2026-04-01-burger-patch.txt` als statischen Input.
- `PATCH_STEAM_VERSION_TRIGGER_FILE` liest standardmaessig `/home/naniadm/Documents/Deadlock-Steam-Bot/cogs/steam/steam_presence/.steam-data/version_trigger.json`; diese externe JSON-Datei existierte lokal beim Scan nicht und wird nicht vom Patchnotes-Repo besessen.
