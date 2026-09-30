status: aktiv
Datum: 2026-09-30

# Gate-Fix R1 Prüfbericht

## Änderung und Befund

In `rust/crates/dl-moderation/src/moderation_verdict.rs` trennt `explicit_scam_reason` Aussagebereiche zusätzlich am Doppelpunkt. Damit wirken „Ohne Zweifel“, „Keine Entwarnung“ und „Nicht bloß Werbung“ nicht als Negation der folgenden eigenen Scam-Aussage. Die nachgestellte Negationsprüfung wertet nun das vollständige bestehende Fünf-Wort-Fenster aus. Außerdem erkennt die zentrale Kontextprüfung „betrugs“ als Teil einer negierten Aufzählung wie „Scam-/Betrugs-/Phishing-Muster“.

Die zwei im Auftrag genannten harmlosen Gameplay-Begründungsformen wurden als lokale negative Regressionen ergänzt:

- „Kein sichtbares Scam-/Betrugs-/Phishing-Muster.“
- „Keine sichtbaren Scam-, Phishing- oder Betrugs-Elemente.“

Beide Negationsgruppen laufen durch die gemeinsamen Analyse- und Verifikations-Konfliktprüfer. Die Regression in der ersten vollständigen Suite meldete 90 bestanden und 1 fehlgeschlagen. Sie zeigte, dass „betrugs“ in der koordinierten Negation fehlte. Nach der gezielten Korrektur bestand der Einzelfalltest und die vollständige Suite. Es wurden keine Bilddaten oder Community-Begründungen an externe Modelle gesendet.

## Prüfungen

Rust-Arbeitsverzeichnis: `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise/rust`.

- `PATH=/home/nathanael/.cargo/bin:$PATH cargo fmt -p dl-moderation -- --check`: bestanden.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo check -p dl-moderation --features testing`: bestanden.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`: bestanden.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo test -p dl-moderation --features testing negated_scam_mentions_do_not_create_conflicts -- --include-ignored`: 1 bestanden, 0 fehlgeschlagen, 0 ignoriert, 90 gefiltert.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo test -p dl-moderation --features testing -- --include-ignored`: 91 bestanden, 0 fehlgeschlagen, 0 ignoriert, 0 gefiltert; 0 Doctests.
- Der vollständige Testlauf verwendete eine temporäre TimescaleDB `timescale/timescaledb:2.17.2-pg16` mit 512 MB, 2 CPUs und zufälligem Loopback-Port. `CENTRAL_TEST_DSN`, `DEADLOCK_CENTRAL_DSN`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `SQLX_OFFLINE=true` waren ausschließlich für diese Wegwerf-DB gesetzt. Migrationen waren erfolgreich. Der Container wurde nach dem Lauf entfernt und die Entfernung überprüft.
- `git diff --check`: bestanden.

`BERICHT.md` wurde bytegleich auf den Inhalt des Auftrags-Ausgangs-HEAD `e1fe6c6c71088cc1368433ca8355e014d1e4a5be` zurückgesetzt. Das frühere Protokoll bleibt erhalten.

## Stand

Der eigene Quell-Diff umfasst `rust/crates/dl-moderation/src/moderation_verdict.rs`. `REGISTER.md` war vorbestehend geändert und `GATE-FIX-R1.md` vorbestehend ungetrackt; beide wurden nicht gestaged. Der Gate-Selbstreview ist noch offen und wird nach dem Code-Commit genau einmal ausgeführt.

TESTNACHWEIS[TW-1]: 91 passed, 0 ignored | Baseline: nicht erhoben, kein Altfehlerurteil
WIRKUNGSPRUEFUNG[WP-1]: 2 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt bau | Artefakt: .tasks/2026-09-28-moderation-negierte-scam-hinweise
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 1 belegt | Senke: .tasks/2026-09-28-moderation-negierte-scam-hinweise/GATE-FIX-R1-BERICHT.md
