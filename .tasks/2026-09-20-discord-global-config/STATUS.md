# Discord: zentrale TOML-Konfiguration

Stand: 20.09.2026, Europe/Berlin. Fortsetzung des WIP ab `78cbac58`.

## Auftrag und Abnahmestand

Eine bearbeitbare `config/bot.toml` für den Discord-Bot und seine zugehörigen Dienste. Globale Betriebseinstellungen aus der Datei, Zugangsdaten weiter über Infisical. Keine TOML-zu-ENV-Brücke. Andere Bots bleiben in eigenen Aufträgen.

Branch: `feat/discord-global-toml-20260920`. PR: `#447`, weiterhin Draft.
Der vollständige Auftrag ist nicht abgeschlossen und nicht produktiv ausgerollt.

## In dieser Fortsetzung implementiert

Der bestehende `dl_core::Config::from_env()`-Aufruf in `dl-bot` und `dl-web` führt jetzt zu einer prozessweiten TOML-Momentaufnahme statt zur bisherigen ENV-Auswertung. Der Methodenname bleibt für die vorhandenen Aufrufstellen erhalten; seine Implementierung liest keine Konfigurationswerte aus ENV. Eine fehlende oder ungültige Datei beendet diesen Startpfad mit einem wertfreien Fehler.

Die fünf bisherigen Dienstports stehen in `[services]`; der bestehende `Config::db_path`-Vertrag kommt aus `[storage].legacy_snapshot_path`. Dies ist keine Umstellung von Postgres auf SQLite. Der Zugang zur zentralen Postgres-Datenbank bleibt unverändert. Relative Snapshot-Pfade beziehen sich wie zuvor auf das WorkingDirectory.

Dateiauswahl: `config/bot.toml` relativ zum WorkingDirectory oder ein explizites `--config PFAD` beziehungsweise `--config=PFAD`. Doppelte, leere und fehlende Config-Pfade werden abgelehnt. Der Startstand bleibt bis zum Prozessneustart fest. Der separate BotConfigStore ist dadurch kein Hot-Reload für laufende Dienste.

Das Prüfbinary verwendet denselben Startpfad. Seine Ausgabe nennt die beiden projizierten Broker-/Changelog-Ports, aber keine vollständige Config und keine Zugangsdaten. Die Meldung grenzt die Prüfung ausdrücklich von der Dienstfunktion und den noch nicht migrierten Modulen ab.

Zusätzlich sind die Kollisions-/Bereichsprüfung der fünf Ports, die Validierung des Snapshot-Pfads und die Ablehnung von Knowledge-Port 0 ergänzt. Beim separat verwendbaren BotConfigStore bleibt der gewählte Dateiname erhalten, damit ein ausgetauschter Symlink bei reload nicht auf dem alten Ziel festhängt.

## Tests und überprüfbare Grenzen

In den bearbeiteten Rust-Config-Dateien stehen 47 Testfunktionen: 28 für Schema/Auswahl/Store, 11 für den Startpfad und acht neue Prozess-Integrationstests. Diese Zahlen sind Quelltextzählungen, keine erfolgreichen Testläufe.

Die Prozess-Tests prüfen vergiftete alte Port-/Pfad-ENV-Werte, fehlende Dateien, den Standardpfad, Änderungen beim nächsten Prozessstart, wertfreie Fehler, ungültige CLI-Argumente, die Gleichheitsform von --config sowie das unveränderte Betreiber-TOML nach der Prüfung.

Sieben lokale Python-Strukturprüfungen waren erfolgreich: TOML-Syntax und bisherige Portdefaults, Schemafeld-Abgleich, Projektion der fünf Ports, fehlende ENV-Wertleser/-Schreiber im neuen Startcode, keine Credential-Felder in der Beispieldatei und BM25-Erhalt, CI-Testpfade sowie Erhalt der bestehenden Modellauswahl-Testnamen. Das ersetzt weder Rust-Kompilation noch das Test-Gate.

Der CI-Workflow enthält jetzt außerdem `cargo check -p dl-bot -p dl-web`, Clippy für dl-core und die zusätzlichen Formatprüfungen. Der neue CI-Lauf muss anhand seines Ergebnisses bewertet werden.

In dieser Sitzung sind kein Produktions-Checkout, kein Host-/Codespace-Terminal und keine Rust-Toolchain verfügbar. Die Codex-MCP-Discovery liefert kein entsprechendes Werkzeug. Ein Toolchain-Download aus dem Arbeitscontainer scheitert an fehlender DNS-Auflösung. Deshalb wurden Rust-Tests, rustfmt und der Workspace-Build lokal nicht ausgeführt.

Der bisherige Config-Lauf `35476278927`, Job `105985964781`, endete mit failure und ohne ausgeführte Job-Schritte. Die Fehlerursache ist nicht abschließend festgestellt. Der GitGuardian-Erfolg dieses alten Commits ist kein Compiler- oder Laufzeitnachweis.

## Vor Main und Deploy offen

- Live-Betriebswerte redigiert inventarisieren und das vollständige Setting-Inventar erstellen. Die Feature-Schalter, Moderation, Concierge, Gateway und übrigen Modulkonfigurationen sind noch nicht durchgängig angebunden. Auch Hostadressen und WebConfig haben noch getrennte ENV-Leser. Die Prüfdefaults in der TOML nicht als Produktionswerte ausrollen.
- Den aktuellen dl-answer-/Knowledge-Pfad und beide Fireworks-Konstruktoren direkt anbinden. Katalog-HTTP-Adapter, Paginierung, Funktionsproben, Refresh, policygebundener befristeter letzter geprüfter Modellstand und Austausch laufender Clients fehlen weiterhin. Der vorhandene Auswahlkern führt keine Netzwerkanfragen aus. BM25 beibehalten.
- Rust-Formatierung, Cargo.lock-Abgleich aus dem ersten WIP, Unit-/Prozess-/Workspace-Regressionstests und beide Host-Merge-Gates ausführen. Danach Main-Integration und Push, Release-Builds mit -j 2, betroffene Dienste restarten, Knowledge-/Concierge-E2E, Modellstatus, PID/Binary, Fehlerjournal und Heartbeat prüfen. Dev-/Support-Doku nach Deadlock-Docs übernehmen. Branch/Worktree erst nach Merge und Live-Nachweis bereinigen.

## Nachweise

MERGEPROTOKOLL[MS-1]: 0 Main-Git-Schritte einzeln | Anläufe: 0 | Gate: nicht ausgeführt, WIP auf Feature-Branch
LIVEBEWEIS[DV-1]: nicht ausgeführt | keine Produktionsänderung | Build, Restart und Funktionsnachweis offen
TEXTNACHWEIS[DR-1]: Gedankenstriche 0 | ae/oe/ue/ss-Ersatz 0 | Absolutwörter 0 belegt | Senke: repo-nahe Task-Akte
