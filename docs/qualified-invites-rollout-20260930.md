# Gemeinsamer Rollout der persönlichen Einladungen

Der Auftrag vom 30. September umfasst jetzt den Live-Abschluss. Der alte Draft-Vertrag bleibt als historischer Prüfstand erhalten.

Diese frische Integration enthält nur die drei Eigencommits von PR #466 auf `42175e5f` und die additive Migration `2026093001_twitch_effort_read_access.sql`. Die Migration gibt den vorhandenen Twitch-Leserollen ausschließlich SELECT auf die tatsächlich verwendeten zentralen Quellen und USAGE auf deren Schemas. Sie vergibt keine Schreibrechte und legt keine Secrets an.

Zusammengehörige Pakete: Deadlock-Twitch-Bot #995 bis #999, Deadlock-Bots #466 sowie caddy-config #3/#7. Die zentrale Migration `2026092701_qualified_twitch_invites.sql` muss vor dem Twitch-Punkteverbraucher und vor den persönlichen Brokerlinks laufen.

Vor Auslieferung: unabhängige Rust-/Security-/DB-Abnahme des gesamten gekoppelten Pfads, zentrale Datenbankmigrationen auf einer Wegwerf-Datenbank sowie bestehende Invite-/Broker-/Voice-Tests prüfen und lokales Gate freigeben lassen. Die in `qualified-invites-evidence.md` belegten Tests stammen vom ursprünglichen PR-Stand und ersetzen keine Prüfung dieses Integrationsstands.

Reihenfolge nach Freigabe: frischen `dl-central-migrate` mit beiden Migrationen bauen und anwenden; Discord-Bot und Invite-Sync aus demselben geprüften Paket ausliefern; betroffene User-Units neu starten und internen Feed samt Broker-Vertrag prüfen; Twitch-Migrationen und beide Twitch-System-Units ausliefern; Caddy-Konfiguration validieren und gezielt reloaden. Einen laufenden Uplink-Stream nicht beeinflussen. Gemeinsame Peer-Read-Pools und Datenbankrechte prüfen, ohne Zugangsdaten auszugeben.

Live-Abnahme: keine künstlichen Community-Nachrichten senden. Interne Feed-Verfügbarkeit, monotone Cursor, gesperrte anonyme Mutationen und öffentlich erreichbare Clip-/Challenges-Seiten prüfen; Datenentstehung bei echten Ereignissen passiv nachhalten. Anschließend PRs dem tatsächlichen Integrationsstand zuordnen und gesicherte, saubere Arbeitszweige/Worktrees entfernen.

Stand dieser Übergabe: Migration ergänzt, kein Build/Deploy/Neustart, keine Produktionsdaten geändert. Caddy-Paket separat mit 15/15 Routentests und erfolgreichem vollständigem `caddy adapt` geprüft. Rust-Prüfungen und Gate gehören in den koordinierten Integrationslauf.
