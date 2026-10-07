# Concierge: Antwortrolle, Coaching und Wissenslücken

status: aktiv, 2026-10-07

Auftraggeber: d3a1741e-82bc-4a48-865b-2845c663dca7.

## Bestand und Ursache

BESTAND[BS-1]: ja | Fundort: rust/bin/dl-bot/src/modglue.rs:718 | Anknüpfung: vorhandener Spiel-Fallback und laufende zentrale Brain-API.

Der Produktivdienst ist `deadlock-bot-rust.service`, nicht `dl-bot.service`. Er ruft die zentrale Rust-Brain-API auf `127.0.0.1:8788` auf. Das Journal bestätigt um 08:07:58 und 08:16:59 `InsufficientEvidence`, dazwischen um 08:10:30 `Answered`. Der Weg ist erreichbar; die beiden Spielfragen sind keine Transportfehler. Der Discord-Adapter verwarf den Text jeder unzureichenden Brain-Antwort und ersetzte ihn pauschal durch den Nani-Verweis.

Der aktive Brain-Stand ist ein Docs-Release mit `release_lexical`, Limit 6. Spielwissen liefert für die beiden Fragen keine ausreichende Antwort. Der Umfang der Reparatur bleibt bei korrekter Antwortrolle und zentraler Behandlung dieser Wissenslücken, nicht beim Umbau des Spielwissens.

## Eigene Worktrees

- Bots: `/home/nathanael/.worktrees/Deadlock-Bots-concierge-coaching-fallback`, origin/main e18f5222.
- Brain: `/home/nathanael/.worktrees/Deadlock-Brain-concierge-coaching-fallback`, origin/main 9711cb63.
- Docs: `/home/nathanael/.worktrees/Deadlock-Docs-concierge-coaching-fallback`, origin/main 6fa4ca3.
- Branch jeweils `fix/concierge-coaching-fallback`.

## Änderungen

Die zentrale Brain-API erstellt den passenden Wissenslücken-Text und ersetzt den Coaching-Platzhalter durch eine konstante Discord-Mention oder den Webverweis. Discord behält den zentralen Text auch bei `InsufficientEvidence`. Leere Altantworten verwenden den bereits vorhandenen Spiel-Fallback. Beide Prompts kennen die Concierge-Identität in Ich-Form, aber kein technisches Innenleben. Der öffentliche Korpus behält Aufgaben und Datenschutzrechte, entfernt Beschreibungen der eigenen Wissenswege und verwendet Ich-Form für Concierge-Aussagen.

## Prüfstand

Formatprüfung der sechs geänderten Rust-Dateien ist grün. Brain-Clippy ist grün. Der serielle Brain-Testlauf hat 126 passed, 0 failed, 0 ignored; der erste parallele Lauf scheiterte an einem HTTP-Testtimeout. Alle fünf geänderten öffentlichen HTML-Seiten haben 0 Validatorverstöße. Der Gesamtkorpus hat unverändert 13 Verstöße, auf dem Ausgangsstand separat nachgemessen.

Bots-Clippy hat drei Fehler in dl-brain; der unveränderte Ausgangsstand reproduziert dieselben drei Fehler. Der vorhandene DB-Testwrapper scheitert vor den Tests an einer fehlenden Migrator-Konfiguration. Die API-Projektionsfixtures und Discord-Handler werden separat mit dem vorhandenen testing-Feature geprüft.

Docs 534eb4c und Brain f6f5cef6 sind nach ALLOW vom lokalen Gate auf origin/main. Der Brain-Release-Build und der Import der öffentlichen Docs laufen noch. Für Bots fehlt bisher ein allgemeiner freigegebener Deploy-Wrapper; die gefundenen alten Installer sind an fremde Aufgaben und SHAs gebunden. Der Restart-Wrapper enthält brain-serve noch nicht.

Der Twitch-Zwilling in tb-knowledge verwirft weiterhin InsufficientEvidence-Texte. Er ist nicht geändert; beantwortete Coaching-Verweise werden von der zentralen API plattformgerecht ausgegeben.

## Noch offen

Bots-Tests und Gate, Bots-Merge, freigegebene Deploy-Wege, tatsächlich aktiver Korpus, Live-Beweise für Pocket, Haze und Coaching, Cleanup. Eine MCP-Nachricht des Bots ist kein menschlicher Eingang und gilt nicht als Beweis der Discord-Eingangsroute.
