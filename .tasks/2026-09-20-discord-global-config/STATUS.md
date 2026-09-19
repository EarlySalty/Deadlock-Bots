# Discord: zentrale TOML-Konfiguration

Stand: 20.09.2026, Europe/Berlin.

## Auftrag

Eine bearbeitbare `config/bot.toml` für den Discord-Bot und seine zugehörige Knowledge-Anbindung. Globale nicht geheime Betriebseinstellungen aus der Datei, Zugangsdaten weiter über Infisical. Keine TOML-zu-ENV-Brücke. Andere Bots bleiben in eigenen Aufträgen.

## WIP, nicht produktiv integriert

Feature-Branch: `feat/discord-global-toml-20260920`.
Ausgangsstand: `371a90e8bcf3a1a6cae8cd285867b0e1dffcd0e7`.

Vorbereitet sind:

- `dl_core::bot_config`: typisierter TOML-Lader, Schema-Prüfung, begrenztes Einlesen, wertfreie Fehlermeldungen, validierte Momentaufnahmen und Reload ohne Überschreiben des letzten gültigen Stands bei Parse-/Validierungsfehlern.
- `config/bot.toml`: WIP-Konfiguration mit Bereichen für Discord, Dienste, Features, Moderation, Concierge, Knowledge und KI. Die ausgeschalteten Features sind sichere Prüfdefaults, keine übernommenen Live-Werte.
- `dl-config-check`: Prüfbinary, das Syntax und Schema prüft, aber keine Dienstfunktion behauptet.
- Reiner Modellauswahlkern für die freigegebene DeepSeek-Flash-Namensfamilie, numerische Versionen, Serverless-/Bereitschafts-/Probe-Status und Pins.
- 25 neue Testfälle und ein schreibgeschützter GitHub-Actions-Testworkflow.

Wichtig: `dl-bot`, `dl-knowledge` und die Provider lesen die neue Config noch nicht. Die bisherigen ENV-Leser wurden nicht umgestellt. Es gibt noch keinen Katalog-HTTP-Adapter, Probe-Runner, periodischen Refresh, persistenten Last-known-good-Status oder Austausch der laufenden Clients. Der Auswahlkern führt keine Netzwerkaufrufe aus.

## Prüfstand

Im verfügbaren Container fehlen Rust-Toolchain und Produktions-Checkout. Der Host-/Codespace-Terminalzugriff ist über die angebotenen Integrationen nicht verfügbar.

Ein GitHub-Actions-Lauf zur Bereitstellung des versionierten Rust-Bestands schlug vor ausgeführten Schritten fehl:

- Run: `35476007818`
- Job: `105985258882`
- Ergebnis: failure, keine Job-Schritte und keine abrufbaren Job-Logs.

Die Ursache wurde nicht abschließend festgestellt. Das ist kein Nachweis eines grünen Tests und keine Aussage über einen Compilerfehler. Der temporäre Source-Snapshot-Workflow wird mit diesem Stand wieder entfernt.

Die 25 neuen Rust-Tests, rustfmt, Workspace-Integration und der finale Cargo.lock-Abgleich sind noch offen. Der zusätzliche Config-CI-Lauf muss ebenfalls anhand seines Ergebnisses geprüft werden.

## Nächster Implementierungsschritt

1. Tatsächliche Live-Einstellungen redigiert inventarisieren. Die Prüfdefaults nicht als Produktionswerte ausrollen.
2. Vollständiges Mapping nicht geheimer Einstellungen erstellen, inklusive dynamischer Schlüssel, direkter ENV-Leser und Wrapper. Den Schema-Entwurf an die belegten Verbraucher anpassen.
3. Discord- und Knowledge-Einstieg auf einen zentralen Config-Lader umstellen; die zugehörigen Bibliotheken erhalten dieselbe validierte Konfiguration. Kein `set_var` oder Shell-Export als Ersatz für die Umstellung.
4. Geheimnisse über die bestehende Infisical-Anbindung beziehen und getrennt an Clients geben. Nicht geheime ENV-Overrides aus diesen Pfaden entfernen.
5. Beide Fireworks-Pfade, direkte Textgenerierung und Chat-Provider-Fabrik, auf dieselbe Modellauflösung umstellen. Aktuellen `dl-answer`-Pfad aus dem Ausgangsstand berücksichtigen.
6. Offiziellen Katalog mit Paginierung und Zeitlimit anbinden. Kandidaten vor Umschaltung funktional prüfen. Letzten geprüften Stand policygebunden und befristet außerhalb der Config speichern. Pins und anwendungsfallspezifische Prioritäten vollständig testen.
7. Schema für noch fehlende echte Module/Parameter ergänzen. Die vorliegende Feature-Auswahl ist kein vollständiges Inventar des Discord-Bots.
8. Prüfen, welche Änderungen wirklich reloadfähig sind und welche einen Neustart benötigen. Ein getauschtes Config-Objekt allein aktualisiert keine Clients oder Scheduler.
9. Rust formatieren, Cargo.lock abgleichen, Unit-/Integrations-/Regressionstests ausführen. ENV-Unwirksamkeit und tatsächliche Weitergabe der Werte an Verbraucher mit Tests belegen.
10. Test-Gate und Merge-Kritiker durchlaufen, erst danach nach main integrieren und pushen. Kein API-Merge als Ersatz für das Host-Gate.
11. Discord- und Knowledge-Dienste aus dem geprüften Main-Stand bauen und deployen. Knowledge/Concierge-E2E, effektives Modell, PID/Binary, Fehlerjournal und Heartbeat prüfen. BM25 nicht ungeprüft auf Hybrid umstellen.
12. Dev-/Support-Doku in Deadlock-Docs nachziehen. Den Branch nach erfolgreichem Merge und Live-Nachweis sicher bereinigen, nicht vorher.

## Nachweise

MERGEPROTOKOLL[MS-1]: 0 Main-Git-Schritte | Anläufe: 0 | Gate: nicht ausgeführt, WIP auf Feature-Branch
LIVEBEWEIS[DV-1]: nicht ausgeführt | keine Produktionsänderung | Laufzeitintegration, Build, Restart und Funktionsnachweis offen
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: repo-nahe Task-Akte
