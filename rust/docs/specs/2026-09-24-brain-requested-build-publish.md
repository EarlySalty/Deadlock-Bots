# Brain: Builds auf ausdrückliche Anfrage

Stand: Feature-PR vom 24. September 2026, noch nicht ausgeliefert.

Mit `/brain frage: Bau mir einen Build für Warden` wird die bestehende Brain-CLI zur Erstellung und Veröffentlichung über den Steam-Bot angesprochen. Das gilt auch außerhalb des offenen Testmodus. Fragen wie „Ist mein Build gut?“ oder „Erkläre diesen Build“ bleiben Wissensfragen und veröffentlichen nichts. Verneinte Veröffentlichungen und Entwurfswünsche werden konservativ nicht als Schreibauftrag eingeordnet.

Der reguläre Pfad ruft `publish-build-query` auf. Er benötigt den Brain-PR `feat/direct-build-publish-20260924`, der diesen Befehl vom bisherigen Review-Alias trennt. Die vorhandene Familien-, Patch-, Stichproben- und Skillorder-Prüfung bleibt aktiv. Reicht die Datenbasis nicht aus oder fehlt eine eindeutige Variante, wird nichts veröffentlicht und Brain nennt den fehlenden Schritt.

`BRAIN_OPEN_TEST_MODE` hat im bestehenden Startpfad den Standardwert `false`. Ist der vorhandene offene Testmodus eingeschaltet, bleibt der gesonderte `review-build`-Pfad erhalten und die Antwort bezeichnet den Build als experimentelles Review-Build. Dieser PR ändert den Konfigurationswert nicht. Der aktuelle produktive Wert wurde für diese PR-Abnahme nicht erneut ausgelesen.

Eine Build-ID gilt als veröffentlicht, wenn die CLI einen abgeschlossenen Steam-Auftrag und eine positive Build-ID zurückliefert. Bei `PENDING` oder `RUNNING` wird der Auftrag als eingereiht gemeldet, nicht als fertiger Build. Fehlende IDs sowie fehlgeschlagene, abgebrochene oder unbekannte Zustände erzeugen keine Erfolgsmeldung.

Bestehende Kanalgrenzen, DM-Sperre, Fragegrenzen, Cooldown und KI-Modellwahl bleiben unverändert. Die CLI erhält die Frage als einzelnes Argument nach `--`; es findet keine Shell-Auswertung der Frage statt. Die Wissensroute bleibt unverändert.

Vor einer späteren Auslieferung muss die neue Brain-CLI vorhanden sein. Im aktuellen PR-first-Testbetrieb werden keine Produktions-Builds erstellt, keine Dienste neu gestartet und die PRs nicht automatisch gemergt.
