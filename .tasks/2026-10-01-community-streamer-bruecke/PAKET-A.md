# Paket A: Twitch-Verknüpfung (Deadlock-Bots)

status: umgesetzt (Branch `wip/link`)
datum: 2026-10-01
vertrag: `.tasks/2026-09-07-twitch-connection-discord-oauth/CONTRACT.md` (REQ-01 bis REQ-06)

## Was gebaut ist

### REQ-01 Tabelle

- Migration `rust/crates/dl-central-db/migrations/2026100111_discord_platform_connections.sql`
- `core.discord_platform_connections (discord_id, platform, platform_user_id, platform_login, verified, updated_at)`, PK `(discord_id, platform)`.
- Zusätzlich: Unique-Index `(platform, platform_user_id)`, damit ein Twitch-Konto nie zwei Discord-IDs gleichzeitig zugeordnet ist (wichtig für Punkte in Paket C). Die jüngste Verknüpfung gewinnt; der Schreiber entfernt die alte Zuordnung in derselben Transaktion.
- CHECKs: Twitch-ID nur Ziffern ohne führende Null, Login nicht leer, Plattformname klein.
- Einträge in `core.privacy_field_registry`, GRANT an Rolle `deadlock` (nur falls vorhanden, Muster der 202609300x-Migrationen).
- Kein FK auf `core.users`, keine Änderung an `core.steam_links`.
- Kein Rückweg unter `rollbacks/`: dort liegen nur Rückwege für nicht rein additive Migrationen (2026081301). Rückbau wäre `DROP TABLE core.discord_platform_connections` plus Löschen der drei Registry-Zeilen.

### Lesefunktionen für spätere Pakete (dl-central-db)

Datei `rust/crates/dl-central-db/src/platform_connections.rs`, re-exportiert aus `dl_central_db`:

| Funktion | Zweck |
| --- | --- |
| `upsert_twitch_connection(pool, discord_id: i64, &TwitchConnection) -> TwitchUpsertOutcome` | Schreiben (nur Dashboard). Idempotent, respektiert Privacy-Grabstein (`PrivacyOptedOut`), meldet verdrängte Discord-IDs. |
| `twitch_link_for_discord(pool, discord_id: i64) -> Option<TwitchLink>` | Verknüpfung eines Mitglieds (Concierge, Leaderboard-Anzeige). |
| `discord_ids_for_twitch_ids(pool, &[String]) -> HashMap<String, i64>` | Twitch-User-IDs zu Discord-IDs (Sync Paket C). Max 5000 IDs je Aufruf, ungültige IDs werden ignoriert. |
| `list_twitch_links(pool) -> Vec<TwitchLink>` | Alle Verknüpfungen (Broker-Endpunkt). |

Alle Lesefunktionen blenden Mitglieder mit `core.user_privacy.opted_out` oder `deleted_at` aus. `TwitchLink` trägt `discord_id, twitch_user_id, twitch_login, verified, updated_at`. Konstante `PLATFORM_TWITCH = "twitch"`, Prüfer `is_valid_twitch_user_id`.

### REQ-02 Lesepfad (dl-dashboard)

- `oauth::extract_twitch_connection` liest `type == "twitch"`, `id` als Twitch-User-ID, `name` als Login, `verified`; verifizierte Verbindung hat Vorrang. Steam-Extraktion unverändert.
- `consume-result` (delegierter Flow) mit Scope `connections`: liefert wie bisher `steam_connection_ids` und zusätzlich `twitch_link` (`linked`, `no_twitch_connection`, `privacy_opted_out`, `connections_unavailable`, `store_failed`) und `twitch_connection` (`{twitch_user_id, twitch_login, verified}` oder `null`). Ohne Twitch-Verbindung wird nichts geschrieben. Flows mit Scope `identify` sind unverändert (Test `consume_nur_identify_bleibt_unveraendert`).

### REQ-03 Button "Twitch verknüpfen"

- Verify-Panel (#deadlock-rang, `rust/bin/dl-bot/src/serversync/rang_guide_publish.rs`): neuer Button im Abschnitt "Fertig" (Schritt 3), `custom_id = twitch_link_panel:open`. Eigenes Präfix, damit die Steam-Bridge den Klick nicht an den Steam-Bot weiterreicht. Label und Hinweis sind per `assets/rang_guide_texts.toml` (`texts.twitch_hint`, `buttons.twitch_link`) überschreibbar, Default im Code.
- Klick (`rust/bin/dl-bot/src/serversync/twitch_link.rs`): dl-bot holt beim Dashboard per `POST http://127.0.0.1:<ports.dashboard>/internal/v1/discord/twitch-link/initiate` (Header `X-Internal-Token`, bestehende Token-Kette `MASTER_BROKER_TOKEN`/`MAIN_BOT_INTERNAL_TOKEN`/`TWITCH_INTERNAL_API_TOKEN`, kein neues Secret) einen frischen Discord-Link und antwortet nur für den Klickenden mit Link-Button.
- Dashboard-Endpunkt legt einen DB-State an: Flow-Typ `delegated:twitch-link`, Scope `identify connections`, Callback der bestehende `/callback/discord`, Weiterleitung danach auf `<MASTER_DASHBOARD_PUBLIC_URL>/twitch-verknuepfen/fertig` (Default `https://admin.deutsche-deadlock-community.de`).
- Im Callback verarbeitet das Dashboard für diesen Flow die Verbindungen sofort und legt im State nur das Ergebnis ab, nie das Token (INV-02, Test prüft, dass kein `access_token` in `bot.oauth_states.metadata` landet).
- Abschlussseite `GET /twitch-verknuepfen/fertig?state_id=...` löst den State genau einmal ein und zeigt in Nutzersprache: verknüpft (mit Login, HTML-escaped), kein Twitch-Konto gefunden (mit Anleitung Discord-Einstellungen, Verbindungen), abgebrochen, Link abgelaufen, Widerspruch, technischer Fehler. Nimmt nur States des Twitch-Flows an.

### REQ-04 Hinweis nach Verify

- Text im Abschnitt "Fertig" des Verify-Panels direkt über dem Button: "Optional: Wenn du auf Twitch zuschaust, verknüpf dein Twitch-Konto, dann erkennt dich der Bot in Partner-Streams." Keine DM, kein Timer.

### REQ-05 Broker-Endpunkt

- `GET /internal/master/v1/discord/twitch-links` (loopback-only, ohne Token wie `members`).
- Antwort: `{"ok":true,"links":[{"discord_id":"789","twitch_user_id":"123","twitch_login":"name","verified":true,"updated_at":"2026-10-01T20:00:00.000Z"}]}`; Fehler: 503 mit Broker-Fehler-Envelope.
- Code: Trait `dl_broker::TwitchLinkSource`, Router `dl_broker::twitch_links_router`, Handler `handlers::twitch_links`; Implementierung `CentralTwitchLinks` in `rust/bin/dl-bot/src/main.rs` (liest `list_twitch_links`).

### REQ-06 Sichtbarkeit

- Log-Zeilen im Dashboard, nur IDs: `Twitch-Verknuepfung gespeichert` (discord_id, twitch_user_id, verified, replaced_discord_ids), `Twitch-Verknuepfung fehlgeschlagen: keine Twitch-Verbindung im Discord-Profil` (discord_id), außerdem Widerspruch, Verbindungen nicht abrufbar, Speicherfehler. dl-bot loggt Linkausgabe und Fehler mit user_id. Keine Betreiber-Nachricht in Discord.

## Abweichungen vom erlaubten Änderungsbereich

- `rust/crates/dl-community/src/privacy.rs`: ein `TableSpec` für `core.discord_platform_connections` (Löschantrag entfernt die Zeile über `discord_id`) und ein Allowlist-Eintrag für `platform_user_id` (Twitch-ID, keine Discord-ID, geht mit der Zeile). Ohne beides würde ein Löschantrag die Zeilen nicht treffen und der Vertragstest `alle_migration_user_id_spalten_sind_im_privacy_vertrag` zusätzlich rot. Hinweis: dieser Test ist schon auf dem Ausgangsstand rot (Twitch-Invite-Tabellen aus 202609300x fehlen im Vertrag), das gehört nicht zu Paket A.
- `rust/bin/dl-bot/src/main.rs` und `rust/bin/dl-bot/src/serversync.rs`: nur Verdrahtung (Button-Registrierung, Broker-Router, DB-Adapter).

## Wie getestet

- `cargo test -p dl-broker`: Endpunkt-Tests (Loopback, Inhalt, leer, Fehler ohne interne Details).
- `central_test_db.sh ... cargo test -p dl-central-db -p dl-dashboard --features ...testing`:
  - `tests/platform_connections_roundtrip.rs`: Upsert idempotent, Kontowechsel, ein Twitch-Konto je Mitglied, Privacy-Grabstein beim Schreiben und Lesen, ungültige Eingaben, DB-CHECK.
  - `tests/fresh_migrations_schema.rs`: Spalten, PK, Registry-Zeilen, Unique auf Twitch-Konto, Migrator idempotent (Signatur 2026100111).
  - dl-dashboard: Extraktion (Twitch und Steam, nur Steam, keine Verbindung, Priorität), Initiate (Token, Scope, Callback, Flow-Typ), kompletter Flow gegen einen Discord-Mock (Speichern ohne Token, Bestätigungsseite, einmalig einlösbar), kein Twitch, Abbruch, fremde States, `consume-result` mit und ohne Twitch, `identify` unverändert, Texte ohne Fachwörter und Gedankenstriche.
- dl-bot: Panel-Payload (Button, Hinweis, Budgets), Klick-Handler (Link-Antwort ephemer, Fehlerantwort, Registrierung).

## Rest-Risiken und offene Punkte

- Bekannte rote Tests ohne Bezug zu Paket A: dl-dashboard-Tests mit Unix-Socket-Harness (`tests/postgres.json`, 9 Stück) laufen in dieser Umgebung nicht; dl-community `alle_migration_user_id_spalten_sind_im_privacy_vertrag` (Twitch-Invite-Tabellen) und `privacy_loeschung_funktioniert_im_turniere_runtime` (braucht DB) sind schon auf dem Ausgangsstand rot.

- Die eigentliche Steam-Verify-Erfolgsmeldung erzeugt der Steam-Bot (anderes Repo, Weg über `dl-bridges`, hier verboten). REQ-04 sitzt deshalb im Abschnitt "Fertig" des Verify-Panels direkt neben dem Button, nicht in der Steam-Bot-Antwort.
- Die Abschlussseite liegt unter `MASTER_DASHBOARD_PUBLIC_URL` (Default `admin.deutsche-deadlock-community.de`). Die Caddy-Route dorthin muss `/twitch-verknuepfen/fertig` an :8766 durchreichen; bei der Admin-Domain ist das der Fall, wenn der ganze Host auf das Dashboard zeigt. Vor dem Rollout einmal prüfen.
- Nach dem Deploy das Verify-Panel einmal neu ausspielen (`POST /serversync/rang-guide-apply`), sonst fehlt der Button (Payload-Hash ändert sich).
- Discord zeigt Bestandsnutzern beim Klick einen neuen Zustimmungsdialog (Scope `connections`), gewollt und nur am Button.
- Twitch-Bot-Seite (Relay zieht `twitch-links`, `hard`-Map in `score()`) ist eigener Vertrag im Twitch-Bot-Repo.
