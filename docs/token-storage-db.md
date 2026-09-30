# Datenbankgebundene Tokenablage

Zentrale additive Migrationen für gehashte Turnier-/Steam-Bearer sowie verschlüsselte Steam- und Browser-Zugänge. Die alte Node-Steam-Brücke bricht ausdrücklich ab, statt einen Dateispeicher wieder einzuschalten.

## Verbindliche Speichergrenze

Kontozugänge und wieder benötigte Geheimwerte werden nur feldverschlüsselt in der Datenbank gespeichert. Für reine Bearer-Prüfungen wird der Rohwert nicht zurückgewonnen, sondern ein irreversibler Lookup-Schlüssel verglichen. Die vorhandene tb-crypto-Implementierung bleibt die gemeinsame Feldkrypto. Konto- und Feldkontext müssen authentifiziert sein.

Keine neuen Token-JSONs, Refresh-Dateien, dotenv-Zugänge oder persistenten Browserprofile. Kein stiller Datei-, Klartext- oder Fremdkonto-Fallback. Keine Tokenwerte, verschlüsselten Blobs, Session-URIs oder Browserzustände in Logs, Berichten oder Shellargumenten.

Master-Key, OAuth-Anwendungssecrets, Datenbankverbindung und Infisical-Bootstrap bleiben außerhalb der Anwendungstabellen im bestehenden Secret-Manager. Sie sind keine zweite Ablage der rotierenden Kontotokens. Ein Verschlüsselungsschlüssel wird nicht neben seine eigenen Ciphertexte gelegt. TradingBot gehört nicht zu dieser Umstellung.

## Lokaler Arbeitsstand

Die zusammengehörigen Worktrees liegen als Geschwister unter /home/nathanael/.worktrees/token-db-local-20260930/. Die vorhandenen relativen Abhängigkeiten auf Deadlock-Bots und tb-crypto werden dort wiederverwendet. Quellkopien, ein zweites Kryptopaket und Änderungen an geteilten Checkouts sind nicht nötig. Alle Rust-Prüfungen verwenden den vorhandenen sccache und die gemeinsame Zwischenablage /home/nathanael/.cache/rust-build/{workspace-path-hash}.

Produktive Datenbanken und Konten wurden für diesen Arbeitsstand nicht migriert. Änderungen sind keine Aussage über die laufenden Dienste. Tests dürfen nur explizite Wegwerf-Datenbanken und synthetische Konten verwenden.

## Späterer koordinierter Cutover

1. Zugehörige neue Leser und Writer gemeinsam bereitstellen, vorhandene verschlüsselte Sicherung und Wiederherstellungsweg prüfen. Keine angewandte Migration ändern.
2. Alte Writer anhalten. Neue zentrale Schema-Migrationen anwenden. Gehashte Session-IDs nicht mit einem alten Consumer mischen. Bestehende Restore-/ETL-Werkzeuge vor einem Import auf das neue Tokenformat abstimmen; die Constraints lehnen Rohwerte ab.
3. Bestehende Steam-Guard-Werte ausdrücklich per privater Pipe an das Beispielprogramm import_guard im Steam-Core geben. Es nutzt dieselbe Kontokonfiguration und denselben Secret-Launcher wie der Dienst. Gleiche Freigaben dürfen erneut importiert werden, andere bestehende Werte und Widerrufe werden nicht überschrieben. Keine alten Dateien durch den Agenten öffnen.
4. VOD-Resume-Werte vor dem Start mit dem Beispiel migrate_resume_sessions im Twitch-VOD-Paket und beim separaten Archiv mit --migrate-token-storage transaktional umstellen. Danach den NOT-VALID-Constraint der Twitch-Tabelle validieren. Ein Fehler lässt den jeweiligen Migrationsbestand unverändert. Abgeschlossene Uploads behalten ihre Video-ID.
5. Kontozuordnung, Entschlüsselung und Neustart-Wiederaufnahme prüfen. Erst danach alte Credential-Dateien oder Bootstrap-Kontotokens kontrolliert außer Betrieb nehmen. Die vorhandenen Dateien werden hier weder gelöscht noch als Backup verdoppelt.

Ein Code-Rollback allein reicht nach einem irreversiblen Hash-Cutover nicht. Entweder die neuen Lookup-Verträge beibehalten oder gemeinsam auf einen zuvor geprüften Datenbankstand zurückgehen. Ein nicht durchgeführter Restore-Test ist keine bestätigte Rollback-Fähigkeit.
