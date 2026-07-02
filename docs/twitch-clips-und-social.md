# Twitch-Clips und Social Media

## Worum geht es?

Der Twitch-Bot hat eine Social-Media-Pipeline für Clips: sammeln, aufbereiten, vor dem Posten prüfen, auf Plattformen verteilen und später wieder aus dem lokalen Speicher entfernen. Gedacht als Brücke zwischen Twitch-Momenten und TikTok, YouTube Shorts oder Instagram Reels.

**Wichtig vorab: Die Pipeline ist gebaut, aber aktuell größtenteils nicht scharf geschaltet.** Der automatische Clip-Fetch ist per Konfiguration deaktiviert, und Upload/Analytics laufen nur mit hinterlegten Plattform-Zugängen. Als Streamer merkst du von diesem System heute im Normalfall nichts — es postet nichts automatisch.

## Wie ist die Pipeline gedacht (wenn aktiv)?

1. Der Bot durchsucht aktive Partnerkanäle regelmäßig (6-Stunden-Takt) nach neuen Twitch-Clips und zieht pro Streamer bis zu 20 Stück. Kandidaten sind nur aktive Partner (nicht de-partnert, nicht archiviert).
2. Neue Clips landen als `pending` im System.
3. Ein Enrichment-Schritt baut Titel, Beschreibungen und Hashtags pro Plattform (LLM-gestützt). Eine automatische Transkription ist derzeit bewusst abgeschaltet — Transkript-Felder bleiben leer und werden übersprungen.
4. Vor dem Upload steht ein Approval-Schritt: Clips gehen nicht blind live, Plattformen können einzeln freigegeben, übersprungen oder verworfen werden. Die Freigabe läuft über den Admin-Bereich (`/social-media-admin` im Dashboard); ein DM-Freigabe-Flow existiert im aktuellen System nicht.
5. Erst nach Freigabe wandert der Clip in die Upload-Queue; nach dem Upload sammelt das System Performance-Daten.

## Upload und Plattformen

Gebaut ist die Pipeline für TikTok, YouTube und Instagram. Praktische Einschränkung: Instagram braucht eine öffentlich erreichbare Video-URL — lokale Dateien lehnt der Uploader ab, ein Zwischen-Hosting dafür gibt es noch nicht. TikTok/YouTube laden direkt hoch. Manuelle Uploads (Datei statt Twitch-Clip) kennt die Pipeline ebenfalls.

## Retention: wie lange bleibt ein Clip im System?

Die lokale Clip-Retention ist konkret: 14 Tage ab Erstellung. Der Retention-Worker räumt Clips weg, wenn sie auf allen aktiven Plattformen veröffentlicht oder bewusst verworfen wurden — erst die lokale Datei, dann der Datenbankeintrag. Das System ist keine Dauerablage, sondern eine Upload-Pipeline mit begrenzter Zwischenhaltung.

## Analytics: was kommt nach dem Upload?

Nach veröffentlichten Clips sammelt der Bot Leistungsdaten nach (Views, Likes, Kommentare, Shares, je nach Plattform mehr) — in Buckets für `24h`, `7d` und `30d`, daraus entstehen Wochen- und Monatsreports.

## Grenzen und wichtige Hinweise

- **Aktueller Status: inaktiv bis zur Freischaltung.** Clip-Fetch hängt an einem Aktivierungs-Schalter, Upload/Analytics an hinterlegten Plattform-Secrets. Ohne beides passiert nichts Automatisches.
- Der Social-Media-Bereich ist admin-only (`/social-media-admin`), kein Streamer-Self-Service.
- Freigabe ist ein echter Stopper: ohne Approval kein Upload.
- Retention nach 14 Tagen betrifft die lokale Zwischenhaltung, nicht die bereits geposteten Plattform-Inhalte.
- Analytics kommen zeitversetzt, nicht sofort im Moment des Uploads.
