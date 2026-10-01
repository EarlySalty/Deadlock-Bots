# Datenbankgebundene Tokenablage

Zentrale additive Migrationen für gehashte Turnier-/Steam-Bearer sowie verschlüsselte Steam- und Browser-Zugänge. Die alte Node-Steam-Brücke bricht ausdrücklich ab, statt einen Dateispeicher wieder einzuschalten.

## Verbindliche Speichergrenze

Kontozugänge und wieder benötigte Geheimwerte werden nur feldverschlüsselt in der Datenbank gespeichert. Für reine Bearer-Prüfungen wird der Rohwert nicht zurückgewonnen, sondern ein irreversibler Lookup-Schlüssel verglichen. Die vorhandene tb-crypto-Implementierung bleibt die gemeinsame Feldkrypto. Konto- und Feldkontext müssen authentifiziert sein.

Keine neuen Token-JSONs, Refresh-Dateien, dotenv-Zugänge oder persistenten Browserprofile. Kein stiller Datei-, Klartext- oder Fremdkonto-Fallback. Keine Tokenwerte, verschlüsselten Blobs, Session-URIs oder Browserzustände in Logs, Berichten oder Shellargumenten.

Master-Key, OAuth-Anwendungssecrets, Datenbankverbindung und Infisical-Bootstrap bleiben außerhalb der Anwendungstabellen im bestehenden Secret-Manager. Sie sind keine zweite Ablage der rotierenden Kontotokens. Ein Verschlüsselungsschlüssel wird nicht neben seine eigenen Ciphertexte gelegt. TradingBot gehört nicht zu dieser Umstellung.

## Lokaler Arbeitsstand

Die zusammengehörigen Worktrees liegen als Geschwister unter /home/nathanael/.worktrees/token-db-local-20260930/. Die vorhandenen relativen Abhängigkeiten auf Deadlock-Bots und tb-crypto werden dort wiederverwendet. Quellkopien, ein zweites Kryptopaket und Änderungen an geteilten Checkouts sind nicht nötig. Alle Rust-Prüfungen verwenden den vorhandenen sccache und die gemeinsame Zwischenablage /home/nathanael/.cache/rust-build/{workspace-path-hash}.

Ein Quellstand allein belegt keinen produktiven Cutover. Migrationen, Neustarts und Wiederaufnahme müssen im Betriebsnachweis zusammen bestätigt sein. Tests dürfen nur explizite Wegwerf-Datenbanken und synthetische Konten verwenden.

## Späterer koordinierter Cutover

1. Zugehörige neue Leser und Writer gemeinsam bereitstellen, vorhandene verschlüsselte Sicherung und Wiederherstellungsweg prüfen. Keine angewandte Migration ändern.
2. Alte Writer anhalten. Neue zentrale Schema-Migrationen anwenden. Gehashte Session-IDs nicht mit einem alten Consumer mischen. Bestehende Restore-/ETL-Werkzeuge vor einem Import auf das neue Tokenformat abstimmen; die Constraints lehnen Rohwerte ab.
3. Bestehende Steam-Guard-Werte ausdrücklich per privater Pipe an das Beispielprogramm import_guard im Steam-Core geben. Es nutzt dieselbe Kontokonfiguration und denselben Secret-Launcher wie der Dienst. Gleiche Freigaben dürfen erneut importiert werden, andere bestehende Werte und Widerrufe werden nicht überschrieben. Keine alten Dateien durch den Agenten öffnen.
4. VOD-Resume-Werte vor dem Start mit dem auf bereits geladenen Bot-Secrets beruhenden Wartungsmodus tb-bot --config <normale Config> --migrate-token-storage --apply und beim separaten Archiv mit --migrate-token-storage transaktional umstellen. Danach den NOT-VALID-Constraint der Twitch-Tabelle validieren. Ein Fehler lässt den jeweiligen Migrationsbestand unverändert. Abgeschlossene Uploads behalten ihre Video-ID.
5. Kontozuordnung, Entschlüsselung und Neustart-Wiederaufnahme prüfen. Erst danach alte Credential-Dateien oder Bootstrap-Kontotokens kontrolliert außer Betrieb nehmen. Die vorhandenen Dateien werden hier weder gelöscht noch als Backup verdoppelt.

Ein Code-Rollback allein reicht nach einem irreversiblen Hash-Cutover nicht. Entweder die neuen Lookup-Verträge beibehalten oder gemeinsam auf einen zuvor geprüften Datenbankstand zurückgehen. Ein nicht durchgeführter Restore-Test ist keine bestätigte Rollback-Fähigkeit.
# Privater Start der Token-DB-Verbraucher

Die normalen Betriebswerte werden zuerst aus der expliziten TOML geprüft.
Daneben liegt die normale Metadatenkonfiguration `infisical.json` mit
`secret_values_fd: 3`. Bots (`dl-bot`, `dl-web`) und `turnier-bot` lesen
diesen privaten Snapshot genau einmal vor Pool, Auth-Konfiguration und Clients.
Fehlender Snapshot oder DB-Zugang beendet den Start; keine Rückkehr zum alten
ENV-Loader und kein erneutes Lesen des FD3. Die bisherigen Secret-Aliase und
alle Betriebs- und Modellentscheidungen bleiben erhalten.

Der vorhandene root-eigene `dl-infisical-env --token-pipe --config … --uid 1000
--gid 1000 -- <Binary> --config <TOML>` ist der einzige privilegierte Launcher.
Seine normale Config liegt unter root-geschützten Vorfahren und referenziert
die vorhandene Bootstrap-Credential, ohne Secretwerte in neue Dateien zu
kopieren. Er verwirft Zusatzgruppen und Umgebung vor dem exec. Die User-Units
starten das Rust-Binary direkt darüber, ohne den bisherigen ENV-Wrapper.
Metadaten für das Kind liegen neben der TOML, root-privilegierte Metadaten
separat unter `/etc/deadlock-token-launchers/`.

Für Dienste mit bestehender `NoNewPrivileges`-Grenze unterstützt der private
Launcher zusätzlich `--child-no-new-privileges`, ausschließlich zusammen mit
`--token-pipe`. Nach dem Gruppen-, GID- und UID-Wechsel setzt er vor dem exec
`PR_SET_NO_NEW_PRIVS=1`; ein Fehler verhindert den Start. Der Steam-Bot nutzt
diese Option: Der privilegierte Bootstrap benötigt `NoNewPrivileges=no` in der
User-Unit, der eigentliche App-Prozess behält dagegen `NoNewPrivs: 1`.
Dieser tatsächliche Kindstatus, die Dienstidentität und das Beenden der ganzen
Cgroup müssen vor der Live-Aktivierung gemessen werden.

Systemdienste können im privaten Launcher zusätzlich explizite
`--supplementary-group <GID>` angeben; Standard ist weiterhin die leere Liste.
Nur positive Gruppen-IDs, maximal16, sind erlaubt. Die normale Unitkonfiguration
bleibt rootkontrolliert. `--child-clear-capability-bounding-set` erfordert sowohl
`--token-pipe` als auch `--child-no-new-privileges`: vor dem UID-Wechsel wird
der BoundingSet vollständig geleert, nach dem Wechsel auch Inheritable,
Permitted und Effective; Ambient wird ebenfalls explizit geleert.
Die Twitch-Systemunit behält ihre Sandbox und NNP-Grenze. Ausschließlich der
Bootstrap benötigt SETUID/SETGID/SETPCAP/KILL; das Appkind erhält keine Caps.
SIGTERM/SIGINT werden an das eigene noch nicht reapte Kind weitergegeben.
Pipefehler oder30Sekunden-Pipetimeout brechen ausschließlich dieses Kind ab;
Reap ist auf5Sekunden begrenzt. Normaler Stop hat10Sekunden Grace, danach ist
er ausdrücklich ein Fehler. Killfehler werden nicht verschluckt. Unter dem
tatsächlich eingeschränkten Parent müssen Stop und Pipeabort vor Live geprüft
werden; reine Compiler-/Quellenabnahme behauptet diese Laufzeitbelege nicht.
