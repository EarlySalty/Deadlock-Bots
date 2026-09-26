# Betriebseinstellungen im Admin-Dashboard

## Umfang

`#betrieb` verwendet einen versionierten, serverseitigen Einstellungskatalog statt einer eigenen kurzen Feldliste im Browser. Der Discord-Katalog klassifiziert 220 Felder: 167 sind betrieblich änderbar, 53 betreffen geschützte Infrastruktur. Die Steam-Seite erweitert die bisherigen Betriebswerte um den vollständigen typisierten Steam-Katalog; kontobezogene Einträge werden für die tatsächlich konfigurierten Konto-IDs erzeugt.

Das ist die Konfiguration der an dieser Seite angebundenen Discord- und Steam-Dienste. Eigenständige Twitch-/Brain-Dienstkonfigurationen und bislang fest programmierte Konstanten anderer Repositories werden dadurch nicht automatisch zu Dashboard-Einstellungen. Die Discord-seitigen Brain-Schalter und KI-Anwendungsfälle sind enthalten.

Geschützte Einträge werden auf Wunsch mit Begründung angezeigt. Ihre Werte werden nicht ausgeliefert. Dazu gehören Zugangsdaten und ihre Infisical-Referenzen, ausführbare Programme/Dateipfade, Auth-Grenzen, OAuth- und interne Zieladressen. Eine vollständige Betriebsadministration ist kein beliebiger Datei-, Shell- oder Secret-Editor.

## Bedienung

Über die Suche sind Beschriftung, Bereich, Konfigurationspfad und Hinweis auffindbar. Die Gruppen umfassen KI, Bot-Pate/Onboarding, Community/Voice, Moderation, Streamer-Zuordnung, Startoptionen und die Steam-Funktionen. Listen werden mit einem Eintrag je Zeile bearbeitet. Discord- und Steam-IDs bleiben Dezimalstrings; sie werden weder im Browser gerundet noch als JavaScript-Number gespeichert.

„Dienststandard“ entfernt einen optionalen Override. Das ist etwas anderes als `false`, `0` oder eine leere Liste und behauptet keinen aktuell laufenden Wert. Änderungen werden vor dem Speichern als Vorher/Nachher-Entwurf angezeigt. Der Browser sendet nur geänderte Pfade. Ein Konflikt mit einem zwischenzeitlich geänderten Dateistand erhält den Entwurf; Neuladen fragt vor dessen Verwerfen nach.

Speichern startet keinen Dienst neu. Die Anzeige unterscheidet gespeicherten Dateistand, bestätigten Prozessstand und unbekannten Prozessstand. Der Discord-Bot wird über die authentifizierte Broker-Gesundheitsantwort geprüft; Steam-Cores über frische Heartbeats mit Konfigurationsfingerprint. Ein nicht erreichbarer Dienst gilt nicht als aktuell.

## KI: Modellwahl und Aufrufparameter

Für jeden der 14 KI-Anwendungsfälle sind Anbieter, Modell-Pin, Ausgabetoken-Limit, Temperatur, Denkaufwand und Zeitlimit je Anfrageversuch einstellbar. Nicht gesetzte Werte behalten die bisherige Aufrufsemantik bei, einschließlich JSON-Ausgabe und explizitem Denk-Aus der Klassifizierer. Ein äußeres Zeitlimit des jeweiligen Bot-Flows bleibt bestehen.

Die Modellpriorität ist:

1. Expliziter Pin des KI-Anwendungsfalls.
2. Expliziter Standard-Pin des zugeordneten Anbieters.
3. Bisherige Modellvorgabe des Aufrufers bzw. Providers.

Der zentrale Provider-Wrapper setzt die ersten beiden Ebenen auch durch, wenn ein älterer Konsument bei jeder Anfrage noch sein bisheriges Modell mitsendet. Das Transparenzprotokoll erhält denselben effektiven Modellnamen. Separate Bildmodell-Aufrufe bleiben bei ihrer eigenen Konfiguration.

Der Knopf „DeepSeek V4.1 Flash für Fireworks wählen“ setzt `accounts/fireworks/models/deepseek-v4p1-flash` als Fireworks-Standard sowie als Pin aller derzeit Fireworks zugeordneten Anwendungsfälle. Vorhandene Einzelpins bleiben dadurch nicht versehentlich auf V4 stehen. OpenAI-Zuordnungen werden nicht geändert, es findet weder ein automatischer Anbieterwechsel noch ein Wechsel zu Pro statt. Der Knopf bearbeitet nur den Entwurf; danach muss gespeichert werden.

Modell-ID belegt durch den öffentlichen Fireworks-Katalog vom 10.09.2026: https://fireworks.ai/models/deepseek-ai/deepseek-v4p1-flash . Das belegt die Modell-ID, nicht die Freischaltung oder den Kontostand eines konkreten API-Kontos. Ein Modellaufruf ist kein Teil der Speichern-Operation.

## HTTP-Vertrag und Speicherung

Die bisherigen Endpunkte bleiben erhalten:

- `GET/PATCH /api/admin/betriebskonfiguration`
- `GET/PATCH /api/admin/steam-betriebskonfiguration`
- Steam intern: `GET/PATCH /internal/config`

GET liefert zusätzlich `catalog: {version: 1, fields: [...], values: {...}}`. PATCH unterstützt `{revision, changes: {"konfigurations.pfad": wert}}`; die älteren eng typisierten `options`-/`patch`-Verträge bleiben erhalten. Ganzzahlen und Ganzzahllisten verwenden Dezimalstrings. Unbekannte oder geschützte Pfade, falsche Typen und ungültige Gesamtkonfigurationen werden vor dem Austausch der Datei abgewiesen.

Die Server erhalten Session-/Vollzugriffsprüfung, Origin-/CSRF-Gate, festen authentifizierten Steam-Proxy ohne Redirects sowie `no-store`. Der Schreibweg verwendet Dateisperre, revisionsgesichertes Compare-and-Swap, vollständige Konfigurationsvalidierung, atomaren Dateiaustausch und fsync. Kommentare, nicht geänderte Werte und Dateirechte bleiben erhalten. Laufende Snapshots werden nicht ausgetauscht.

## Release und Rückweg

Zuerst die neue Steam-API mit dem Katalog ausrollen, danach Discord-Web und Discord-Bot gemeinsam. Alle Verbraucher einer Betriebsdatei müssen das neue Schema mit den optionalen KI-Parametern verstehen, bevor solche Werte gespeichert werden. Der Browser zeigt bei einem alten Steam-Dienst ohne Katalog eine klare Versionsmeldung statt eine unvollständige Scheinverwaltung.

Keine Datenbankmigration erforderlich. Vor einem Release die vorhandenen Betriebsdateien außerhalb von Git mit bestehenden Betriebsrechten sichern, ohne Secrets auszugeben. Beim Zurückrollen auf eine ältere Binary-Version müssen neu eingeführte KI-Parameter entfernt bzw. die dazu passende Konfigurationssicherung wiederhergestellt werden; die alten Loader weisen unbekannte Felder absichtlich ab. Keine alte Sicherung blind über später vorgenommene Betreiberänderungen schreiben.

Ein Wechsel des katalogpflegenden Steam-Kontos benötigt einen koordinierten Stopp aller Steam-Cores, damit nie alter und neuer Katalogpfleger gleichzeitig laufen. Die bestehende Prüfung „höchstens ein Katalogpfleger in der gespeicherten Konfiguration“ bleibt erhalten.

## Reproduzierbare Prüfungen

Im Discord-Rust-Workspace:

```sh
cargo test -p dl-core
cargo test -p dl-ai configured_chat::tests
cargo test -p dl-bridges steam_operating::tests
cargo test -p dl-dashboard operating_config::
cargo clippy -p dl-core --all-targets -- -D warnings
```

Zusätzlich `node --test service/static/operating-config.test.cjs`. Das Beispiel `cargo run -p dl-core --example admin-catalog-fixture` erzeugt ausschließlich synthetische Browser-Testdaten aus dem echten Rust-Katalog. Das entsprechende Steam-Beispiel heißt ebenfalls `admin-catalog-fixture` in `steam-config`. Die beiden JSON-Ausgaben werden als `{ "discord": <Discord-Ausgabe>, "steam": <Steam-Ausgabe> }` zusammengefasst. Der Browserlauf ist dann:

```sh
node scripts/test_operating_config_ui.cjs fixtures.json /pfad/zu/playwright-core /pfad/zu/testausgabe /pfad/zu/chromium
```

Playwright ist ausschließlich eine Testabhängigkeit und wird außerhalb des Repositories installiert. Der Test routet sämtliche HTTP-Anfragen auf synthetische Antworten; er verwendet weder Live-Sessions noch Produktiv-APIs. Screenshots zeigen den sichtbaren Desktop-/Mobilbereich, um sehr hohe GPU-Surfaces bei ausgeklappten Gesamtkatalogen zu vermeiden.

Die Coverage-Tests verlangen, dass jedes Feld der typisierten Konfiguration ausdrücklich klassifiziert ist. Die Editor-Tests prüfen exakte IDs, echte Runtime-Lookups nach erneutem Laden, unveränderte laufende Snapshots, Kommentare, Rechte, Konflikte und atomare Ablehnung ungültiger Mischänderungen. Der Browser-Smoke-Test prüft beide vollständigen Kataloge, den V4.1-Knopf, geänderte Einzelpfade, Konflikterhalt, nullable Boolean und die schmale Ansicht mit synthetischen Daten. Ein solcher Test ersetzt keine Prüfung nach dem produktiven Neustart.
