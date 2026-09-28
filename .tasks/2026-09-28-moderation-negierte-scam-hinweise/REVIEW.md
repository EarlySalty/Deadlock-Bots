# Unabhängige Abnahme R1

Datum: 2026-09-29  
Branch: `fix/moderation-negierte-scam-hinweise`  
Geprüfter HEAD: `4d6377aa65b42c9ca774611bcee058b4a6823d55`  
Basis: `42175e5fd68a1c83bf4201b4c40cca1099f00652`

**Fertig: N. Inhaltliche Abnahme: BLOCK. Fix nötig: J.** Zwei Gegenbeispiele betreffen den gemeinsamen Konfliktprüfer. Der bestehende lokale Merge-Gate-Weg steht nach fünf Selbstreviews weiterhin auf BLOCK; diese unabhängige Prüfung erteilt weder ein Gate-ALLOW noch eine Merge- oder Deploy-Freigabe. Der Rundenzähler bleibt unangetastet.

WIRKUNGSPRUEFUNG[WP-1]: 2 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 3/3 geprüft  
TESTNACHWEIS[TW-1]: 13 passed, 0 ignored | Baseline: 0 rot  
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne REVIEW.md

## Befunde

1. **Hoch, erneuter Fehlalarm bei verneintem Befund.** `rust/crates/dl-moderation/src/moderation_verdict.rs:118-124`: Die Trennung bei ` aber ` und ` jedoch ` erfolgt vor der Auswertung der nachgestellten Verneinung. Gegenbeispiele: `Phishing ist aber nicht sichtbar.` und `Scam-Muster ist jedoch nicht belegt.` Die unveränderten Produktionsfunktionen `explicit_scam_reason` und `scam_mention_is_asserted`, direkt aus dem geprüften Quelltext in eine temporäre Rust-Probe übernommen, lieferten für beide Aussagen `true`. Soll: `false`. Bei `other` mit hoher Urteilssicherheit startet damit die Konsistenzwiederholung. Bleibt die harmlose Aussage auch nach der Wiederholung bestehen, führt `unresolved_consistency` über `content_analyzer.rs:410-443` und `moderation_system.rs:303-320` wieder zu einem Moderationsvorschlag statt zum Ignorieren. Die beiden Satzformen sind gewöhnliche Verneinungen, keine zusätzlichen Spezialfälle des Pizza-Bilds.

2. **Hoch, positiver Befund durch nachgestellten Warnhinweis verdeckt.** `rust/crates/dl-moderation/src/moderation_verdict.rs:244-251`: Der nachgestellte Warnkontext greift auch, wenn die Warnung an die Moderation gerichtet ist und der Bildinhalt zuvor ausdrücklich als Betrug bezeichnet wurde. Gegenbeispiel: `Sichtbarer Scam im Bild, Warnung an Moderatoren.` Die identische Rust-Probe lieferte `false`; Soll: `true` für den sichtbaren Scam-Befund. Das Wort `im` vor `Warnung` erfüllt die zu breite Bedingung. Bei einem hochsicheren `other`-Urteil findet so weder die Analyzer-Konsistenzwiederholung noch, bei erneut verneinendem Verifier, eine Widerspruchseskalation statt. Der eigenständige Verifier bleibt erreichbar, ersetzt aber den ausfallenden Widerspruchsschutz nicht. Der bestehende Test für `Sichtbarer Scam und Warnung an Moderatoren.` deckt die Kommaform nicht ab.

## Geprüfte Wirkung und Grenzen

- Die direkte Rust-Probe verwendete den Quellabschnitt `moderation_verdict.rs:96-278` ohne Änderung. Kontrollwerte: Die echte Pizza-Begründung ergibt `false`; `Kein Phishing, aber ein sichtbarer Scam mit Auszahlungsversprechen.` ergibt `true`. Die obigen drei Gegenbeispiele ergaben `true`, `true`, `false`. Die Probe wurde außerhalb des Arbeitsbaums kompiliert und wieder entfernt; kein Anwendungscode wurde geändert.
- Beide Analyzer-Eintritte und ihre Wiederholungsprüfungen (`content_analyzer.rs:403-531`) sowie die Verifier-Prüfung und Wiederholung (`content_verifier.rs:114-167`) rufen denselben unteren Konfliktprüfer auf. Die Zwillingssuche mit `rg -n 'high_confidence_scam_reason_conflict|high_confidence_scam_verification_conflict|CompactCaseEmbedInput|build_compact_case_embed' rust/crates/dl-moderation/src/{moderation_verdict,content_analyzer,content_verifier,moderation_system,case_embed}.rs` fand keine zweite produktive Implementierung des Konfliktprüfers oder Kartenaufbaus.
- Analyzer und Verifier verwenden die vorhandenen Provider-Schnittstellen; der Discord-Post läuft über `post_moderation_case` in `moderation_system.rs:582-593`. Erfolg des Posts wird über die Nachrichten-ID sichtbar; ein fehlgeschlagener Post setzt diese nicht. Kein externer Dienst wurde für die Abnahme aufgerufen. Die geprüfte Änderung führt keinen zusätzlichen Provider-Aufrufpfad ein.
- Der reale Pizza-Wortlaut und die Verifier-Antwort `other`, `confirmed=false`, 92 % führen im Test zu `ignored`, ohne Moderationskarte, Löschung, Timeout oder Bann (`moderation_system.rs:1840-1869`). Der interne Audit-Eintrag mit `action=ignored` ist auftragsgemäß zulässig. Die Karte benennt Urteile und Urteilssicherheit statt Scam-Wahrscheinlichkeit und kennzeichnet `confirmed=false` sowie ungelöste Widersprüche (`case_embed.rs:37-68,192-239`). Ein Live-Beweis ist ohne Deploy nicht erfolgt.

## Eigene Prüfungen

Arbeitsverzeichnis: Worktree-Root. Die vier gezielten Läufe verwendeten `PATH=/home/nathanael/.cargo/bin:$PATH` und `--features testing` sowie `--include-ignored`:

- `cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored`: 9 passed, 0 failed, 0 ignored, 81 filtered.
- `cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing benign_voting -- --include-ignored`: 2 passed, 0 failed, 0 ignored, 88 filtered.
- `cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing suspicious_other_analysis_still_reaches_verifier -- --include-ignored`: 1 passed, 0 failed, 0 ignored, 89 filtered.
- `cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing unconfirmed_verdict_and_unresolved_conflict_are_explicit -- --include-ignored`: 1 passed, 0 failed, 0 ignored, 89 filtered.

`git diff --check 42175e5fd68a1c83bf4201b4c40cca1099f00652...HEAD` war unauffällig. Die 90 bestandenen Tests sowie fmt und clippy sind Belege aus `BERICHT.md:21-32`, nicht von dieser Abnahme erneut vollständig ausgeführt. Die dort genannte Baseline mit null alten Fehlern wurde hier nicht eigenständig gemessen; es wird kein Altfehler geltend gemacht. Weder Gate noch Build, Merge, Push, Deploy oder Produktionsfall wurden in diesem Review geändert.
