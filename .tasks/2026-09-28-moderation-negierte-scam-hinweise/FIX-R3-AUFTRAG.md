# Fix R3: Satzteilgrenze im direkten Warnkontext

Weiterarbeit im selben Luna-Thread und Worktree `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise`, Branch `fix/moderation-negierte-scam-hinweise`, Basis `773b95ededa6550c6e5c5bbc505789020dfd1c40`. Intent `68739830-327f-4691-8b9f-9d70971465fd`. REVIEW-R3.md lesen und mit dem nächsten Commit sichern. Kein Plan, kein neuer Thread.

R2-Gegenbeispiele sind behoben. Genau den neuen belegten Befund aus REVIEW-R3.md:21 korrigieren:
- `Sichtbarer Scam in der Anzeige: Warnung an Moderatoren.` ist von true nach false gekippt; muss true sein.
- `Sichtbarer Scam in der Gruppe: Warnung an Moderatoren.` ebenso.
Ursache in moderation_verdict.rs:205-243,357-365: `direct_warn_report_context` trennt nicht am Doppelpunkt. `Anzeige` und `Gruppe` werden wegen Endung e als Adjektiv-Modifikatoren des späteren Warnworts akzeptiert. Die Satzteilgrenze muss den direkten sprachlichen Bezug beenden.

Minimal an der Ursache korrigieren, keine ganzen Ausnahmesätze. Die Satzteiltrenner im neuen Helfer gegen die bereits im übrigen Konfliktprüfer verwendeten Grenzen abgleichen; bestätigte Grenzen nicht wieder verlieren. Vorherige R1/R2/Pizza- und Warn-/Berichtskontrollen erhalten. Gezielt Grenzen samt Komma, Doppelpunkt und den bereits implementierten Satztrennern prüfen, nicht eine neue Sprachheuristik aufbauen.

Nur vorhandener Helfer und Tests sowie Aufgabenberichte. Keine Code-Kommentare, kein repo-weites fmt, keine Modelle/Provider ändern. Rustfmt, Clippy, relevante Tests und vorhandene Suite nach FIX-R2-BERICHT.md. Keine externen Community-Daten oder Live-Sanktionen.

Neuer Commit, kein amend, nur eigener Branch pushen. Bericht FIX-R3-BERICHT.md mit SHA und Prüfungen. REVIEW-R3.md nicht inhaltlich ändern. Kein Gate-Aufruf über die bestehende Rundengrenze hinaus, kein Reset, kein Merge/Deploy. Anschließend unabhängige R4 gegen diesen Befund durch den Orchestrator. Keine Unter-Threads oder Unter-Agenten.
