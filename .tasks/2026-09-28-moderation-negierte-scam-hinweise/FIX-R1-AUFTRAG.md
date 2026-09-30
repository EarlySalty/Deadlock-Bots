# Fix R1: zwei bestätigte Befunde

Direkt umsetzen, kein Plan zur Freigabe. Intent `68739830-327f-4691-8b9f-9d70971465fd`.
Worktree `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise`, Branch `fix/moderation-negierte-scam-hinweise`, HEAD `4d6377aa65b42c9ca774611bcee058b4a6823d55`, auf origin gesichert. Einzige ungetrackte Datei ist der unabhängige REVIEW.md aus dieser Aufgabe. Sie erhalten und mit sichern. Autor und Reviewer sind ready; du bist der einzige aktive Implementierer. Keine Unter-Threads oder Unter-Agenten, keine Code-Kommentare.

Auftrag und bisherigen Bericht unter `.tasks/2026-09-28-moderation-negierte-scam-hinweise/` lesen. REVIEW.md ist die verbindliche Mängelliste; exakt diese Befunde korrigieren, kein Refactoring, kein repo-weites fmt, keine neuen Modelle/Provider oder Grenzwerte.

## Befund 1
`rust/crates/dl-moderation/src/moderation_verdict.rs:118-124`: Die Trennung bei ` aber ` und ` jedoch ` trennt Subjekt und nachgestellte Verneinung.
- `Phishing ist aber nicht sichtbar.` liefert true, muss false liefern.
- `Scam-Muster ist jedoch nicht belegt.` liefert true, muss false liefern.
Gegenrichtung erhalten: `Kein Phishing, aber ein sichtbarer Scam mit Auszahlungsversprechen.` bleibt true.

## Befund 2
`moderation_verdict.rs:244-251`: Nachgestellter Warnkontext reicht zu weit.
- `Sichtbarer Scam im Bild, Warnung an Moderatoren.` liefert false, muss true liefern.
Das vor der Warnung stehende `im` reicht derzeit aus. Eine Warnung an Moderatoren ist kein entlastender Kontext des Bildinhalts. Echte zitierte Warnungen und der vorhandene Fall mit `und Warnung an Moderatoren` müssen ihre korrekten Ergebnisse behalten.

## Arbeitsweise und Beweis
Nicht zwei weitere harte Ausnahmesätze einbauen. Die bereits mehrfach korrigierten Scope-Grenzen im gemeinsamen Helfer systematisch prüfen: ALLE Segmentierungs- und Vor-/Nachkontextregeln jeweils gegen konkrete Verneinung, echten positiven Befund, bloße Warnung/Zitat und Hinweis an Moderatoren. Minimalen generischen Fix an der Ursache bauen. Review-Gegenbeispiele und echte Pizza-Begründung als Gegenproben nutzen. Die vorhandenen Analyzer-/Verifier-Aufrufer weiter gemeinsam versorgen.

Gezielte Tests plus bestehende 90er-Suite nach BERICHT.md, passendes fmt/clippy und diff-check. Keine Community-Daten an externe Anbieter und keine Live-Sanktionen. Der interne `action=ignored`-Audit-Eintrag ist erlaubt, keine Warnkarte/Sanktion bei harmlos ist entscheidend.

## Abschluss und Gate
Nur eigenen Feature-Branch committen/pushen. REVIEW.md und FIX-R1-BERICHT.md mit eigenen Änderungen sichern; Bericht enthält exakte Belege und SHA. Kein main-Merge, kein Deploy. Die fünf bisherigen lokalen Selbstreviews waren BLOCK, danach wurde jeweils repariert; KEIN ALLOW liegt vor. Gate-Zähler oder Gate-Dateien niemals verändern oder zurücksetzen, keine künstliche neue Branch-Identität zum Umgehen. Falls `gate_hook.py --review` wegen Rundengrenze nicht mehr zulässig ist, den unveränderten Blocker im Bericht festhalten statt neu zu würfeln. Eigene Compiler-/Testprüfung trotzdem fertig machen. Danach unabhängige Nachprüfung im bestehenden Reviewer-Thread gegen die beiden Befunde; die übernimmt der Orchestrator.

Bump-up, nur wenn nötig: `[Bump-up] Paket A Fix R1: Grund: ... Erledigt: ... Worktree: ... Offen: ...` an Intent `68739830-327f-4691-8b9f-9d70971465fd`, dann stoppen. Normal im eigenen Thread SHA, Bericht und Ergebnis melden. Du arbeitest auf dem vorhandenen Stand weiter, kein Neubeginn.
