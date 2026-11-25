# Autoverknüpfung einmal je Twitch-ID

Stand: gebaut auf `779e7014`, Gate und unabhängige Abnahme blocken den Versand nach einem Prozessabbruch. Frischer Fixer übernimmt gemäß HANDOFF.md. Merge und Deploy warten auf unabhängige Abnahme.

## Ursache

Der alte Matcher speicherte abgeschlossene Versuche nach Twitch-Login. Offene manuelle Eingaben wurden bei der Einmaligkeitsprüfung ignoriert. Schreib- und Umbenennungsfehler blieben unbemerkt.

Live läuft `deadlock-bot-rust.service` mit `WorkingDirectory=/opt/deadlock/bots/current`. Der aktuelle Release `dd58c0afd4f410ae5624b669137d4ef46d859079` gehört root und enthält kein Verzeichnis `data/`. In der normalen Betriebskonfiguration war kein eigener Matcher-Pfad gesetzt. Der relative Standardpfad konnte dort nicht zuverlässig geschrieben werden.

Belegte Altdaten liegen in `/home/nathanael/repos/Deadlock-Bots/data/streamer_link_state.json`: 20 abgeschlossene Fälle und sechs offene manuelle Eingaben, keine offenen Review-Vorschläge. Dieser bestehende Pfad gehört dem Dienstnutzer nathanael, liegt außerhalb der Releases und wird beim Deploy als `runtime.bridges.matcher_state_path` gesetzt. Die Quelle bleibt erhalten.

## Änderung

- Automatische Versuche werden dauerhaft nach der Twitch-Plattform-ID gespeichert. Dieselbe ID bleibt nach einem Neustart und einer Umbenennung geschlossen.
- Belegte alte abgeschlossene Fälle, offene Vorschläge und manuelle Eingaben werden übernommen. Bestehende manuelle Buttons bleiben verfügbar.
- Speichern umfasst Datei- und Verzeichnis-Synchronisierung sowie atomisches Umbenennen. Fehler beim Laden oder Speichern sperren den automatischen Abgleich vor Nebenwirkungen.
- KI-Scoring entfällt vollständig. Der bestehende kostenlose Ratcliff/Obershelp-Vergleich bleibt erhalten. Nur eindeutige exakte normalisierte Treffer dürfen automatisch verknüpft werden; ähnliche oder mehrdeutige Namen benötigen Bestätigung.
- Fehler der Verknüpfungs-API werden wieder geöffnet. Ein unterbrochener automatischer Schreibversuch bleibt als erneut prüfbar gespeichert.
- Der bewusste Admin-Neuversuch löst die aktuelle Twitch-ID über die bestehende Kandidaten-API auf. Manuelle Verknüpfungen bleiben benutzbar.
- Der vorhandene Loop führt nach dem Start einen stillen Scan aus und protokolliert ausschließlich Zähler sowie den Zustandspfad. Leere erfolgreiche Läufe senden keine Nachricht.

Der Nutzer meinte mit „Jav“ das Decision Model Jev von TypeSafe. Für diesen Fix wird kein Modell eingebaut oder gewechselt. Die zentrale KI-Konfiguration für andere Aufgaben bleibt unverändert.

Eigene Codepfade: `rust/crates/dl-bridges/src/matcher.rs`, `rust/crates/dl-bridges/src/glue.rs` und `rust/bin/dl-bot/src/main.rs`.

## Prüfungen

`cargo check -p dl-bot --jobs 3`: Exit 0. Eine bestehende Warnung in dl-brain über `fetch_update` liegt außerhalb dieser Änderung.

`cargo test -p dl-bridges matcher::tests --jobs 3`: 19 Tests grün. Enthalten sind zweite Scans und Neustarts ohne erneute Meldung oder KI-Aufruf, Umbenennung derselben Twitch-ID, Übernahme negativer Altdaten und manueller Eingaben, beschädigter Speicher, technische API-Fehler, manuelle Verknüpfung und mehrdeutige exakte Namen. Der abschließende Lauf nach der Gateway- und Speicherkorrektur ist grün.

Alle Cargo-Prüfungen laufen mit einem Host-Buildslot. Release-Build, unabhängige Abnahme und Live-Beweis stehen noch aus. Die eigene Gate-Nachprüfung ist mit Exit 1 beendet.

## Nachprüfung der Speicherstrecke

Der erste eigene Gate-Lauf gegen `558b9151` blockte einen leeren Elternpfad bei bloßen Dateinamen und verlorene Vorschläge bei Speicherfehlern. Als Hinweis kam eine mögliche Kollision mit dem bewussten Admin-Neuversuch hinzu. Der Folgecommit `779e7014` korrigiert alle drei Punkte: aktuelles Verzeichnis als Elternpfad, wiederholbare Auswertungsphase und gemeinsam gespeicherte Entscheidung samt Versanddaten sowie derselbe Scan-Lock beim Admin-Reset. Drei zusätzliche Tests prüfen Dateinamen ohne Verzeichnis, unterbrochene Auswertung und erfolglosen Nachrichtenversand nach Neustart.

Der Matcher-Port lieferte bisher einen unvollständigen Gateway-Cache. Er lädt jetzt alle Mitglieder in REST-Seiten mit höchstens 1000 Einträgen. Fehler, fehlender Fortschritt und ein leerer Gesamtbestand sperren die Entscheidung. Der Startscan wartet zusätzlich auf die Bereitschaft der Guild im Gateway, damit auch die Rollenliste vorliegt.

Der Versand wird vor dem Discord-Aufruf dauerhaft reserviert. Ein bestätigter Versandfehler bleibt als gespeicherter Auftrag wiederholbar. Nach einem Prozessabbruch mitten im Versand wird keine mögliche bereits gesendete Nachricht erneut erzeugt; bei diesem seltenen Abbruch kann eine Meldung ausbleiben. Offene manuelle Eingaben und Reviewdaten bleiben gespeichert. Die automatische Entscheidung wird dabei nicht erneut berechnet.

Die behauptete Korrektur der Versandstrecke hat die Nachprüfung nicht bestanden: Ein Abbruch vor Discord-Aufruf kann die Nachricht auslassen. Dieser Fund bleibt offen und muss vor Merge und Deploy durch einen frischen Fixer korrigiert werden.

## Tatsächlicher Abschluss

Der Nutzer hatte das Abschalten als Rückfall ausdrücklich erlaubt. Diese Lösung ist jetzt live: ausschließlich matcher_enabled=false und der belegte dauerhafte Zustandspfad in normaler Config, Neustart des bisherigen geprüften Releases, geladenen Fingerprint und aktive Gateway-Verbindung geprüft. Alle Nachweise und Grenzen stehen in LIVE-SOFORTMASSNAHME.md. Die Codekorrektur bleibt wegen eines weiteren Gate-Blocking und erreichtem Gate-Deckel ungemergt erhalten. Keine blockierte Änderung wurde deployt.
