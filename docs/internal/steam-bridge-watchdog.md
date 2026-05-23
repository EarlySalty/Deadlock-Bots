# Steam-Bridge-Watchdog

## Zweck
Der Steam-Bridge-Watchdog ist ein externer Python-Prozess ausserhalb des Discord-Cog-Lifecycles. Er ueberwacht, ob der Steam-Host fachlich gesund ist, und startet den Host-Service neu, wenn die Bridge zwar laeuft, aber logisch haengt. Die bestehende Doku unter `docs/steam-bridge-watchdog.md` war noch grob korrekt; diese Fassung zieht die Details direkt aus `standalone/steam_bridge_watchdog.py`.

## Architektur
Der Watchdog laeuft als separater Prozess und macht periodisch einen Snapshot aus zwei Quellen:

- `standalone_bot_state` fuer Heartbeat und Runtime-Payload (`standalone/steam_bridge_watchdog.py:118`)
- Diagnosedaten aus `steam_tasks` und `steam_friend_requests` (`standalone/steam_bridge_watchdog.py:139`)

Erkannte Problemklassen:

- Bridge nicht eingeloggt (`not_logged_in`)
- Login gemeldet, aber keine `steam_id64` (`missing_steam_id`)
- festhaengende Friend-Requests mit Timeouts (`friend_requests_stalled`)
- veralteter Heartbeat (`stale_heartbeat`)

Der Watchdog speichert seinen eigenen Zustand in einer JSON-Datei unter `XDG_STATE_HOME` oder `~/.local/state/deadlock-bots/steam_bridge_watchdog.json` (`standalone/steam_bridge_watchdog.py:32`). Dadurch kennt er `first_seen_at`, `last_restart_at` und kann Grace-Period und Cooldown sauber auswerten.

## Konfiguration
CLI-Parameter und Env-Fallbacks:

- `--db-path` bzw. `DEADLOCK_DB_PATH`/`DEADLOCK_DB_DIR`
- `--state-path`
- `--restart-command` bzw. `STEAM_BRIDGE_WATCHDOG_RESTART_COMMAND`
- `--interval` bzw. `STEAM_BRIDGE_WATCHDOG_INTERVAL`
- `--grace-period` bzw. `STEAM_BRIDGE_WATCHDOG_GRACE_PERIOD`
- `--heartbeat-max-age` bzw. `STEAM_BRIDGE_WATCHDOG_HEARTBEAT_MAX_AGE`
- `--restart-cooldown` bzw. `STEAM_BRIDGE_WATCHDOG_RESTART_COOLDOWN`
- `--dry-run`
- `--verbose` bzw. `STEAM_BRIDGE_WATCHDOG_VERBOSE`

Default-Restart-Command ist aktuell `systemctl --user restart deadlock-bot.service` (`standalone/steam_bridge_watchdog.py:318`).

## Admin-Workflow
1. Einmalig lokal testen mit `python standalone/steam_bridge_watchdog.py --once --verbose`.
2. Fuer Dauerbetrieb als `systemd --user` Service starten.
3. Bei Restart-Schleifen erst die State-Datei und die erkannte `reason` pruefen.
4. Nicht sofort die DB "reparieren"; haeufig reicht ein sauberer Host-Restart und die Bridge baut sich neu auf.

## Datenmodell
Der Watchdog schreibt keine DB-Tabellen. Gelesen werden:

- `standalone_bot_state`
- `steam_tasks`
- `steam_friend_requests`

Geschrieben wird nur die lokale State-Datei mit:

- `reason`
- `summary`
- `details`
- `first_seen_at`
- `last_restart_at`
- `last_restart_reason`

## Wartung & Troubleshooting
- Wenn immer nur `No standalone state for steam bridge yet` erscheint, kommt die Bridge nicht bis zum State-Publish.
- Wenn Cooldown aktiv ist, restarten weitere Checks absichtlich nicht sofort erneut (`standalone/steam_bridge_watchdog.py:293`).
- `--dry-run` ist nuetzlich, um Erkennung zu testen, ohne wirklich neu zu starten.
- Die alte Top-Level-Doku warf den Watchdog noch als "externer Helferprozess" aus; das stimmt weiterhin. Neu hier sind vor allem die konkreten Erkennungsgrenzen und der State-File-Mechanismus.

## Code-Referenz
- Einstieg: `standalone/steam_bridge_watchdog.py:311`
- Health-Detection: `standalone/steam_bridge_watchdog.py:63`
- Snapshot-Lesen: `standalone/steam_bridge_watchdog.py:118`
- Restart-Logik: `standalone/steam_bridge_watchdog.py:228`
- Legacy-Doku: `docs/steam-bridge-watchdog.md`
