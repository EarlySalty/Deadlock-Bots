# Paket D: Clip-Contest mit Community-Voting

status: gebaut (Branch `wip/clips`)
datum: 2026-10-01
plan: `PLAN.md` (Abschnitt "Clip-Einreichung aus Twitch", Punkteregeln Clip-Contest)

## Was gebaut ist

- Wöchentlicher Contest auf dem bestehenden Wochenfenster (`clips.rs`, Sonntag 00:00 bis Samstag 23:00 Europe/Berlin). Einsendung per Button/Modal unverändert, custom_ids `clip_submit_btn_v1`, `clip_perm_yes_v1`, `clip_submit_modal_v1` unverändert.
- Nach Fensterende: Stimmzettel einfrieren, genau ein Voting-Post im Clip-Kanal (Embed mit nummerierten Clips und Credit, String-Select zum Abstimmen, Button "Meine Stimme anzeigen"), 48 Stunden offen.
- Nach Voting-Ende: Auszählung, Top 3 in `clips.clip_contest_results`, Ergebnis-Post im Kanal, Auswahl am Voting-Post entfernt, Top 3 mit Links und Credits als DM an den Kurator (Fallback Clip-Kanal, gleicher Weg wie der TXT-Dump). Der TXT-Dump bleibt und enthält jetzt auch Twitch-Einsendungen.
- Broker-Endpunkt `POST /internal/master/v1/clips/submit` für Paket E.
- Dump robuster: nicht mehr nur in der Stunde Samstag 23:00 bis 00:00, sondern jedes abgelaufene, nicht gedumpte Fenster der letzten 7 Tage.

Code:

| Datei | Inhalt |
| --- | --- |
| `rust/crates/dl-community/src/clip_contest.rs` | Reine Logik, Store, Scheduler-Schritte, Vote-Handler, Twitch-Einsendung |
| `rust/crates/dl-community/src/clip_contest_tests.rs` | Unit- und DB-Tests |
| `rust/crates/dl-community/src/clips.rs` | Port erweitert, Dump fensterbasiert, Scheduler ruft `process_contest` |
| `rust/crates/dl-broker/src/clips.rs` (+ `clips_tests.rs`) | Route, Formprüfung, Port-Trait |
| `rust/bin/dl-bot/src/modglue.rs` | `ClipGlue` (neue Port-Methoden), `ClipSubmitGlue` (Broker-Port) |
| `rust/bin/dl-bot/src/main.rs` | Broker-Router um `dl_broker::clips::router` ergänzt |
| `rust/crates/dl-central-db/migrations/2026100112_clip_contest_voting.sql` | Migration |
| `rust/crates/dl-community/src/privacy.rs` | Löschvertrag für neue Spalten |

## Ablauf und Zustände

Scheduler: bestehender Fenster-Loop (alle 2 Minuten) ruft `process_window` (Dump) und `process_contest`. Der Zustand steht ausschließlich in `clips.clip_votings.status`:

1. Fenster abgelaufen (höchstens 36 Stunden her) und noch kein Voting: Zeile anlegen und Stimmzettel in `clip_voting_entries` einfrieren, in einer Transaktion, `ON CONFLICT DO NOTHING`. Keine gültigen Clips: `skipped`.
2. `pending` → `publishing`: Claim mit Zeitstempel. Erster Versuch: geplante Zeiten speichern, dann posten (Discord-Nonce mit `enforce_nonce`). Wiederholung nach Abbruch: vorher unter den letzten 50 eigenen Nachrichten im Kanal nach dem Footer `Clip-Contest · Runde <window_id>` suchen und eine gefundene Nachricht übernehmen. Danach `open` mit `message_id`, `voting_start_at`, `voting_end_at`. Ein hängender Claim wird nach 10 Minuten neu versucht.
3. `open`, Ende erreicht: Auszählen unter exklusiver Zeilensperre, Top 3 schreiben (`ON CONFLICT DO NOTHING`), `closing`. Stimmen nehmen eine geteilte Sperre und prüfen Status und Ende in derselben Transaktion; nach dem Auszählen rutscht keine Stimme mehr durch.
4. `closing`: Ergebnis-Post (Claim; bei Wiederholung Footer-Suche `Clip-Contest · Ergebnis Runde <window_id>`), Voting-Post ohne Auswahl editieren (kosmetisch), Kurator-DM (Claim, danach `curator_dm_sent_at`). Beides erledigt → `closed`.

Stimmrecht: eine Stimme je Mitglied und Woche (PK `window_id, voter_user_id`), änderbar bis Ende. Nicht für den eigenen Clip: Discord-Einsender per `user_id`, Twitch-Clips per verknüpftem Twitch-Konto aus `core.discord_platform_connections` (Paket A; fehlt die Tabelle, wird nur die Discord-Regel geprüft). Mindestalter der Mitgliedschaft 3 Tage über `joined_at` aus dem Gateway-Cache; ist das Mitglied nicht im Cache, wird nicht blockiert.

Ranking: Stimmen absteigend, Gleichstand frühere Einsendung (`created_at`, dann kleinere ID). Ein Platz braucht mindestens 1 Stimme. Gleicher Clip mehrfach eingereicht: auf dem Stimmzettel nur die früheste Einsendung (Schlüssel: Twitch-Slug bzw. normalisierte URL). Mehr als 25 Clips: die ersten 25 nach Einsendezeit.

## Tabellen (Migration `2026100112_clip_contest_voting.sql`, additiv)

- `clips.clip_submissions`: `user_id` jetzt nullable; neu `source` (`discord`|`twitch`, Default `discord`), `streamer_twitch_user_id`, `streamer_login`, `submitted_by_twitch_user_id`, `title`, `idempotency_key` (unique, partiell). CHECK: Twitch-Zeilen haben `user_id IS NULL`, gültige Twitch-ID und einen Schlüssel.
- `clips.clip_window_submissions.user_id` nullable (Twitch-Einsendungen).
- `clips.clip_votings`: ein Datensatz je Fenster, Status, Message-IDs, Voting-Start/-Ende, Claims.
- `clips.clip_voting_entries`: eingefrorener Stimmzettel (Position 1..25).
- `clips.clip_votes`: `(window_id, voter_user_id)` → `submission_id`, `created_at`, `updated_at`.
- `clips.clip_contest_results`: `(window_id, place)`, `guild_id`, `week_start_at`, `week_end_at`, `submission_id`, `source`, `user_id` (Discord-Einsender) oder `streamer_twitch_user_id` + `streamer_login`, `votes`, `decided_at`.

Für Paket C:

```sql
-- Plätze (100/60/40)
SELECT window_id, place, source, user_id, streamer_twitch_user_id, week_start_at, decided_at
  FROM clips.clip_contest_results;
-- Stimmen (2 Punkte je Wähler und Woche), erst nach Abschluss
SELECT v.window_id, v.voter_user_id, w.start_at AS week_start_at, cv.closed_at
  FROM clips.clip_votes v
  JOIN clips.clip_votings cv ON cv.window_id = v.window_id AND cv.status = 'closed'
  JOIN clips.clip_windows w ON w.id = v.window_id;
```

Rollback: kein Skript unter `rollbacks/`. Das Repo führt dort nur Rückwege für riskante Schlüsseländerungen; diese Migration fügt nur hinzu. Ein Rückbau wäre `DROP TABLE` der vier neuen Tabellen und `DROP COLUMN` der neuen Spalten; `user_id SET NOT NULL` nur, wenn keine Twitch-Zeilen existieren.

## Endpunkt `POST /internal/master/v1/clips/submit`

Auth wie alle Broker-Routen: nur Loopback, `X-Internal-Token`. Body höchstens 8 KiB, unbekannte Felder werden abgelehnt.

```json
{"source":"twitch","clip_url":"https://clips.twitch.tv/...","streamer_twitch_user_id":"456","streamer_login":"name","submitted_by_twitch_user_id":"456","title":"...","idempotency_key":"twitch-clip-<clip_id>"}
```

Pflicht: `source` (nur `twitch`), `clip_url`, `streamer_twitch_user_id`, `streamer_login`, `idempotency_key` (1 bis 128 sichtbare ASCII-Zeichen). Optional: `submitted_by_twitch_user_id`, `title` (bis 200 Zeichen). Login wird klein geschrieben.

Antwort (Broker-Envelope, HTTP 200):

```json
{"ok":true,"request_id":"...","idempotency_key":"twitch-clip-Abc","cached":false,
 "result":{"status":"accepted","submission_id":123,"reason":null},"error":null}
```

- `accepted`: gespeichert. Identischer `idempotency_key`, Clip und Metadaten liefern erneut `accepted` mit derselben ID.
- `duplicate`: derselbe Clip steht schon im Wochenfenster (auch per Discord eingereicht), mit `submission_id` und `reason` `duplicate_clip_this_week`. Beim Replay desselben Schlüssels und Clips mit abweichenden Metadaten bleibt der erste Datensatz unverändert und die bestehende `submission_id` kommt mit `reason` `idempotency_metadata_drift` zurück.
- `rejected`: `reason` `invalid_clip_url` (nur `https://clips.twitch.tv/<slug>` und `https://(www.|m.)twitch.tv/<kanal>/clip/<slug>`), `not_partner`, `idempotency_conflict` (Schlüssel schon für anderen Clip benutzt).
- Formfehler 400 `bad_request`, zu groß 413, DB nicht erreichbar 503 `unavailable`, Auth 401/403.

Zuordnung: laufendes Wochenfenster; in der Stunde Samstag 23:00 bis Sonntag 00:00 das nächste. Gespeichert wird die kanonische `https://clips.twitch.tv/<slug>`-URL, Credit = Streamer-Login, `user_id` NULL. Partnerprüfung: Zeile in `bot.twitch_streamer_invites` mit passender `twitch_user_id` (ersatzweise Login, solange die ID fehlt).

## Konstanten (im Code, keine ENV)

| Konstante | Wert |
| --- | --- |
| Clip-Kanal (Einsendung, Voting, Ergebnis) | `1425215762460835931` |
| Kurator (Dump und Top 3) | `388772056717590539` |
| Guild | `1289721245281292288` |
| `VOTING_DURATION_HOURS` | 48 |
| `MAX_BALLOT_ENTRIES` | 25 |
| `MIN_MEMBERSHIP_DAYS` | 3 |
| `MIN_VOTES_FOR_PLACE` | 1 |
| `VOTING_CATCHUP_HOURS` | 36 |
| `DUMP_CATCHUP_DAYS` | 7 |
| `STALE_CLAIM_MINUTES` | 10 |
| custom_id-Präfixe | `clip_vote_select_v1:<window_id>`, `clip_vote_mine_v1:<window_id>` |

## Tests

- Unit (`cargo test -p dl-community --lib clip`): URL-Validierung, Duplikat-Schlüssel, Stimmzettel (Sortierung, Dedupe, 25er-Grenze), Ranking und Gleichstand, Stimmrecht (eigener Clip Discord/Twitch, Mindestalter), Fensterzuordnung inkl. Samstag 23:30, Embed-Budget, keine Gedankenstriche in Texten.
- Broker (`cargo test -p dl-broker`): Auth vor Portzugriff, Plan-Payload, Normalisierung, Formfehler 400/413, Portfehler 503, Serialisierung.
- DB (`scripts/central_test_db.sh cargo test -p dl-community --features testing --lib clip`): vollständiger Ablauf mit genau einem Voting-, Ergebnis-Post und Kurator-DM trotz doppelter Scheduler-Läufe, Stimmrecht im Handler, Stimme ändern, Stimme nach Ende, Ergebniszeilen; Twitch-Einsendung (Partner, Replay, Duplikat in anderer URL-Form, Konflikt, ungültige URL, Fenster, Dump, Login-Fallback); Neustart nach Senden ohne Doppelpost; leeres Fenster `skipped`, alte Fenster ohne Voting.
- Schema (`scripts/central_test_db.sh cargo test -p dl-central-db --test fresh_migrations_schema -- --ignored`): neue Spalten, PKs, Nullability, CHECK für Twitch-Zeilen.

## Rest-Risiken

- Footer-Suche umfasst die letzten 50 Nachrichten. Schreiben zwischen Absturz und Neuversuch (10 Minuten) mehr als 50 Nachrichten in den Kanal, kann ein zweiter Voting-Post entstehen. Die Nonce fängt nur kurze Abstände ab.
- Kurator-DM ist mindestens-einmal: Absturz nach dem Senden, vor dem Vermerk, führt nach 10 Minuten zu einer zweiten DM.
- Mindestalter greift nur, wenn das Mitglied im Gateway-Cache ist.
- Eigener-Clip-Schutz für Twitch-Clips braucht Paket A (`core.discord_platform_connections`, Spalten `discord_id`, `platform`, `platform_user_id`).
- Partnerprüfung über `bot.twitch_streamer_invites`: Partner ohne Eintrag dort werden mit `not_partner` abgelehnt. Paket E prüft den Partnerstatus zusätzlich auf seiner Seite.
- Sehr lange Links (Budget 5800 Zeichen je Post) erscheinen ohne Link mit Hinweis auf den Wochen-Dump; abstimmen geht trotzdem über das Auswahlmenü.
- Bei einem Deploy mehr als 36 Stunden nach Fensterende gibt es für diese Woche kein Voting (gewollt, damit alte Wochen nicht nachträglich starten).
- Erasure (`/datenschutz`): Einsendungen eines Mitglieds werden gelöscht, damit fallen Stimmzettel-Eintrag und Stimmen anderer für diesen Clip weg; im Ergebnis werden `user_id` und `submission_id` genullt.
- Der Privacy-Vertragstest `alle_migration_user_id_spalten_sind_im_privacy_vertrag` war schon vor Paket D rot (Twitch-Invite-Tabellen aus 2026092701 bis 2026093005); die neuen Clip-Spalten sind eingetragen.
