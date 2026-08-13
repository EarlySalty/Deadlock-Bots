# Rückwege (nicht von sqlx verwaltet)

Dateien hier liegen bewusst **nicht** unter `migrations/`: sqlx würde sie sonst
als Migration anwenden. Sie werden von Hand gefahren und von
`tests/fresh_migrations_schema.rs` mitgefahren — ein Rückweg, der nie läuft, ist
nicht getestet, sondern geraten.

## 2026081301 — Linked-Role-Provider-Dimension

**Zustand:** am 2026-08-13 auf der zentralen Produktions-DB angewendet
(`_sqlx_migrations` trägt Version `2026081301`).

**Die Migrationsdatei ist damit eingefroren.** Jede Änderung an
`migrations/2026081301_discord_role_connection_provider.sql` — auch ein
Kommentar — ändert die Prüfsumme, und der nächste `dl-central-migrate` bricht
mit `VersionMismatch` ab. Er läuft bei jedem Deploy dieses Repos. Korrekturen
gehören in eine neue Migration, nicht in diese Datei.

### Reihenfolge beim Ausrollen (nicht vertauschbar)

1. Caddy-Block `@linked_roles` installieren und reloaden
   (`Caddy/hosts/v50671/Caddyfile`). Fehlt er, läuft Discords Callback in
   einen 404.
2. `dl-central-migrate` gegen die zentrale DB.
3. Erst danach das Website-Binary tauschen und den Dienst neu starten.

Beide Richtungen sind eng: das Backend prüft die `provider`-Spalte beim Start
und bricht ohne sie ab (sonst schriebe es stumm falsche Zeilen). Umgekehrt läuft
das **alte** Binary nach der Migration nicht mehr — `ON CONFLICT (discord_id)`
findet keinen passenden Index und wirft SQLSTATE 42P10. Ein Rollback per
Binary-Swap allein reicht deshalb nicht; dafür ist die SQL-Datei hier.

### Rückweg fahren

```bash
psql "$DEADLOCK_CENTRAL_DSN" -f 2026081301_discord_role_connection_provider_rollback.sql
```

Er ist mehrfach ausführbar, löscht alle Creator-OAuth-Tokens aus den
Live-Tabellen und sichert sie vorher nach `core.*_rollback_backup`. Die Zeile in
`_sqlx_migrations` bleibt absichtlich stehen — ohne sie wendet der nächste
Migrator-Lauf die Migration erneut an und das zurückgerollte Binary ist ohne
einen einzigen Logeintrag wieder tot. Einzelheiten und die drei Dinge, die
zurückgebaut werden, stehen im Kopf der SQL-Datei.

Die Sicherungskopien tragen `rollback_expires_at` (180 Tage) und werden vom
Retention-Job in `dl-community/src/privacy.rs` geräumt. Der Name ist nicht
`expires_at`: den gibt es in der Tokens-Tabelle schon, und er ist der OAuth-
Ablauf des Tokens.
