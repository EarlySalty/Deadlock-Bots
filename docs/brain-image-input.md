# Bilder bei /brain

## Bedienung nach einer späteren Freigabe

Bei `/brain` zuerst `frage` ausfüllen und über die optionale Auswahl `bild` einen Screenshot anhängen. Unterstützt werden PNG, JPEG und WebP mit einer Größe von 1 Byte bis einschließlich 8 MiB, ein Bild pro Aufruf. Das Bildfeld ist ein Discord-Dateiupload, kein Eingabefeld für beliebige Internetadressen. Ohne Bild bleibt der bisherige Textpfad bestehen. Bilder in gewöhnlichen `!brain`-Nachrichten sind nicht Teil dieser Erweiterung.

Mit Bild gibt Brain eine Beratung. Auch eine Buildfrage mit Bild veröffentlicht keinen In-Game-Build; die Antwort kennzeichnet diese Grenze. Der bisherige Review-Build-Pfad ohne Bild wird durch diese Erweiterung nicht verändert. Ein Text im Bild kann keinen Buildauftrag auslösen.

## Umsetzung und Grenzen

Discord löst die Attachment-ID über `data.resolved.attachments` auf. Ein fehlender oder ungültiger Anhang bleibt als fehlerhaft erkennbar, statt in eine Textfrage ohne Bild umzuschlagen. Die Metadaten werden vor dem Download geprüft. Der Download akzeptiert die HTTPS-Attachment-Pfade von `cdn.discordapp.com` und `media.discordapp.net`, folgt keinen Weiterleitungen, prüft Content-Type und Dateisignatur und begrenzt die übertragenen Bytes während des Lesens. Es wird keine lokale Bilddatei angelegt. Der vorhandene Bildanbieter erhält die Bilddaten als Data-URI und nicht als nachzuladende Discord-Adresse.

Die vorhandene zentrale Bildanalyse für die Moderation liefert eine kurze Beschreibung. Der AnswerEngine bekommt die ursprüngliche Frage und diese ungeprüfte Beschreibung in getrennten Feldern. Im beleggebundenen Modus ist die Beschreibung eine eigene `UserImage`-Quelle, kein verifizierter Spielbeleg. Im offenen Testmodus bleibt sie ebenfalls als ungeprüfter Bildkontext gekennzeichnet. Die Spielsuche darf Bildbeobachtungen zur Suche verwenden; die unveränderte Nutzerfrage bleibt für die Antwort erhalten. Die beiden Promptfelder haben getrennte Längenlimits.

Der Download hat ein Zeitlimit von 10 Sekunden, die Bildanalyse 45 Sekunden und die gesamte Bildantwort 100 Sekunden. Höchstens zwei Bildantworten werden gleichzeitig bearbeitet; der bestehende Nutzer-Cooldown und die zentrale Antwortbegrenzung bleiben wirksam. Bei einer ausgelasteten Bildanalyse, einem nicht lesbaren Anhang, fehlendem Analysezugang oder einem Timeout kommt eine konkrete Fehlermeldung. Ein fehlgeschlagener Bildaufruf wird nicht still als bildlose Frage beantwortet und belegt keinen erfolgreichen Nutzer-Cooldown.

Die Bildanalyse nutzt dieselbe bereits konfigurierte Client-Instanz und dasselbe Modell wie der vorhandene Analysepfad. Für diese Erweiterung wurde kein Modell gewechselt. Pro erfolgreicher Bildfrage kommt vor der bestehenden Antwortgenerierung ein zusätzlicher Analyseaufruf hinzu. Im Transparenzprotokoll bleiben Modell, Dauer und Erfolg beziehungsweise Fehlerstatus sichtbar; Bildkontext und Bildantwort werden dort inhaltlich ausgeblendet. Die Antwort an den fragenden Nutzer bleibt erhalten. Aussagen über die Speicherung beim Modellanbieter werden hier nicht getroffen.

## Bestehende Schalter

`BRAIN_CMD_ENABLED` übernimmt ohne eigene Einstellung, ob das konfigurierte Brain-Binary vorhanden ist. `BRAIN_OPEN_TEST_MODE` ist standardmäßig `false`; dann gelten die vorhandenen Kanalgrenzen. `BRAIN_MAX_QUESTION_LEN` ist standardmäßig 300 Zeichen und `BRAIN_COOLDOWN_SECS` 20 Sekunden. Die Discord-Frage wird höchstens mit 4000 Zeichen registriert. Die Bildbeschreibung ist separat auf 2200 Zeichen begrenzt.

Die bestehenden Werte stammen aus der zentralen Betriebskonfiguration. Für Bilder wurde kein neuer Betriebsschalter eingeführt. Diese Änderung verändert keine laufende Konfiguration; der tatsächliche Live-Zustand wurde im PR-Testbetrieb nicht umgestellt.

## Abnahme

Die lokalen Tests verwenden synthetische Discord-Payloads, lokale HTTP-Testserver und Ersatzantworten für die KI. Sie veröffentlichen keine Builds und senden keine Community-Nachrichten. Details zu Läufen, Ergebnissen und offenen Punkten stehen in `.tasks/2026-09-24-brain-image-input/STATUS.md` und im PR. Der aktive PR-first-Testbetrieb erlaubt hier keinen Merge, Produktiv-Deploy oder Bot-Neustart.
