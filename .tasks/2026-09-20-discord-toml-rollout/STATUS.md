# Discord-TOML-Rollout

Basis origin/main fb357052; drei ursprüngliche Eigencommits 5f26a544,78cbac58,04b94e7f als 0e4e61d1,3bfa414a,c4616ec1 übernommen. Fremdes kanonisches Checkout unverändert.

In Arbeit: vollständige Betriebsleser-Migration und vorhandenes Admin-Dashboard. Die produktive Betriebsmatrix fehlt; Beispieldatei ist keine Produktionsfreigabe. Kein Deploy erfolgt.

Erstes zusammenhängendes Paket: versionierter Kommentar-erhaltender Store, Fingerprint des gestarteten Prozesses, Admin-GET/PATCH mit vorhandener Auth/CSRF, vorhandener Rust-SteamBotClient als begrenzter serverseitiger Proxy, Formularseite im vorhandenen Dashboard. Zwei Discordregler sind angeschlossen: Moderationsmaßnahmen und Conciergezeitlimit. Steam-Felder entsprechen dem Vertrag des separaten Steam-Pakets7e51dee. Kein LLM-Provider-/Modellwechsel und keine automatische Modellauflösung aktiviert.

Offen vor vollständiger Freigabe:
- Alle übrigen Runtime-Einstellungen typisiert an TOML anbinden; keine Betriebs-ENV-Fallbacks.
- Produktive Werte/Persistenzpfade aus belegten nicht geheimen Quellen vollständig verifizieren; keine Beispiele übernehmen.
- Auth-/Editor-/Proxyprüfungen, Frontendprüfung, Rust/Security-/Intentreview und Selbstgate.
- Bot-Aktivfingerprint über bestehende authentifizierte Statusstrecke belegen; bis dahin zeigt Editor Bot-Stand unbekannt.
- Externe Betriebsdatei und Startwrapper, Dienste nach geprüftem Merge bauen/restarten/live prüfen.

Patchnotes-Editor wird separat geplant; Architekturentscheidung im übergreifenden INVENTAR-DISCORD-STEAM.md. Keine zweite Python- oder Rust-Konfigurationsquelle einführen.

## Prüfung des ersten Editorpakets

Bot/Web-Produktionscheck und Clippy für dl-core,dl-bridges,dl-dashboard,dl-bot,dl-web ohne Testtargets bestanden. Core-Suite51 (40 Bibliothek,8 Startprozess,3 Writer),3 neue isolierte Steam-Proxytests und die erweiterte Auth-/CSRF-Routenmatrix bestanden. JavaScript-Syntax mit node --check geprüft. Kein verbundener Browser, daher noch keine visuelle Abnahme.

Der zusätzliche all-targets-Clippylauf über dl-dashboard zeigt bestehende unwrap_used-Befunde in den fremden visual_brain-Tests sowie dem bestehenden Testaufbau der Routenmatrix. Neue eigene Unwraps wurden durch aussagekräftige Expect-Aufrufe ersetzt; fremde Visual-Brain-Dateien bleiben unverändert. Dieses Baselineproblem ist kein grüner all-targets-Nachweis.
