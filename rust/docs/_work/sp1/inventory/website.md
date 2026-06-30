# SP1 Phase 0 Inventory: WEBSITE-COACHING

Read-only Inventar der GOLDEN Website-Coaching-DB.

## Live-DB

- Pfad: `/home/naniadm/Documents/Website/builds/backend/deadlock.db`
- Projekt: `Website`
- DB-Tech: `aiosqlite`
- Tabellen: 21
- Datei/Inode: dev=64769, inode=7127670, size=262144
- Bot-DB Vergleich: `/home/naniadm/Documents/Deadlock-Bots/data/deadlock.sqlite3` dev=64769, inode=3671569, size=47407104 -> eigene Datei/Inode, nicht Bot-DB

## proposed_schema=coaching

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `coach_applications` | 0 | 14 | app.database, app.routers.coaching | Coach-Bewerbungen; aktuell leer, aber create/review-Routen aktiv. |
| `coach_reviews` | 0 | 8 | app.database, app.routers.coaching | Review-Historie aus Surveys; aktuell leer; Hypertable-Kandidat nur bei starkem Wachstum. |
| `coachees` | 14 | 10 | app.database, app.routers.coaching_platform, tests.test_appointments | Coaching-Plattformprofil fuer gecoachte Spieler; Bot-Sync und Coach/User-Routen aktiv. |
| `coaches` | 5 | 15 | app.database, app.routers.auth, app.routers.coaching, app.routers.coaching_platform, tests.test_appointments | Coach-Roster/Profile; Bot-Roster-Sync, Public-Profil und Auth-Coach-Gate aktiv. |
| `coaching_appointments` | 0 | 13 | app.database, app.routers.coaching_platform, tests.test_appointments | Termin- und Notification-Queue; aktuell leer; Hypertable-Kandidat bei laengerer Termin-Historie. |
| `coaching_goals` | 0 | 12 | app.database, app.routers.coaching_platform, tests.test_appointments | Coach-gepflegte Ziele; aktuell leer, aber Plattform-Routen aktiv. |
| `coaching_milestones` | 0 | 8 | app.database, app.routers.coaching_platform, tests.test_appointments | Meilensteine unter coaching_goals; aktuell leer, aber Plattform-Routen aktiv. |
| `coaching_requests` | 21 | 21 | app.database, app.routers.coaching, app.routers.coaching_platform, tests.test_appointments | SP1/#387: Website id TEXT plus Plattform-Spalten assigned_coach_username, bot_request_id, preferred_coach_id, notify_discord_at; Bot-Seite hat id INTEGER plus Discord-Message/Rollen/Scheduling-Linkfelder. Kanonisch braucht TEXT-id, bot_request_id, website_request_id, gemeinsame Request-Felder, Status/Timestamps, Coach-Claim-Felder, Notify-Felder, Discord-Message/Rollenfelder, scheduled_slot und coachee_id. Hypertable-Kandidat fuer Request-Historie. |
| `coaching_sessions` | 16 | 13 | app.database, app.routers.coaching, app.routers.coaching_platform, tests.test_appointments | Session-Historie aus Bot/API-Match und Plattform-Sync; Hypertable-Kandidat ueber started_at/completed_at/created_at. |
| `coaching_surveys` | 0 | 8 | app.database, app.routers.coaching | Post-Coaching-Surveys; aktuell leer, Bot-Route schreibt; Hypertable-Kandidat bei Survey-Historie. |
| `session_notes` | 0 | 8 | app.database, app.routers.coaching_platform, tests.test_appointments | Coach/User-Notizen; aktuell leer, aktive Plattform-Routen; eher Dokumentdaten, kein primaerer Hypertable-Kandidat. |

## proposed_schema=OFFEN

| Tabelle | Rows | Cols | Konsumenten | Notizen |
|---|---:|---:|---|---|
| `meta_announcements` | 0 | 4 | app.database, app.routers.admin | OFFEN: Admin-Announcement-Meta, nicht Coaching; aktuell leer, aber Writer aktiv. |
| `meta_builds` | 5 | 12 | app.database, app.routers.admin, app.routers.builds | OFFEN: Build-/Hero-Content, nicht Coaching. |
| `meta_heroes` | 38 | 9 | app.database, app.routers.heroes | OFFEN: Hero-Katalog/Seed-Daten, Referenzdaten statt Coaching. |
| `meta_items` | 0 | 5 | app.database, app.routers.items | OFFEN/tot: aktuell leer; nur Read-Router, kein Writer/Seed in Live-DB gefunden. |
| `meta_patch_notes` | 1 | 5 | app.database, app.routers.patchnotes | OFFEN: Patchnote-Content, nicht Coaching. |
| `meta_reports` | 0 | 7 | app.database, app.routers.admin, app.routers.builds | OFFEN: Build-Reports/Ticketdaten, nicht Coaching; aktuell leer, Writer aktiv. |
| `meta_tier_history` | 0 | 7 | app.database, app.routers.history | OFFEN/tot: aktuell leer; Read-Route vorhanden, kein Writer gefunden; Hypertable-Kandidat falls Tier-Audit reaktiviert wird. |
| `meta_tier_lists` | 1 | 8 | app.database, app.routers.tierlists | OFFEN: Tierlist-Content, nicht Coaching. |
| `meta_users` | 5 | 6 | app.database, app.routers.auth, app.routers.admin, tests.test_appointments | OFFEN: Website-Auth/Admin-Rollen, quer genutzte Identitaetstabelle. |
| `meta_votes` | 0 | 5 | app.database, app.routers.admin | OFFEN/tot: aktuell leer; Admin liest/deletet, Build-Vote schreibt nur Zaehler in meta_builds; kein Insert in meta_votes gefunden; Hypertable-Kandidat falls Vote-Events reaktiviert werden. |

## #387 coaching_requests Vergleich

Website `coaching_requests` Spalten exakt aus `PRAGMA table_info`:

- `id TEXT PK=1`
- `discord_user_id INTEGER NOT NULL`
- `discord_username TEXT`
- `rank TEXT NOT NULL`
- `subrank TEXT NOT NULL`
- `hero TEXT`
- `games_played TEXT`
- `hours_played TEXT`
- `availability TEXT`
- `current_problems TEXT`
- `ai_summary TEXT`
- `ai_insights_json TEXT`
- `status TEXT`
- `created_at TIMESTAMP`
- `updated_at TIMESTAMP`
- `assigned_coach_id TEXT`
- `assigned_coach_username TEXT`
- `reserved_until INTEGER`
- `bot_request_id INTEGER`
- `preferred_coach_id TEXT`
- `notify_discord_at TIMESTAMP`

Bot `coaching_requests` Spalten aus `deadlock-bots.tables.json`:

- `id INTEGER PK=1`
- `discord_user_id INTEGER NOT NULL`
- `discord_username TEXT`
- `rank TEXT NOT NULL`
- `subrank TEXT NOT NULL`
- `hero TEXT`
- `games_played TEXT`
- `hours_played TEXT`
- `availability TEXT`
- `current_problems TEXT`
- `ai_summary TEXT`
- `ai_insights_json TEXT`
- `status TEXT`
- `message_id INTEGER`
- `channel_id INTEGER`
- `role_assigned_at INTEGER`
- `role_expires_at INTEGER`
- `role_removed_at INTEGER`
- `created_at INTEGER NOT NULL`
- `updated_at INTEGER NOT NULL`
- `scheduled_slot TEXT`
- `assigned_coach_id TEXT`
- `reserved_until INTEGER`
- `website_request_id TEXT`
- `coachee_id TEXT`

Direkter Befund:

- Gemeinsam: `id`, `discord_user_id`, `discord_username`, `rank`, `subrank`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `ai_summary`, `ai_insights_json`, `status`, `created_at`, `updated_at`, `assigned_coach_id`, `reserved_until`
- Nur Website: `assigned_coach_username`, `bot_request_id`, `preferred_coach_id`, `notify_discord_at`
- Nur Bot: `message_id`, `channel_id`, `role_assigned_at`, `role_expires_at`, `role_removed_at`, `scheduled_slot`, `website_request_id`, `coachee_id`
- Hauptkonflikt: Website nutzt `id TEXT`, Bot nutzt `id INTEGER`; `created_at`/`updated_at` sind Website-`TIMESTAMP`, Bot-`INTEGER`.

Kanonische vereinende Form braucht konkret:

- `id TEXT PRIMARY KEY` als kanonische Request-ID; Bot-Alt-ID ueber `bot_request_id INTEGER UNIQUE` behalten.
- `website_request_id TEXT` fuer Uebergang/Legacy-Verlinkung, solange Bot und Website noch getrennte IDs kennen.
- Gemeinsame Nutzerdaten: `discord_user_id`, `discord_username`.
- Gemeinsame Request-Nutzdaten: `rank`, `subrank`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `ai_summary`, `ai_insights_json`.
- Status/Zeit: `status`, `created_at`, `updated_at` mit einheitlichem Zeittyp.
- Claim/Coach: `assigned_coach_id`, `assigned_coach_username`, `reserved_until`, `preferred_coach_id`.
- Discord/Bot-Lifecycle: `message_id`, `channel_id`, `role_assigned_at`, `role_expires_at`, `role_removed_at`, `notify_discord_at`.
- Planung/Profil-Link: `scheduled_slot`, `coachee_id`.

## Unsicherheiten

- Konsumenten wurden wortgenau in `.py`/`.rs` gesucht; Frontend nutzt API-Endpunkte und keine Tabellennamen direkt.
- `tests.test_appointments` ist als Konsument aufgefuehrt, weil die Testmodule die Tabellennamen wortgenau enthalten.
- `tot` bedeutet hier: Live-DB leer und kein Writer/Insert im Website-Code gefunden, nicht geloeschte Schemaentscheidung.
- Hypertable-Kandidat ist eine Migrationsnotiz fuer Zeit-/Historientabellen; keine Schemaaenderung wurde vorgenommen.
