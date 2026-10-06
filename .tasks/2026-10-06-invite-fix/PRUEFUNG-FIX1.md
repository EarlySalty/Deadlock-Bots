# Fixer Runde 1: Prüfnachweis

## Auftrag und Änderungen

Grundlage: `BRIEFING-B-FIX1.md`, Gate-BLOCK aus `REVIEW.md` und Nutzerentscheidung „ein Brain“ aus `VON_HAUPT.md:9-11`. Die nachfolgende Klarstellung des Haupt-Orchestrators erlaubt das Abschalten der fehlerhaften Lounge-Ergebnisantwort ohne Warten auf Paket A.

1. Rückblick setzt `last_seen` nicht mehr. Historische Speicherung wiederholt CAS-Konflikte und bewahrt neuere Hinweise. Regression für Rückblick vor Gateway und Gateway vor Rückblick, jeweils direkte Bitte und Raumbitte, einschließlich Gateway-Duplikat.
2. Die Uhr wird nach `load` und nach dem erfolgreichen Versand-Claim erneut gelesen. Abgelaufene Claims gehen über denselben Abschlussweg auf `Expired`, ohne Steam-Aufruf. Regression für beide DB-Wartepunkte und beide Bittearten. Der Hinweiszweig prüft bereits nach der Speicherung neu; dessen Regression belegt die Unterdrückung nach Ablauf.
3. Öffentliche Ergebnisantwort und eigene Umformulierung sind entfernt. Erfolg, Audit, laufender Versand und Fehler bleiben gespeichert; der Freundescode-Hinweis ist unverändert. Regressionen prüfen fehlende Ergebnisantworten und erhaltene Zustände auch bei Ablauf während Versand oder Abschlussspeicherung.
4. Test-Uhr pro Instanz injizierbar, keine globale veränderliche Uhr. `unwrap` in der betroffenen Testdatei durch begründete `expect` ersetzt, keine Lints abgeschaltet. Der Bestandsfehler in `platform_connections.rs:30` bleibt unangetastet.

## Tests

```text
PATH=/home/nathanael/.cargo/bin:/usr/local/bin:/usr/bin:/bin SQLX_OFFLINE=true CENTRAL_TEST_DSN=postgres://deadlock:testpw@127.0.0.1:33063/deadlock_test DEADLOCK_CENTRAL_DSN=postgres://deadlock:testpw@127.0.0.1:33063/deadlock_test CARGO_BUILD_JOBS=1 /home/nathanael/.cargo/bin/cargo test --manifest-path /home/nathanael/.worktrees/Deadlock-Bots-invite-fix/rust/Cargo.toml -p dl-community --features testing invite_lounge -- --include-ignored

test result: ok. 48 passed; 0 failed; 0 ignored; 0 measured; 569 filtered out; finished in 8.20s
```

Exit 0, vollständige Ausgabe `/tmp/invite-fix1-tests.log`. Zwei Tests mit echtem Postgres, Zustandsmaschine mit aufzeichnenden Discord- und Steam-Ports. Keine Live-Prüfung daraus abgeleitet. Container `dl-invite-fix1-test-20261006`, Limit 512 MiB, gemessen 90.34 MiB, sofort nach Testende entfernt. Rustfmt für beide geänderten Quelldateien ausgeführt.

TESTNACHWEIS[TW-1]: 48 passed, 0 ignored | Baseline: nicht neu erhoben, keine alten Testfehler behauptet

## Clippy

```text
PATH=/home/nathanael/.cargo/bin:/usr/local/bin:/usr/bin:/bin SQLX_OFFLINE=true CARGO_BUILD_JOBS=1 /home/nathanael/.cargo/bin/cargo clippy --manifest-path /home/nathanael/.worktrees/Deadlock-Bots-invite-fix/rust/Cargo.toml -p dl-community --all-targets --features testing -- -D warnings

PATH=/home/nathanael/.cargo/bin:/usr/local/bin:/usr/bin:/bin SQLX_OFFLINE=true CARGO_BUILD_JOBS=1 /home/nathanael/.cargo/bin/cargo clippy --manifest-path /home/nathanael/.worktrees/Deadlock-Bots-invite-fix/rust/Cargo.toml -p dl-community --all-targets --features testing --no-deps -- -D warnings
```

Beide Exit 101. Standardlauf: unveränderte `explicit_auto_deref` in `dl-central-db/src/platform_connections.rs:30`, wie archivierte Baseline `/tmp/invite-fix-clippy-baseline.log`. Isolierter Paketlauf: 134 Diagnosen statt main 228 und vorherigem Fix 250. Im Invite-Testmodul jetzt 0 statt main 94 und vorher 116. Übrige 134 Diagnosen identisch nach Fehlertyp und Datei, kein eigener neuer Befund. Strenger Workspace-Clippy ist nicht grün. Vollständige Ausgaben `/tmp/invite-fix1-clippy.log` und `/tmp/invite-fix1-clippy-own.log`.

## Wirkung und Zwillinge

`rg -n 'reply_text|last_seen|is_fresh|finish_dispatch|clock_millis' invite_lounge.rs`: Live-Cursor schreibt nur `update_state`. Verlauf liest ihn lediglich. Einziger öffentlicher Lounge-Sendepfad ist der unveränderte Code-Hinweis, mit Uhrprüfung nach CAS. Steam wird nach frischem Claim gerufen und sein Resultat über CAS gespeichert. Öffentliche Ergebnisantworten haben keinen verbleibenden Sendepfad. Steam-Fehler werden im gespeicherten Ergebnis und Journal sichtbar, kein automatischer zweiter Versuch eines abgeschlossenen Claims.

WIRKUNGSPRUEFUNG[WP-1]: 2 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 3/3 geprüft

TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 2 belegt | Senke: PRUEFUNG-FIX1.md

## Abschluss

Gate-Folgerunde, Merge, Release, Neustart und Live-Prüfung folgen. Nicht als erledigt behauptet.
