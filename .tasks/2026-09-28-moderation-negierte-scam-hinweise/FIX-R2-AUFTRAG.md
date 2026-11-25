# Fix R2: Warn- und Berichtskontext ohne Rückfall

Intent `68739830-327f-4691-8b9f-9d70971465fd`. Bestehender Luna-Fixer führt den Stand fort, kein neuer Thread. Worktree `/home/nathanael/.worktrees/dl-moderation-negierte-scam-hinweise`, Branch `fix/moderation-negierte-scam-hinweise`, Basis HEAD `b3269338f5e067a52a19e358c54f51a1dc0049b4`, gepusht. REVIEW-R2.md ist der neue unabhängige Bericht und muss erhalten/mitgesichert werden. Kein anderer Implementierer aktiv.

## Auftrag
R1-Befunde sind unabhängig als behoben bestätigt. Genau die Regression aus REVIEW-R2.md:21 korrigieren:
- `Phishing in einer Warnung vor Betrugsmaschen.` muss false ergeben, liefert seit deinem Fix true.
- `Phishing im Bericht über Betrugsmaschen.` muss false ergeben, liefert seit deinem Fix true.
- `Sichtbarer Scam im Bild, Warnung an Moderatoren.` muss weiterhin true ergeben.

Ursache: In moderation_verdict.rs:292-299 wurden `in` und `im` als Kontext ganz entfernt, statt ihren Bezug zu Warnung/Bericht präzise abzugrenzen. Direkten sprachlichen Bezug zur Warnung bzw. zum Bericht erkennen, nicht irgendein früheres `im` aus `im Bild` benutzen. Kein pauschales Zurückdrehen, keine hart codierten ganzen Beispielsätze und keine breite neue NLP-Architektur. Prüfe das Satzglied samt zulässigen Artikeln/Modifikatoren und respektiere Satzteilgrenzen. Alle bisher bestätigten R1-Korrekturen, Pizza-Kontrolle, tatsächliche positive Scam-Hinweise und echte Warn-/Zitatkontexte müssen ihre Ergebnisse behalten.

## Beweis und Scope
Nur gemeinsamer Helfer und passende Tests in moderation_verdict.rs, plus Aufgabenberichte. Kein Refactoring, kein repo-weites fmt, keine Kommentare, kein Provider-/Modellwechsel, keine externen Community-Daten. Beide neuen Gegenbeispiele und die Gegenkontrolle selbst ausführen. Bestehende Suite, fmt und Clippy wie FIX-R1-BERICHT.md. Unabhängige Reviewberichte nicht inhaltlich ändern; allenfalls ihre erwähnten nachgestellten Leerzeichen entfernen, falls diff-check dies verlangt.

## Übergabe
Als neuen Commit auf denselben Feature-Branch legen, kein amend. REVIEW-R2.md und FIX-R2-BERICHT.md mit eigenen Dateien stagen und nur den eigenen Branch pushen. Bericht mit Prüfkommandos, Ergebnissen und SHA. Kein Merge oder Deploy. Gate-Deckel bleibt unverändert bindend, weder resetten noch erneute Runden erzwingen. Nach Fertigmeldung prüft derselbe unabhängige Reviewer in R3 nur diesen Befund und unmittelbar verursachte Regressionen. Keine Unter-Threads oder Unter-Agenten.

Wenn unerwartet blockiert: `[Bump-up] Paket A Fix R2: Grund: ... Erledigt: ... Worktree: ... Offen: ...` an Intent `68739830-327f-4691-8b9f-9d70971465fd` und stoppen. Sonst direkt umsetzen, keine Planfreigabe erfragen.
