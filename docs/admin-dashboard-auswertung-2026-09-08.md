# Admin-Dashboard: Betriebsbefund und Umbau

Messung am 08.09.2026 zwischen 18:00 und 18:09 Uhr Europe/Berlin. Nur lesende
Abfragen der Produktionsdatenbank `deadlock`, aggregiert ohne einzelne Nutzer,
Nachrichten oder Zugangsdaten; zusätzlich Journal des Insights-Syncs.

## Discord-Einblicke

Beide Timer-Läufe am 07.09. endeten erfolgreich: 06:16:40 und 07:16:35 Uhr,
jeweils 13 offizielle CSV-Dateien und 884 verarbeitete Zeilen. Die Datenbank
bestätigt den letzten Import für alle elf Import-Arten um 07:16:35 Uhr.
Jüngster Datenzeitraum überwiegend 05.09., Retention 23.08. Bestehender
Discord-Login im Brave-Profil bleibt Voraussetzung, Export und Import sind
automatisch. Die bislang separate Insights-Seite zeichnete ausschließlich
Bot-Messungen; offizielle Zahlen wurden nur als Liste der Import-Arten erwähnt.

Der Umbau führt die Ansicht in die Admin-Shell. Die API trennt weiterhin
`live` und `imported`, ergänzt den Importzeitpunkt je Zeile und einen
guildbezogenen Importstatus unabhängig vom gewählten Anzeigezeitraum.
DB-Fehler erscheinen als Fehler, nicht als angeblich leerer Import.
Verschiedene Definitionen und Einheiten dürfen nicht unbemerkt vermischt werden.

## Zweitgehirn

| Messung | Befund |
|---|---|
| Feeder | 06.09., 19:00 Uhr erfolgreich, 62 gesehen, 61 relevant, committed/gepusht |
| Wochenplan | 06.09., 19:02 Uhr erfolgreich, fünf Vorschläge; davor vier Fehlerwochen |
| Historische Planpunkte | 19 offen, keine dokumentierte Entscheidung und kein Kommentar |
| Separater Wochenreport | Letzter Bericht vom 02.08.2026; veraltet |
| Verbinder in sieben Tagen | 5.004 Entscheidungen, sämtlich Shadow; 4.909 unterdrückt |
| Gemessene Treffen | 16 geprüfte Outcomes, alle `not_met` |
| Scrim-Lagebild | Vier erzeugte Snapshots |

Fazit: Wochenlage und Vorschlagsliste funktionieren und bleiben nützlich.
Ausgeführte Community-Maßnahmen oder eine positive Wirkung sind durch diese
Zahlen nicht belegt. Auch ein gemessenes Treffen nach einem Shadow-Urteil wäre
nur eine Beobachtung, kein Nachweis einer durch die KI verursachten Wirkung.
„Übernommen“ beim Planlauf bedeutet in die Vorschlagsliste gespeichert, nicht
vom Betreiber angenommen oder erledigt. Fehlende Kommentare belegen nur die
fehlende Nutzung dieses Rückmeldewegs, nicht dass niemand die Seite gelesen hat.

API-Ergänzungen: `report_stale`, `effectiveness_week` (Entscheidungen, Shadow,
erzeugte Snapshots, geprüfte und positive Treffen) und `plan_usage`
(gesamt/offen/entschieden/kommentiert). Der archivierte Report bekommt seinen
eigenen Altersstatus. Fehlendes/unlesbares Wiki liefert HTTP 503 statt einer
leeren Erfolgsantwort. Der bestehende Wiki-Pfad ist ein funktionierender Symlink
auf `Deadlock-2nd-Brain`, nicht auf das Spielwissen-Repo `Deadlock-Brain`.

## Reproduzierbare SQL-Methodik

```sql
SELECT import_kind, count(*), max(period_start), max(imported_at)
FROM activity.insights_imports GROUP BY import_kind;

SELECT count(*), max(created_at) FROM bot.brain_reports;
SELECT run_at, status, gesehen, relevant, committed, pushed
FROM brain.feeder_runs ORDER BY run_at DESC LIMIT 6;
SELECT run_at, status, vorgeschlagen, uebernommen, verworfen
FROM brain.plan_runs ORDER BY run_at DESC LIMIT 6;
SELECT status, count(*), count(entschieden_am), count(kommentar)
FROM brain.plan_items GROUP BY status;

SELECT source, decision, action_taken, outcome, count(*)
FROM bot.ai_decision_ledger
WHERE decided_at > now() - interval '7 days'
GROUP BY source, decision, action_taken, outcome;
```

Keine neuen KI-Aufrufe, Modelle, Tokens oder externen Nachrichten. Bestehende
Discord-Sitzung, Vollzugriff auf interne Brain-Inhalte und Origin-/CSRF-Prüfung
bei Änderungen bleiben erhalten. Der alte Pfad `/insights` führt nach der
bisherigen Authentifizierung zu `/admin#insights`.

## Namen, Inaktivität und gemeinsame Voice-Zeit

Die gerundeten Discord-IDs entstanden beim Serialisieren als JSON-Zahlen.
Snowflakes werden nun als Strings geliefert, einschließlich Co-Player-,
Guild- und Kanal-IDs in den betroffenen Analytics-Antworten. Namen werden
zuerst aus dem aktuellen Bot-Cache und anschließend aus der neuesten
persistenten Namenshistorie zur unveränderten Plattform-ID aufgelöst.
Die Voice-Suche findet Namen, frühere Namen und vollständige IDs und liefert
maximal 20 Auswahlmöglichkeiten; Namen werden nicht als Identität verwendet.

Alle zehn bisherigen AFK-Zeilen betrafen ehemalige Mitglieder: sieben
Austritte und drei Bans. Die historischen DM-Fehler nannten fehlende gemeinsame
Server. Zugleich beschränkte die Anzeige sich auf ungenutztes DM-Versandbudget;
bereits angeschriebene weiterhin anwesende Mitglieder verschwanden aus der
Übersicht. Der Dashboard-Read zeigt jetzt unabhängig vom DM-Cooldown inaktive
Stammnutzer (mindestens 14 Tage), beachtet weiterhin Opt-outs und sortiert
vorhandene Mitglieder vor unklarer Mitgliedschaft und ehemaligen Mitgliedern,
bevor das Limit von 50 greift. Die Live-Abfrage ergibt 136 weiterhin vorhandene
inaktive Mitglieder. Die Mitgliedschaft stammt aus dem jeweils neuesten
Directory-Sync oder Join-/Leave-/Ban-Ereignis derselben Guild. Status und
Prüfzeit werden explizit mitgeliefert. Der Bot-Versand wurde nicht verändert.

Die bisherige Co-Player-Tabelle enthält maximal 496.955 angebliche Sessions
und 4.530.706 Minuten pro gerichteter Verbindung. `sessions_together` ist im
Rust-Schreiber tatsächlich ein Zehn-Minuten-Pollzähler; der Legacy-Backfill
hatte zudem vollständige Zweiwochen-Aggregate alle sechs Stunden erneut
addiert. Dieser Akkumulationsfehler wird im Rust-Schreiber bereits vermieden,
die historischen Summen bleiben für eine belastbare Rangliste ungeeignet.

Die neue Rangliste berechnet deshalb echte zeitliche Überschneidungen der
abgeschlossenen Voice-Aufenthalte im selben Server und Kanal. Zeitfenster
werden am Anfang und Ende abgeschnitten; ein Paar ist kanonisch angeordnet,
überlappende oder doppelte Aufzeichnungen werden per `range_agg` vereinigt.
Die UI zeigt gemeinsame Zeit, keine angeblichen Sessions. Laufende, noch
nicht abgeschlossene Aufenthalte fehlen ausdrücklich in dieser historischen
Auswertung. Bot-/Privatsphäre-Scope stammt unverändert aus den Voice-Logs.

Live-Grundlage: etwa 81.500 gespeicherte Aufenthalte, davon rund 8.590 in
30 Tagen und 22.070 in 90 Tagen. Die 30-Tage-Auswertung fand 2.281 Paare;
das stärkste Paar lag bei 87,4 gemeinsamen Stunden. Der erste korrekte
Overlap-Join brauchte 288 ms für 30 Tage und 2,25 s für 90 Tage. Zusätzliche
gemeinsame Tages-Buckets verkürzten den 90-Tage-Fall auf 455 ms bei identischen
Ergebnissen. Gemessen mit `EXPLAIN (ANALYZE, BUFFERS)` auf Live-Daten.

Regressionen decken große Snowflakes, ungültige Personenfilter, historische
Namenssuche ohne Gateway, wörtliche Suche ohne SQL-Wildcards, doppelte
Voice-Logs, Zeitfenstergrenzen, getrennte Server/Kanäle und die Priorität
aktueller Mitgliedschaft vor dem Zeilenlimit ab. DB-Tests verwenden einen
expliziten Unix-Socket mit Peer-Authentifizierung und eine eindeutig benannte
Wegwerf-Datenbank samt Cleanup; Konfiguration unter
`rust/crates/dl-dashboard/tests/postgres.json`, keine Test-Secrets oder
ENV-Konfiguration.

Prüfstand nach dem Umbau: `cargo check` und `cargo clippy -- -D warnings`
für `dl-dashboard --all-targets --features testing` erfolgreich; geänderte
Rust-Dateien bestehen `rustfmt --check`. 82 Standard-Unit-Tests erfolgreich.
Zusätzlich tatsächlich ausgeführt: sieben DB-Regressionen zu Overlap und
Mitgliedschaft, Namenshistorie, Importzeit/Guildfilter, Importhistorie,
Shadow-Auswertung und TurnierOnly-Zugriff. Auch der zunächst fehlerhafte
TurnierOnly-Testaufbau wurde korrigiert: gespeicherte Sessions müssen vor der
Initialisierung des persistenten Session-Stores angelegt werden. Die
Namenssuche benötigt auf Live-Daten 141 ms für eine Namenssuche und 122 ms
für eine exakte ID; die Tages-Buckets sind unabhängig von der
PostgreSQL-Session-Zeitzone ausdrücklich UTC.
