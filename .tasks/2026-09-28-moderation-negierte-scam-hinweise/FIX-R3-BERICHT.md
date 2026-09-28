status: erledigt
Datum: 2026-09-29

# Fix R3: Abschlussbericht

ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt done | Artefakt: .tasks/2026-09-28-moderation-negierte-scam-hinweise/
WIRKUNGSPRUEFUNG[WP-1]: 1 Befund | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
TESTNACHWEIS[TW-1]: 91 passed, 0 ignored | Baseline: 0 rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne FIX-R3-BERICHT.md
MERGEPROTOKOLL[MS-1]: 5 Git-Schritte einzeln | Anläufe: 0 | Gate: nicht aufgerufen, kein Merge/Deploy

## Fix und Gegenproben

`direct_warn_report_context` in `rust/crates/dl-moderation/src/moderation_verdict.rs` beendet den direkten Warn- oder Berichtsbezug an den vorhandenen Satzteilgrenzen. Berücksichtigt werden Satzzeichen einschließlich Doppelpunkt und Komma sowie die vorhandenen Kontrastverknüpfungen. Dadurch reicht „in der Anzeige“ nicht über den Doppelpunkt in `Sichtbarer Scam in der Anzeige: Warnung an Moderatoren.`. Der Scam-Hinweis bleibt positiv.

Die beiden R3-Gegenbeispiele mit „Anzeige“ und „Gruppe“ ergeben `true`. Tests prüfen zusätzlich Doppelpunkt, Komma, Semikolon, Punkt, Ausrufezeichen, Fragezeichen, Zeilenumbruch und die Kontrastverknüpfungen. R1- und R2-Gegenproben, der Pizza-Text, direkte Warn- und Berichtskontexte sowie die Komma-Gegenprobe bleiben erhalten.

Code und `REVIEW-R3.md` sind in Commit `114698291f19dc19521fe26c3cf497e9359349de` gesichert. Die Änderung betrifft den gemeinsamen Prüfer, keine Analyzer-, Verifier- oder Fremddienst-Aufrufer.

## Prüfungen

- Gezielte Suite: `PATH=/home/nathanael/.cargo/bin:$PATH cargo test --manifest-path Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored` im Rust-Verzeichnis. Ergebnis: 10 passed, 0 failed, 0 ignored, 81 gefiltert.
- Gesamte Suite: `cargo test -p dl-moderation --features testing -- --include-ignored` nach `cargo run -p dl-central-migrate`. Ergebnis: 91 passed, 0 failed, 0 ignored, 0 gefiltert. Verwendet wurde ein lokaler Wegwerfcontainer `timescale/timescaledb:2.17.2-pg16` mit 512 MB und 2 CPUs. `CENTRAL_TEST_DSN`, `DEADLOCK_CENTRAL_DSN`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `SQLX_OFFLINE=true` waren gesetzt. Der Container wurde nach dem Lauf entfernt.
- `cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`, `rustfmt --check --edition 2021 crates/dl-moderation/src/moderation_verdict.rs` und `git diff --cached --check`: bestanden.

## Grenzen

Eine unabhängige R4-Prüfung fand wegen der Vorgabe „keine Unter-Threads“ nicht statt. Das lokale Main-Gate wurde nicht aufgerufen oder verändert. Main-Merge und Deploy fanden nicht statt.
