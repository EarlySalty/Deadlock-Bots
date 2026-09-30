status: aktiv
Datum: 2026-09-30

# Auftrag: Moderations-KI nur noch bei Heuristik-Verdacht

## Intent des Nutzers
Die KI-Inhaltsprüfung hat nie einen echten Scam gefunden. Echte Treffer liefert nur die heuristische Verhaltenserkennung (Account-Takeover, Nachrichtenwellen, fremde Discord-Einladungen). Die KI erzeugt dagegen laufend falsche „Bitte prüfen“-Karten, zuletzt für einen harmlosen Gameplay-Screenshot. Der Nutzer will nicht, dass jede Nachricht zufällig durch die KI läuft.

Belege aus dem Journal von `deadlock-bot-rust` (7 Tage): 314 KI-Urteile in #allgemein, darunter „bro“, Emojis und GIFs; kein einziges echtes Scam-Urteil der Inhalts-KI; 8 Moderationskarten, alle über den Pfad „ungelöster KI-Konsistenzkonflikt“, alle falsch.

## Ursachen im Code (Basis origin/main de34acc8)
1. `rust/crates/dl-moderation/src/moderation_system.rs`, `handle_message`: `content_input` wird für jede Nachricht in `config.scan_channel_ids` (Standard #allgemein, `moderation_channel.rs`) gebaut und über `evaluate_with_analysis` an die KI geschickt, auch ohne jedes Verhaltenssignal.
2. `moderation_verdict.rs:76/86`, `high_confidence_scam_reason_conflict` und `high_confidence_scam_verification_conflict`: durchsuchen den Freitext der Begründung nach Wörtern wie „scam“ oder „phishing“. Eine harmlose Begründung wie „kein Phishing“ zählt als Widerspruch. Über `unresolved_consistency` (`content_analyzer.rs:403-530`) wird daraus in `handle_message` ein `PolicyDecision::Proposal`, also eine Karte.

## Umsetzung
1. Die Inhalts-KI läuft nur noch, wenn die Verhaltenserkennung ein Signal liefert (`behavior_signal.is_some()`), dann wie bisher über `evaluate_behavior_trigger` als Richter. Der Pfad „jede Nachricht in scan_channel_ids an `evaluate_with_analysis`“ entfällt. Die Regel für fremde Einladungen, die weiter `scan_channel_ids` nutzt, bleibt unverändert. Code, der dadurch tot ist, wird entfernt statt stehen gelassen. Dazu gehören ungenutzte Funktionen und Konfigurationen, aber nur, wenn sie wirklich keinen Aufrufer mehr haben.
2. Die Freitext-Wortsuche als Konsistenzprüfung fliegt komplett raus: `high_confidence_scam_reason_conflict`, `high_confidence_scam_verification_conflict`, die Konsistenz-Wiederholungen und das Feld `unresolved_consistency` samt Proposal-Zweig in `handle_message`. Entschieden wird nur noch über das strukturierte Urteil (Kategorie, Sicherheit, Verifikation) und die bestehende Policy. Keine neue Wortliste und keine Negationsregeln als Ersatz.
3. Bestehende Tests, die das entfernte Verhalten prüfen, entfernen oder auf das neue Verhalten umstellen. Nachzuweisen ist: Nachricht ohne Verhaltenssignal, auch mit Bild, führt zu keinem KI-Aufruf und keinem Fall. Ein Verhaltenssignal (etwa Takeover) führt weiterhin zu KI-Richter, Fall und Sanktionspfad wie bisher. Fremde Einladungen werden weiterhin gelöscht.
4. Kein Modell- oder Providerwechsel, keine geänderten Sanktionsschwellen, keine Migration, keine Code-Kommentare, kein repo-weites fmt. Keine Community-Daten an externe Modelle, keine Test-Sanktionen an echten Mitgliedern.

## Nicht übernehmen
Der Branch `fix/moderation-negierte-scam-hinweise` (Negationsparser) wird durch diesen Auftrag ersetzt. Nichts daraus übernehmen.

## Arbeitsort und Git
Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder Unter-Agenten spawnen.
Worktree `/home/nathanael/.worktrees/dl-moderation-ki-nur-bei-verdacht`, Branch `fix/moderation-ki-nur-bei-verdacht`, Basis `de34acc80dc3c1723397021593576ca94b20558d`. Nur eigene Dateien stagen, Git-Schritte einzeln, nur den Feature-Branch pushen. Kein Merge, kein Deploy.

Prüfungen im Verzeichnis `rust` mit `PATH=/home/nathanael/.cargo/bin:$PATH`:
- `cargo fmt -p dl-moderation -- --check`
- `cargo clippy -p dl-moderation --features testing --all-targets -- -D warnings`
- `cargo test -p dl-moderation --features testing -- --include-ignored` (DB-Tests wie in `.tasks/2026-09-28-moderation-negierte-scam-hinweise/GATE-FIX-R1-BERICHT.md` auf dem Branch `fix/moderation-negierte-scam-hinweise` beschrieben: Wegwerf-TimescaleDB, danach entfernen)
- `cargo check -p dl-bot`, falls sich öffentliche Schnittstellen ändern

Danach genau ein Selbstreview:
`python3 /home/nathanael/Documents/.claude/gpt-workers/gate_hook.py --review --repo /home/nathanael/.worktrees/dl-moderation-ki-nur-bei-verdacht --base origin/main --head HEAD --timeout 900`
Bei BLOCK den Befund prüfen und beheben, den Gate-Status nie selbst verändern.

## Übergabe
Bericht in `.tasks/2026-09-30-moderation-ki-nur-bei-verdacht/BERICHT.md`: SHA, Diff-Stat, Prüfkommandos mit Ergebnis, Gate-Urteil. Fertigmeldung im eigenen Thread. Bei echtem Blocker `[Bump-up] Paket A: Grund: ... Erledigt: ... Worktree: ... Offen: ...` an den Orchestrator und stoppen.
