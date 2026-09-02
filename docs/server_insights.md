# Server-Einblicke (Admin-Dashboard)

Eigene Nachbildung der Discord-Server-Insights auf Basis unserer Bot-Daten.
Seite: `GET /insights` im dl-web-Dashboard (gleiche Discord-OAuth-Session wie `/admin`),
statisches Frontend `service/static/insights.html`, Daten über `/api/insights/*`.

## Datenpfade

Drei bewusst getrennte Welten. Keine darf die andere ersetzen:

| Pfad | Quelle | Was sie kann | genutzt von |
|---|---|---|---|
| Live / nutzerspezifisch | `activity.message_metadata_events`, `voice_metadata_events`, `interaction_events`, `presence_daily_seen`, `journey_user_state` + Tagesaggregate | User, Kanal, Zeitpunkt. Unsere Auswertungen. | alle `/api/insights/*` unter `live` |
| Discord-CSV / anonym | `activity.insights_imports` aus den Portal-Buttons „CSV exportieren“ | Offizielle Discord-Zahlen, ohne User. Abgleich und Historie über 120 Tage hinaus. | dieselben Endpunkte unter `imported` |
| Legacy-Sessions | `activity.voice_session_log`, `activity.message_activity` | Punkte/Peak/Kanalnamen seit Tracking-Beginn | `/api/voice-history`, `/api/server-stats` |

Der Wochenjob schreibt nur in `insights_imports`. Er löscht und überschreibt niemals die Live-Tabellen.

Innerhalb von `insights_imports` gilt: jeder Import überschreibt seine eigenen
Zeilen (`guild_id`, `import_kind`, `period_start`, `dimension`) und sonst nichts.
Einzige Ausnahme sind die Quellen-Exporte (`joins_by_source`): dort kann eine
Quelle aus der Liste verschwinden, deshalb wird der Zeitraum ersetzt, den die
Datei abdeckt, von ihrem frühesten bis zu ihrem spätesten Datum. Ältere Wochen
bleiben stehen. Import und Ersetzen laufen in einer Transaktion, ein Abbruch
mittendrin lässt also keinen halb geleerten Zeitraum zurück.

Ältere Zeiträume für Insights kommen über den CSV-Import (unten), nicht aus dem Legacy-Pfad.

## Neue Bausteine

- **Presence-Tracking** (`DL_ENABLE_PRESENCE_INTENT=1`, Default aus): fordert den
  privilegierten `GUILD_PRESENCES`-Intent an (Portal-Schalter muss an sein, sonst
  verbindet das Gateway nicht!) und schreibt pro User+Tag eine Zeile
  `activity.presence_daily_seen` („Besucher"-Näherung). Verdichtung nach 180 Tagen
  in `presence_daily_aggregates`, Löschpfad in `dl-community/privacy.rs` erweitert.
- **Vanity-Attribution** (`dl-bot/src/vanity.rs`): pollt minütlich
  `GET /guilds/{id}/vanity-url`; Snapshots nur bei Änderung
  (`activity.vanity_uses_snapshots`, Purge nach 90 Tagen). Bei uses-Anstieg werden
  rückwirkend bis zu `delta` unattribuierte Joins im Fenster (letzter Snapshot −90 s
  Kulanz bis jetzt) auf Bucket `vanity` umgeschrieben. Näherung mit bis zu
  Poll-Intervall Latenz.
- **Member-Directory-Sweep** (`dl-bot/src/vanity.rs`): täglich (und beim Start)
  REST-Pagination über alle Guild-Members → `activity.guild_member_directory`
  (joined_at, Kontoalter aus Snowflake, present-Flag). Liefert exakte
  Mitgliedsdauer-Verteilung und den Anker für den Mitglieder-Gesamtverlauf.
- **CSV-Import** (`POST /api/insights/import?guild_id=…`): frisst die
  „CSV exportieren"-Dateien aus dem Discord-Portal, erkennt den Export-Typ an der
  Header-Signatur (Registry in `insights.rs::detect_import`, bei unbekannten Headern
  kommt HTTP 400 mit den gefundenen Spalten zurück → Registry erweitern) und
  upsertet idempotent in `activity.insights_imports`. Live- und Import-Daten werden
  in den API-Antworten strikt getrennt (`live` vs. `imported`).

## Endpunkte

Alle hinter `guard_read`, Parameter `interval=weekly|daily`, `from`, `to` (Default:
letzte 8 Wochen), optional `guild_id`:

`/api/insights/overview` (Kennzahlen-Kacheln + Vorperioden-Vergleich) ·
`/growth` (Joins nach Quelle, Leaves nach Mitgliedsdauer, Mitgliederverlauf) ·
`/activation` (Interaktion am Beitrittstag) · `/retention` (Woche-1-Bindung:
Kohorte = Beitrittswoche Mo–So UTC, gebunden = Aktivität in W+1, nur abgeschlossene
Kohorten) · `/engagement` (Besucher/Beiträger ≥3 Nachrichten oder Voice, Nachrichten,
Sprachminuten) · `/audience` (Mitgliedsdauer, Kontoalter der Neuzugänge) ·
`/top-invites` (28 Tage) · `POST /import` (guard_mutate + CSRF).

## Betrieb

- Migration `2026070610_server_insights_backend.sql` (additiv) via dl-central-migrate.
- Bot-Service braucht `DL_ENABLE_PRESENCE_INTENT=1` für Besucher-Zahlen.
- Wöchentlicher Sync über die **eingeloggte Brave-Sitzung**: `dl-insights-sync`
  hängt sich per CDP an (`DevToolsActivePort`), dumpt Highcharts aus dem
  Developer-Portal und spielt die CSVs ein. Kein gespeichertes User-Token.
  Remote Debugging bleibt unter `brave://inspect` an. Läuft Brave nicht, startet
  der Job das Default-Profil mit `--remote-debugging-port` und
  `--remote-allow-origins=*`. Das Overlay „Allow remote debugging?“ klickt der
  Job selbst weg: zuerst Tab und Return auf dem schon offenen Overlay. Nur wenn
  der Handshake trotzdem scheitert, kommt ein Inspect-Tab, dann noch einmal Tab
  und Return. Cancel trägt den Fokusring, Return allein würde Cancel treffen.
  Welcher Tab vorher aktiv war, ist egal. Danach klickt der Job
  die Buttons „CSV exportieren“, bestätigt die Download-Leiste und spielt
  genau diese offiziellen Dateien ein. Ohne offizielle CSV bricht der Job ab;
  der Highcharts-Weg läuft nur noch, wenn man ihn über `INSIGHTS_BRAVE_DUMP_DIR`
  ausdrücklich anstößt, nie im regulären Lauf. Timer montags 06:15 und
  07:15, Persistenz an, User-Linger an. Die vom Job geöffneten Tabs
  gehen danach wieder zu. Ein zweiter DevTools-Client (MCP) blockiert den
  Handshake trotzdem; der Timer versucht es montags 06:15 und 07:15. Manueller
  Probe: `INSIGHTS_CDP_SMOKE=1 dl-insights-sync`. User-Token-Pfad bleibt
  ungenutzt.
- Manueller Nachzug: Portal → CSVs → Dropzone auf `/insights`.
- Grenzen: Länder/Geräte/Referrer liefert die API nicht (nur via CSV-Import);
  Besucher ist eine Presence-Näherung, nicht Discords Kanal-View-Definition;
  Retention-Rohdaten reichen 180 Tage zurück.
