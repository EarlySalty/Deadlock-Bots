status: erledigt
datum: 2026-08-29
klasse: mittel
repo: Deadlock-Bots

Dieser Contract ist der Maßstab für Implementierung und Merge-Kritiker. Nach dem
Anlegen ist er unveränderlich: der Hook lässt nur noch die `status:`-Zeile und
Anhänge unter `## Amendments` zu. Wer ein REQ oder INV ändern will, schreibt ein
Amendment mit Begründung; Produkt-, API- oder Datenänderungen entscheidet der User.

## Ziel

Der wöchentliche Discord-Insights-Sync klickt das Brave-Overlay „Allow remote
debugging?“ selbst weg und spielt die offiziellen CSVs ein, ohne manuellen Klick
und ohne Infisical-Startfehler.

## Anforderungen (user-sichtbares Verhalten)

- REQ-01: Wenn das Overlay „Allow remote debugging?“ erscheint (Cancel ist der
  Fokus, Allow rechts daneben), bestätigt der Job Allow. Return allein darf das
  Overlay nicht schließen.
- REQ-02: Der systemd-Timer startet den Job über denselben Infisical-Weg wie die
  anderen Bot-Dienste (Config-Datei plus Credential plus Loader). Montag 06:15
  darf nicht mehr mit `INFISICAL_PROJECT_ID is required` sterben, bevor Brave
  überhaupt angefasst wird.
- REQ-03: `INSIGHTS_CDP_SMOKE=1` schließt den CDP-Handshake gegen die laufende
  Brave-Sitzung auf DISPLAY=:10 ab.

## Invarianten (darf sich nicht ändern)

- INV-01: Sync läuft über die eingeloggte Brave-Sitzung per CDP, kein
  gespeichertes Discord-User-Token.
- INV-02: Regulärer Lauf spielt nur offizielle „CSV exportieren“-Dateien ein,
  nicht die Highcharts-Rekonstruktion.
- INV-03: Live-User-Tabellen bleiben unangetastet; geschrieben wird nur
  `activity.insights_imports`.
- INV-04: Bestehende Tests werden nicht gelöscht oder abgeschwächt.

## Nicht-Ziele

- brave-mcp / zweiter DevTools-Client abschalten
- Discord-Portal-UI oder Dashboard-Seite `/insights` umbauen
- Highcharts-Pfad reaktivieren

## Erlaubter Änderungsbereich

- `rust/bin/dl-insights-sync/src/allow.rs`
- `rust/bin/dl-insights-sync/src/brave_cdp.rs`
- `scripts/run_dl_insights_sync.sh` (neu, nach dem Muster der anderen run_*.sh)
- `service/systemd/dl-insights-sync.service`
- `docs/server_insights.md` (Betrieb, Allow-Klick)
- User-Unit `~/.config/systemd/user/dl-insights-sync.service` (Deploy-Kopie)

## Verbotene Änderungen

- `dl-dashboard` Insights-API und Import-Schema
- Infisical-Token-Dateien oder Secrets im Klartext
- Neue `*_ENABLED`-Flags
- Lint-Config, andere systemd-Units

## Offene Produktfragen

- keine

## Amendments
