status: aktiv
Datum: 2026-09-29

# Bericht: Paket A, negierte Scam-Hinweise

ORCHESTRIERUNG[OR-1]: Stufe mittel | Schritt bau | Artefakt: .tasks/2026-09-28-moderation-negierte-scam-hinweise/
BESTAND[BS-1]: ja | Fundort: rust/crates/dl-moderation/src/moderation_verdict.rs:96 | Anknüpfung: gemeinsamer Konfliktprüfer, bestehender Kartenaufbau und Recording-Provider
WIRKUNGSPRUEFUNG[WP-1]: 9 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 3/3 geprüft
TESTNACHWEIS[TW-1]: 90 passed, 0 ignored | Baseline: 0 alt rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: Moderationskarte
MERGEPROTOKOLL[MS-1]: 0 Git-Schritte einzeln | Anläufe: 0 | Gate: BLOCK nach fünf Selbstreviews, kein main-Merge

## Stand und Belege

Code-Commit: `705d89411364e53dfbc5780e9e12b8b81ea5ed4e` auf `fix/moderation-negierte-scam-hinweise`. Diff gegen `origin/main`: sechs Dateien, 557 Einfügungen, 40 Löschungen. Der spätere Bericht-Commit liegt darüber. Kein Merge nach main und kein Deploy.

Der gemeinsame Konfliktprüfer bewertet Hinweise und Negationen im jeweiligen Satzzusammenhang. Die echte Begründung mit `kein sichtbarer Scam-/Phishing-` löst keine Konsistenzwiederholung aus. Das hohe `other`-Urteil wird wie bisher einmal verifiziert. `confirmed=false`, `other` und 92 % führen zu `Ignore`: keine Review-Karte, keine Löschung, kein Timeout, kein Bann. Die bestehende nicht öffentliche Prüfspur mit `action=ignored` bleibt erhalten. Das ist eine Abweichung vom wörtlichen Beweisziel „ohne Fall“, falls damit auch dieser interne Audit-Eintrag gemeint war. Ein bearbeitbarer Moderationsfall entsteht nicht.

Die Karten zeigen nun deutsche Kategorien und die Sicherheit jedes zugehörigen Urteils. Eine nicht bestätigte Scam-Kategorie wird als nicht bestätigt ausgewiesen; ein nach Wiederholung ungelöster Widerspruch zeigt den Grund für die manuelle Prüfung. Die positiven Widerspruchs- und Sanktionspfade wurden nicht abgeschaltet. Der gefährliche `Other`-Fall mit Gewinnversprechen erreicht weiterhin den Verifier.

## Prüfungen

Arbeitsverzeichnis für Rust-Befehle: `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise/rust`. User-Toolchain: `PATH=/home/nathanael/.cargo/bin:$PATH`.

- `rustfmt --check --edition 2021 crates/dl-moderation/src/{moderation_verdict,content_analyzer,moderation_system,case_embed}.rs`: bestanden.
- `cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`: bestanden.
- `cargo test -p dl-moderation --features testing -- --include-ignored`: 90 bestanden, 0 fehlgeschlagen, 0 ignoriert, 0 gefiltert. Die Tests liefen nach `cargo run -p dl-central-migrate` mit `SQLX_OFFLINE=true`, `TURNIER_TEST_DB_CONFIRM=throwaway-only` und `CENTRAL_TEST_DSN`/`DEADLOCK_CENTRAL_DSN` auf einem eigenen Wegwerfcontainer `timescale/timescaledb:2.17.2-pg16`. Der Container nutzte `TS_TUNE_MEMORY=512MB`, `TS_TUNE_NUM_CPUS=2`, zufälligen Loopback-Port und wurde nach dem Lauf entfernt. Keine Produktionsdatenbank wurde verwendet.
- `git diff --check`: bestanden.

Der vorhandene Wrapper `rust/scripts/central_test_db.sh` scheiterte beim ersten Versuch: Timescale stellte ohne Speicherbegrenzung 12 GB Shared Buffers ein und PostgreSQL konnte nicht starten. Ein direkter Testlauf ohne `CENTRAL_TEST_DSN` hatte 10 Fehler, darunter fehlende Datenbankumgebung und drei damals noch offene Code-/Testerwartungen. Nach Korrektur und eigenem begrenzten Testcontainer lief die vollständige Suite grün. Es wird kein vorbestehender Testfehler behauptet.

`python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review --repo /home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise --base origin/main --head HEAD --timeout 420` wurde fünfmal ausgeführt. Alle fünf Durchläufe meldeten `BLOCK`. Neun konkrete Befunde wurden jeweils am Code bearbeitet. Der letzte Durchlauf benannte `Kein Zweifel: sichtbarer Scam` und `Sichtbarer Scam und Phishing ist unbestätigt`; beide Fälle sind nach dem fünften Lauf korrigiert und als Tests belegt. Das Gate untersagt danach weitere Selbstreview-Runden; für den aktuellen Code liegt deshalb kein `ALLOW` vor. Unabhängiger Review ist zwingend, bevor ein Merge erwogen wird.

Zwillingssuche: `rg -n 'high_confidence_scam_reason_conflict|high_confidence_scam_verification_conflict|CompactCaseEmbedInput' rust/crates/dl-moderation/src/{moderation_verdict,content_analyzer,content_verifier,moderation_system,case_embed}.rs` belegte beide Analyzer-Eintrittspfade, den Verifier und den einzigen produktiven Kartenaufbau. Die drei Fremddienst-Pfade Analyzer, Verifier und Discord-Post wurden im vorhandenen Datenfluss geprüft; die Tests verwenden Recording-Provider und einen Counting-Port, keine externen LLM- oder Discord-Aufrufe. Der Live-Endzustand bleibt ungetestet, weil kein Deploy freigegeben ist.

## Fallabschluss nach unabhängigem Review

Die bestehende Schaltfläche „Verwerfen“ öffnet in `rust/bin/dl-bot/src/modglue.rs:1269-1299` das Pflichtgrund-Modal und ruft `ModerationSystem::deny_case` in `rust/crates/dl-moderation/src/moderation_system.rs:727-737` auf. Der Orchestrator kann damit den bereits offenen Vorschlag über den Dienstweg verwerfen. Die vorhandene Discord-Karte muss anschließend an ihrer bestehenden Nachrichten-ID bearbeitet werden. Hier wurde weder eine Nachricht gesendet noch ein Produktionsfall oder eine Karte geändert. Nach Review folgen lokales Merge-Gate, Merge, Deploy, Neustart und Live-Beweis durch den Orchestrator.
