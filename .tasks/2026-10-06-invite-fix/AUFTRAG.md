# Invite-Lounge: verspätete Statusantworten beenden

Quelle: Deadlock-Brain `.tasks/2026-10-06-brain-abschluss/BRIEFING-B.md`.

## Ziel

Antworten auf Playtest-Bitten kommen mit echtem Steam-Status zeitnah oder bleiben aus. Historische Nachrichten erzeugen keine neuen Einladungen. Keine Eingriffe an echten Nutzerkonten zur Prüfung.

## Beleg und bestehender Vertrag

Am 06.10.2026 wurde die Anfrage `1557090663835762829` um 18:02:23 UTC geschrieben, die Einladung durch Steam um 18:03:50 UTC bestätigt und der Lounge-Auftrag erst um 19:02:33 UTC bearbeitet. Die Aussage stammte aus dem Steam-Audit, nicht aus einem Modell. Der einmalige Sieben-Tage-Rückblick hat außerdem erledigte alte Bitten wieder als offene Aufträge angelegt.

Die bestehende automatische Hilfe für Raumbitten wartet weiterhin eine Stunde. Ihr Ergebnis bleibt im gespeicherten Zustand und im Journal sichtbar, ohne verspätete öffentliche Statusantwort. Eine direkte aktuelle Bot-Bitte erhält weiterhin den Steam-Handler-Status. Die fünf Minuten Frischebudget begrenzen verspätete Gateway-Ereignisse, direkte Antworten und die Wiederaufnahme alter Aufträge; bei Raumbitten kommen die bereits vorgesehenen 60 Minuten bis zur Fälligkeit dazu.

## Änderungen

- Verlauf als Beobachtung von Codes und bereits vorhandenen Hinweisen, ohne Bitten oder Nachrichten zu erzeugen; getrennt vom Live-Empfang.
- Frischeprüfung vor der Verarbeitung, vor einem offenen Versandauftrag und vor Hinweisen oder direkten Ergebnisantworten.
- Abgelaufene offene Aufträge werden durch den Dienst selbst dauerhaft beendet. Compare-and-swap-Konflikte beim Live-Empfang werden erneut gelesen.
- Historische Beobachtung überschreibt keinen Code eines vorhandenen Auftrags.

## Abnahme

Rustfmt, Clippy und bestehende Invite-Regressionen einschließlich isoliertem Postgres. Gate-Selbstprüfung und lokaler Merge-Gate. Auslieferung über die bestehende Releasewurzel, Neustart und nachweisbarer Binary-Stand. Keine neue Deployment-Automatisierung.
