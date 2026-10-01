# Paket F: Streamer vorschlagen (Discord-Seite) und Concierge-Anschluss an Paket A

status: gebaut (Branch `wip/suggest`, auf A, D, G)
datum: 2026-10-01
plan: `PLAN.md` (Abschnitt "Streamer-Vorschlag (Paket F)", Leitplanken)
gegenstück: Twitch-Seite im Repo Deadlock-Twitch-Bot (`tb-internal-api/src/handlers/scout_community.rs`, `tb-scout/src/community.rs`)

## Was gebaut ist

### Knopf und Fenster

- Ort: das vorhandene Clip-Panel im Clip-Kanal `1425215762460835931` (`clips::SUBMIT_CHANNEL_ID`, Konstante `streamer_suggest::PANEL_CHANNEL_ID`). Zweiter Knopf neben "Clip einsenden", dazu ein Satz unter den Clip-Regeln (`PANEL_HINT`).
  - Warum dort: Es ist das schlichteste vorhandene Muster im Community-Bereich. Das Clip-Panel wird ohnehin alle 5 Minuten per `upsert_interface` aktualisiert, es braucht also keinen neuen Publisher, keinen Payload-Hash und kein manuelles Neuausspielen. Ein eigener Vorschlags-Kanal existiert nicht; der Server-Bot-Fragen-Kanal ist für Fragen an Menschen gedacht.
  - Nach dem Deploy erscheint der Knopf spätestens nach 5 Minuten.
- custom_ids: `streamer_suggest_btn_v1` (öffnet das Fenster), `streamer_suggest_modal_v1` (Absenden). Felder `twitch_channel` (Pflicht, bis 100 Zeichen) und `reason` (Pflicht, 3 bis 300 Zeichen).
- Eingabe: `name`, `@name` oder Link `twitch.tv/name` (mit `https://`, `www.`, `m.`, Pfad und Query werden abgeschnitten). Ergebnis klein, `a-z0-9_`, 1 bis 25 Zeichen, gleiche Regel wie `normalisiere_vorschlag_login` im Twitch-Bot. Fremde Links (YouTube, `clips.twitch.tv`) und twitch.tv-Seiten wie `directory` oder `videos` gelten nicht als Kanal.

### Speicherung (Migration `2026100104_streamer_suggestions.sql`, additiv)

`community.streamer_suggestions`:

| Spalte | Inhalt |
| --- | --- |
| `id` | BIGSERIAL, Grundlage des Idempotency-Keys |
| `discord_id` | Vorschlagendes Mitglied |
| `twitch_login` | normalisierter Login |
| `twitch_user_id` | aus der Antwort des Twitch-Bots, sonst NULL |
| `reason` | Grund, bereinigt, höchstens 300 Zeichen (DB-Grenze 500) |
| `status` | `pending`, `created`, `already_known`, `already_partner`, `blocked`, `not_found`, `rejected` |
| `forward_attempts`, `last_attempt_at` | Retry-Buchhaltung |
| `created_at`, `forwarded_at` | `forwarded_at` NULL genau dann, wenn `status = 'pending'` (CHECK) |

- Unique `(discord_id, twitch_login)`: ein Vorschlag je Mitglied und Kanal. Andere Mitglieder dürfen denselben Kanal vorschlagen.
- Tageslimit: höchstens 3 neue Vorschläge je Mitglied in 24 Stunden (rollierend, `DAILY_LIMIT`). Prüfung und Einfügen laufen in einer Transaktion unter `lock_user_privacy_and_is_opted_out`; die Sperre serialisiert auch gleichzeitige Klicks.
- Widerspruch (`core.user_privacy.opted_out` oder `deleted_at`): nichts wird gespeichert; offene Weitergaben solcher Mitglieder schickt der Retry-Loop nicht mehr.
- Registry-Zeilen in `core.privacy_field_registry` (`discord_id`, `reason`, `twitch_user_id`, alle `delete_row_on_user_delete`), GRANT an `deadlock` auf Tabelle und Sequenz, Muster wie 2026100101.
- `privacy.rs`: `TableSpec` über `discord_id` (Löschantrag löscht die Zeilen), Allowlist für `twitch_user_id` (Twitch-ID des vorgeschlagenen Kanals, nicht des Mitglieds).
- Kein Rückweg unter `rollbacks/` (rein additiv). Rückbau: `DROP TABLE community.streamer_suggestions` und die drei Registry-Zeilen löschen.

### Weitergabe an den Twitch-Bot

- Client: vorhandener `dl_bridges::twitch::TwitchApiClient` (in dl-bot `TwitchApiClient::from_env(operating_value)`, also `runtime.bridges.twitch_api_url` bzw. Host/Port und `TWITCH_INTERNAL_API_TOKEN` aus dem Infisical-Bootstrap). Neue Methode `post_community_suggestion` liefert HTTP-Status und Body.
- `POST /internal/twitch/v1/scout/community-suggestion`, Header `X-Internal-Token` und `Idempotency-Key`, Body:
  `{"twitch_login","suggested_by_discord_id","reason","idempotency_key":"discord-suggest-<id>"}`.
- Einordnung (`classify_response`): 2xx mit bekanntem Status → gespeichert, `forwarded_at = now()`; 400/409/422 → `rejected`, endgültig (Log-Warnung); alles andere, Zeitüberschreitung oder 2xx ohne bekannten Status → bleibt `pending`.
- Sofortversuch beim Absenden (höchstens 8 Sekunden; Discord-Antwort wird nach 2 Sekunden automatisch verzögert). Danach Retry-Loop `streamer_suggest::spawn` (gateway-gated wie die Clip-Loops, alle 2 Minuten): fällig ist ein offener Vorschlag nach 5 Minuten mal 2 hoch Versuche, höchstens 6 Stunden; Claim per `FOR UPDATE SKIP LOCKED`. Kein Aufgeben: ein Vorschlag bleibt offen, bis der Twitch-Bot antwortet. Immer derselbe Schlüssel, der Twitch-Bot liefert bei Wiederholung den damals vergebenen Status.
- Ohne Twitch-Token startet dl-bot ohne Weitergabe; Vorschläge werden gespeichert und nach einem Start mit Token nachgereicht.

### Antworten (ephemer)

| Stand | Text (gekürzt) |
| --- | --- |
| `created` | Danke, wir schauen uns den Kanal an, Team entscheidet; wird er Partner, gibt es 150 Punkte |
| `pending` / `rejected` | Danke, wir schauen uns den Kanal an, Team entscheidet |
| `already_known` | Den Kanal kennen wir schon, steht bereits auf unserer Liste |
| `already_partner` | Ist schon Partner, Verweis auf den Partner-Stream-Kanal |
| `not_found` | Finden wir nicht, Schreibweise prüfen, Link aus der Adresszeile kopieren |
| `blocked` | Können wir leider nicht aufnehmen (ohne Details, ohne Kanalnamen) |
| eigener Doppelvorschlag | Hast du schon vorgeschlagen (bei endgültigem Stand dessen Text) |
| Limit, Widerspruch, ungültige Eingabe, Grund fehlt, DB-Fehler | eigene kurze Texte |

## Punkte (Paket C, hier nicht vergeben)

Regel aus PLAN.md: Wird ein vorgeschlagener Kanal Partner, bekommt der erste Vorschlagende je Kanal 150 Punkte. Maßgeblich ist der Outcomes-Endpunkt des Twitch-Bots (`GET /internal/twitch/v1/scout/community-suggestions/outcomes`, Felder `suggested_by_discord_id`, `is_partner_active`, `partner_since`); der Twitch-Bot führt den ersten Vorschlagenden je Twitch-User-ID.

Abgleich auf Discord-Seite (frühestes `created_at` je Kanal, nur Vorschläge, die der Twitch-Bot angenommen hat):

```sql
SELECT DISTINCT ON (twitch_user_id) twitch_user_id, discord_id, created_at
  FROM community.streamer_suggestions
 WHERE twitch_user_id IS NOT NULL
   AND status IN ('created', 'already_known')
 ORDER BY twitch_user_id, created_at, id;
```

Hinweise für C: Weicht das Ergebnis vom Outcomes-Endpunkt ab (zum Beispiel weil eine spätere Weitergabe früher ankam als eine wiederholte), gilt der Twitch-Bot. Gelöschte Mitglieder fehlen hier; ihre Discord-ID kann im Twitch-Bot noch stehen, Paket C muss `core.user_privacy` beim Gutschreiben prüfen. Die Punkte gehören in den Ledger aus C mit einem Schlüssel je Twitch-User-ID, damit sie nur einmal fließen.

## Concierge-Anschluss an Paket A

- `rust/bin/dl-bot/src/serversync/twitch_link.rs`: `ConciergeTwitchLink` implementiert `dl_community::concierge_community::TwitchLinkSource` über dieselbe `TwitchLinkUrlSource` wie der Verify-Knopf (`DashboardTwitchLinkClient`, `POST /internal/v1/discord/twitch-link/initiate`). Kein zweiter Client, kein doppelter Code.
- `serversync::register_twitch_link_components` registriert den Verify-Knopf wie bisher und gibt die Concierge-Quelle zurück; `main.rs` installiert sie direkt nach `Concierge::with_answers(...)` per `install_twitch_link_source`.
- Ergebnis: Der Concierge-Knopf "Twitch verknüpfen" liefert den echten persönlichen Link (ephemer, Link-Button "Jetzt mit Twitch verknüpfen"). Bei jedem Fehler bleibt der feste Verweis auf `<#1398021105339334666>`.
- Concierge-Texte: `ANSWER_STREAMER_VORSCHLAGEN` und `ANSWER_STREAMER_VORTEILE` nennen jetzt `<#1425215762460835931>` als Ort des Knopfs, die Vorschlags-Antwort nennt das Limit von 3 am Tag. Konstante `concierge_community::STREAMER_SUGGEST_CHANNEL_ID`; ein Test hält sie gleich mit `streamer_suggest::PANEL_CHANNEL_ID` (das Concierge-Modul bleibt ohne Abhängigkeit auf andere Module).

## Dateien

| Datei | Inhalt |
| --- | --- |
| `rust/crates/dl-community/src/streamer_suggest.rs` | Parsing, Texte, Speicher, Weitergabe, Retry-Loop, Handler |
| `rust/crates/dl-community/src/streamer_suggest_tests.rs` | Unit- und DB-Tests |
| `rust/crates/dl-community/src/clips.rs` | Zweiter Knopf und Hinweis im Clip-Panel (`interface_components`) |
| `rust/crates/dl-community/src/concierge_community.rs` | Kanal-Konstante, Texte |
| `rust/crates/dl-community/src/privacy.rs` | Löschvertrag |
| `rust/crates/dl-bridges/src/twitch.rs` | `post_community_suggestion` |
| `rust/crates/dl-central-db/migrations/2026100104_streamer_suggestions.sql` | Migration |
| `rust/crates/dl-central-db/tests/fresh_migrations_schema.rs` | Schema, Registry, CHECKs, Migrator idempotent |
| `rust/bin/dl-bot/src/serversync/twitch_link.rs`, `serversync.rs`, `main.rs` | Concierge-Quelle, Verdrahtung |
| `docs/community-punkte-und-streamer.md` | Abschnitt Streamer vorschlagen |

## Tests

- `cargo test -p dl-community --lib streamer_suggest`: Login- und Link-Parsing, ungültige Eingaben, Grund, Idempotency-Key und Payload, Einordnung aller Antworten, Texte (keine Gedankenstriche, keine Fachwörter), jeder Status hat seinen Text, `blocked` ohne Details, Modal-Grenzen, Clip-Panel-Knopf, Kanal gleich Concierge, Handler (Modal, ungültige Eingabe ohne DB).
- DB (`scripts/central_test_db.sh cargo test -p dl-community --features testing --lib streamer_suggest`): speichern und weitergeben, ein Vorschlag je Mitglied und Kanal, Limit 3 in 24 Stunden, Widerspruch, Retry mit Backoff bis zur Antwort (gleicher Schlüssel), endgültige Ablehnung, Start ohne Twitch-Anbindung und späteres Nachholen, erster Vorschlagender je Kanal.
- `cargo test -p dl-bridges --lib twitch::`: Methode reicht Token, Schlüssel, Body, Status durch.
- `cargo test -p dl-bot --bin dl-bot twitch_link`: Concierge-Quelle liefert denselben Link, Fehler ergibt `None`.
- Schema: `scripts/central_test_db.sh cargo test -p dl-central-db --features testing --test fresh_migrations_schema -- --ignored`.
- Concierge, Clips, Clip-Contest, Privacy mit DB: grün bis auf den bekannten Vertragstest `alle_migration_user_id_spalten_sind_im_privacy_vertrag` (Twitch-Invite-Tabellen; die neuen Spalten fehlen dort nicht).

## Rest-Risiken und offene Punkte

- Ort des Knopfs ist das Clip-Panel. Soll es einen eigenen Kanal bekommen, reicht ein eigenes Panel mit `streamer_suggest::panel_button()` und das Ändern der beiden Kanal-Konstanten (Test hält sie gleich).
- Löschantrag entfernt die Zeilen hier; die Discord-ID im Twitch-Bot (`twitch_scout_candidates.suggested_by_discord_id`) wird dadurch nicht gelöscht. Gehört zum Löschvertrag der Twitch-Seite.
- `rejected` (400/409/422) wird nur geloggt. Solche Zeilen deuten auf einen Vertragsbruch hin und sollten vom Betrieb angeschaut werden: `SELECT * FROM community.streamer_suggestions WHERE status = 'rejected'`.
- Der Twitch-Bot muss den Endpunkt vor oder mit diesem Stand ausrollen. Fehlt er (404), bleiben Vorschläge `pending` und werden höchstens alle 6 Stunden erneut versucht.
- Nach dem Deploy ändert sich das Clip-Panel einmal (neuer Knopf, neuer Hinweis); das passiert automatisch beim nächsten 5-Minuten-Refresh.
