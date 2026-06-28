# DB-Vertrag: data/deadlock.sqlite3

Die SQLite-Datei `data/deadlock.sqlite3` (WAL-Modus) ist der gemeinsame Vertrag zwischen:

1. dem laufenden **Python-Bot** (Schema-Besitzer bis zum jeweiligen Domänen-Cutover),
2. dem **Rust-Steam-Bot** (eigenes Repo, liest/schreibt steam_*-Tabellen),
3. diesem **Rust-Rewrite**.

## Regeln

1. **Schema-Besitz:** Bis zum Cutover einer Domäne legt ausschließlich der Python-Bot
   deren Tabellen an und ändert sie. Rust-Code führt bis dahin kein `CREATE`/`ALTER`
   auf Bestands-Tabellen aus.
2. **Pfad-Auflösung** (identisch zu `service/db.py`):
   `DEADLOCK_DB_PATH` (kompletter Pfad) → `DEADLOCK_DB_DIR` (+ `deadlock.sqlite3`)
   → Default `data/deadlock.sqlite3` relativ zum Arbeitsverzeichnis.
3. **Kein stilles Neuanlegen:** `dl_db::Db::open` schlägt fehl, wenn die Datei fehlt.
4. **Nebenläufigkeit:** WAL erlaubt parallele Leser + einen Writer. dl-db serialisiert
   eigene Writes über eine Writer-Verbindung mit `busy_timeout` 5 s; Python und
   Steam-Bot schreiben parallel weiter — SQLite-Locking regelt das.
5. **Tests:** niemals gegen die Produktions-DB. Temp-DB + Vertrags-DDL aus
   `db-schema.sql`.

## Schema-Snapshot

`db-schema.sql` enthält den kompletten generierten DDL-Stand (Tabellen, Indizes,
Trigger). Regenerieren:

```bash
python3 rust/scripts/dump_schema.py   # read-only, schreibt rust/docs/db-schema.sql
```

## Themen-Cluster (114 Tabellen, Stand 10.6.2026)

| Cluster | Tabellen (Auswahl) | Künftiger Besitzer |
|---|---|---|
| KV/Infra | kv_store, persistent_views, schema_version | dl-db / dl-discord |
| Voice | voice_stats, voice_session_log, voice_feedback_*, voice_channel_* | dl-voice |
| TempVoice | tempvoice_*, router_user_prefs | dl-voice |
| Steam-Link | steam_links(+archive), steam_nudge_state, steam_friend_*, steam_launch_tokens | dl-bridges |
| Beta-Invite | steam_beta_invites, beta_invite_* | dl-bridges |
| Builds/Heroes | deadlock_heroes, deadlock_hero_builds, hero_build_* | dl-web (Tierlist/Builds) |
| Tierlist | tierlist_* | dl-web |
| Aktivität | user_activity_patterns, user_co_players, member_events, message_activity, text_* | dl-activity |
| Retention | user_retention_*, dm_response_tracking | dl-activity |
| Coaching | coaching_*, coaches, coach_applications | dl-coaching |
| Turnier (Legacy) | customgames_*, tournament_periods, turnier_auth_tokens | — Code 2026-06-28 entfernt; Tabellen verwaist, DB-Drop separat später |
| Moderation | ai_moderation_*, security_guard_incidents, user_mod_tags | dl-moderation |
| Community | clip_*, faq_chat_*, server_faq_logs, rename_*, member_leave_surveys, issue_reports | dl-community |
| Changelogs | changelog_entries, changelog_posts, deadlock_changelogs | dl-bot |
| Standalone | standalone_bot_state, standalone_commands, steam_tasks | nur lesen (ADR 0002) |
| Live-State | live_player_state (befüllt vom Steam-Bot) | nur lesen |

Verwaiste/leere Tabellen, die NICHT übernommen werden, listet `00-rewrite-plan.md`
(Abschnitt „Nicht portiert").

## Bekannte Trigger

- `trg_cap_steam_tasks` — kappt `steam_tasks` nach INSERT auf 1000 Zeilen
- `trg_steam_links_owner_guard_insert/_update` — verhindert doppelte steam_id-Zuordnung
