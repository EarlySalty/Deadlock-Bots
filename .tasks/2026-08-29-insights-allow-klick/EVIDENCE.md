status: erledigt
datum: 2026-08-29
contract: CONTRACT.md

Repo-Aufklärung vor dem ersten Edit. Jede Zeile ist eine Fundstelle `pfad:zeile`,
keine Vermutung. Der Hook (R11) gibt Quellcode-Edits erst frei, wenn hier
mindestens 3 Fundstellen stehen. Drei ist die Untergrenze, nicht das Ziel.

## Analoge Implementierungen (wie löst das Repo so etwas schon?)

- `scripts/run_twitch_invite_sync.sh:16` — sourced `~/.config/deadlock-bots/infisical.conf` (Project-ID, API-URL, Env), dann Loader `--profile all`
- `scripts/run_dl_bot_service.sh:21` — dasselbe Infisical-Muster, Credential aus `CREDENTIALS_DIRECTORY`
- `rust/bin/dl-insights-sync/src/allow.rs:67` — bestehender Overlay-Klick per xdotool, wird korrigiert statt neu gebaut

## Bestehende Abstraktionen (werden wiederverwendet, nicht nachgebaut)

- `rust/bin/dl-insights-sync/src/brave_cdp.rs:382` — `connect_browser` / `handshake_with_keys` bleibt der CDP-Einstieg
- `rust/bin/dl-insights-sync/src/allow.rs:28` — `open_inspect_tab` bleibt der Weg, `brave://inspect/#remote-debugging` im laufenden Fenster zu öffnen
- `service/systemd/dl-insights-sync.service:1` — bestehende Unit, nur ExecStart auf das Run-Skript umbiegen

## Relevante Tests (laufen vorher, laufen nachher)

- `rust/bin/dl-insights-sync/src/allow.rs:151` — Inspect-Titel/URL
- `rust/bin/dl-insights-sync/src/allow.rs:157` — Fensterwahl bevorzugt Inspect vor New Tab
- `rust/bin/dl-insights-sync/src/brave_cdp.rs:717` — Port-Datei, Origin-loser Handshake, `/json/version`-Parser

## Öffentliche Schnittstellen und Verträge (dürfen nicht brechen)

- `docs/server_insights.md:70` — Betrieb: Brave-CDP, offizielle CSVs, Timer montags 06:15 und 07:15
- `rust/bin/dl-insights-sync/src/main.rs:1` — Import nur nach `activity.insights_imports`

## Änderungsfläche (welche Dateien voraussichtlich angefasst werden)

- `rust/bin/dl-insights-sync/src/allow.rs` — Fokus heben, Tasten per XTEST, nicht XSendEvent `--window`
- `rust/bin/dl-insights-sync/src/brave_cdp.rs` — erster Handshake Tab+Return, nicht Return
- `scripts/run_dl_insights_sync.sh` — Infisical wie die anderen Dienste
- `service/systemd/dl-insights-sync.service` — ExecStart auf das Skript
- `docs/server_insights.md` — Allow-Klick-Beschreibung an die echte Overlay-Tastenfolge

## Offene Architekturfrage

- keine
