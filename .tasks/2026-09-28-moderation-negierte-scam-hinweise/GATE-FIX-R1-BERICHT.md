status: aktiv
Datum: 2026-09-30

# Gate-Fix R1 Prüfbericht

## Änderung und Befund

In `rust/crates/dl-moderation/src/moderation_verdict.rs` werden Doppelpunkte nicht mehr pauschal als Aussagegrenze behandelt. `scoped_assertion_suffix` prüft den Geltungsbereich anhand der lokalen Satzstruktur und grenzt eigenständige Folgesätze ab. Die Präfixprüfung unterscheidet Negationen des Scam-Hinweises von Formulierungen wie „Ohne Zweifel“, „Keine Entwarnung“ und „Nicht bloß Werbung“. Die bisherige beliebige Fünf-Wort-Suche nach dem Scam-Begriff wurde durch direkte epistemische Negationsmuster ersetzt. Dadurch negiert „nicht eindeutig“ den Scam-Hinweis, aber „nicht erkennbar“ beim Logo hebt „Sichtbarer Scam“ nicht auf. Die zentrale Kontextprüfung erkennt weiterhin „betrugs“ als Teil negierter Aufzählungen wie „Scam-/Betrugs-/Phishing-Muster“.

Die zwei im Auftrag genannten harmlosen Gameplay-Begründungsformen wurden als lokale negative Regressionen ergänzt:

- „Kein sichtbares Scam-/Betrugs-/Phishing-Muster.“
- „Keine sichtbaren Scam-, Phishing- oder Betrugs-Elemente.“

Beide Negationsgruppen laufen durch die gemeinsamen Analyse- und Verifikations-Konfliktprüfer. Die erste vollständige Suite meldete 90 bestanden und 1 fehlgeschlagen. Sie zeigte, dass „betrugs“ in der koordinierten Negation fehlte. Nach der gezielten Korrektur bestand der Einzelfalltest. Die endgültige vollständige Suite nach allen Regressionsergänzungen bestand mit 91 Tests. Es wurden keine Bilddaten oder Community-Begründungen an externe Modelle gesendet.

## Prüfungen

Rust-Arbeitsverzeichnis: `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise/rust`.

- `PATH=/home/nathanael/.cargo/bin:$PATH cargo fmt -p dl-moderation -- --check`: bestanden.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo check -p dl-moderation --features testing`: bestanden.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`: bestanden.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo test -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored`: 10 bestanden, 0 fehlgeschlagen, 0 ignoriert, 81 gefiltert.
- `PATH=/home/nathanael/.cargo/bin:$PATH cargo test -p dl-moderation --features testing -- --include-ignored`: 91 bestanden, 0 fehlgeschlagen, 0 ignoriert, 0 gefiltert; 0 Doctests. Der Lauf erfolgte nach den abschließenden Regressionsergänzungen.
- Der vollständige Testlauf verwendete eine temporäre TimescaleDB `timescale/timescaledb:2.17.2-pg16` mit 512 MB, 2 CPUs und zufälligem Loopback-Port. `CENTRAL_TEST_DSN`, `DEADLOCK_CENTRAL_DSN`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `SQLX_OFFLINE=true` waren ausschließlich für diese Wegwerf-DB gesetzt. Migrationen waren erfolgreich. Der Container wurde nach dem Lauf entfernt.
- `git diff --check`: bestanden.

`BERICHT.md` wurde bytegleich auf den Inhalt des Auftrags-Ausgangs-HEAD `e1fe6c6c71088cc1368433ca8355e014d1e4a5be` zurückgesetzt. Das frühere Protokoll bleibt erhalten.

## Stand

Die R1-Prüfung hatte zwei blockierende Befunde:

1. Eine pauschale Doppelpunkt-Grenze hätte „Nicht belegt: sichtbarer Scam.“ fälschlich positiv eingestuft. Doppelpunkte werden nun nur innerhalb der lokal erkannten Aussagegrenzen behandelt; diese Negation unterdrückt die Scam-Aussage.
2. Ein beliebiges Wortfenster nach „Scam“ hätte die Logo-Negation auf die Scam-Aussage übertragen. Die Nachprüfung ist jetzt aussagebezogen; „Sichtbarer Scam, das Logo ist nicht erkennbar.“ bleibt positiv.

Beide ursprünglichen Fälle sowie die Präfix-Negationen und die angegebenen harmlosen Aufzählungen sind Regressionen durch beide gemeinsamen Konfliktprüfer. Formatter, Check, Clippy und vollständige Suite sind für den geprüften Stand erfolgreich.

Der eine finale Selbstreview für Commit `da1e75bf` ergab `BLOCK`. Die beiden vorherigen Befunde wurden als behoben bestätigt. Der neue blockierende Befund lautet:

`rust/crates/dl-moderation/src/moderation_verdict.rs:261 | BLOCKING | Relative-clause negation is discarded | “Ein Scam-Muster, das bisher nicht belegt ist.” treats “das bisher…” as an independent statement, truncating the suffix before its negation. Both shared checks now falsely report a contradiction; the previous scan recognized “nicht belegt”.`

Kein weiterer Review-Aufruf. Der Befund ist offen und vor einem Push oder Merge zu beheben.

Der eigene Code-Diff betrifft `rust/crates/dl-moderation/src/moderation_verdict.rs`. `REGISTER.md` war vor Beginn dieser Runde bereits geändert und `GATE-FIX-R1.md` bereits ungetrackt; beide wurden nicht geändert oder gestaged. `BERICHT.md` bleibt unverändert bytegleich zum Inhalt des Auftrags-Ausgangs-HEAD `e1fe6c6c71088cc1368433ca8355e014d1e4a5be`.

TESTNACHWEIS[TW-1]: 91 passed, 0 ignored | Baseline: nicht erhoben, kein Altfehlerurteil
WIRKUNGSPRUEFUNG[WP-1]: 2 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt bau | Artefakt: .tasks/2026-09-28-moderation-negierte-scam-hinweise
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 1 belegt | Senke: .tasks/2026-09-28-moderation-negierte-scam-hinweise/GATE-FIX-R1-BERICHT.md
