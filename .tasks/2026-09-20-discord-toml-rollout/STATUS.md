# Discord-TOML-Rollout

Basis origin/main fb357052; drei ursprüngliche Eigencommits 5f26a544,78cbac58,04b94e7f als 0e4e61d1,3bfa414a,c4616ec1 übernommen. Fremdes kanonisches Checkout unverändert.

In Arbeit: vollständige Betriebsleser-Migration und vorhandenes Admin-Dashboard. Die produktive Betriebsmatrix fehlt; Beispieldatei ist keine Produktionsfreigabe. Kein Deploy erfolgt.

Erstes zusammenhängendes Paket: versionierter Kommentar-erhaltender Store, Fingerprint des gestarteten Prozesses, Admin-GET/PATCH mit vorhandener Auth/CSRF, vorhandener Rust-SteamBotClient als begrenzter serverseitiger Proxy, Formularseite im vorhandenen Dashboard. Zwei Discordregler sind angeschlossen: Moderationsmaßnahmen und Conciergezeitlimit. Steam-Felder entsprechen dem Vertrag des separaten Steam-Pakets7e51dee. Kein LLM-Provider-/Modellwechsel und keine automatische Modellauflösung aktiviert.

Offen vor vollständiger Freigabe:
- Alle übrigen Runtime-Einstellungen typisiert an TOML anbinden; keine Betriebs-ENV-Fallbacks.
- Produktive Werte/Persistenzpfade aus belegten nicht geheimen Quellen vollständig verifizieren; keine Beispiele übernehmen.
- Auth-/Editor-/Proxyprüfungen, Frontendprüfung, Rust/Security-/Intentreview und Selbstgate.
- Bot-Aktivfingerprint nach Deployment live über bestehende authentifizierte Statusstrecke belegen.
- Externe Betriebsdatei und Startwrapper, Dienste nach geprüftem Merge bauen/restarten/live prüfen.

Patchnotes-Editor wird separat geplant; Architekturentscheidung im übergreifenden INVENTAR-DISCORD-STEAM.md. Keine zweite Python- oder Rust-Konfigurationsquelle einführen.

## Prüfung des ersten Editorpakets

Bot/Web-Produktionscheck und Clippy für dl-core,dl-bridges,dl-dashboard,dl-bot,dl-web ohne Testtargets bestanden. Core-Suite51 (40 Bibliothek,8 Startprozess,3 Writer),3 neue isolierte Steam-Proxytests und die erweiterte Auth-/CSRF-Routenmatrix bestanden. JavaScript-Syntax mit node --check geprüft. Kein verbundener Browser, daher noch keine visuelle Abnahme.

Der zusätzliche all-targets-Clippylauf über dl-dashboard zeigt bestehende unwrap_used-Befunde in den fremden visual_brain-Tests sowie dem bestehenden Testaufbau der Routenmatrix. Neue eigene Unwraps wurden durch aussagekräftige Expect-Aufrufe ersetzt; fremde Visual-Brain-Dateien bleiben unverändert. Dieses Baselineproblem ist kein grüner all-targets-Nachweis.

## Runtimepaket 1: Start, Brücken, Web/Admin

In Arbeit nach73d47b12. runtime.start, runtime.bridges, runtime.dashboard und runtime.web sind streng typisierte Sektionen mit unbekannte-Felder-Abweisung. Nur vorhandene interne Lookup-Namen werden projiziert, keine frei befüllbare ENV-Tabelle. Nicht gesetzte Optionen behalten den jeweiligen bisherigen Konstruktor-Default; alte Aliasprioritäten werden beim Übertrag der belegten Werte zu genau einem Feld zusammengefasst. Die Projektion fällt für Betriebswerte nicht auf Prozessumgebung zurück. OAuth-Client-ID ist ausdrücklich Betriebskonfiguration; Client-Secret bleibt Infisical.

Angebunden: Gateway/Presence/Commands/Owner/MCP/Broker-Listen und -Fristen, Steam-/Twitch-/Turnier-/Websitebrücken, Matcherwerte, Dashboard-Authidentitäten/Sessionfristen/URLs, Stats-Cookies/CORS/Listener/Static-Dateien und Wiki-/Insights-Pfade. Der bestehende Brokerhealth-Endpunkt erhält den gestarteten Fingerprint und PID. Dashboard prüft ihn authentifiziert live, direkt ohne Systemproxy/Redirect, mit Zeitlimit; bei fehlendem Nachweis bleibt Zustand unbekannt. Webstand kommt aus dem eigenen unveränderlichen Prozesssnapshot. Keine neue Statusroute oder neue Secrets.

Explizite relative neue Pfade beziehen sich auf das Verzeichnis der gewählten Config. Nicht gesetzte bestehende Persistenzdefaults werden nicht umgedeutet oder verschoben. storage.legacy_snapshot_path behält ausdrücklich seinen früheren Vertrag. Betreiberdatei muss für Liveübernahme die belegten absoluten Daten-/PID-/Static-/Matcherpfade enthalten. Kanal-/Nutzer-/Guild-bezogene dynamische Datenbankeinstellungen bleiben unangetastet.

Noch folgende Bereiche2+3+6: Community/Voice/LFG/Scrim-Aufnahme, übrige Moderationsgrenzen/IDs, vorhandene KI-/Knowledge-Betriebsparameter. Bis dahin kein Vollmigrations-/Deploylabel. Keine automatische Modellwahl oder neue Modelle aktivieren.

Prüfung Runtimepaket 1: Bot/Web-Check und Produktions-Clippy für dl-bot, dl-web, dl-core, dl-broker und dl-webcore mit -D warnings bestanden. 97 Tests (Core54, Broker32, Webcore11) bestanden; Kommentar-/CAS-Test enthält nun einen relativen Datenpfad, damit gespeicherter und erneut geladener Fingerprint gleich bleiben. Zusätzliche ID-/Präfixvalidierung anschließend gezielt geprüft. UI unverändert, weiterhin kein Browsernachweis. Selbstgate und unabhängiger Paketreview folgen auf dem festen Commit.
