# Unabhängige Abnahme R2

Datum: 2026-09-29
Branch: `fix/moderation-negierte-scam-hinweise`
Geprüfter HEAD: `b3269338f5e067a52a19e358c54f51a1dc0049b4`
Delta: `4d6377aa65b42c9ca774611bcee058b4a6823d55...HEAD`

**Fertig: N. Inhaltliche Abnahme: BLOCK. Fix nötig: J.** Beide R1-Befunde sind behoben, aber die Korrektur des zweiten Befunds erzeugt einen belegten Fehlalarm für Warn- und Berichtskontext. Der lokale Gate-Deckel nach fünf BLOCK-Durchläufen bleibt bindend. Diese Prüfung erteilt weder Gate-ALLOW noch Merge- oder Deploy-Freigabe.

WIRKUNGSPRUEFUNG[WP-1]: 1 Befund | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
TESTNACHWEIS[TW-1]: 10 passed, 0 ignored | Baseline: 0 rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne REVIEW-R2.md

## Status der R1-Mängelliste

1. **Behoben:** `moderation_verdict.rs:124-179` behält den Kontrast beim ursprünglichen Satz, wenn danach kein eigener Scam-Hinweis folgt. Die direkt aus dem aktuellen Quelltext kompilierte Rust-Probe lieferte für `Phishing ist aber nicht sichtbar.` und `Scam-Muster ist jedoch nicht belegt.` jeweils `false`, zuvor jeweils `true`. Der Kontrollfall `Kein Phishing, aber ein sichtbarer Scam mit Auszahlungsversprechen.` bleibt `true`. Der weitere Kontrastfall `Phishing ist aber nicht sichtbar, jedoch ein sichtbarer Scam.` ergibt `true`.
2. **Behoben:** `moderation_verdict.rs:292-299` unterdrückt den vorangestellten Scam-Befund nicht mehr wegen `im Bild, Warnung an Moderatoren`. Die Probe lieferte für `Sichtbarer Scam im Bild, Warnung an Moderatoren.` jetzt `true`, zuvor `false`.

## Neue, unmittelbar durch R2 entstandene Regression

3. **Offen, hoch, Fix nötig:** `rust/crates/dl-moderation/src/moderation_verdict.rs:292-299`. Durch das Entfernen von `im` und `in` als Kontext vor `Warnung` oder `Bericht` gelten auch eindeutige Warn- und Berichtsbeschreibungen als positiver Scam-Beleg. Gegenbeispiele: `Phishing in einer Warnung vor Betrugsmaschen.` und `Phishing im Bericht über Betrugsmaschen.`. Eine Rust-Probe mit den jeweiligen Produktionsfunktionen ergab für beide bei R1 `false` und bei R2 `true`; Soll ist `false`. `Phishing als Warnung vor Betrugsmaschen.` bleibt in R2 `false`, der Unterschied liegt somit an der entfernten Kontextregel. Bei hochsicherem `other`-Urteil startet der Analyzer wegen dieses Fehlwerts erneut die Konsistenzprüfung; bleibt die harmlose Einordnung bestehen, kann `unresolved_consistency` über `content_analyzer.rs:410-443` und `moderation_system.rs:303-320` wieder eine Moderationskarte auslösen. Das ist ein direktes Gegenstück zum ursprünglich behobenen Fehlalarm, kein neues Sprachuniversum.

## Belege und Grenzen

Die beiden Proben kompilierten die unveränderten Hilfsfunktionen direkt aus `moderation_verdict.rs` des jeweiligen Commits per `rustc` in ein temporäres Programm. Die Programme wurden nach Ausführung entfernt; keine Quelldatei wurde verändert. Der reale Pizza-Wortlaut ergibt weiterhin `false`. `Sichtbarer Scam und Phishing ist unbestätigt.` ergibt weiterhin `true`. Der neue Segmentierungspfad trennt nur bei einem weiteren expliziten Scam-Hinweis; die bestehende Prüfung von Analyzer und Verifier verwendet dieselbe untere Funktion, wie in `REVIEW.md` mit Aufrufersuche belegt. Der Delta fügt keinen Fremddienstpfad hinzu.

Eigener gezielter Lauf im Worktree-Root: `PATH=/home/nathanael/.cargo/bin:$PATH cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored`. Ergebnis: 10 passed, 0 failed, 0 ignored, 81 filtered. Die 91 bestandenen Tests, fmt und clippy in `FIX-R1-BERICHT.md:18-23` stammen vom Fixer und wurden hier nicht vollständig erneut ausgeführt; die dort genannte Baseline wurde nicht neu gemessen. `git diff --check 4d6377aa...HEAD -- rust/crates/dl-moderation/src/moderation_verdict.rs` ist sauber. Der Gesamtdiff enthält außerdem fünf bereits im übernommenen `REVIEW.md:3-5,10-11` vorhandene Zeilen mit nachgestellten Leerzeichen und besteht `git diff --check` deshalb nicht. Das ist kein zusätzlicher inhaltlicher Befund an der Korrektur. Kein Gate-Aufruf, Commit, Merge, Push, Deploy oder Produktionszugriff erfolgte in R2.
