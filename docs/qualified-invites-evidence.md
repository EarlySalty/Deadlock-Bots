# Prüfprotokoll: qualifizierte Einladungen

Arbeitsbasis: `42175e5f`. Branch: `codex/qualified-discord-invites`. Fachlicher Vertrag, Graphify-Befund, Discord-Quellen, Dateikonfiguration und Grenzen stehen in `qualified-invites.md`.

## Bereits ausgeführte Prüfungen

- `cargo fmt` für dl-activity, dl-broker, dl-bot, dl-core, dl-discord, dl-twitch-invite-sync und dl-voice; zusätzliche explizite rustfmt-Prüfung sämtlicher geänderter/neuer Rust-Dateien. Unbeteiligte Formatierungsänderungen wurden zurückgenommen.
- `cargo clippy` mit denselben Paketen und `--all-targets -j 2`: erfolgreich. Vorhandene Warnungen im bisherigen Broker-Testmodul bleiben getrennt von den neuen Implementierungen sichtbar.
- `cargo test -p dl-activity --features testing qualified_invites`: zwölf Tests bestanden, darunter acht Tests mit vollständig migrierten, isolierten Postgres-Datenbanken.
- `cargo test -p dl-broker -p dl-bot twitch_invites`: zehn Tests bestanden, darunter vier Postgres-Tests für persönliche Links, Parallelität, Deckel, Widerruf und Rollback.
- Weiterer Qualifikationstestlauf nach den ersten Voice-Uhren: 15 Tests bestanden. Zusätzliche Live-Voice- und ID-Retentionsprüfungen sowie der breite Regressionstestlauf werden vor dem Abschluss erneut ausgewertet.

Verwendet wird Cargo 1.97.1, explizit über `/home/nathanael/.cargo/bin/cargo`, und der bestehende SQLx-Offline-Modus. Alle neuen SQL-Abfragen werden zusätzlich gegen das reale Migrationsschema getestet, nicht gegen eine Produktionsdatenbank.

## Testdatenbank

Eigener Wegwerf-Container mit `timescale/timescaledb:2.17.2-pg16`, ausschließlich an 127.0.0.1 gebunden. Der vorhandene Test-Harness erstellt für die zentralen Datenbanktests jeweils eine eigene Datenbank, führt alle Migrationen aus und entfernt sie anschließend. Die bestehenden Testvariablen werden nur per Cargo-Testkonfiguration gesetzt. Es wurden keine Produktionssecrets gelesen oder produktiven Datenbankänderungen ausgeführt.

## Abnahme und CI

Die PR bleibt Draft, weil der vorhandene `PR Release Gate` sonst automatisch mergen kann. Der Nutzerauftrag erteilt keine Merge-, Deploy- oder Restart-Freigabe. Exakter PR-Link, finaler Head-SHA, GitHub-Actions-Läufe und verbleibende Prüfpunkte werden in der PR-Beschreibung festgehalten. Ein erfolgreicher lokaler Teiltest ist kein Ersatz für noch ausstehende oder fehlgeschlagene Prüfungen.
