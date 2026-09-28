# Unabhängige Abnahme R3

Datum: 2026-09-29
Branch: `fix/moderation-negierte-scam-hinweise`
Geprüfter HEAD: `773b95ededa6550c6e5c5bbc505789020dfd1c40`
Delta: `b3269338f5e067a52a19e358c54f51a1dc0049b4...HEAD`

**Fertig: N. Inhaltliche Abnahme: BLOCK. Fix nötig: J.** Der offene R2-Befund ist für seine beiden Gegenbeispiele behoben. Die neue Zuordnung unterdrückt jedoch einen ausdrücklich positiven Scam-Befund, wenn eine getrennte Warnung nach einem Doppelpunkt folgt. Der Gate-Deckel nach fünf BLOCK-Durchläufen bleibt unabhängig vom inhaltlichen Urteil bindend; kein Gate-ALLOW, Merge oder Deploy.

WIRKUNGSPRUEFUNG[WP-1]: 1 Befund | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
TESTNACHWEIS[TW-1]: 10 passed, 0 ignored | Baseline: 0 rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne REVIEW-R3.md

## R2-Befund und Kontrollen

- **R2-Befund behoben:** `Phishing in einer Warnung vor Betrugsmaschen.` und `Phishing im Bericht über Betrugsmaschen.` ergeben mit dem aktuellen Prüfer jeweils `false`. Auch `Phishing in der offiziellen Warnung vor Betrugsmaschen.` ergibt `false`. Die Änderung liegt in `rust/crates/dl-moderation/src/moderation_verdict.rs:181-243,357-365`.
- **Positive Kontrolle erhalten:** `Sichtbarer Scam im Bild, Warnung an Moderatoren.` ergibt `true`. Die R1-Gegenbeispiele `Phishing ist aber nicht sichtbar.` und `Scam-Muster ist jedoch nicht belegt.` ergeben `false`; `Kein Phishing, aber ein sichtbarer Scam mit Auszahlungsversprechen.` ergibt `true`. Der reale Pizza-Wortlaut ergibt `false`.

## Direkt entstandene Regression

1. **Offen, hoch:** `rust/crates/dl-moderation/src/moderation_verdict.rs:205-243,357-365`. Gegenbeispiel: `Sichtbarer Scam in der Anzeige: Warnung an Moderatoren.`. Der Prüfer liefert am R2-Stand `true`, am R3-Stand `false`; Soll ist `true`, denn der Scam wird für die Anzeige bejaht und die Warnung richtet sich an die Moderation. `direct_warn_report_context` trennt nicht am Doppelpunkt und behandelt `Anzeige` wegen der Endung `e` als zulässigen Modifikator einer unmittelbar folgenden Warnung. Dasselbe passiert bei `Sichtbarer Scam in der Gruppe: Warnung an Moderatoren.` (`true` in R2, `false` in R3). Dagegen bleibt `Sichtbarer Scam im Bild: Warnung an Moderatoren.` `true`, weil `Bild` nicht auf eine der erlaubten Endungen passt. Bei hochsicherem `other`-Urteil fällt damit der Widerspruchsschutz für diesen ausdrücklich positiven Befund weg; wenn auch der Verifier ihn verneint, erfolgt keine Konsistenzeskalation. Dies ist eine Regression im neuen Warnkontext-Helfer, kein Befund aus einem neuen Voll-Audit.

## Eigene Belege

Die Gegenproben kompilierten die unveränderten Hilfsfunktionen aus dem R2-Commit und dem aktuellen R3-Quelltext jeweils per `rustc` in ein temporäres Programm; die Programme wurden danach entfernt. Gezielter Testlauf im Worktree-Root: `PATH=/home/nathanael/.cargo/bin:$PATH cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored`: 10 passed, 0 failed, 0 ignored, 81 filtered. `git diff --check b3269338...HEAD -- rust/crates/dl-moderation/src/moderation_verdict.rs` ist sauber. Die 91 Tests, fmt und clippy in `FIX-R2-BERICHT.md:20-24` wurden vom Fixer berichtet, hier nicht vollständig wiederholt. Dessen Baseline wurde nicht eigenständig neu gemessen. Der Delta ändert keine Analyzer-, Verifier- oder Fremddienst-Aufrufer. Keine Codeänderung, kein Commit, Gate-Aufruf, Merge, Deploy oder Produktionszugriff erfolgte in R3.
