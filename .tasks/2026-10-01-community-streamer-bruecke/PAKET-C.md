# Paket C: Leaderboard und Sync (Deadlock-Bots)

status: gebaut, Branch `wip/leaderboard`
datum: 2026-10-01

## Was gebaut ist

### Migration `2026100103_community_points.sql`

Neues Schema `community_points`, rein additiv, Registry-Zeilen in `core.privacy_field_registry` und GRANT an `deadlock` (nur falls die Rolle existiert, Muster 2026100101):

- `twitch_viewer_daily` (PK `twitch_user_id, channel_twitch_user_id, day`): Tageswerte je Zuschauer und Partnerkanal. Kein Twitch-Login gespeichert (Zuschauer ohne Verknüpfung sollen nirgends mit Namen auftauchen).
- `twitch_streamer_daily` (PK `streamer_twitch_user_id, day`): Tageswerte je Partner, `discord_user_id` als BIGINT (ungültige Werte aus der Quelle werden `NULL`).
- `ledger (id, discord_id, streamer_twitch_user_id, source, ref, points, occurred_at, meta, created_at)`, `UNIQUE (source, ref)`. Zusätzlich zur Vorgabe: Spalte `streamer_twitch_user_id`, weil Streamer-Punkte (qualifizierte Beitritte, Twitch-Clip-Plätze) einem Kanal gehören und dessen Discord-ID oft fehlt. CHECK: genau ein Empfänger. Quellen: `clip_place`, `clip_vote`, `streamer_suggestion`, `streamer_qualified_join`.
- `sync_state (name, cursor, updated_at)`.
- Kein Rückweg unter `rollbacks/` (rein additiv wie 2026100101). Rückbau: `DROP SCHEMA community_points CASCADE` plus die 8 Registry-Zeilen.

Mitgezogen: `tests/fresh_migrations_schema.rs` (Spalten, PKs, Schema-Anzahl 17, Registry-Zeilen, `(source, ref)` eindeutig, Empfänger-CHECK, Signatur 2026100103 idempotent) und `dl-community/src/privacy.rs` (Löschantrag entfernt `ledger`-Zeilen über `discord_id` und `twitch_streamer_daily`-Zeilen über `discord_user_id`; Twitch-ID-Spalten stehen mit Begründung in der Allowlist).

### Lese- und Schreibfunktionen `dl_central_db::community_points`

- Punkteregeln als Konstanten: `STREAMER_WATCH_MINUTES_PER_POINT = 30`, `STREAMER_RAID_POINTS = 25`, `STREAMER_QUALIFIED_JOIN_POINTS = 50`, `CLIP_PLACE_POINTS = [100, 60, 40]`, `CLIP_VOTE_POINTS = 2`, `STREAMER_SUGGESTION_POINTS = 150`.
- `Period::{Gesamt, Season, Woche}` mit `bounds(today)`: Season = Kalendermonat, Woche ab Montag, Tage nach Europe/Berlin, Zeitpunkt-Grenzen = Berliner Mitternacht in UTC.
- Sync: `load_cursor`, `apply_viewer_page`, `apply_streamer_page` (Seite + Cursor in einer Transaktion, ältere Quellstände überschreiben nie jüngere, `null`-Cursor löscht keinen gespeicherten).
- Ledger: `record_ledger_event` (für Paket F), `import_qualified_join_ledger`, `import_clip_contest_ledger`.
- Lesen: `community_board(pool, period, today)`, `streamer_board(pool, period, today)`, `member_rank`.

### Punkte-Aggregation

Gemeinsames Leaderboard je Discord-ID:

- Voice: gesamt aus `voice.voice_stats.total_points`, Season/Woche aus `activity.voice_session_log.points` nach `ended_at` (der Tracker schreibt beides in derselben Transaktion).
- Twitch (`points_watch`, `points_chat`, `points_discovery`): nur über `core.discord_platform_connections` (Plattform `twitch`). Zuschauer ohne Verknüpfung erscheinen nie. Punkte von vor der Verknüpfung zählen ab der Verknüpfung mit (PLAN: "erst nach Verknüpfung zugerechnet").
- Ledger: `clip_place` + `clip_vote` als Clip-Contest, `streamer_suggestion` als Vorschläge, nur Zeilen mit `discord_id`.
- Mitglieder mit `core.user_privacy.opted_out` oder `deleted_at` fehlen ganz (gleiche Bedingung wie in `platform_connections.rs`). Mitglieder ohne Punkte im Zeitraum fehlen.
- Gleichstand teilt den Platz (1 + Anzahl Mitglieder mit mehr Punkten).

Partner-Leaderboard je Streamer (Twitch-ID, nur Kanäle mit Tageszeilen aus Paket B, Login und Discord-ID aus der jüngsten Zeile):

- Watchtime: Summe `twitch_viewer_daily.watch_minutes` im eigenen Kanal, nur verknüpfte Zuschauer ohne Privacy-Grabstein, je Tag summiert und dann `/ 30` abgerundet (nicht `viewer_minutes` gesamt). Eigenes Zuschauen im eigenen Kanal zählt nicht.
- Raids: `25 * raids_to_partners`.
- Qualifizierte Beitritte: `bot.twitch_invite_joins` mit `status = 'qualified'`, zugeordnet über `streamer_twitch_user_id` (Kanal der Einladung; persönliche Links von Zuschauern zählen für den Kanal, in dem sie entstanden sind). Ledger-Ref `qualified_invite:<join_id>` wie in `docs/qualified-invites.md` vorgeschlagen, `occurred_at = qualified_at`. 50 Punkte.
- Clip-Plätze: `clip_place`-Zeilen mit `streamer_twitch_user_id` (Einsendung aus Twitch).
- Streamer, deren Discord-ID einen Privacy-Grabstein hat, fehlen.

### Sync-Bin `dl-community-points-sync`

`rust/bin/dl-community-points-sync` (lib + main). Gleiche URL-Quelle (`runtime.bridges.twitch_api_url`, Standard `http://127.0.0.1:8776`), nur numerisches Loopback ohne Zugangsdaten/Pfad, keine Redirects, kein Proxy, Token `TWITCH_INTERNAL_API_TOKEN` aus dem Infisical-Bootstrap, DSN aus `DEADLOCK_CENTRAL_DSN`. Ablauf je Lauf:

1. `viewers`, dann `streamers`: ab gespeichertem Cursor, `limit=1000`, solange `has_more`. `next_updated_since` wird unverändert gespeichert und zurückgegeben. Ungültige Zeilen (IDs nicht numerisch, Tag kaputt, negative Werte) werden gezählt und übersprungen, der Cursor läuft weiter. Abbruch bei `has_more` ohne neuen Cursor und nach 500 Seiten.
2. `import_qualified_join_ledger`, `import_clip_contest_ledger`.

Betrieb: `scripts/run_dl_community_points_sync.sh` (Infisical-Loader wie `run_brain_feeder.sh`, in `.gitignore` freigeschaltet), `service/systemd/dl-community-points-sync.{service,timer}` (alle 10 Minuten, `OnBootSec=5min`). Für `dl-twitch-invite-sync` gibt es im Repo keine Unit; Muster sind die vorhandenen Units in `service/systemd`.

### Discord-Befehle (`dl-voice`)

In `stats.rs` am bestehenden Subscriber, mit demselben Rate-Limit (5 in 30 s je Person) und Embed-Muster; Formatierung in `dl-voice/src/community_points.rs`. Bestehende Befehle unverändert (Test `community_befehle_antworten_und_vleaderboard_bleibt` prüft `!vlb` mit).

- `!lb` / `!leaderboard` (`gesamt`, `woche`, Standard Season): Top 10 mit Herkunft (Voice, Twitch, Clips, Vorschläge), Fußzeile mit eigenem Platz.
- `!punkte`: Season-Aufschlüsselung Voice/Zuschauen/Chat/Entdecken/Clip-Contest/Streamer-Vorschläge, Platz in Season und gesamt, Punkte der Woche. Ohne Twitch-Verknüpfung Hinweis auf den Knopf in `<#1398021105339334666>` (Konstante aus Paket G). Mit Widerspruch nur ein Satz, keine Zahlen.
- `!streamerlb` (`gesamt`, `woche`): Top 10 Partner mit Community-Zuschauzeit, Raids, neuen Mitgliedern, Clip-Punkten.
- Keine Verdrahtungsänderung in `dl-bot` nötig.

## Clip-Contest (Paket D)

Paket D war beim Bau noch nicht in `wip/leaderboard`. Der Orchestrator hat den Merge von `claude/elegant-planck-ii29cu` (08a85f85) angefordert, der Merge wurde in dieser Sitzung aber von der Rechteprüfung abgelehnt und ist **nicht** passiert. Der Clip-Teil ist trotzdem fertig angeschlossen, abgegrenzt in `import_clip_contest_ledger`:

- Läuft nur, wenn `clips.clip_contest_results`, `clips.clip_votes` und `clips.clip_votings` existieren (`to_regclass`), sonst meldet der Sync "Clip-Contest-Tabellen fehlen noch".
- Plätze: `clips.clip_contest_results (window_id, place, source, user_id, streamer_twitch_user_id, submission_id, decided_at)`, Ref `clip_place:<window_id>:<place>`, Punkte 100/60/40, Empfänger `user_id` bei `source = 'discord'`, `streamer_twitch_user_id` bei `source = 'twitch'`.
- Stimmen: `clips.clip_votes (window_id, voter_user_id, updated_at)` join `clips.clip_votings (window_id, status, closed_at, voting_end_at)` mit `status = 'closed'`, Ref `clip_vote:<window_id>:<voter_user_id>`, 2 Punkte, Zeitpunkt `closed_at`.
- Spalten nach dem Stand der D-Migration `2026100102_clip_contest_voting.sql` (Worktree `bots-clips`). Getestet gegen einen Spaltenausschnitt dieser Tabellen im Test.

Nach dem Merge von D zu tun: Abfragen gegen `.tasks/.../PAKET-D.md` abgleichen (konnte ich wegen der Ablehnung nicht mehr lesen), Migrationen per `touch` auffrischen, `community_points_roundtrip` und `fresh_migrations_schema` erneut laufen lassen (D erhöht die Schema-Anzahl nicht, `clips` existiert schon). Beim Merge ist ein Textkonflikt in `fresh_migrations_schema.rs` (Signaturzeilen nach 2026100101) und `privacy.rs` wahrscheinlich; beide Seiten behalten.

## Nicht gebaut (nächste Schritte)

- **Stufen-Rollen:** Es gibt keine vorhandene Rollen-Automatik nach Aktivität (nur Reaction Roles, Server-as-Code ohne Stufenrollen, einzelne Rollen-IDs in `runtime.*` über Kompatibilitätsschlüssel). Kein sauberes Muster für eine Rollenliste aus TOML, deshalb weggelassen. Vorschlag: TOML-Sektion `[community_points] tier_role_ids = []` und `tier_thresholds = []` (leer = aus) in `BotConfig`, Task in `dl-bot` alle 30 Minuten: `community_board(Season)`, je Mitglied höchste erreichte Stufe, per Discord-Adapter genau diese Stufenrolle setzen und die anderen Stufenrollen entfernen (idempotent über den Gateway-Cache-Vergleich), Monatswechsel setzt automatisch zurück.
- **Monatlicher Streamer-Spotlight:** Kein vorhandenes Muster für monatliche Posts (nur Concierge-Scheduler und Clip-Contest-Scheduler). Vorschlag: im Muster des Clip-Contest-Schedulers (Zustand in DB, Claim, Post) am 1. des Monats `streamer_board(Season)` des Vormonats in einen Kanal posten; Kanal-ID als TOML-Wert mit leerem Default.
- **Streamer-Vorschlag (Paket F):** F ruft beim Partnerwerden `record_ledger_event` mit `LedgerRecipient::Member(<erster Vorschlagender>)`, `source = SOURCE_STREAMER_SUGGESTION`, `reference = "streamer_suggestion:<id>"`, `points = STREAMER_SUGGESTION_POINTS`. Idempotent, Widerspruch wird beachtet.

## Tests

- `cargo test -p dl-central-db --lib community_points`: 5 (Formeln, Zeiträume inkl. Sommer-/Winterzeit, Berliner Tagesgrenze, Ränge, Streamer-Summe).
- `cargo test -p dl-community-points-sync`: 8 ohne DB (Fake-HTTP-Server mit Seitensemantik aus Paket B: mehrere Seiten ohne Verlust/Doppel, exakter Cursor mit Mikrosekunden, Fortsetzen ab Cursor, leere Quelle mit `null`, ungültige Zeilen, Abbruch ohne Fortschritt, falsches Token, Loopback-Prüfung, Vertrags-JSON), plus 1 Ende-zu-Ende gegen die DB mit `--features testing`.
- `cargo test -p dl-voice --lib community_points`: 7 (Formatierung, Zeitraum-Argumente, Beschriftung, Gleichstand, Grenze 10, Verknüpfungshinweis, keine Gedankenstriche).
- Über `scripts/central_test_db.sh`: `community_points_roundtrip` (2: Upsert/Cursor/Rollback; Leaderboards je Zeitraum mit Voice, Twitch, Ledger, Clip-Import, Datenschutz, Streamer-Watchtime je Tag), `fresh_migrations_schema` (`--include-ignored dl_central_migrate_builds_contract_schema_and_is_idempotent`), `platform_connections_roundtrip`, `dl-voice stats::` (8, inkl. neuer Befehle und `!vlb`).
- Build `dl-bot`, `dl-community-points-sync` grün; clippy `--all-targets` für dl-central-db, dl-community-points-sync, dl-voice, dl-community ohne Warnungen.
- Bekannt rot ohne Bezug zu Paket C: dl-community `alle_migration_user_id_spalten_sind_im_privacy_vertrag` (Twitch-Invite-Tabellen aus 202609300x) und `privacy_loeschung_funktioniert_im_turniere_runtime` (braucht DB).

## Rest-Risiken

- Twitch-IDs der Zuschauer bleiben bei einem Löschantrag in `twitch_viewer_daily` (Quelle ist der Twitch-Bot, ein Löschen hier würde beim nächsten geänderten Tag wiederkommen). Zugerechnet wird nur über die Verknüpfung, die der Löschantrag entfernt.
- `!lb` lädt das ganze Board je Aufruf (Voice-Mitglieder plus Verknüpfte). Bei einigen tausend Zeilen unkritisch; wird es größer, Rang und Top-N in SQL rechnen.
- Voice gesamt kommt aus `voice_stats`, Season/Woche aus `voice_session_log`. Weichen beide historisch voneinander ab (z. B. Altdaten ohne Log), ist nur die Summe gesamt betroffen.
