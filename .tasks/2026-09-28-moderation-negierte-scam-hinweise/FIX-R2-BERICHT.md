status: erledigt
Datum: 2026-09-29

# Fix R2: Abschlussbericht

ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt done | Artefakt: .tasks/2026-09-28-moderation-negierte-scam-hinweise/
WIRKUNGSPRUEFUNG[WP-1]: 1 Befund | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
TESTNACHWEIS[TW-1]: 91 passed, 0 ignored | Baseline: 0 rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne FIX-R2-BERICHT.md
MERGEPROTOKOLL[MS-1]: 5 Git-Schritte einzeln | Anläufe: 0 | Gate: Main-Gate nicht aufgerufen, kein Merge/Deploy

## Fix und Ergebnis

`direct_warn_report_context` in `rust/crates/dl-moderation/src/moderation_verdict.rs` prüft den unmittelbaren Bezug zwischen „in/im“ und „Warnung/Bericht“. Zulässige Artikel und adjektivische Modifikatoren bleiben Teil der Wortgruppe. Satzzeichen beenden die lokale Wortgruppe. So wird `Phishing in einer Warnung vor Betrugsmaschen.` als Warnkontext erkannt, ohne `im Bild` aus `Sichtbarer Scam im Bild, Warnung an Moderatoren.` auf die spätere Warnung zu beziehen.

Die zwei R2-Beispiele ergeben `false`, die positive Moderator-Gegenprobe bleibt `true`. Die Tests decken zusätzlich eine modifizierte Warnung, bestehende Zitatfälle, die R1-Verneinungen, den Pizza-Text und positive Scam-Hinweise ab. Analyzer und Verifier nutzen unverändert denselben Prüfer. Der Diff fügt keinen Fremddienstpfad hinzu.

Code und `REVIEW-R2.md` sind in Commit `9219428d2e494bdc01f4c29d7b751a12165eb82a` gesichert.

## Prüfungen

- Gezielte Suite: `PATH=/home/nathanael/.cargo/bin:$PATH cargo test --manifest-path Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored` im Rust-Verzeichnis. Ergebnis: 10 passed, 0 failed, 0 ignored, 81 gefiltert.
- Gesamte Suite: `cargo test -p dl-moderation --features testing -- --include-ignored` nach `cargo run -p dl-central-migrate`. Ergebnis: 91 passed, 0 failed, 0 ignored, 0 gefiltert. Verwendet wurde ein lokaler Wegwerfcontainer `timescale/timescaledb:2.17.2-pg16` mit 512 MB und 2 CPUs. `CENTRAL_TEST_DSN`, `DEADLOCK_CENTRAL_DSN`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `SQLX_OFFLINE=true` waren gesetzt. Der Container wurde nach dem Lauf entfernt.
- `cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`, `rustfmt --check --edition 2021 crates/dl-moderation/src/moderation_verdict.rs` und `git diff --cached --check`: bestanden.
- Keine externen Anbieter, Community-Daten oder Produktionsdienste wurden verwendet.

## Grenzen

Eine R3-Prüfung fand wegen der Vorgabe „keine Unter-Threads oder Unter-Agenten“ nicht statt. Das lokale Main-Gate wurde nicht aufgerufen und nicht verändert. Kein Main-Merge und kein Deploy.
