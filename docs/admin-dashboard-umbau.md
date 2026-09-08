# Discord-Admin-Dashboard: Umbau

Auftrag vom 08.09.2026: vorhandenes Admin-Dashboard zusammenführen, defekte
Anzeigen an ihrer Ursache reparieren und den tatsächlichen Nutzen der
Auswertungen sichtbar machen.

## Abnahme

- Eine Navigation und eine Anmeldung für Admin und Discord-Insights.
- Offizielle Discord-Importe sind als Zahlen und Zeitreihen sichtbar, mit
  eindeutigem Datenstand und einer Abgrenzung zu selbst gemessenen Bot-Daten.
- Die lange Twitch-Invite-Link-Liste entfällt.
- Discord-IDs bleiben auf dem Weg in den Browser verlustfreie Zeichenketten.
  Anzeigenamen kommen aus dem vorhandenen Mitglieder- und Verlaufsspeicher.
- Voice-Historie kann über einen Namen gesucht und auf die ausgewählte
  Plattform-ID eingeschränkt werden.
- Mitspieler werden anhand echter gemeinsamer Voice-Zeiten angezeigt.
  Poll-Zähler werden nicht als gemeinsame Spielsessions ausgegeben.
- Scrims zeigen zuerst die laufenden Begegnungen. Erstellung und technische
  Einstellungen stehen in eigenen, übersichtlichen Bereichen.
- Zweitgehirn trennt Vorschläge, tatsächlich ausgeführte Aktionen und
  gemessene Ergebnisse. Alte Berichte erscheinen mit ihrem tatsächlichen Alter.
- Deutsche Texte und Datumsangaben, bedienbare Tastatursteuerung sowie
  lesbare Ansichten auf schmalen und breiten Bildschirmen.

## Prüfung und Auslieferung

Regressionstests für Datenverträge und Berechtigungen; Messung der neuen
Voice-Abfragen mit Produktionsdaten; Prüfung der Browser-Skripte und
kritisches Gesamt-Review der gemeinsam geänderten Schreib- und Lesepfade.
Anschließend Merge, Push, Release-Build, Neustart des Webdienstes und
Kontrolle der ausgelieferten Routen. Visuelle Browserprüfung wird separat
dokumentiert; eine fehlende Browser-Verbindung gilt nicht als bestandene
Sichtprüfung.

Keine zusätzlichen KI-Modelle oder automatischen Community-Nachrichten.

## Reproduzierbare Frontend-Prüfung

`service/static/dashboard.html` enthält alle acht tatsächlich unterstützten Bereiche.
Die alte Insights-Seite und die nicht angebundenen Legacy-Steuerungen für Dienste,
Cogs und Logdateien sind entfernt. `/api/auth/me` initialisiert die gemeinsame
Session; jede Schreibaktion wartet auf den CSRF-Token.

DOM- und Interaktionstest (kein Browserprofil und keine echten Daten):

```sh
npm install --prefix /tmp/ddc-ui-test --no-audit --no-fund jsdom
node scripts/test_dashboard_ui.cjs /tmp/ddc-ui-test/node_modules/jsdom
```

Der Test lädt das vollständige Dashboard, klickt alle vorhandenen Tabs und prüft
Namensduplikate, exakte Discord-IDs, Suchfehler, XSS-Ausgaben, verspätete
Voice-Antworten, CSV-Bestandswerte und Datumsfilter, AFK-Mitgliedschaft, Scrim-Formular,
Zweitgehirn-Bilanz sowie die verzögerte Anmeldung mit anschließendem CSRF-geschütztem
Schreibaufruf und verständlichem 403-Fehler. JSDOM dient ausschließlich diesem Test;
das produktive Frontend bekommt keine neue Abhängigkeit.

Visuelle Prüfung bleibt zusätzlich erforderlich. In der Umbau-Session meldeten
sowohl Browser-Runtime als auch Vorschau-Runtime, dass kein Browserhost verfügbar ist.
Ein bestandener DOM-Test ersetzt diese Prüfung nicht.
