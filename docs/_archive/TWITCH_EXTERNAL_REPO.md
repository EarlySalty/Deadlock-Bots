# Externe Streamer-/Twitch-Integration

Deadlock enthält keine Twitch-Verwaltung, kein Twitch-Dashboard, keine Twitch-Runtime-Sonderbehandlung und keine Twitch-Migrationsskripte mehr.

Diese Funktionen wurden in das separate Repo `Deadlock-Twitch-Bot` ausgelagert.

## Was in Deadlock bewusst bleibt

- Der Discord-Slash-Command `/streamer`
- Die lokale Discord-Rollenvergabe nach bestätigter externer Rückmeldung
- Die lokale Discord-Benachrichtigung nach erfolgreichem Partner-Onboarding
- Die kleine Adapter-Schicht [cogs/welcome_dm/twitch_partner_integration.py](/C:/Users/Nani-Admin/Documents/Deadlock/cogs/welcome_dm/twitch_partner_integration.py)

## Voraussetzungen für `/streamer`

- `Deadlock-Twitch-Bot` ist lokal verfügbar
- Standardmäßig erwartet Deadlock ein Geschwisterverzeichnis `../Deadlock-Twitch-Bot`
- Alternativ kann der Pfad über `DEADLOCK_TWITCH_BOT_DIR` gesetzt werden
- Für die externe Autorisierung müssen `TWITCH_CLIENT_ID`, `TWITCH_CLIENT_SECRET` und `TWITCH_RAID_REDIRECT_URI` verfügbar sein

## Wo Twitch-Betrieb jetzt stattfindet

- Twitch-OAuth und Discord-Zuordnung
- Blacklist-/Opt-out-Prüfung
- Raid-/Analytics-/Social-Media-Betrieb
- Twitch-spezifische Datenmigrationen und Dashboards

Dafür ist ausschließlich `Deadlock-Twitch-Bot` zuständig.
