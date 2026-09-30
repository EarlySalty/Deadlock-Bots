status: aktiv
Datum: 2026-09-30

# Gate-Fix Runde 1: Aussagebezogene Scam-Negation

## Kontext und Fundstellen

Bestehender Auftrag: `.tasks/2026-09-28-moderation-negierte-scam-hinweise/AUFTRAG.md`. Der geprüfte Branch wurde noch nie nach main gemergt oder live geschaltet. Der Nutzer hat den Gate-Zähler selbst zurückgesetzt. Der reguläre neue Vollreview meldet zwei blockierende Befunde im gemeinsamen Parser `rust/crates/dl-moderation/src/moderation_verdict.rs`:

1. `:380`: `after.iter().take(3)` schneidet die Verneinung hinter einer positiven Erwähnung zu früh ab. „Sichtbarer Scam ist nach Prüfung nicht belegt“ und „Sichtbarer Scam ist nach Prüfung nicht eindeutig“ werden fälschlich als bestätigte Betrugsaussage behandelt.
2. `:273`: Jedes frühere `ohne`, `kein/keine` oder `nicht` unterdrückt auch eine später eigenständig bejahte Betrugsaussage. „Ohne Zweifel: sichtbarer Scam“, „Keine Entwarnung: sichtbarer Scam“ und „Nicht bloß Werbung: sichtbarer Scam“ verlieren die Schutzprüfung.

Die gemeinsamen Eintrittspfade heißen `high_confidence_scam_reason_conflict` (`moderation_verdict.rs:76`) und `high_confidence_scam_verification_conflict` (`:86`). `explicit_scam_reason` (`:124`) teilt Sätze und Gegensatzklauseln; `scam_mention_is_asserted` (`:257`) prüft den lokalen Kontext. Bestehende positive/negative Beispiele und Tests ab `:516`, einschließlich Warnungs- und Mischkontext, müssen weiter bestehen. Die am 29.09. im Live-System erneut beobachtete harmlose Gameplay-Abbildung führte über einen ungelösten Konsistenzwiderspruch zu einem manuellen Vorschlag. Deren Texte und Anhänge weder in externe Modelle noch in weitere Prompt-Beispiele kopieren.

## Scope

Exakt diese beiden Gate-Befunde im bestehenden Rust-Pfad reparieren, kein Neubau, keine Konfiguration, kein Modellwechsel, kein großflächiges Refactoring und kein repo-weites Formatieren. Die Ursache ist die Bezugsweite einer Negation und die Begrenzung des nachgestellten Wortfensters: eindeutige Negation des gerade genannten Sachverhalts muss gelten, Negationen zu einem anderen Sachverhalt dürfen eine eigenständige positive Aussage nicht aushebeln. Echte gemischte Begründungen und sichtbare Betrugsindikatoren müssen weiterhin zur Widerspruchsprüfung führen. Kein neues Python, keine Code-Kommentare.

## Arbeitsort und Git

Einziger Worker für dieses Paket, keine Unter-Threads oder Unter-Agenten. Worktree `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise`, Branch `fix/moderation-negierte-scam-hinweise`, Ausgangs-HEAD `e1fe6c6c71088cc1368433ca8355e014d1e4a5be`, vor dem Auftrag sauber. Andere Worktrees und unbeteiligte Dateien nicht anfassen. Eigene Änderungen im Feature-Branch committen und nur diesen Branch pushen. Kein Merge, kein Deploy, keine Bot-Nachricht. Kein Git-Bypass; Git-Schritte einzeln, nur eigene Dateien stagen. Commit-Attribution nach lokalen Regeln.

## Beweisziel

Compiler, Formatter, Clippy und bestehende betroffene Tests prüfen; neue gezielte Testfälle für beide Richtungen sind sinnvoll. Beide Beispielgruppen des Gate-Befunds korrekt klassifizieren, bestehende positiven, negativen und gemischten Beispiele weiter grün. Das harmlose Urteil ohne wirksames Betrugssignal darf keine Moderationskarte erzeugen. Selbstprüfung vor Übergabe über `gate_hook.py --review` einmal erst nach lokalem Test und Prüfung der eigenen Änderung; nicht fünfmal auf Verdacht und keinen Gate-Status selbst verändern. Fehler unverändert melden. Bericht mit Diff-Stat, Testanzahl und Ursache in dieser Akte.

## Verifizierte Auflösung des Werkzeug-Blockers am 30.09.2026

Kein neuer Worker und kein Toolchain-Upgrade nötig. Der Orchestrator hat diese Programme direkt geprüft:

- `/home/nathanael/.cargo/bin/cargo --version`: cargo 1.97.1.
- `/home/nathanael/.cargo/bin/rustc --version`: rustc 1.97.1.
- `python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review --help`: Review-Modus vorhanden; Pflichtargumente sind `--repo` und `--base`. Der bisherige Exit 2 war ein unvollständiger Aufruf, kein fehlendes Feature.

Für Rust-Befehle `/home/nathanael/.cargo/bin` vor den System-PATH setzen, damit Cargo UND rustc aus derselben Toolchain kommen. Im vorhandenen `rust`-Verzeichnis:

```bash
PATH=/home/nathanael/.cargo/bin:$PATH cargo check -p dl-moderation --features testing
PATH=/home/nathanael/.cargo/bin:$PATH cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings
PATH=/home/nathanael/.cargo/bin:$PATH cargo test -p dl-moderation --features testing -- --include-ignored
```

Die bestehende Suite benötigt für DB-Tests die Wegwerf-Testdatenbank nach FIX-R3-BERICHT.md. Derselbe begrenzte Testdatenbank-Weg ist bereits mit 91 Tests gelaufen. Fehlende DB-Konfiguration nicht als Rust-Codefehler behandeln, keine Produktion dafür benutzen. Bestehende Konfiguration über die vorgesehenen Verwaltungswerkzeuge verwenden, keine Secrets ausgeben.

Nach erfolgreichen lokalen Prüfungen und einem neuen eigenen Code-Commit genau ein regulärer Selbstreview-Aufruf mit vollständig angegebenem Vergleich:

```bash
python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review --repo /home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise --base origin/main --head HEAD --timeout 900
```

Keine Gate-Dateien, Zähler oder Vergleichsgrundlagen zum Umgehen eines BLOCK ändern. Falls das Gate erneut blockiert, den konkreten Befund übergeben, nicht eigenständig weitere Runden drehen. Bereits vorhandene Prüfhistorie in BERICHT.md erhalten: die neue Gate-Fix-Dokumentation in GATE-FIX-R1-BERICHT.md ablegen, den ursprünglichen Bericht anhand des Ausgangs-HEAD wiederherstellen, ohne Code oder andere eigene Änderungen zu verwerfen. Als Ergebnis Compiler-/Teststatus und tatsächliches Gate-Urteil getrennt melden; nur eigenen Branch pushen, kein Merge/Deploy.

## Übergabe

Intent-Thread-ID `68739830-327f-4691-8b9f-9d70971465fd`. Melde dich mit `[Bump-up] Paket A: Grund: ... Erledigt: ... Worktree: /home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise Offen: ...` an den Intent-Thread und stoppe danach, falls ein echter Blocker entsteht. Ansonsten Fertigmeldung mit Branch und SHA nur in deinem eigenen Worker-Thread. Der Orchestrator führt unabhängige Prüfung, lokales Gate, Merge, Deploy und Live-Beweis durch.
