# Begrenzte Steam-Migration

`dl-central-migrate --steam-credentials-only --config <Bot-TOML>`
ergänzt in einer bestehenden zentralen Datenbank ausschließlich die beiden
Versionen `20260930220100` und `20260930220400`. Sie legen die verschlüsselte
Steam-Zugangsablage und deren Refresh-Revision an.

Dieser operative Modus verlangt eine bereits kanonische Migrationshistorie.
Er repariert keine fremden Prüfsummen. Eine bekannte transiente Scrim-Prüfsumme
führt deshalb ausdrücklich zum Abbruch. Dieser Abbruch ist die vorgesehene
Grenze des Modus. Der allgemeine Migrator behält seinen vorhandenen
Kompatibilitätspfad; seine Ausführung wäre ein separater Auftrag mit größerem
Umfang.

Alle angewandten Versionen bleiben in der ausgewählten Liste. SQLx prüft ihre
Prüfsummen und lehnt unbekannte oder fehlerhafte Historie weiterhin ab.
`ignore_missing` bleibt `false`, `locking` bleibt `true`. Der begrenzte Modus
führt den historischen manuellen Scrim-Ledgerpatch nicht aus. Der offene
Tokenhash-Cutover und offene Browsermigrationen werden nicht aufgenommen.

Beide Modi laden die normale Bot-TOML über den vorhandenen gemeinsamen Lader.
Der zentrale Datenbankzugang kommt ausschließlich aus dem privaten FD3-Snapshot
des bestehenden Infisical-Launchers. Der begrenzte Modus verlangt einen
expliziten TOML-Pfad. Der normale Modus akzeptiert denselben `--config`-Pfad
oder die bestehende Standarddatei `config/bot.toml`.

Der operative Start erfolgt über denselben Rust-FD3-Launcher wie beim Bot.
Die bereits vorhandenen Bootstrapmetadaten liegen neben der Bot-TOML;
es gibt keine zusätzliche Zugangsdatei. Ohne gültige TOML, private FIFO oder
zentralen Snapshotwert bricht der Start vor der Datenbankverbindung ab.

## Geprüfter Live-Vertrag am 2. Oktober 2026

In `deadlock` fehlte `steam.account_credentials`. Der zentrale Pool verband
sich, der Core brach beim ersten Zugriff auf die fehlende Tabelle ab. Die
vorhandene Historie umfasste 111 erfolgreiche Einträge. Ihre Versionen lagen
alle vor den beiden erforderlichen Steam-Migrationen. Damit prüft SQLx diese
gesamte Historie vor der neuen Steam-DDL.

Die read-only Abfrage der Scrim-Version `2026071602` ergab bereits die
kanonische SHA384-Prüfsumme:

```
423022abe243dbe00f81ac78fa7b159d842b5257cd38d67abf1fc4b432c00a471645a5fcb60d9fcfd301c74d625ddd78
```

Die bekannte transiente Variante lag nicht vor. Der unabhängige DB-Prüfer
bestätigte diesen Befund sowie Schema- und Rollenrechte. Die beiden additiven
Steam-Versionen benötigen keine weiteren Migrationen oder Datenübernahme.
Der Apply und die anschließende Live-Abnahme bleiben getrennte Schritte.
