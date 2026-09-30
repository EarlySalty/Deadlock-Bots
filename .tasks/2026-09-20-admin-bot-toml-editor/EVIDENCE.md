# Nachweise und Rolloutstand

## Implementiert

Registry-basierter TOML-Editor für die bestehenden Bot-Konfigurationsdateien, gemeinsame Browser-Oberfläche, unabhängige Validierung, Byte-CAS, native Dateisperren, Kommentarerhalt, atomare Speicherung, private Historie (höchstens 20 Versionen) und explizite beaufsichtigte Aktivierung. Discord-Seite unter `/admin#betrieb`; Twitch hat eigene authentifizierte Proxy-Routen und eine eigene Admin-Seite in einem separaten Repo-Branch.

## Tatsächlich ausgeführte Prüfungen

- `cargo check -p dl-core`, erfolgreich.
- `cargo check -p dl-dashboard`, erfolgreich (Job j-1789921296-1637).
- `cargo test -p dl-core`, zuletzt 57 Tests erfolgreich: 32 Unit-, 11 Editor-, 8 Startup-, 3 bestehende Editor- und 3 Runtime-Tests. Enthält native/std-Dateisperren-Kompatibilität für Discord, Steam und Twitch sowie den Erhalt unveränderter Array-of-Tables-Kommentare.
- `cargo clippy -p dl-core --all-targets -- -D warnings`, erfolgreich vor dem letzten Kommentarerhalt-/CLI-Prüfzusatz; finale Wiederholung vor Merge erforderlich.
- `cargo test -p dl-dashboard --lib visual_brain_route_tests`, erfolgreich: alle alten und neuen Routen der Matrix weisen unberechtigte Sitzungen vor Datei-/Prozesszugriff ab; Schreibzugriffe ohne CSRF bleiben gesperrt (Job j-1789924591-1877).
- Browser-Smoke mit echtem Chrome und ausschließlich simulierten API-Antworten: alle 11 Abläufe erfolgreich, finaler Prozess Exit 0. Entwurfserhalt bei Fehlern/Konflikten, Trennung Speichern/Aktivieren, Versionsentwurf, Abbruch beim Bot-Wechsel, gesperrte nicht verbundene Bots, Cleanup. Mobilmessung: 390 px Viewport, 390 px Seitenbreite, kein horizontaler Seitenüberlauf.
- Browser-Artefakte nur lokal auf dem Server: `/home/nathanael/.cache/admin-bot-toml-qa-final/{report.json,desktop.jpg,mobile.jpg}`. Dies ist kein authentifizierter produktiver Browsertest.

## Produktionsinventar vor eigenem Deploy

Discord-Bot und dl-web starten mit `/home/nathanael/.config/deadlock-bots/bot.toml`. Steam-Bot und beide Steam-Cores starten mit `/home/nathanael/.config/deadlock-steam-bot/bot.toml`; der native Steam-Prüfer akzeptiert die unveränderte Datei. Patchnotes startet mit `/home/nathanael/.config/deadlock-bots/patchnotes/bot.toml`; `main.py --check-config` akzeptiert die unveränderte Datei vor jedem Client-/Secret-Zugriff.

`dl-knowledge` verwendet eine eigene JSON-Datei und wird nicht als Verbraucher der Discord-TOML registriert. Brain-Site und Turniere hatten beim Inventar keinen expliziten TOML-Startpfad. Twitch läuft als getrennte Systemrollen `twitchbot` / `twitchdash` aus `/opt/deadlock/twitch/current`; der produktive Release war 818e2152 ohne expliziten TOML-Startpfad. Der zwischenzeitlich gemergte Twitch-TOML-Code erwartet `/var/lib/deadlock-twitch/config/bot.toml`; dieses Verzeichnis existierte bei der letzten Prüfung noch nicht. Deshalb werden diese drei Ziele im Beispiel nicht fälschlich aktiviert.

## Freigabegrenzen

Noch kein eigener produktiver Rollout und keine Live-Aktivierungsbestätigung in diesem Dokument. Registry, CLI und Dashboard müssen gemeinsam aus einem geprüften Main-Stand ausgerollt werden. Vorhandene Produktionsdateien werden dabei weder mit Beispieldaten ersetzt noch mit erfundenen Werten ergänzt. Die Versionshistorie ist manuelle Wiederherstellung als Entwurf, kein behaupteter automatischer Rollback.
