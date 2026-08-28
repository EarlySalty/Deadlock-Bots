status: erledigt
datum: 2026-08-29
klasse: mittel
research: RESEARCH.md

## Ziel

Fertig, wenn (1) der CDP-Smoke gegen die laufende Brave-Sitzung den Handshake
schafft, (2) der systemd-Start nicht mehr an `INFISICAL_PROJECT_ID` stirbt,
(3) Allow per Tab+Return bestätigt wird, nicht per Return auf Cancel.

## Nicht-Ziele

- MCP abschalten
- Dashboard oder Import-Schema anfassen

## Milestones

### M1 — Test für die Tastenfolge
Änderungen: `allow.rs` Tests: Konstante `ALLOW_CONFIRM_KEYS` ist Tab dann Return
Erwarteter Zwischenzustand: neuer Test rot auf main, weil der Handshake noch
Return zuerst sendet
Validierung: `cargo test -p dl-insights-sync -- --include-ignored`
Stop-Regel: Test lässt sich nicht rot bekommen, weil er den Produktivpfad nicht
trifft

### M2 — Klick und Infisical-Start
Änderungen: `allow.rs` (Fenster heben, XTEST-Tasten), `brave_cdp.rs` (erster
Versuch Tab+Return), `scripts/run_dl_insights_sync.sh`, systemd-Unit, Doku
Erwarteter Zwischenzustand: M1-Test grün; Smoke schließt Handshake
Validierung: Unit-Tests plus `INSIGHTS_CDP_SMOKE=1` über das Run-Skript
Stop-Regel: Smoke hängt weiter am Overlay oder Infisical-Start bleibt rot

### M3 — Deploy und Live-Sync
Änderungen: Binary installieren, User-Unit neu laden, Job einmal laufen lassen
Erwarteter Zwischenzustand: Journal ohne Infisical-Fehler, CSVs importiert oder
klarer Portal-Fehler danach
Validierung: `systemctl --user start dl-insights-sync.service` plus Journal
Stop-Regel: Allow-Klick grün, aber Import bricht aus anderem Grund ab — dann
diesen Grund benennen, nicht als Allow-Fix verkaufen

## Verlauf

- 2026-08-29: M1 verifiziert (Test `allow_bestaetigt_nicht_den_cancel_fokus` rot auf `["Return"]`)
- 2026-08-29: M2 verifiziert (`cargo test -p dl-insights-sync`: 8 passed; Smoke `CDP smoke ok (Chrome/151.0.7922.108)`)
- 2026-08-29: M3 verifiziert (`systemctl --user start dl-insights-sync.service` Exit 0, 26 Dateien / 1700 Zeilen, kein Infisical-Fehler)
