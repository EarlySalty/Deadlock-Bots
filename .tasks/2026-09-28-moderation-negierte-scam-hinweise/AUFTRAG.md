# Auftrag: Falschen Scam-Alarm beheben

## Intent und Freigabe
Der Nutzer beauftragt ausdrücklich Umsetzung und Delegation, keinen weiteren Plan und keine Freigaberückfrage. Das harmlose Pizza-Abstimmungsbild darf weder eine Moderationswarnung noch eine Sanktion erzeugen. Urteilssicherheit darf in Karten nicht als Scam-Wahrscheinlichkeit erscheinen.
Intent-Thread: `68739830-327f-4691-8b9f-9d70971465fd`.
Stufe mittel, Kontingent-Fallback auf Sol aus worker_gross, da beide worker_mittel-Modelle gesperrt sind. Vorcheck ist bereits vollständig durch den Orchestrator erfolgt. Nicht erneut inventarisieren.

## Stand und Arbeitsort
Repo: `/home/nathanael/repos/Deadlock-Bots`.
Eigener Worktree: `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise`.
Branch: `fix/moderation-negierte-scam-hinweise`.
Basis: `42175e5fd68a1c83bf4201b4c40cca1099f00652` von origin/main, frisch angelegt, anfänglich sauber.
Im Kanon gibt es fremde ungetrackte Daten unter `docs/insights/discord/2026-09-28/`. Nicht anfassen.
Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder Unter-Agenten spawnen. Keine Code-Kommentare.

## Belegte Ursache und Referenzauszüge
`rust/crates/dl-moderation/src/moderation_verdict.rs:96-145`:
`explicit_scam_reason` sucht unter anderem `"phishing"` und nimmt anschließend nur starre negative Teilstrings wie `"kein scam"`, `"kein betrug"`, `"kein phishing"` aus. Das schlägt bei eingeschobenen Wörtern und verbundenen Begriffen fehl.
`high_confidence_scam_reason_conflict` und `high_confidence_scam_verification_conflict` rufen diesen gemeinsamen Helfer auf. Analyzer und Verifier haben bereits Konsistenz-Wiederholungen.
`rust/crates/dl-moderation/src/moderation_system.rs:296-306`:
```
if content_evaluation
    .as_ref()
    .is_some_and(|evaluation| evaluation.unresolved_consistency)
{
    let timeout_minutes = if behavior_signal.is_some() {
        self.policy.config().behavior_proposal_timeout_minutes
    } else {
        self.policy.config().timeout_minutes
    };
    outcome.source = Some(PolicyDecisionSource::Content);
    outcome.decision = PolicyDecision::Proposal { timeout_minutes };
}
```
`case_embed.rs:40-46` zeigt aktuell beide Zahlen ohne zugehöriges Urteil:
```
"name": "Sicherheit",
"value": format!(
    "Analyse {:.0}% · Verifikation {:.0}%",
    input.verdict.analysis.confidence * 100.0,
    input.verdict.verification.confidence * 100.0
),
```

Journal vom 28.09.2026 um 22:13:30 Ortszeit beweist exakt den gemeldeten Fall:
- Originalnachricht `1554224525426565242`, Kanal `1289721245281292291`, Guild `1289721245281292288`.
- Analyzer-Konsistenzprüfung other/0.86, Verifier-Konsistenzprüfung other/0.92, Wiederholung other/0.86, danach ungelöster Konflikt und proposal.
- Finale Begründung wörtlich: `Nur ein harmloses Voting-/Event-Poster mit Pizzen („Who’s next?“, „You have 0 votes“, „Vote for your favorite pizza“); kein sichtbarer Scam-/Phishing-, Gewinn- oder Auszahlungsversprechen.`
- Darin matcht phishing, keiner der vorhandenen negativen Teilstrings matcht. Die 86 % stützen das harmlose Urteil, nicht Scam.
- Es wurde ein Vorschlag erstellt, kein automatischer Timeout.

## Umsetzung und Scope
Exakt dieser Auftrag, kein Refactoring und kein repo-weites fmt. Rust, vorhandene zentrale Funktionen wiederverwenden.
1. Den gemeinsamen Konsistenz-Helfer so korrigieren, dass negierte Scam-/Phishing-Aussagen, eingeschobene Wörter und verbundene Begriffe keinen positiven Scam-Beleg erzeugen. Negation aussagebezogen auswerten, kein globales Bypass-Wort wie harmlos und keine Pizza-Sonderregel. Gemischte Begründungen mit anschließend wirklich positivem Scam-Befund müssen weiter erkannt werden. Bestehende Warn-/Zitat-Kontexte beachten.
2. In `case_embed.rs` die Zahlen den Urteilen zuordnen, verständliche deutsche Kategorien statt `other`, klar sagen, dass es Urteilssicherheit ist. Verifikation confirmed=false darf niemals wie bestätigter Scam erscheinen. Bei tatsächlich ungelösten Widersprüchen den manuellen Prüfgrund sichtbar machen, sofern nötig mit kleinem Zusatz im vorhandenen Datenfluss.
3. Vorhandene Tests und Recording-Provider in `moderation_verdict.rs`, `content_analyzer.rs`, `moderation_system.rs` und `case_embed.rs` nutzen. Beweisziel: echte Live-Begründung führt trotz hoher Sicherheit zu Ignore, ohne zusätzliche Konsistenzschleife, Fall, Karte oder Sanktion; echte Scam-Widersprüche und bestätigte gefährliche Inhalte behalten ihren Schutzpfad. Negation, zusammengesetzte Begriffe und gemischte Sätze gegenprüfen. Passendes fmt/check/clippy und betroffene bestehende Suites ausführen, Baselinefehler offen unterscheiden.
4. Keinen Provider- oder Modellwechsel, keine geänderten Sanktionsschwellen, keine externen LLM-Aufrufe mit Community-Daten und keine Prod-Sanktionen als Test. Keine DB-Migration vorgesehen. Gleichzeitig gesehene Text-Providerfehler sind außerhalb des Auftrags.
5. Vorhandenen Abschlussweg für diesen falschen Moderationsfall lokalisieren und im Bericht nennen, nicht eigenständig neue Produktionsnachrichten senden oder den Fall per DB-Schreibzugriff manipulieren. Der Orchestrator schließt ihn nach Prüfung über den vorhandenen Dienstweg und korrigiert die Karte in place.

## Git und Freigabepunkt
Nur eigenen Feature-Branch committen und pushen. Keine fremden Dateien, kein Merge nach main, kein Deploy in der Bauphase. Schutz-Hooks nicht umgehen. Selbstprüfung vor Übergabe über vorhandenen `gate_hook.py --review` nach Repo-Regeln. Bericht und Belege im eigenen Worktree unter `.tasks/2026-09-28-moderation-negierte-scam-hinweise/BERICHT.md`; Auftrag und Register aus dem Kanon in denselben relativen Ordner übernehmen, damit die Akte auf Git landet. Nur eigene Dateien stagen. Commit-Attribution: `Co-Authored-By: Claude Code <noreply@anthropic.com>`.
Nach Fertigmeldung folgt unabhängiger Review, lokales Merge-Gate, Merge/Push, Deploy, Neustart, Live-Beweis und Cleanup durch den Orchestrator. GitHub Actions sind kein Gate.

## Übergabe
Bericht mit SHA, Diff-Stat, exakten Prüfkommandos und Ergebnissen, Beweis gegen Nutzer-Intent, verbleibenden Abweichungen und gefundenem Deploy-/Fallabschlussweg. Kein Fertig-Urteil für bloß geschriebenen Code.
Melde dich mit `[Bump-up] Paket A: Grund: ... Erledigt: ... Worktree: ... Offen: ...` an den Intent-Thread `68739830-327f-4691-8b9f-9d70971465fd` und stoppe danach, falls echte Blocker bleiben. Rückmeldung im Worker-Thread und Bericht genügt; kein Kontakt zu fremden Sessions nötig.
