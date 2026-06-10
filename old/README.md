# old/ — Archiv der abgelösten Node-Bridge-Reste

Wiederhergestellt aus der Git-History (letzter Stand vor Lösch-Commit f8e8606),
nur als Nachschlage-Backup. Nichts hiervon läuft produktiv.

- `cogs/steam/steam_presence/` — die im Hauptbot-Repo gespiegelte Bridge-Kopie
  (Vollversion liegt im Steam-Bot-Repo unter `old/steam_presence/`).
- `standalone/steam_bridge_watchdog.py` + `docs/deadlock-steam-watchdog.service`
  — der frühere Bridge-Watchdog samt systemd-Unit.

Produktiv ist der Rust-Stack im Deadlock-Steam-Bot-Repo (steam-core/steam-bot).
