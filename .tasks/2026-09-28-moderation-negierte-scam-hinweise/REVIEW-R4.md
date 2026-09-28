# Unabhängige Abnahme R4

Datum: 2026-09-29
Branch: `fix/moderation-negierte-scam-hinweise`
Geprüfter HEAD: `2c7f52ac6b903b63021d5c2714dc759f7532399a`
Delta: `773b95ededa6550c6e5c5bbc505789020dfd1c40...HEAD`

**Fertig: J, bezogen auf den R3-Befund und dessen unmittelbare Regressionen. Inhaltliche Abnahme: ALLOW. Fix nötig: N.** Der lokale Gate-Deckel nach fünf BLOCK-Durchläufen bleibt davon getrennt bindend. Diese Abnahme ist kein Gate-ALLOW und erlaubt weder Merge noch Deploy.

WIRKUNGSPRUEFUNG[WP-1]: 0 Befunde | Zwillingssuche: grep-belegt | Fremddienst-Pfade: 0/0 geprüft
TESTNACHWEIS[TW-1]: 10 passed, 0 ignored | Baseline: 0 rot
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: interne REVIEW-R4.md

## Mängelliste

1. **R3-Befund behoben:** `rust/crates/dl-moderation/src/moderation_verdict.rs:200-218` begrenzt den direkten Warn- und Berichtsbezug jetzt am letzten Satzzeichen oder Gegensatzwort. Die aus dem jeweiligen Quelltext kompilierten Funktionen ergaben für `Sichtbarer Scam in der Anzeige: Warnung an Moderatoren.` und `Sichtbarer Scam in der Gruppe: Warnung an Moderatoren.` am R3-Stand `false`, am R4-Stand jeweils `true`. Soll: `true`. Als unmittelbare Satzteil-Kontrollen ergaben auch die Komma- und `aber`-Varianten mit positiver Anzeige in R4 `true`.
2. **Keine unmittelbare Regression in den geprüften Kontrollen:** `Sichtbarer Scam im Bild, Warnung an Moderatoren.` bleibt `true`. `Phishing in einer Warnung vor Betrugsmaschen.`, `Phishing im Bericht über Betrugsmaschen.` und `Phishing in der offiziellen Warnung vor Betrugsmaschen.` bleiben `false`. `Phishing ist aber nicht sichtbar.` und `Scam-Muster ist jedoch nicht belegt.` bleiben `false`; `Kein Phishing, aber ein sichtbarer Scam mit Auszahlungsversprechen.` bleibt `true`. Die echte Pizza-Begründung bleibt `false`. Zusätzlich blieb `Phishing: in einer Warnung vor Betrugsmaschen.` `false` und `Sichtbarer Scam in der Anzeige, Warnung an Moderatoren.` `true`.

## Eigene Belege und Grenze

Die Gegenproben verwendeten die unveränderten Hilfsfunktionen aus R3 und R4, jeweils direkt per `rustc` temporär kompiliert und danach entfernt. Gezielter Lauf im Worktree-Root: `PATH=/home/nathanael/.cargo/bin:$PATH cargo test --manifest-path rust/Cargo.toml -p dl-moderation --features testing moderation_verdict::tests:: -- --include-ignored`: 10 passed, 0 failed, 0 ignored, 81 filtered. `git diff --check 773b95ed...HEAD` bestand. Die 91 Tests, fmt und clippy in `FIX-R3-BERICHT.md:20-24` sind Angaben des Fixers und wurden nicht vollständig erneut ausgeführt; dessen Baseline wurde hier nicht eigenständig gemessen. Der Delta verändert den gemeinsamen Prüfer und seine Tests, nicht die Aufrufer oder Fremddienstpfade. Ein Live-Beweis und ein Gate-Lauf gehörten nicht zu dieser Abnahme. Keine Codeänderung, kein Commit, Merge, Deploy oder Produktionszugriff durch den Reviewer.
