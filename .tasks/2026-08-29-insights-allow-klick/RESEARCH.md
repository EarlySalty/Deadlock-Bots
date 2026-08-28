status: erledigt
datum: 2026-08-29
klasse: mittel

## Auftrag

Der wöchentliche Insights-Sync soll das Brave-Overlay „Allow remote debugging?“
zuverlässig bestätigen und danach die offiziellen CSVs einspielen.

## Beobachtungen (belegt, Datei:Zeile)

- Overlay sitzt in der Browser-Chrome. Screenshot 2026-08-29, DISPLAY=:10, Brave
  151.1.93.134: drei Knöpfe links nach rechts „Turn off in settings“, „Cancel“
  (Fokusring), „Allow“. `allow.rs:60` behauptet Allow sei oft der Default.
- `brave_cdp.rs:394` sendet zuerst `Return`, bei Fehler erst dann `Tab`+`Return`.
  Return auf dem Default-Fokus klickt Cancel und räumt das Overlay weg. Der
  zweite Versuch trifft nichts mehr.
- `allow.rs:73` sendet `xdotool key --window ID`. Das ist XSendEvent. Chromium
  ignoriert das, solange das Fenster nicht über XTEST bedient wird. Live-Probe:
  `--window` bewegt den Fokusring nicht; `windowactivate` plus `key --clearmodifiers
  Tab` ohne `--window` setzt den Ring auf Allow.
- Gleicher Display, gleiche Sitzung: Websocket-Connect parallel zu Tab+Return
  liefert `HTTP/1.1 101 WebSocket Protocol Handshake`. Return allein würde
  Cancel treffen.
- `/json/version` und `/json/list` bleiben 404, auch mit gesetztem Häkchen
  „Allow remote debugging for this browser instance“ und Server
  `127.0.0.1:9222`. Der Job fällt in `brave_cdp.rs:321` bereits auf den
  Websocket-Pfad aus `DevToolsActivePort` zurück. Das ist kein neuer Bug, der
  Handshake muss trotzdem das Overlay wegklicken.
- Timer 2026-08-24 06:15: `INFISICAL_PROJECT_ID is required`, Exit 1, bevor das
  Binary startet. `service/systemd/dl-insights-sync.service:15` ruft
  `dl-infisical-env` direkt. Die anderen Dienste sourcen vorher
  `scripts/run_dl_bot_service.sh:22` / `scripts/run_twitch_invite_sync.sh:18`.
- Brave läuft ohne `--remote-debugging-port` in der Commandline, lauscht aber auf
  9222 (Inspect-Seite, Häkchen an). `ensure_brave` startet deshalb nicht neu.
- Zweiter DevTools-Client (brave-mcp, Infobar „controlled by automated test
  software“) ist bekannt (`docs/server_insights.md:83`) und bleibt Nicht-Ziel.

## Hypothesen (unbelegt — nie als Fakt weiterreichen)

- keine, der Klick-Pfad ist live gegen das Overlay geprüft.

## Wahrscheinlich zu ändernde Dateien

- `rust/bin/dl-insights-sync/src/allow.rs`
- `rust/bin/dl-insights-sync/src/brave_cdp.rs`
- `scripts/run_dl_insights_sync.sh` (neu)
- `service/systemd/dl-insights-sync.service`
- `docs/server_insights.md`

## Risiken / Seiteneffekte

- xdotool ohne `--window` braucht ein fokussiertes Brave-Fenster auf DISPLAY=:10.
  Der Job stiehlt kurz den Fokus auf diesem Display, das ist schon der Vertrag.
- Ein zweiter DevTools-Client kann den Handshake weiter blockieren. Timer hat
  06:15 und 07:15 als zweiten Versuch.
- Voller Sync schreibt nach `activity.insights_imports`. Smoke prüft nur CDP.

## Offene Fragen

- keine
