# Brain: Builds auf ausdrückliche Anfrage

Stand: Feature-PR vom 24. September 2026, noch nicht ausgeliefert.

Nur der strukturierte Discord-Befehl `/brain-build held:<Held> spielstil:<Waffen|Geist>` erteilt einen Schreibauftrag. `/brain` und `!brain` bleiben reine Wissensfragen. Auch Formulierungen wie „Bau mir einen Build, aber nur als Vorschlag, ohne ihn hochzuladen“ sowie zitierte Bitten lösen keine Veröffentlichung aus.

Vor dem Aufruf der Brain-CLI reserviert der Bot die Discord-Interaktions-ID dauerhaft in `brain.discord_build_publish_requests`. Der Primärschlüssel verhindert, dass parallele Zustellungen oder ein Neustart denselben Auftrag erneut ausführen. Ein Wiederholungsversuch erhält das gespeicherte Ergebnis oder die Meldung, dass der erste Auftrag noch läuft. Bleibt ein Auftrag nach einem Absturz offen, wird er nicht automatisch nochmals veröffentlicht; die Interaktion muss vor einer manuellen Wiederholung geprüft werden.

Der reguläre Pfad ruft `publish-build-query` auf. Er benötigt den Brain-PR `feat/direct-build-publish-20260924`, der diesen Befehl vom bisherigen Review-Alias trennt. Die vorhandene Familien-, Patch-, Stichproben- und Skillorder-Prüfung bleibt aktiv. Reicht die Datenbasis nicht aus oder fehlt eine eindeutige Variante, wird nichts veröffentlicht und Brain nennt den fehlenden Schritt.

`BRAIN_OPEN_TEST_MODE` hat im bestehenden Startpfad den Standardwert `false`. Ist der vorhandene offene Testmodus eingeschaltet, nutzt `/brain-build` weiterhin `review-build` und bezeichnet den Build als experimentelles Review-Build. Dieser PR ändert den Konfigurationswert nicht.

Eine Build-ID gilt als veröffentlicht, wenn die CLI einen abgeschlossenen Steam-Auftrag und eine positive Build-ID zurückliefert. Bei `PENDING` oder `RUNNING` wird der Auftrag als eingereiht gemeldet. Fehlende IDs sowie fehlgeschlagene, abgebrochene oder unbekannte Zustände erzeugen keine Erfolgsmeldung.

Bestehende Kanalgrenzen, DM-Sperre und KI-Modellwahl bleiben unverändert. Die CLI erhält die strukturierte Build-Anfrage als einzelnes Argument nach `--`; es findet keine Shell-Auswertung statt. Vor einer Auslieferung müssen die neue Brain-CLI und die Datenbankmigration vorhanden sein.
