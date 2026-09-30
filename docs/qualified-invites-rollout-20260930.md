# Gemeinsamer Rollout der persönlichen Einladungen

Der Auftrag vom 30. September umfasst jetzt den Live-Abschluss. Der alte Draft-Vertrag bleibt als historischer Prüfstand erhalten.

Dieser Gate-Fix basiert auf `428ec16597067c3650ea6fac2b5fae73e7ddc7aa`. Er ergänzt die Migrationen `2026092701_qualified_twitch_invites.sql`, `2026093001_twitch_effort_read_access.sql`, `2026093002_twitch_invite_qualification_readiness.sql` `2026093003_twitch_invite_identity.sql` und `2026093004_twitch_invite_mapping_history.sql`. Die Identitätsmigration ergänzt einen partiellen eindeutigen Index für nicht-null Twitch-IDs. Die Historienmigration speichert beobachtete Besitzer-/Code-/Guild-/Kanalintervalle mittels Trigger; unbekannte Altzuordnungen erhalten keine erfundene Besitzer-ID. Die vorhandenen Mappingzeilen und historischen Zuordnungen werden dabei nicht umgeschrieben.

Zusammengehörige Pakete: Deadlock-Twitch-Bot #995 bis #999, Deadlock-Bots #466 sowie caddy-config #3/#7. Die zentrale Migration `2026092701_qualified_twitch_invites.sql` muss vor dem Twitch-Punkteverbraucher und vor den persönlichen Brokerlinks laufen. Die Berechtigungs-, Readiness- und Identitätsmigrationen `2026093001` bis `2026093004` müssen ebenfalls vor den betroffenen Diensten angewendet werden.

Vor Auslieferung: unabhängige Rust-/Security-/DB-Abnahme des gesamten gekoppelten Pfads, zentrale Datenbankmigrationen auf einer Wegwerf-Datenbank sowie bestehende und neue Invite-/Broker-/Voice-Regressionen prüfen und lokales Gate freigeben lassen. Die in `qualified-invites-evidence.md` belegten Tests stammen vom ursprünglichen PR-Stand und ersetzen keine Prüfung dieses Fixes.

Reihenfolge nach Freigabe: frischen `dl-central-migrate` mit allen noch ausstehenden Migrationen bauen und anwenden; Discord-Bot und Invite-Sync aus demselben geprüften Paket ausliefern; betroffene User-Units neu starten und internen Feed samt Broker-Vertrag prüfen; Twitch-Migrationen und beide Twitch-System-Units ausliefern; Caddy-Konfiguration validieren und gezielt reloaden. Einen laufenden Uplink-Stream nicht beeinflussen. Gemeinsame Peer-Read-Pools und Datenbankrechte prüfen, ohne Zugangsdaten auszugeben.

Live-Abnahme: keine künstlichen Community-Nachrichten senden. Interne Feed-Verfügbarkeit, monotone Cursor, gesperrte anonyme Mutationen und öffentlich erreichbare Clip-/Challenges-Seiten prüfen; Datenentstehung bei echten Ereignissen passiv nachhalten. Anschließend PRs dem tatsächlichen Integrationsstand zuordnen und gesicherte, saubere Arbeitszweige/Worktrees entfernen.

Stand dieses Gate-Fixes: D1 bis D4 und die additive Identitätsmigration sind umgesetzt, lokale Cargo-Prüfungen und unabhängiger Review stehen aus. Keine Migration wurde ausgeführt; kein Build, Test, Deploy oder Neustart erfolgte, Produktionsdaten blieben unverändert. Regressionen und Gate gehören in den koordinierten Integrationslauf.

## Lokaler Migrator ohne Umgebungsvariablen

Der vorhandene Rust-Migrator unterstützt zusätzlich `--config <JSON-Datei>` im
bereits verwendeten normalen `database_url`-Format. Beispiel:
`config/central-migrate.example.json`. Dieser explizite Weg akzeptiert nur eine
benannte Datenbank und Rolle über einen absoluten lokalen PostgreSQL-Socketpfad;
Passwörter, entfernte Server und zusätzliche URL-Optionen werden abgewiesen.
Die Peer-Identität des ausführenden Betriebssystemkontos muss zur angegebenen
Datenbankrolle passen. Es werden weder neue Credentials noch Passwortdateien
angelegt. Ohne Argumente bleibt der bisherige Aufrufer kompatibel; der koordinierte
Rollout verwendet ausschließlich den expliziten Configweg.

Vor Produktivlauf wird dasselbe frisch gebaute Binary mit einer separaten normalen
Config gegen eine isolierte Datenbank ausgeführt. Erst nach unabhängiger Abnahme
und Gate erfolgt der Lauf gegen `deadlock`, unter der dafür vorhandenen lokalen
Administratoridentität. Die Startwrapper im gemeinsamen Checkout bleiben bestehen.
