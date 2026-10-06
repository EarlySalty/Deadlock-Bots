# Invite-Lounge: verspätete Statusantworten beenden

Quelle: Deadlock-Brain `.tasks/2026-10-06-brain-abschluss/BRIEFING-B-FIX1.md` und die verbindliche Nutzerentscheidung in `VON_HAUPT.md`, 06.10.2026, 23:10 CEST.

## Ziel

Antworten auf Playtest-Bitten kommen mit echtem Steam-Status zeitnah oder bleiben aus. Historische Nachrichten erzeugen keine neuen Einladungen. Keine Eingriffe an echten Nutzerkonten zur Prüfung.

## Beleg und bestehender Vertrag

Am 06.10.2026 wurde die Anfrage `1557090663835762829` um 18:02:23 UTC geschrieben, die Einladung durch Steam um 18:03:50 UTC bestätigt und der Lounge-Auftrag erst um 19:02:33 UTC bearbeitet. Die Aussage stammte aus dem Steam-Audit, nicht aus einem Modell. Der einmalige Sieben-Tage-Rückblick hat außerdem erledigte alte Bitten wieder als offene Aufträge angelegt.

Die bestehende automatische Hilfe für Raumbitten wartet weiterhin eine Stunde. Direkte und verzögerte Versandversuche speichern ausschließlich den echten Steam-Handler-Status im Zustand und im Journal. Die Invite-Lounge sendet keine eigene öffentliche Ergebnisantwort mehr. Antworten formuliert künftig das gemeinsame Brain; Paket A baut den lesenden Invite-Skill. Der bestehende Freundescode-Hinweis bleibt unverändert. Die fünf Minuten Frischebudget begrenzen verspätete Gateway-Ereignisse, Versandversuche und Hinweise; bei Raumbitten kommen die bereits vorgesehenen 60 Minuten bis zur Fälligkeit dazu.

Klarstellung des Haupt-Orchestrators im Fixer-Thread: Die fehlerhafte öffentliche Lounge-Ergebnisantwort darf nach Gate-ALLOW ohne Warten auf Paket A abgeschaltet und ausgeliefert werden. Die Schutzgrenze für andere sichtbare Antwortpfade aus `VON_HAUPT.md`, Punkt 3, gilt hier nicht. Paket A liefert den Brain-Skill getrennt nach.

## Änderungen

- Verlauf als Beobachtung von Codes und bereits vorhandenen Hinweisen, ohne Bitten oder Nachrichten zu erzeugen; getrennt vom Live-Empfang.
- Frischeprüfung nach dem Laden und unmittelbar nach dem Versand-Claim vor Steam; Hinweise lesen die Uhr nach ihrer Speicherung neu. Öffentliche Ergebnisantworten entfallen vollständig.
- Abgelaufene offene Aufträge und bereits beanspruchte Versuche werden durch den Dienst selbst dauerhaft ohne Versand beendet. Compare-and-swap-Konflikte beim Live-Empfang und bei historischen Beobachtungen werden erneut gelesen.
- Historische Beobachtung überschreibt keinen Code eines vorhandenen Auftrags.

## Abnahme

Rustfmt, Clippy und bestehende Invite-Regressionen einschließlich isoliertem Postgres. Gate-Selbstprüfung und lokaler Merge-Gate. Auslieferung über die bestehende Releasewurzel, Neustart und nachweisbarer Binary-Stand. Keine neue Deployment-Automatisierung.
