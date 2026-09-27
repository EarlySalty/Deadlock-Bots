# Prüfprotokoll: qualifizierte Einladungen

Arbeitsbasis: `42175e5fd68a1c83bf4201b4c40cca1099f00652`. Branch: `codex/qualified-discord-invites`. Fachlicher Vertrag, Graphify-Befund, Discord-Quellen, Dateikonfiguration und Grenzen stehen in `qualified-invites.md`.

PR: https://github.com/EarlySalty/Deadlock-Bots/pull/466
Gekoppelte Twitch-PR: https://github.com/EarlySalty/Deadlock-Twitch-Bot/pull/999

## Lokale Prüfungen

Cargo 1.97.1 wird explizit über `/home/nathanael/.cargo/bin/cargo` verwendet. Shell-Testskripte laufen mit `rustup run stable`, damit auch deren Cargo-Aufrufe dieselbe Toolchain verwenden.

- `cargo fmt` wurde für dl-activity, dl-broker, dl-bot, dl-core, dl-discord, dl-twitch-invite-sync und dl-voice ausgeführt. Ein erneutes `cargo fmt --check` meldet vorbestehende Formatabweichungen in unbeteiligten Dateien, unter anderem modglue, outbox und TempVoice. Diese wurden nicht mitformatiert. Der anschließende explizite `rustfmt --check --edition 2021 --config skip_children=true` über alle 22 geänderten und neuen Rust-Dateien besteht. Eine durch `include!` eingebundene Testdatei wurde dabei zusätzlich korrigiert.
- `cargo clippy --locked` für dieselben sieben Pakete, `--all-targets -j 2`: erneut Exit 0. Zwölf vorbestehende Warnungen stammen aus dem bisherigen Broker-Testmodul; sie sind keine fehlerfreie Gesamt-Lintbilanz.
- `central_test_db.sh cargo test -p dl-activity --features testing invite -j 2 -- --test-threads=2`: 17 fachliche Tests bestanden. Darunter zehn Tests mit frisch migrierten isolierten Postgres-Datenbanken, drei Live-Voice-Tests und vier reine Qualifikationsprüfungen. Drei ebenfalls durch den Filter gefundene, bereits explizit ignorierte LFG-Tests sind nicht Teil dieses Nachweises.
- Die Postgres-Prüfungen umfassen Nachrichten- und Tagesgrenzen, Voice-Ausschlüsse, laufende Voice-Belege, Rejoins, Bots, Altmitglieder, zeitversetzte Auswertung, Parallelität, unveränderliche Historie, mehrdeutige Invite-Codes, Cursor-Seiten und nach Retention nicht wiederverwendete Quellereignis-IDs.
- `cargo test --locked -p dl-broker twitch_invites -j 2`: erneut sechs Tests bestanden. Beide Routen prüfen echte Loopback-Herkunft und internen Token vor jedem Port-Zugriff; hinzu kommen Guild-/Kanal-Allowlist, Payload-/Cursor-Validierung und Antwortvertrag ohne Discord-Identifikatoren.
- Im vorherigen Implementierungslauf bestanden zusätzlich die vier Postgres-Tests des dl-bot-Moduls für persönliche Links, Parallelität, Deckel, Widerruf und Rollback. Die erweiterte PR-CI führt diese jetzt ebenfalls aus.

Ein erster Wiederanlauf ohne Test-DSN scheiterte erwartungsgemäß an der fehlenden Testkonfiguration. Der erfolgreiche Nachweis verwendet den vorhandenen Wegwerf-Datenbank-Wrapper, nicht die Produktionsdatenbank.

## Testdatenbank und CI

Der bestehende `rust/scripts/central_test_db.sh` erstellt einen eigenen Container mit `timescale/timescaledb:2.17.2-pg16` auf einem freien Loopback-Port. Der Test-Harness erzeugt zusätzliche isolierte Datenbanken, führt die echten zentralen Migrationen aus und räumt sie wieder auf. Es wurden keine Produktionssecrets gelesen, keine Produktionsmigration ausgeführt und keine laufenden Botdienste verändert.

Der ursprüngliche PR-Head `84681072e057f8f3d62217516d7a49eeff5ef30d` bestand den vorhandenen Rust-Workspace-Gate, die Konfigurationsprüfung und beide Knowledge-Prüfungen. Beispiel: https://github.com/EarlySalty/Deadlock-Bots/actions/runs/36299781107

`Rust PR CI` führt nun vor dem bestehenden vollständigen Repository-Gate zusätzlich die Invite-Tests von dl-activity, dl-broker, dl-bot, dl-core und dl-discord gegen eine eigene Datenbank aus. Dadurch werden neue fachliche Tests nicht nur lokal geprüft oder durch einen reinen Workspace-Build ersetzt. Endgültiger Head-SHA, zugehörige GitHub-Actions-Läufe, Ergebnisse und etwaige offene Befunde werden in der PR-Beschreibung festgehalten.

## Abschlussgrenze

Beide PRs bleiben Draft. Der vorhandene `PR Release Gate` könnte sonst automatisch mergen; ein grüner Draft-Release-Check ist kein Merge-Nachweis. Das semantische Modellreview wird im Draft übersprungen und bleibt offen. Kein Merge, kein Deploy, keine Produktionsmigration und kein Dienstneustart sind Bestandteil dieses Abschlusses.
