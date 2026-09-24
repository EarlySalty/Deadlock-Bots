# Brain: Bild als Eingabe

## Auftrag und Eigentum

Der Nutzer möchte beim Discord-Befehl `/brain` ein Bild zusammen mit seiner Frage anhängen können. Er hat am 24. September 2026 ausdrücklich beauftragt: „Übernehmen und weiter / fertig machen“. Damit ist die Übernahme der vier bereits veränderten Dateien in diesem Worktree freigegeben.

Repo: EarlySalty/Deadlock-Bots.
Worktree: /home/nathanael/.worktrees/deadlock-bots-brain-image-input-20260924
Branch: feat/brain-image-input-20260924
Ausgangs-HEAD: ff635f7b354cb09909c01ddd6f773d0682dd89c9
Vorhandene Änderungen: rust/bin/dl-bot/src/main.rs, rust/bin/dl-bot/src/modglue.rs, rust/crates/dl-brain/src/lib.rs, rust/crates/dl-discord/src/dispatch.rs.
Intent: diese ChatGPT-MCP-Session, keine T3-Intent-ID vorhanden. Rückfragen und Abschluss in deinem eigenen Thread und in STATUS.md. Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder Unter-Agenten spawnen.

## Arbeitsweise

Du implementierst als opus48. Lies ~/CLAUDE.md, die eingebundene PR-first-Entscheidung, Arbeitsregeln und Skills. PR-first gilt: keine Merge-, main-Push-, Auto-Merge-, Deploy-, Produktionsmigrations-, Dienstneustart- oder Cleanup-Aktionen. Kein Zugriff auf fremde CI-Branches, Workflow-Dateien, Regeln oder fremde Änderungen. Eigener bestehender Worktree, kein weiterer Worktree nötig. Secrets nicht lesen oder ausgeben. Keine Produktionsdaten oder Community-Nachrichten als Test verändern. Keine Hooks umgehen. Keine Code-Kommentare ergänzen. Texte mit echten Umlauten, ohne Gedankenstriche.

Graphify ist unter /home/nathanael/.local/bin/graphify erreichbar; der Graph liegt im kanonischen Repo /home/nathanael/repos/Deadlock-Bots. Vor Code-Suche dort Graphify verwenden, Quelltexte in diesem Worktree nachlesen.

## Fertigkriterien

1. `/brain frage:... bild:...` hat eine optionale Discord-Attachment-Auswahl und reicht das tatsächlich aufgelöste Bild an die vorhandene freigegebene Bildanalyse durch. Text ohne Bild bleibt unverändert nutzbar. Ein Bild pro Slash-Aufruf genügt für diesen Auftrag. Kein neuer Modellanbieter, kein neues/teureres Modell. Bestehendes Analyse-Client/Modell aus der zentralen Verdrahtung wiederverwenden.
2. Bildinhalt wird als nicht vertrauenswürdiger Kontext getrennt von der ursprünglichen Nutzerfrage behandelt. Aus Bildtext oder dessen Zusammenfassung dürfen keine Build-Veröffentlichung, Tools, fremden Aktionen oder Änderungen der Intent-Erkennung entstehen. Der angefangene Weg self.answer(enriched) ist hierfür ungeeignet. Build-Wünsche mit Bild brauchen einen ehrlichen sicheren Weg; kein stilles Verwerfen des Bildes, kein vorgetäuschter Bildbezug. Prüfe bestehende AnswerEngine-Kontext-APIs und verwende die zentrale Wissensantwort.
3. Anhänge vor kostenpflichtiger Analyse validieren: eng begrenzte unterstützte Rasterformate, positive Größe bis 8 MiB, echte Discord-HTTPS-Attachment-Hosts und zulässige Pfade. Kein beliebiger URL-Input, keine Weiterleitungen zu privaten/externen Zielen, Streaming-Größenlimit statt unbeschränktem bytes(), kein Dateischreiben und keine Bild-URLs/Bildinhalte im Log. Bestehende sichere Download-Funktionen wiederverwenden. Fehlender Content-Type, defekte Auflösung, falscher Typ, zu große Datei, fehlende Bildanalyse, Timeout oder leeres Modellresultat ergeben einen verständlichen Fehler statt bildloser Antwort.
4. Eigene Zeit- und Ressourcenlimits für Bilddownload/Analyse; Nutzer-Cooldown/Admission nicht durch Bildpfad umgehen. Eingabelänge bleibt die Länge der Nutzerfrage, Bildkontext separat begrenzt. Keine Abschneide-Fehler bei langen gültigen Fragen. AiAnswerer-Default darf vorhandene Bilder nicht stillschweigend ignorieren.
5. Discord-Dispatch-Auflösung deterministisch testen, inklusive fehlendem resolved-Anhang. Bestehende Slash-/Textpfadtests erhalten. Ergänze gezielte Tests für gültiges Bild, Grenzen/ungültige Hosts/Redirects/Typen, sichere Intent-Trennung (Bild behauptet Buildauftrag), Fehler und erfolgreiche Durchleitung. Keine echten KI-/Discord-/Build-Publish-Aufrufe im Test.
6. Nutzer zeigte zusätzlich: „Mein Hirn hakt grad. Probier's in ein paar Sekunden nochmal.“ beim Mo-&-Krill-Buildwunsch. Diagnostiziere diesen bestehenden Fehler anhand Code und secret-sicherer read-only Laufzeitdaten; kein Produktions-Build oder Publish. Root Cause und Zusammenhang separat dokumentieren. Falls Ursache klein und im eigenen Scope, Regression beheben, sonst genauen Blocker statt Spekulation festhalten.
7. Relevante Formatierung, cargo check/clippy und Tests ausführen; höchstens -j 2 und ressourcenschonend, Logs und Exitcodes nachvollziehbar. Kein Release-Build/Produktiv-Binary nötig. Vor Build den Deploy-Verifizierer-Skill lesen. Im Scope erforderliche kleine Bibliothekserweiterungen sind erlaubt, z. B. dl-answer oder dl-ai, keine unabhängigen Umbauten.
8. Review deinen eigenen Diff kritisch. Sobald lokal verifiziert, Commit mit expliziter Dateiliste und Co-authored-by, unverzüglich push -u origin feat/brain-image-input-20260924. Vor Commit git log -1, status, worktree list, Schritte einzeln mit literalen Pfaden. Bei unvollständigem Stand wip-Commit und Push statt lokaler Restarbeit. PR gegen main erstellen/aktualisieren, offen lassen. Bestehende GitHub-CI auswerten, behebbare featurenahe Fehler korrigieren. Fremde CI-Baustelle nicht duplizieren. Unabhängiges Review organisiert die Hauptsession.

## Berichte

STATUS.md: Stand, exakt geprüfter SHA, Befehle, Testanzahlen/Skips, Exitcodes, PR und Run-URLs mit Status, offene Blocker. REVIEW.md: eigene gefundene und behobene Risiken. Kurze repo-nahe Nutzungs-/Betriebsdoku in docs/ ist erlaubt, keine CHANGELOG.md anlegen und nichts öffentlich posten. REGISTER.md pflegt die Hauptsession; nicht mitcommitten, bis sie freigegeben hat. Kein Fertig-Claim ohne Nachweis. Nicht auf Rückfragen warten, wo die Umsetzung innerhalb dieser Grenzen entscheidbar ist.
