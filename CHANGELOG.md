## #3 — Tierlist-Backend: WR-Daten alle 8 Stunden automatisch

- Neuer Service liefert die Hero-Tierliste der Website mit Live-Winrates pro Skill-Bucket
- Daten werden alle 8 Stunden automatisch aktualisiert, immer auf Basis des aktuellen Patches
- Drei Skill-Buckets verfügbar: All, Phantom+, Eternus
- Admin-Endpunkte für Beschreibungen, Streamer-Listen und Schwellen — Login über bestehenden Discord-Flow
- Build-Voting (👍 / 👎) mit Rate-Limit pro Browser

## #2 — Voice Feedback geht nicht mehr an bestehende User nach Bot-Neustart

- Nutzer, die beim Neustart bereits im Voice-Call saßen, bekommen kein fälschliches „erstes Mal"-Feedback mehr
- Prüfung erweitert: beide Tabellen (voice_stats und voice_session_log) werden gecheckt, nicht nur eine
- Feedback-Retry bei DMs-deaktiviert läuft jetzt nicht mehr ewig: der ursprüngliche Zeitstempel bleibt erhalten und fällt nach 72 Stunden aus dem Fenster

## #1 — Sicherheitslücke: Bot-API nicht mehr von außen erreichbar

- Der interne Statistik-Server (Port 8768) war versehentlich von außen direkt erreichbar
- Jetzt lauscht er nur noch auf localhost — externer Zugriff ohne Caddy nicht mehr möglich
