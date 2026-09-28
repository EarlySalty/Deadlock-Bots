status: erledigt
Datum: 2026-09-29

# Fix R1: Abschlussbericht

ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt done | Artefakt: .tasks/2026-09-28-moderation-negierte-scam-hinweise/
WIRKUNGSPRUEFUNG[WP-1]: 2 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
TESTNACHWEIS[TW-1]: 91 passed, 0 ignored | Baseline: 0 alt rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne FIX-R1-BERICHT.md
MERGEPROTOKOLL[MS-1]: 5 Git-Schritte einzeln | Anläufe: 0 | Gate: Main-Gate nicht aufgerufen, kein Merge/Deploy

## Änderungen und Belege

Der gemeinsame Prüfer segmentiert jetzt an einem Gegensatzwort, wenn der Folgesatz einen eigenen Scam-Hinweis enthält. Eine nachgestellte Verneinung bleibt beim ursprünglichen Subjekt. Die sieben Segmentierungswörter haben Gegenproben für Verneinung und positiven Wechsel. Warnhinweise im Bildkontext lösen die Entlastung nicht mehr durch das Wort „im“ oder „in“ aus. Der Fall mit „und Warnung an Moderatoren“, zitierte Hinweise, der Pizza-Text und der positive Kommafall bleiben in den Tests abgedeckt.

Code und vorhandenes `REVIEW.md` sind in Commit `1595c39994f231dde4cbdb8289470bc6d816d7bb` gesichert. Die Analyzer- und Verifier-Aufrufer verwenden weiter denselben gemeinsamen Prüfer. Der Änderungspfad fügt keinen Fremddienstaufruf hinzu.

## Prüfungen

- Gezielte Suite: `PATH=/home/nathanael/.cargo/bin:$PATH cargo test --manifest-path Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored` im Rust-Verzeichnis. Ergebnis: 10 passed, 0 failed, 0 ignored, 81 gefiltert.
- Gesamte Suite: `cargo test -p dl-moderation --features testing -- --include-ignored` nach `cargo run -p dl-central-migrate`. Ergebnis: 91 passed, 0 failed, 0 ignored, 0 gefiltert. Verwendet wurde ein lokaler Wegwerfcontainer `timescale/timescaledb:2.17.2-pg16` mit 512 MB und 2 CPUs. `CENTRAL_TEST_DSN`, `DEADLOCK_CENTRAL_DSN`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `SQLX_OFFLINE=true` waren gesetzt. Der Container wurde nach dem Lauf entfernt.
- `cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`, `rustfmt --check --edition 2021 crates/dl-moderation/src/moderation_verdict.rs` und `git diff --check`: bestanden.
- Ein Suite-Anlauf ohne Test-DSN ergab 84 passed und 7 Datenbankfehler mit der Meldung, dass `CENTRAL_TEST_DSN` fehlt. Mit der dokumentierten Wegwerf-DB bestand die Suite mit 91 Tests. Ein erster Lauf mit dem systemweiten alten Cargo konnte das Lockfile-Format 4 nicht lesen; die Tests wurden danach mit `/home/nathanael/.cargo/bin/cargo` ausgeführt.

## Grenzen und Git-Stand

Der bestehende unabhängige Review mit den zwei Befunden ist unter `REVIEW.md` erhalten. Eine weitere unabhängige Reviewer-Runde fand wegen der Vorgabe „keine Unter-Threads oder Unter-Agenten“ nicht statt. Die Prüfung hier umfasst den Diff, die gezielten Gegenbeispiele, die bestehende Suite und Clippy. Das lokale Main-Gate wurde nicht erneut aufgerufen; es hatte im bestehenden Bericht fünf BLOCK-Ergebnisse. Kein Merge und kein Deploy.

Der Code-Commit lautet `1595c39994f231dde4cbdb8289470bc6d816d7bb`. Der Bericht wird separat auf `fix/moderation-negierte-scam-hinweise` gesichert und zusammen mit diesem Branch gepusht.
