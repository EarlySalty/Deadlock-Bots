# Sofortmaßnahme live abgeschlossen

Der automatische Matcher ist auf dem bisherigen geprüften Release `dd58c0afd4f410ae5624b669137d4ef46d859079` deaktiviert. Kein neuer Code wurde ausgeliefert.

In `/home/nathanael/.config/deadlock-bots/bot.toml` wurden ausschließlich diese beiden vorher nicht gesetzten Werte unter `[runtime.bridges]` ergänzt:

```toml
matcher_enabled = false
matcher_state_path = "/home/nathanael/repos/Deadlock-Bots/data/streamer_link_state.json"
```

Die gesamte geparste Konfiguration blieb außerhalb dieser zwei Werte unverändert. Die Datei behält Modus 0600 und Eigentümer/Gruppe nathanael. `LIVE-CONFIG-AENDERUNG.json` sichert nur die vorherigen Schlüsselwerte, Dateihashen und Metadaten; es enthält keine Zugangsdaten und keine komplette Configkopie. Der bestehende Altbestand wurde weder kopiert noch überschrieben.

`systemctl --user restart deadlock-bot-rust.service` ist ausgeführt. Start: 05.10.2026 um 20:04:47 CEST. Neuer PID: 2539351. Nachkontrolle: active/running, NRestarts=0. `/opt/deadlock/bots/current` zeigt unverändert auf den bisherigen Release.

Der tatsächliche Start protokolliert den Config-Fingerprint `823839e8b75107bace8f0a618340a0a568a6d465658ead91bd136bc3ad6f9d1c`. Der bestehende strikte Rust-Configloader lieferte für die Datei denselben Fingerprint und `matcher_enabled=Some(false)` sowie den angegebenen dauerhaften Zustandspfad. Der dafür kurzzeitig außerhalb des Repos verwendete lokale Nachweisaufruf wurde nicht deployt; seine temporären Dateien sind gelöscht.

Das Journal seit Neustart enthält einen Gateway-READY-Nachweis und zwei Cache-Bereitschaftsnachweise. Es enthält keine Matcher-Scan-, Negativmeldungs- oder Notify-Markierung. Der bisherige produktive Code kehrt bei deaktiviertem Matcher vor dem automatischen Loop und vor einem Scan zurück. Damit entstehen aus diesem Matcher keine weiteren automatischen Vergleiche oder KI-Aufrufe.

Die vorhandenen manuellen Button- und Modalhandler bleiben auch bei deaktiviertem automatischem Scan registriert. Der Zustand enthält weiterhin 20 abgeschlossene Fälle und sechs offene manuelle Eingaben, keine Reviews; Dateigröße und Änderungszeit sind unverändert. Es wurde kein Discord-Button ausgelöst und keine externe Testnachricht gesendet. Die Erreichbarkeit ist durch aktive Gateway-Verbindung und unverändertes Handler-Wiring belegt; ein echter manueller Verknüpfungsvorgang wurde live nicht ausgeführt.

Der neue Code bleibt ungemergt im eigenen Worktree erhalten. Das Gate blockt nach weiterer Fixrunde die Überschreibung eines älteren offenen manuellen Auftrags bei wiederverwendetem Login. Der Gate-Deckel ist erreicht. Es gibt keinen Merge, Push oder Deploy dieser blockierten Änderung. Die vom Nutzer ausdrücklich erlaubte Rückfalllösung bleibt aktiv.
