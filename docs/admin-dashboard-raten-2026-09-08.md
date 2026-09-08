# Interaktion und Bindung: Ursachen und Korrektur

Stand: 08.09.2026, Untersuchung ab 18:40 Uhr Europe/Berlin.

## Zwei tatsächliche Messfehler

1. Der Gateway-Writer ließ jede Nachricht mit menschlichem Autor durch, auch
   Discord-Systemmeldungen über Beitritt und Boosts. Discord führt dabei das
   betreffende Mitglied als Autor. Diese Einträge landeten als Chat und als
   `first_message` im Journey-Speicher. 435 Rohdatenmatches der Erstnachrichten
   von Beigetretenen seit 06.07. waren ohne Text/Anhang im System-/Logkanal;
   diese Auffälligkeit war der Suchhinweis, **kein Löschkriterium**.
2. Die bisherige Woche-1-Quote zählte Nachrichten, Voice, Interaktionsknöpfe und
   `presence_daily_seen`. Bloßes Online-Sein bei Discord wurde damit als
   Community-Rückkehr ausgewiesen. In der Woche 10.08. hatten 42 von 61
   Beigetretenen einen Presence-Treffer ohne belegte Chat-/Voice-Teilnahme.

Der Writer akzeptiert jetzt ausschließlich `MessageType::Regular` und
`InlineReply` von Nicht-Bots. Leerer Inhalt bleibt erlaubt: Anhänge, Sticker
und Weiterleitungen sind echte Nachrichten. Systemmeldungen gelangen bereits
vor dem Dispatcher nicht mehr in Nachrichtenstatistiken oder Erstchat-Ereignisse.

## Genaue Definition

- Nenner: unterschiedliche Personen je Guild und Beitrittsperiode, aus
  `member_events`. Mehrere Beitritte derselben Person innerhalb einer Periode
  zählen einmal. Später Ausgetretene bleiben enthalten. Ein Wiederbeitritt in
  einer anderen Woche zählt dort als Beitritt; keine Behauptung eines erstmaligen
  Beitritts zur gesamten Community.
- Interaktion am Beitrittstag: echte Nachricht oder Voice-Ereignis **nach** dem
  Beitrittszeitpunkt und vor Mitternacht UTC. Das ist kein rollendes 24-Stunden-Fenster.
- Bindung: echte Nachricht oder Voice in der folgenden vollständigen UTC-Woche
  Montag bis Sonntag. Online-Status und Verify-/Onboarding-Knöpfe zählen nicht.
- Wochen werden nur ausgewertet, wenn sie vollständig im gewählten Zeitraum
  liegen. Die Folgewoche muss abgeschlossen sein. Übersichtskarte und Trend
  verwenden denselben Helper und denselben ausgewählten Zeitraum.
- Vor Beginn vollständiger Rohdaten bzw. außerhalb ihrer 180-Tage-Aufbewahrung
  bleibt der Zähler/die Quote `null` (`data_available=false`). Fehlende Daten
  werden nicht als Inaktivität ausgegeben.

## Messbeleg vor der Bereinigung

Die alte Anzeige lässt sich direkt mit ihrer bisherigen SQL-Definition
reproduzieren. Zähler/Nenner aus der Produktionsdatenbank:

| Beitrittswoche | Aktivierung bisher | Bindung bisher | Online ohne Chat/Voice in Folgewoche |
|---|---:|---:|---:|
| 10.08. | 59 / 61 (96,7 %) | 51 / 61 (83,6 %) | 42 |
| 17.08. | 42 / 43 (97,7 %) | 30 / 44 (68,2 %) | 26 |
| 24.08. | 38 / 39 (97,4 %) | 31 / 42 (73,8 %) | 24 |

Die alte Aktivierung verwendete den Journey-Zustand, die Bindung sämtliche
Join-Ereignisse ohne Deduplikation. Daher unterschieden sich teilweise schon
ihre Nenner. Die neue Produktionsabfrage wurde am 08.09. um 19:05 Uhr in
derselben Transaktion nach der ID-verifizierten Reparatur ausgeführt.
Anschließend wurde alles zurückgerollt: ein belastbarer Probelauf, noch kein
Nachweis des produktiven Deployments.

| Beitrittswoche | Aktivierung nach Korrektur | Bindung nach Korrektur |
|---|---:|---:|
| 10.08. | 17 / 61 (27,9 %) | 9 / 61 (14,8 %) |
| 17.08. | 11 / 44 (25,0 %) | 4 / 44 (9,1 %) |
| 24.08. | 18 / 42 (42,9 %) | 7 / 42 (16,7 %) |

Vorab ermittelte Leertext-Ausschlüsse wären niedriger ausgefallen, werden aber
bewusst nicht übernommen: ohne Typnachweis ist eine leere Nachricht kein
beweisbares Systemereignis.

Offizielle Discord-CSV-Zahlen bleiben als eigene Quelle getrennt. Sie enthalten
andere Exportzeiträume/Kohorten; etwa `pct_communicated` und `pct_opened_channels`
sind verschiedene Größen. Ihre Export-Prozentwerte sind bereits auf der Skala
0–100; die eigenen API-Raten sind Brüche 0–1. Ein direkter Gleichstand wird nicht
erwartet und nicht künstlich hergestellt.

## Nachweisbare historische Reparatur

`scripts/maintenance/repair_discord_system_messages_20260908.sql` arbeitet nur
mit per Discord-REST bestätigten Message-IDs und Typen (7 MemberJoin,
8 NitroBoost, 10 NitroTier2). Es korrigiert Rohmetadaten und genau die daran
hängenden Erstchat-Ereignisse/-Zeiger. Ein letzter Ereigniszeiger wird nur
ersetzt, wenn er exakt auf den entfernten Erstchat zeigte; spätere echte
Interaktionen bleiben erhalten. Alle geänderten Zeilen werden vorher gesichert.
Standardlauf endet mit ROLLBACK, Anwenden benötigt ausdrücklich `-v apply=true`
und erfolgt erst nach dem Writer-Deploy.

Alle 509 Kandidaten wurden mit Discord abgeglichen: 463 Typ 7, sieben Typ 8,
eine Typ 10; diese 471 IDs sind reparierbar. 25 reguläre Nachrichten bleiben
erhalten. 13 bereits gelöschte Nachrichten (404/Unknown Message) bleiben ohne
Typnachweis ebenfalls unverändert: eine benannte Restunsicherheit.
Die Vorschau korrigierte 453 Erstchat-Zeiger und vier betroffene letzte
Ereigniszeiger. Backup: 1.377 ursprüngliche Zeilen.

Legacy-Zähler, Textsession-Logs und
Textpunkte haben keine Message-IDs; gleiche Gesamtsummen beweisen keine
Ereignisidentität. Dort gibt es **keine geratenen Abzüge oder Zeitkorrekturen**.
Der verbleibende Bestand: 439 Konten mit 456 Textsessions im betroffenen Kanal
vom 11.05. bis 08.09. sowie 454 Legacy-Nachrichtenzähler. Diese Einträge sind
nicht pauschal falsch, ihre einzelnen Systemanteile sind nicht ID-genau
zuordenbar. Nachrichten- und
Journey-Tagesaggregate waren bei der Prüfung leer; das Skript bricht ab, falls
zwischenzeitlich verdichtete Historie entstanden ist.
