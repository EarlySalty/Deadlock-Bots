# Coaching Platform Router - Verhaltens-Inventar

Stand: 2026-06-29

Quellen:
- `/home/naniadm/Documents/Website/builds/backend/app/main.py`
- `/home/naniadm/Documents/Website/builds/backend/app/routers/coaching_platform.py`
- `/home/naniadm/Documents/Website/builds/backend/app/routers/auth.py`
- `/home/naniadm/Documents/Website/builds/backend/app/routers/coaching.py`
- `/home/naniadm/Documents/Website/builds/backend/app/database.py`
- Frontend-Vertraege: `Website/dl-coaching/src/api/client.ts`, `Website/builds/frontend/src/api/client.ts`
- Bot-Verbraucher: `Deadlock-Bots/rust/crates/dl-community/src/coaching.rs`, `coaching_requests.rs`

## Mount und Scope

`coaching_platform.router` wird in `app/main.py` mit `prefix="/api/coaching"` eingebunden. Alle Routen unten sind daher vollstaendig:

`/api/coaching/platform/...`

Router-intern existieren 24 HTTP-Routen.

## Auth- und Identitaetsvertrag

### `require_bot_token`

Quelle: `app/routers/coaching.py`.

- Kein User-Kontext, Dependency-Parameter ist `_bot: None`.
- Akzeptiert Header `X-Internal-Token` oder `X-Bot-Token`.
- Secret-Env-Namen, nur Namen: `TWITCH_INTERNAL_API_TOKEN`, `MASTER_BROKER_TOKEN`, `COACHING_BOT_TOKEN`.
- Fehler:
  - kein konfigurierter Token: `503 Internal API token is not configured`
  - falscher Token: `401 Invalid internal token`

### `require_authenticated_user`

Quelle: `app/routers/auth.py`.

- Normalfall: Session-Cookie `ddc_session` bzw. Legacy `auth_token`.
- JWT-Felder werden zu User-Dict:
  - `id`: String aus JWT `sub`
  - `sub`: String aus JWT `sub`
  - `username`: String aus JWT `username`
  - `displayName`: JWT `display_name` oder `displayName` oder `username`
  - `avatarUrl`: JWT `avatar_url` oder `avatarUrl`
  - `role`: aus DB `meta_users.role` via `SELECT role FROM meta_users WHERE id=?`, Fallback JWT `role` oder `user`
- Sonderfall Caddy/Admin-Forward-Auth:
  - nur von localhost und Header `X-Admin-Validated: 1`
  - `sub="caddy-validated-admin"`, `role="admin"`, `username` aus `X-Admin-User` oder `admin`

### `require_coach_user`

Quelle: `app/routers/auth.py`.

- Erst `require_authenticated_user`.
- Admin (`user["role"] == "admin"`) darf immer durch.
- Sonst aktive Coach-Zeile erforderlich:
  - SQL: `SELECT 1 FROM coaches WHERE discord_user_id=? AND status='active'`
  - Identitaet: `int(user["sub"])`
- Ungueltige/nicht-numerische `sub` scheitert intern und liefert `403 Coach only`.

### `_acting_coach_id`

Quelle: `coaching_platform.py`.

- Nutzt `int(user["sub"])` plus `user.get("displayName")`.
- Fuehrt `_upsert_coach` aus und liefert interne `coaches.id`.
- Bei nicht-numerischer `sub` (z. B. Caddy-Admin) liefert `None`. Dadurch koennen Admin-Aktionen `coach_id=NULL` schreiben.

## Relevante Tabellen und Spalten

Website-DB (`deadlock.db`) aus `database.py`:

- `meta_users`: `id TEXT`, `username`, `display_name`, `avatar_url`, `role`, `created_at`. Nur Auth-Rollen-Read.
- `coaches`: `id TEXT PK`, `discord_user_id INTEGER UNIQUE NOT NULL`, `discord_username`, `display_name`, `avatar_url`, `bio`, `specialties_json TEXT DEFAULT '[]'`, `availability_json TEXT DEFAULT '{}'`, `status TEXT DEFAULT 'active'`, `avg_rating REAL DEFAULT 0`, `total_reviews INTEGER DEFAULT 0`, `total_sessions INTEGER DEFAULT 0`, `created_at`, `updated_at`, spaeter per ALTER `twitch_url TEXT`.
- `coachees`: `id TEXT PK`, `discord_user_id INTEGER UNIQUE NOT NULL`, `discord_username`, `display_name`, `rank`, `main_heroes_json TEXT DEFAULT '[]'`, `current_focus`, `notes`, `created_at`, `updated_at`.
- `coaching_requests`: `id TEXT PK`, `discord_user_id INTEGER NOT NULL`, `discord_username`, `rank TEXT NOT NULL DEFAULT ''`, `subrank TEXT NOT NULL DEFAULT ''`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `preferred_coach_id`, `ai_summary`, `ai_insights_json`, `status TEXT DEFAULT 'pending'`, `notify_discord_at`, `created_at`, `updated_at`, plus ALTER: `assigned_coach_id`, `assigned_coach_username`, `reserved_until INTEGER`, `bot_request_id INTEGER`.
- `coaching_sessions`: `id TEXT PK`, `request_id`, `coach_id`, `discord_user_id INTEGER NOT NULL`, `discord_username`, `discord_channel_id`, `status TEXT DEFAULT 'active'`, `scheduled_at`, `started_at DEFAULT CURRENT_TIMESTAMP`, `completed_at`, `created_at`, plus ALTER: `coachee_id TEXT`, `bot_session_id TEXT`.
- `coaching_goals`: `id TEXT PK`, `coachee_id`, `coach_id`, `session_id`, `title TEXT NOT NULL`, `description`, `status TEXT DEFAULT 'open'`, `sort_order INTEGER DEFAULT 0`, `target_date`, `completed_at`, `created_at`, `updated_at`.
- `coaching_milestones`: `id TEXT PK`, `goal_id`, `title TEXT NOT NULL`, `description`, `achieved INTEGER DEFAULT 0`, `achieved_at`, `sort_order INTEGER DEFAULT 0`, `created_at`.
- `session_notes`: `id TEXT PK`, `session_id`, `coachee_id`, `coach_id`, `content`, `visibility TEXT DEFAULT 'coach_only'`, `created_at`, `updated_at`.
- `coaching_appointments`: `id TEXT PK`, `coach_id`, `coachee_id`, `scheduled_at TEXT NOT NULL`, `duration_minutes INTEGER DEFAULT 60`, `title`, `note`, `status TEXT DEFAULT 'scheduled'`, `notify_created_at`, `notify_reminder_at`, `notify_cancelled_at`, `created_at`, `updated_at`.

Bot-DB-Kontext fuer heutige HTTP-Verbraucher:

- Rust `dl-community::coaching_requests` hat eigene `coaching_requests` mit `role_expires_at`, `role_removed_at`, `reserved_until`, `website_request_id`, `coachee_id`, `message_id`, `channel_id`.
- `ROLE_EXPIRY_HOURS = 168` (1 Woche), `CLAIM_RESERVATION_HOURS = 24`, `COACHING_ACTIVE_ROLE_ID = 1371929762913587292`.

## Gemeinsame Helper und SQL

### `_upsert_coach(db, discord_user_id, username)`

- Read: `SELECT id FROM coaches WHERE discord_user_id=?`
- Wenn vorhanden: keine Aktualisierung, Rueckgabe `coaches.id`.
- Wenn nicht vorhanden:
  - Insert: `INSERT INTO coaches (id, discord_user_id, discord_username, display_name, status) VALUES (?, ?, ?, ?, 'active')`
  - `id` ist `secrets.token_urlsafe(12)`.

### `_upsert_coachee(db, discord_user_id, username)`

- Read: `SELECT id FROM coachees WHERE discord_user_id=?`
- Wenn vorhanden und `username` truthy:
  - Update: `UPDATE coachees SET discord_username=?, updated_at=? WHERE id=?`
- Wenn vorhanden und kein `username`: keine Aktualisierung.
- Wenn nicht vorhanden:
  - Insert: `INSERT INTO coachees (id, discord_user_id, discord_username, display_name) VALUES (?, ?, ?, ?)`
  - `display_name=username`.

### `_goals_with_milestones(db, coachee_id)`

- Read Goals:
  - `SELECT * FROM coaching_goals WHERE coachee_id=? ORDER BY sort_order, created_at`
- Pro Goal:
  - `SELECT * FROM coaching_milestones WHERE goal_id=? ORDER BY sort_order, created_at`
- Response-Logik: haengt `milestones: [...]` an jedes Goal-Dict.

### `_table_columns(db, table_name)`

- `PRAGMA table_info(<table_name>)`, genutzt fuer optionale Spalte `preferred_coach_id`.

## Pydantic-Request-Modelle

`SyncPayload`

| Feld | Typ | Default |
|---|---|---|
| `bot_request_id` | `int` | required |
| `website_request_id` | `Optional[str]` | `None` |
| `discord_user_id` | `int` | required |
| `discord_username` | `Optional[str]` | `None` |
| `rank` | `Optional[str]` | `None` |
| `subrank` | `Optional[str]` | `None` |
| `hero` | `Optional[str]` | `None` |
| `games_played` | `Optional[str]` | `None` |
| `hours_played` | `Optional[str]` | `None` |
| `availability` | `Optional[str]` | `None` |
| `current_problems` | `Optional[str]` | `None` |
| `ai_summary` | `Optional[str]` | `None` |
| `status` | `str` | `"analyzed"` |
| `assigned_coach_discord_id` | `Optional[int]` | `None` |
| `assigned_coach_username` | `Optional[str]` | `None` |
| `reserved_until` | `Optional[int]` | `None` |
| `coach_discord_id` | `Optional[int]` | `None` |
| `coach_username` | `Optional[str]` | `None` |
| `session_status` | `Optional[str]` | `None` |

`CoacheeUpdate`: `display_name: Optional[str]=None`, `rank: Optional[str]=None`, `main_heroes_json: Optional[str]=None`, `current_focus: Optional[str]=None`, `notes: Optional[str]=None`.

`GoalCreate`: `title: str` required, `description: Optional[str]=None`, `target_date: Optional[str]=None`, `session_id: Optional[str]=None`.

`GoalUpdate`: `title: Optional[str]=None`, `description: Optional[str]=None`, `status: Optional[str]=None`, `sort_order: Optional[int]=None`, `target_date: Optional[str]=None`.

`MilestoneCreate`: `title: str` required, `description: Optional[str]=None`.

`MilestoneUpdate`: `title: Optional[str]=None`, `achieved: Optional[bool]=None`, `sort_order: Optional[int]=None`.

`NoteCreate`: `content: str` required, `visibility: str="coach_only"`, `session_id: Optional[str]=None`.

`NoteUpdate`: `content: Optional[str]=None`, `visibility: Optional[str]=None`.

`CoachSyncEntry`: `discord_user_id: int` required, `discord_username: Optional[str]=None`, `display_name: Optional[str]=None`, `avatar_url: Optional[str]=None`.

`CoachSyncPayload`: `coaches: List[CoachSyncEntry]` required.

`AppointmentCreate`: `coachee_id: str` required, `scheduled_at: str` required, `duration_minutes: int=60`, `title: Optional[str]=None`, `note: Optional[str]=None`.

`AppointmentUpdate`: `scheduled_at: Optional[str]=None`, `duration_minutes: Optional[int]=None`, `title: Optional[str]=None`, `note: Optional[str]=None`, `status: Optional[str]=None`.

`AckItem`: `appointment_id: str` required, `type: str` required (`created | reminder | cancelled` effektiv erlaubt).

`AckPayload`: `items: List[AckItem]=Field(default_factory=list)`, `request_ids: List[str]=Field(default_factory=list)`.

`CoachProfileUpdate`: `bio: Optional[str]=None`, `specialties: Optional[List[str]]=None`, `twitch_url: Optional[str]=None`.

## Response-Basistypen

SQLite gibt `TEXT` als String, `INTEGER` als Number, `REAL` als Number, NULL als JSON `null` zurueck. `SELECT *` liefert alle aktuellen Tabellenspalten inklusive spaeterer ALTER-Spalten.

Wichtige Row-Shapes:

- `coaching_requests *`: `id: string`, `discord_user_id: number`, `discord_username: string|null`, `rank: string`, `subrank: string`, `hero: string|null`, `games_played: string|null`, `hours_played: string|null`, `availability: string|null`, `current_problems: string|null`, `preferred_coach_id: string|null`, `ai_summary: string|null`, `ai_insights_json: string|null`, `status: string`, `notify_discord_at: string|null`, `created_at: string`, `updated_at: string`, `assigned_coach_id: string|null`, `assigned_coach_username: string|null`, `reserved_until: number|null`, `bot_request_id: number|null`.
- `coachees *`: `id`, `discord_user_id`, `discord_username`, `display_name`, `rank`, `main_heroes_json`, `current_focus`, `notes`, `created_at`, `updated_at`.
- `coaching_goals *`: `id`, `coachee_id`, `coach_id`, `session_id`, `title`, `description`, `status`, `sort_order`, `target_date`, `completed_at`, `created_at`, `updated_at`, plus `milestones`.
- `coaching_milestones *`: `id`, `goal_id`, `title`, `description`, `achieved`, `achieved_at`, `sort_order`, `created_at`.
- `session_notes *`: `id`, `session_id`, `coachee_id`, `coach_id`, `content`, `visibility`, `created_at`, `updated_at`.
- `coaching_sessions s.*`: `id`, `request_id`, `coach_id`, `discord_user_id`, `discord_username`, `discord_channel_id`, `status`, `scheduled_at`, `started_at`, `completed_at`, `created_at`, `coachee_id`, `bot_session_id`.
- `coaching_appointments` fixed selects variieren je Route; nicht jede Route liefert `coach_id`, `coachee_id`, `note` oder `updated_at`.

## Route-Inventar

### 1. `POST /api/coaching/platform/sync`

Auth: `require_bot_token`, keine User-Identitaet.

Request: `SyncPayload`.

Response:

- Normal: `{ "ok": true, "coachee_id": string }`
- Website-ID fehlt: `{ "ok": true, "skipped": true }`

SQL:

- Wenn `payload.website_request_id` gesetzt:
  - `SELECT id FROM coaching_requests WHERE id=?`
  - wenn keine Zeile: kein Coachee-Upsert, kein Commit, `skipped=true`
  - `_upsert_coachee`
  - `UPDATE coaching_requests SET assigned_coach_id=?, assigned_coach_username=?, status=?, reserved_until=?, updated_at=? WHERE id=?`
- Wenn keine `website_request_id`:
  - `_upsert_coachee`
  - `SELECT id FROM coaching_requests WHERE id=?`, `req_id=str(bot_request_id)`
  - Exists:
    - `UPDATE coaching_requests SET discord_user_id=?, discord_username=?, rank=?, subrank=?, hero=?, games_played=?, hours_played=?, availability=?, current_problems=?, ai_summary=?, status=?, assigned_coach_id=?, assigned_coach_username=?, reserved_until=?, updated_at=? WHERE id=?`
  - Nicht exists:
    - `INSERT INTO coaching_requests (id, discord_user_id, discord_username, rank, subrank, hero, games_played, hours_played, availability, current_problems, ai_summary, status, assigned_coach_id, assigned_coach_username, reserved_until) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`
- Wenn `coach_discord_id` und `session_status` gesetzt:
  - `_upsert_coach`
  - `completed = _iso()` nur fuer `session_status in ("completed", "cancelled")`, sonst `None`
  - `SELECT id FROM coaching_sessions WHERE request_id=?`
  - Exists:
    - `UPDATE coaching_sessions SET coach_id=?, coachee_id=?, discord_user_id=?, discord_username=?, status=?, completed_at=COALESCE(?, completed_at) WHERE id=?`
  - Nicht exists:
    - `INSERT INTO coaching_sessions (id, request_id, coach_id, coachee_id, discord_user_id, discord_username, status, completed_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)`

Business-Logik und Seiteneffekte:

- `assigned_coach_discord_id` wird zu String `assigned_id`; `0`/Falsy wird `None`.
- `rank` und `subrank` werden bei Bot-ID-Upsert als `payload.rank or ""`, `payload.subrank or ""` geschrieben.
- `payload.status` wird ungeprueft uebernommen.
- `session_status` wird ungeprueft uebernommen, nur `completed_at` kennt `completed|cancelled`.
- `bot_session_id` wird vom aktuellen Rust-Bot optional mitgesendet, ist aber kein Feld in `SyncPayload` und wird hier nicht geschrieben.
- Commit nur bei nicht geskipptem Pfad.
- Dies ist der zentrale heutige HTTP-Mirror aus `dl-community::coaching_requests::mirror_to_website`.

Bot-Datenfluss heute:

- Rust liest lokale Bot-DB `coaching_requests` und sendet:
  - `bot_request_id`, optional `website_request_id`, `discord_user_id`, `discord_username`, `rank`, `subrank`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `ai_summary`, `status`, `assigned_coach_discord_id`, `assigned_coach_username`, `reserved_until`.
  - Bei Claim/Session: `coach_discord_id`, `coach_username`, `session_status`.
- Bei Website-Origin-Requests verhindert `website_request_id` Duplikate: Python aktualisiert die Original-ID und legt keine Zeile mit `str(bot_request_id)` an.

`ai_summary`: Schreibstellen hier sind Request-Input, UPDATE-Spalte, INSERT-Spalte. Im Rust-Port soll dieses Feld weggelassen werden; falls kompatible alte Queue-Clients noch `ai_summary` erwarten, bewusst `null`/fehlend entscheiden.

### 2. `GET /api/coaching/platform/overview`

Auth: `require_coach_user`. Identitaet wird nur fuer Gate genutzt.

Request: kein Body, keine Query.

Response:

```json
{
  "coaches": [
    {
      "id": "string",
      "display_name": "string|null",
      "discord_username": "string|null",
      "active": 0,
      "completed": 0,
      "total": 0
    }
  ],
  "recent_sessions": [
    {
      "id": "string",
      "status": "string",
      "started_at": "string|null",
      "completed_at": "string|null",
      "discord_username": "string|null",
      "coachee_id": "string|null",
      "coachee_display": "string|null",
      "coach_display": "string|null"
    }
  ]
}
```

SQL:

- Coach-Stats:
  - `SELECT c.id, c.display_name, c.discord_username, SUM(CASE WHEN s.status='active' THEN 1 ELSE 0 END) AS active, SUM(CASE WHEN s.status='completed' THEN 1 ELSE 0 END) AS completed, COUNT(s.id) AS total FROM coaches c LEFT JOIN coaching_sessions s ON s.coach_id=c.id WHERE c.status='active' GROUP BY c.id ORDER BY total DESC, c.display_name`
- Recent:
  - `SELECT s.id, s.status, s.started_at, s.completed_at, s.discord_username, co.id AS coachee_id, co.display_name AS coachee_display, c.display_name AS coach_display FROM coaching_sessions s LEFT JOIN coaches c ON s.coach_id=c.id LEFT JOIN coachees co ON s.coachee_id=co.id ORDER BY s.started_at DESC LIMIT 40`

Seiteneffekte: keine.

### 3. `GET /api/coaching/platform/queue`

Auth: `require_coach_user`. Identitaet: `str(user["sub"])` wird mit `coaching_requests.assigned_coach_id` verglichen.

Request: kein Body, keine Query.

Response:

`{ "requests": [coaching_requests * plus reserved_for_me: bool, is_open: bool] }`

SQL:

- `SELECT * FROM coaching_requests WHERE status='analyzed' AND (assigned_coach_id IS NULL OR assigned_coach_id=?) ORDER BY created_at DESC LIMIT 50`

Business-Logik:

- `reserved_for_me = assigned_coach_id == str(user["sub"])`
- `is_open = assigned_coach_id IS NULL OR (reserved_until truthy AND int(reserved_until) <= now_unix)`
- Wichtig: SQL filtert fremde `assigned_coach_id` aus. Ein abgelaufenes fremdes `reserved_until` wird in dieser Route nicht sichtbar, solange der Bot nicht vorher `assigned_coach_id=NULL` gesetzt hat (`expire_reservations` im Rust-Bot).

`ai_summary`: durch `SELECT *` wird `ai_summary` aktuell mit ausgeliefert.

### 4. `GET /api/coaching/platform/coachees`

Auth: `require_coach_user`.

Request: kein Body.

Response:

```json
{
  "coachees": [
    {
      "id": "string",
      "discord_username": "string|null",
      "display_name": "string|null",
      "rank": "string|null",
      "current_focus": "string|null",
      "open_goals": 0,
      "sessions": 0
    }
  ]
}
```

SQL:

- `SELECT co.id, co.discord_username, co.display_name, co.rank, co.current_focus, (SELECT COUNT(*) FROM coaching_goals g WHERE g.coachee_id=co.id AND g.status IN ('open','active')) AS open_goals, (SELECT COUNT(*) FROM coaching_sessions s WHERE s.coachee_id=co.id) AS sessions FROM coachees co ORDER BY co.updated_at DESC LIMIT 200`

Seiteneffekte: keine.

### 5. `GET /api/coaching/platform/coachees/{coachee_id}`

Auth: `require_coach_user`.

Request: Path `coachee_id: str`.

Response:

```json
{
  "profile": "coachees *",
  "goals": ["coaching_goals * plus milestones"],
  "notes": ["session_notes *"],
  "sessions": ["coaching_sessions s.* plus coach_display"],
  "appointments": [
    {
      "id": "string",
      "scheduled_at": "string",
      "duration_minutes": 0,
      "title": "string|null",
      "note": "string|null",
      "status": "string",
      "created_at": "string",
      "coach_display": "string|null"
    }
  ]
}
```

SQL:

- `SELECT * FROM coachees WHERE id=?`
- via `_goals_with_milestones`:
  - `SELECT * FROM coaching_goals WHERE coachee_id=? ORDER BY sort_order, created_at`
  - je Goal `SELECT * FROM coaching_milestones WHERE goal_id=? ORDER BY sort_order, created_at`
- `SELECT * FROM session_notes WHERE coachee_id=? ORDER BY created_at DESC`
- `SELECT s.*, c.display_name AS coach_display FROM coaching_sessions s LEFT JOIN coaches c ON s.coach_id=c.id WHERE s.coachee_id=? ORDER BY s.started_at DESC`
- `SELECT a.id, a.scheduled_at, a.duration_minutes, a.title, a.note, a.status, a.created_at, COALESCE(c.display_name, c.discord_username) AS coach_display FROM coaching_appointments a LEFT JOIN coaches c ON a.coach_id=c.id WHERE a.coachee_id=? ORDER BY a.scheduled_at DESC`

Fehler:

- kein Coachee: `404 Coachee nicht gefunden`.

Seiteneffekte: keine.

### 6. `PATCH /api/coaching/platform/coachees/{coachee_id}`

Auth: `require_coach_user`.

Request: Path `coachee_id`, Body `CoacheeUpdate`.

Response: `{ "ok": true }`

SQL:

- Wenn mindestens ein Feld ungleich `None`:
  - dynamisch `UPDATE coachees SET <field>=?, ..., updated_at=? WHERE id=?`
- Wenn kein Feld gesetzt: kein SQL.

Business-Logik:

- Nur Felder mit Wert `is not None` werden geschrieben.
- Kein Existenzcheck; nicht vorhandene IDs liefern trotzdem `{ok:true}`.
- Erlaubte Update-Spalten kommen direkt aus Pydantic-Modell: `display_name`, `rank`, `main_heroes_json`, `current_focus`, `notes`.

### 7. `POST /api/coaching/platform/coachees/{coachee_id}/goals`

Auth: `require_coach_user`. Identitaet: `_acting_coach_id` aus `int(user["sub"])`, ggf. `NULL`.

Request: Path `coachee_id`, Body `GoalCreate`.

Response: `{ "id": "goal_id" }`

SQL:

- `_acting_coach_id`:
  - `SELECT id FROM coaches WHERE discord_user_id=?`
  - ggf. `INSERT INTO coaches (...)`
- `INSERT INTO coaching_goals (id, coachee_id, coach_id, session_id, title, description, target_date, status) VALUES (?, ?, ?, ?, ?, ?, ?, 'open')`

Business-Logik:

- `goal_id = secrets.token_urlsafe(12)`.
- Kein Coachee-Existenzcheck.
- Status immer `open`.

### 8. `PATCH /api/coaching/platform/goals/{goal_id}`

Auth: `require_coach_user`.

Request: Path `goal_id`, Body `GoalUpdate`.

Response: `{ "ok": true }`

SQL:

- Wenn mindestens ein Feld ungleich `None`:
  - dynamisch `UPDATE coaching_goals SET <field>=?, ..., completed_at=?, updated_at=? WHERE id=?`
- Wenn kein Feld gesetzt: kein SQL.

Business-Logik:

- `completed_at = _iso()` nur wenn `status == "done"` im Patch ist.
- Sonst wird `completed_at=NULL` geschrieben, auch bei title-only/description-only Updates. Das ist bestehendes Verhalten.
- Status wird nicht validiert (`open|active|done|dropped` nur Frontend-Konvention).
- Kein Existenz-/Ownership-Check.

### 9. `DELETE /api/coaching/platform/goals/{goal_id}`

Auth: `require_coach_user`.

Request: Path `goal_id`.

Response: `{ "ok": true }`

SQL:

- `DELETE FROM coaching_milestones WHERE goal_id=?`
- `DELETE FROM coaching_goals WHERE id=?`

Business-Logik:

- Manuelles Cascade fuer Milestones.
- Kein Existenz-/Ownership-Check.

### 10. `POST /api/coaching/platform/goals/{goal_id}/milestones`

Auth: `require_coach_user`.

Request: Path `goal_id`, Body `MilestoneCreate`.

Response: `{ "id": "milestone_id" }`

SQL:

- `INSERT INTO coaching_milestones (id, goal_id, title, description) VALUES (?, ?, ?, ?)`

Business-Logik:

- `milestone_id = secrets.token_urlsafe(12)`.
- Kein Goal-Existenzcheck.
- `achieved=0`, `sort_order=0`, `created_at=CURRENT_TIMESTAMP` aus DB-Defaults.

### 11. `PATCH /api/coaching/platform/milestones/{milestone_id}`

Auth: `require_coach_user`.

Request: Path `milestone_id`, Body `MilestoneUpdate`.

Response: `{ "ok": true }`

SQL:

- Dynamisch `UPDATE coaching_milestones SET <sets> WHERE id=?`
- Set-Logik:
  - `title=?` wenn `title is not None`
  - `sort_order=?` wenn `sort_order is not None`
  - `achieved=?` plus `achieved_at=?` wenn `achieved is not None`

Business-Logik:

- `achieved=True` schreibt `1` und `achieved_at=_iso()`.
- `achieved=False` schreibt `0` und `achieved_at=NULL`.
- Kein `updated_at` fuer Milestones.
- Kein Existenz-/Ownership-Check.

### 12. `DELETE /api/coaching/platform/milestones/{milestone_id}`

Auth: `require_coach_user`.

Request: Path `milestone_id`.

Response: `{ "ok": true }`

SQL:

- `DELETE FROM coaching_milestones WHERE id=?`

Seiteneffekte: keine weiteren.

### 13. `POST /api/coaching/platform/coachees/{coachee_id}/notes`

Auth: `require_coach_user`. Identitaet: `_acting_coach_id`.

Request: Path `coachee_id`, Body `NoteCreate`.

Response: `{ "id": "note_id" }`

SQL:

- `_acting_coach_id` SQL wie oben.
- `INSERT INTO session_notes (id, session_id, coachee_id, coach_id, content, visibility) VALUES (?, ?, ?, ?, ?, ?)`

Business-Logik:

- `note_id = secrets.token_urlsafe(12)`.
- `visibility` wird nur akzeptiert, wenn `coach_only` oder `shared_with_user`; sonst Fallback `coach_only`.
- Kein Coachee-/Session-Existenzcheck.

### 14. `PATCH /api/coaching/platform/notes/{note_id}`

Auth: `require_coach_user`.

Request: Path `note_id`, Body `NoteUpdate`.

Response: `{ "ok": true }`

SQL:

- Dynamisch `UPDATE session_notes SET <field>=?, ..., updated_at=? WHERE id=?`

Business-Logik:

- Felder mit `None` werden ignoriert.
- Wenn `visibility` gesetzt, aber nicht `coach_only|shared_with_user`, wird dieses Feld entfernt.
- Wenn danach kein Feld uebrig ist: kein SQL, `{ok:true}`.
- Kein Existenz-/Ownership-Check.

### 15. `DELETE /api/coaching/platform/notes/{note_id}`

Auth: `require_coach_user`.

Request: Path `note_id`.

Response: `{ "ok": true }`

SQL:

- `DELETE FROM session_notes WHERE id=?`

Seiteneffekte: keine weiteren.

### 16. `GET /api/coaching/platform/me`

Auth: `require_authenticated_user`. Identitaet: `discord_id=int(user["sub"])`.

Request: kein Body.

Response bei nicht-numerischer `sub` oder unbekanntem Coachee:

```json
{ "profile": null, "goals": [], "notes": [], "sessions": [] }
```

Response bei vorhandenem Coachee:

```json
{
  "profile": {
    "id": "string",
    "discord_user_id": 0,
    "discord_username": "string|null",
    "display_name": "string|null",
    "rank": "string|null",
    "main_heroes_json": "string|null",
    "current_focus": "string|null",
    "created_at": "string",
    "updated_at": "string"
  },
  "goals": ["coaching_goals * plus milestones"],
  "notes": ["session_notes * where visibility shared_with_user"],
  "sessions": [
    {
      "status": "string",
      "started_at": "string|null",
      "completed_at": "string|null",
      "coach_display": "string|null"
    }
  ],
  "appointments": [
    {
      "id": "string",
      "scheduled_at": "string",
      "duration_minutes": 0,
      "title": "string|null",
      "status": "string",
      "coach_display": "string|null"
    }
  ]
}
```

SQL:

- Coachee-Profil bewusst ohne interne `coachees.notes`:
  - `SELECT id, discord_user_id, discord_username, display_name, rank, main_heroes_json, current_focus, created_at, updated_at FROM coachees WHERE discord_user_id=?`
- `_goals_with_milestones`
- Shared Notes:
  - `SELECT * FROM session_notes WHERE coachee_id=? AND visibility='shared_with_user' ORDER BY created_at DESC`
- Sessions:
  - `SELECT s.status, s.started_at, s.completed_at, c.display_name AS coach_display FROM coaching_sessions s LEFT JOIN coaches c ON s.coach_id=c.id WHERE s.discord_user_id=? ORDER BY s.started_at DESC`
- Appointments:
  - Cutoff: `datetime.now(UTC) - 6 hours`
  - `SELECT a.id, a.scheduled_at, a.duration_minutes, a.title, a.status, c.display_name AS coach_display FROM coaching_appointments a LEFT JOIN coaches c ON a.coach_id=c.id WHERE a.coachee_id=? AND ((a.status='scheduled' AND a.scheduled_at >= ?) OR a.status IN ('done','cancelled')) ORDER BY CASE WHEN a.status='scheduled' THEN 0 ELSE 1 END, a.scheduled_at ASC LIMIT 50`

Business-Logik:

- `coachees.notes` wird absichtlich nicht an Spieler ausgeliefert.
- Termine:
  - scheduled: alle ab `now - 6h`
  - done/cancelled: aus SQL bis Limit 50, danach im Python-Code nur die letzten 5 (`[-5:]`) behalten
  - finale Reihenfolge: scheduled zuerst, dann letzte 5 closed.

### 17. `POST /api/coaching/platform/coaches/sync`

Auth: `require_bot_token`.

Request: `CoachSyncPayload`.

Response:

- Leere Liste: `{ "ok": true, "skipped": true }`
- Normal: `{ "ok": true, "active": number, "deactivated": number }`

SQL:

- Pro Entry:
  - `SELECT id FROM coaches WHERE discord_user_id=?`
  - Exists:
    - `UPDATE coaches SET discord_username=COALESCE(?, discord_username), display_name=COALESCE(?, display_name), avatar_url=COALESCE(?, avatar_url), status='active', updated_at=? WHERE discord_user_id=?`
  - Nicht exists:
    - `INSERT INTO coaches (id, discord_user_id, discord_username, display_name, avatar_url, status) VALUES (?, ?, ?, ?, ?, 'active')`
- Nach allen Entries:
  - dynamisch `UPDATE coaches SET status='inactive', updated_at=? WHERE status='active' AND discord_user_id NOT IN (?, ?, ...)`

Business-Logik und Seiteneffekte:

- Leere Liste ist Roster-Wipe-Schutz: keine DB-Aenderung.
- Bot-kontrollierte Felder: `discord_username`, `display_name`, `avatar_url`, `status`.
- Coach-eigene Felder bleiben unangetastet: `bio`, `specialties_json`, `availability_json`, `twitch_url`.
- `active = len(incoming_ids)` ist Anzahl eindeutiger Discord-IDs im Payload.
- `deactivated = cursor.rowcount` aus Inaktiv-Update.

Bot-Datenfluss heute:

- Rust `coaching.rs` baut alle Mitglieder mit `COACH_ROLE_ID=1494372744286965941`.
- Payload pro Coach: `discord_user_id`, `discord_username`, `display_name`, `avatar_url`.
- Loop alle 10 Minuten (`ROLE_SYNC_INTERVAL=600s`) plus Debounce nach Rollen-Events (`ROLE_DEBOUNCE=5s`).
- Rust sendet selbst keine leere Liste; Python schuetzt trotzdem.

### 18. `POST /api/coaching/platform/appointments`

Auth: `require_coach_user`. Identitaet: `_acting_coach_id`.

Request: `AppointmentCreate`.

Response: `{ "id": "appointment_id" }`

SQL:

- Vor DB: `datetime.fromisoformat(body.scheduled_at.replace("Z", "+00:00"))`
- `SELECT id FROM coachees WHERE id=?`
- `_acting_coach_id`
- `INSERT INTO coaching_appointments (id, coach_id, coachee_id, scheduled_at, duration_minutes, title, note) VALUES (?, ?, ?, ?, ?, ?, ?)`

Fehler:

- Ungueltige Zeit: `400 scheduled_at muss ISO-8601 UTC sein`
- Coachee fehlt: `404 Coachee nicht gefunden`

Business-Logik:

- `appointment_id = secrets.token_urlsafe(12)`.
- `duration_minutes` default `60`.
- `status='scheduled'` und Notify-Spalten NULL via DB-Default.
- Admin mit nicht-numerischer `sub` kann `coach_id=NULL` erzeugen.

### 19. `GET /api/coaching/platform/appointments`

Auth: `require_coach_user`. Identitaet: bei `scope=mine` `_acting_coach_id`.

Request: Query `scope: str = "mine"`.

Response:

```json
{
  "appointments": [
    {
      "id": "string",
      "coach_id": "string|null",
      "coachee_id": "string|null",
      "scheduled_at": "string",
      "duration_minutes": 0,
      "title": "string|null",
      "note": "string|null",
      "status": "string",
      "created_at": "string",
      "coachee_display": "string|null",
      "coach_display": "string|null"
    }
  ]
}
```

SQL:

- Cutoff: `datetime.now(UTC) - 7 days`
- Bei `scope == "mine"`:
  - `_acting_coach_id`
  - `SELECT a.id, a.coach_id, a.coachee_id, a.scheduled_at, a.duration_minutes, a.title, a.note, a.status, a.created_at, COALESCE(co.display_name, co.discord_username) AS coachee_display, COALESCE(c.display_name, c.discord_username) AS coach_display FROM coaching_appointments a LEFT JOIN coachees co ON a.coachee_id=co.id LEFT JOIN coaches c ON a.coach_id=c.id WHERE a.coach_id=? AND a.scheduled_at >= ? ORDER BY a.scheduled_at ASC`
- Bei jedem anderen `scope`:
  - gleicher Select/Joins ohne `a.coach_id=?`, nur `WHERE a.scheduled_at >= ?`

Business-Logik:

- `scope` wird nur auf exakt `"mine"` geprueft; jeder andere Wert bedeutet alle Coaches.
- Kein Admin-only fuer `scope=all`; jeder Coach kann alle Termine ab Cutoff sehen.
- 7-Tage-Cutoff ist Anzeige-/Listenlogik, keine Rollen-Entzugslogik.

### 20. `PATCH /api/coaching/platform/appointments/{appointment_id}`

Auth: `require_coach_user`. Identitaet: `_acting_coach_id`; Admin darf fremde Termine aendern.

Request: Path `appointment_id`, Body `AppointmentUpdate`.

Response: `{ "ok": true }`

SQL:

- Vor DB:
  - `status` muss `scheduled|done|cancelled` sein, wenn gesetzt.
  - `scheduled_at` muss ISO-parsebar sein, wenn gesetzt.
- `SELECT coach_id, status FROM coaching_appointments WHERE id=?`
- `_acting_coach_id`
- Dynamisch:
  - `UPDATE coaching_appointments SET <field>=?, ..., [notify_created_at=NULL, notify_reminder_at=NULL wenn scheduled_at gepatcht], updated_at=? WHERE id=?`

Fehler:

- Ungueltiger Status: `400 status muss scheduled | done | cancelled sein`
- Ungueltiges Datum: `400 scheduled_at muss ISO-8601 UTC sein`
- Termin fehlt: `404 Termin nicht gefunden`
- Nicht Owner und nicht Admin: `403 Nur der zuständige Coach darf diesen Termin ändern`

Business-Logik:

- Kein Update bei leerem Body.
- Reschedule setzt `notify_created_at` und `notify_reminder_at` zurueck, aber nicht `notify_cancelled_at`.
- Statuswechsel zu `cancelled` setzt keine Notify-Spalte direkt; Cancellation-Notification wird spaeter ueber `/notifications/due` gefunden, aber nur wenn `notify_created_at IS NOT NULL`.
- `appt["status"]` wird gelesen, aber nicht verwendet.

### 21. `GET /api/coaching/platform/notifications/due`

Auth: `require_bot_token`.

Request: kein Body.

Response:

```json
{
  "notifications": [
    {
      "type": "created|reminder|cancelled",
      "appointment_id": "string",
      "discord_user_id": 0,
      "coachee_display": "string|null",
      "coach_display": "string|null",
      "scheduled_at": "string",
      "duration_minutes": 0,
      "title": "string|null",
      "note": "string|null"
    },
    {
      "type": "request_created",
      "request_id": "string",
      "coachee_id": "string",
      "discord_user_id": 0,
      "discord_username": "string|null",
      "rank": "string",
      "subrank": "string",
      "hero": "string|null",
      "games_played": "string|null",
      "hours_played": "string|null",
      "availability": "string|null",
      "current_problems": "string|null",
      "preferred_coach_id": "string|null"
    }
  ]
}
```

SQL:

- Zeitfenster:
  - `now = datetime.now(UTC).isoformat()`
  - `in_2h = now + 2 hours`
- Appointment-UNION:
  - Created:
    - `SELECT 'created' AS type, a.id AS appointment_id, co.discord_user_id, COALESCE(co.display_name, co.discord_username) AS coachee_display, COALESCE(c.display_name, c.discord_username) AS coach_display, a.scheduled_at, a.duration_minutes, a.title, a.note FROM coaching_appointments a JOIN coachees co ON a.coachee_id=co.id JOIN coaches c ON a.coach_id=c.id WHERE a.status='scheduled' AND a.notify_created_at IS NULL`
  - Reminder:
    - same columns, `WHERE a.status='scheduled' AND a.notify_reminder_at IS NULL AND a.notify_created_at IS NOT NULL AND a.scheduled_at BETWEEN ? AND ?`
  - Cancelled:
    - same columns, `WHERE a.status='cancelled' AND a.notify_cancelled_at IS NULL AND a.notify_created_at IS NOT NULL`
- Request-Quelle:
  - `PRAGMA table_info(coaching_requests)`
  - `preferred_coach_select = preferred_coach_id` falls Spalte existiert, sonst `NULL AS preferred_coach_id`
  - `SELECT id AS request_id, discord_user_id, discord_username, rank, subrank, hero, games_played, hours_played, availability, current_problems, <preferred_coach_select> FROM coaching_requests WHERE notify_discord_at IS NULL AND status IN ('pending', 'analyzed', 'open', 'new') ORDER BY created_at ASC`
- Pro Request-Row:
  - `_upsert_coachee`
- Commit nur wenn `request_rows` nicht leer, wegen Coachee-Upserts.

Business-Logik und Seiteneffekte:

- GET ist nicht read-only: `request_created`-Polling kann `coachees` anlegen/aktualisieren.
- Appointment-Notifications werden nicht markiert; Markierung passiert nur in Ack.
- Reminder erscheint erst nach Created-Ack, weil `notify_created_at IS NOT NULL` Pflicht ist.
- Cancelled erscheint nur, wenn der Termin vorher created-geackt wurde.
- Request-Items werden genau einmal ueber `notify_discord_at` geackt.

Bot-Datenfluss heute:

- Rust pollt alle 60 Sekunden.
- Appointment-Items werden als DM an `discord_user_id` zugestellt:
  - `created`: Termin geplant, inkl. Dauer/Titel/Notiz und Link zur Coaching-Seite.
  - `reminder`: Start in unter 2 Stunden.
  - `cancelled`: Absage.
- Wenn DM deaktiviert (`50007`), acked Rust trotzdem, damit kein Endlos-Retry entsteht.
- `request_created` geht nicht per DM, sondern an `RequestNotificationSink`: bestehendes Discord-Request-Embed mit Claim/Release/Link. Nur nach erfolgreichem Post wird request-geackt.

### 22. `POST /api/coaching/platform/notifications/ack`

Auth: `require_bot_token`.

Request: `AckPayload`.

Response: `{ "ok": true, "acked": number }`

SQL:

- Fuer jedes `items[]`:
  - `type -> column`:
    - `created -> notify_created_at`
    - `reminder -> notify_reminder_at`
    - `cancelled -> notify_cancelled_at`
  - unbekannter Typ: skip
  - `UPDATE coaching_appointments SET <column>=? WHERE id=?`
- Fuer jedes `request_ids[]`:
  - `UPDATE coaching_requests SET notify_discord_at=? WHERE id=? AND notify_discord_at IS NULL`

Business-Logik:

- Timestamp `ts=_iso()` wird fuer alle Items im Request geteilt.
- Appointment-`acked` wird pro bekanntem Item hochgezaehlt, unabhaengig von `rowcount`; ein unbekannter Termin zaehlt trotzdem als acked.
- Request-`acked` nutzt `rowcount`, ist also idempotent.
- Rust sendet Appointment-Acks und Request-Acks getrennt:
  - `{ "items": [...] }`
  - `{ "request_ids": [...] }`

### 23. `GET /api/coaching/platform/coaches/me`

Auth: `require_coach_user`. Identitaet: `discord_id=int(user["sub"])`.

Request: kein Body.

Response: `coaches *` plus parsed Felder:

- Alle Spalten aus `coaches`: `id`, `discord_user_id`, `discord_username`, `display_name`, `avatar_url`, `bio`, `specialties_json`, `availability_json`, `status`, `avg_rating`, `total_reviews`, `total_sessions`, `created_at`, `updated_at`, `twitch_url`.
- Zusaetzlich/ueberschrieben:
  - `specialties: list` aus JSON `specialties_json`, bei Parse-Fehler `[]`
  - `availability: object` aus JSON `availability_json`, bei Parse-Fehler `{}`

SQL:

- `SELECT * FROM coaches WHERE discord_user_id=?`

Fehler:

- nicht-numerische `sub`: `404 Coach-Profil nicht gefunden`
- keine Coach-Zeile: `404 Coach-Profil nicht gefunden`

Seiteneffekte: keine.

### 24. `PATCH /api/coaching/platform/coaches/me`

Auth: `require_coach_user`. Identitaet: `discord_id=int(user["sub"])`.

Request: `CoachProfileUpdate`.

Response: `{ "ok": true }`

SQL:

- `SELECT id FROM coaches WHERE discord_user_id=?`
- Wenn Felder gesetzt:
  - dynamisch `UPDATE coaches SET bio=?, specialties_json=?, twitch_url=?, updated_at=? WHERE discord_user_id=?`
  - `specialties_json=json.dumps(body.specialties)` wenn gesetzt.
- Wenn kein Feld gesetzt: kein Update.

Fehler:

- nicht-numerische `sub`: `404 Coach-Profil nicht gefunden`
- keine Coach-Zeile: `404 Coach-Profil nicht gefunden`

Business-Logik:

- `display_name`, `avatar_url`, `discord_username`, `availability_json` koennen hier nicht vom Coach editiert werden.
- `specialties` wird als JSON-String gespeichert.

## `ai_summary` - alle relevanten Lese-/Schreibstellen

Im Router `coaching_platform.py`:

- Request-Feld: `SyncPayload.ai_summary`.
- `POST /platform/sync`, Bot-ID-Upsert:
  - UPDATE schreibt `coaching_requests.ai_summary = payload.ai_summary`.
  - INSERT schreibt `coaching_requests.ai_summary = payload.ai_summary`.
- `GET /platform/queue` liest es indirekt ueber `SELECT * FROM coaching_requests` und liefert es an alte Frontend-Typen aus.

Aus importierter/benachbarter `coaching.py`-Referenz:

- `CoachingRequestCreate.ai_summary`, `CoachingRequest.ai_summary`.
- `_request_from_row` liest `row["ai_summary"]`.
- `POST /api/coaching/requests` kann `ai_summary` in `coaching_requests` schreiben und im Response zurueckgeben.

Aus heutigem Rust-Bot:

- `MirrorRow.ai_summary` liest die Bot-DB und `mirror_to_website` sendet `ai_summary`.
- `RequestCreatedNotification.request_data()` setzt `ai_summary` bewusst leer.
- Neue Rust-Port-Vorgabe laut Design: `ai_summary` und `ai_insights_json` nicht portieren. Paritaetsentscheidung: alte Response-Felder entweder kontrolliert `null` liefern oder Frontend/Bridge vorher bereinigen.

## Appointment-, Reservation- und Rollenlogik

- Queue-Reservation:
  - Bot setzt bei Auto-Assign `reserved_until = now + CLAIM_RESERVATION_HOURS * 3600`, aktuell 24h.
  - Website-Queue zeigt nur eigene Reservierungen oder unreservierte Requests.
  - Website berechnet `is_open`, aber fremde abgelaufene Reservierungen kommen wegen SQL-Filter nicht in die Route. Freigabe passiert im Bot per `expire_reservations`: `status='analyzed' AND assigned_coach_id IS NOT NULL AND reserved_until < now` -> Request wieder offen.
- Appointment-Cutoffs in Python:
  - Coach-Terminliste: `scheduled_at >= now - 7 days`, alle Stati.
  - Spieler-`/me`: scheduled ab `now - 6 hours`; done/cancelled aus der Historie, dann letzte 5.
  - Coachee-Detail: alle Termine des Coachees, absteigend.
- Rollen-Haltedauer im heutigen Rust-Bot:
  - `ROLE_EXPIRY_HOURS = 168` (1 Woche).
  - Bei Claim: `expires_at = existing role_expires_at OR now + 168h`.
  - `expire_roles` entfernt `COACHING_ACTIVE_ROLE_ID`, wenn `role_expires_at < now`.
  - Kommentare im Rust-Code nennen noch 48h, aber die Konstante ist 168h. Nicht den Kommentar portieren.
- Port-Anforderung:
  - Coaching-Rolle soll 1 Woche gehalten werden und am Termin ausgerichtet sein. Python setzt diese Rollen nicht; die neue Rust-In-Process-Implementierung muss Appointment-`scheduled_at` und Role-Expiry zusammenfuehren, z. B. `role_expires_at = scheduled_at + 7 days` oder mindestens nicht vor dem Termin.

## Groesste Paritaetsrisiken

1. `GET /notifications/due` hat Schreib-Seiteneffekte. Ein Rust-Port, der Due als reine Query baut, vergisst den Coachee-Upsert fuer `request_created`; dann fehlen Website-Links `/coaching/coachees/{coachee_id}` im Discord-Embed.

2. Queue-/Reservation-Logik ist zweigeteilt. Python filtert fremde Reservierungen heraus, selbst wenn `reserved_until` abgelaufen ist; die eigentliche Freigabe macht der Bot. Beim In-Process-Port muss diese Freigabe weiterhin laufen, sonst bleiben Requests unsichtbar.

3. Rollen- und Appointment-Zeitfenster sind nicht dieselbe Logik. Python kennt 7-Tage/6h-Cutoffs nur fuer Listen, die Discord-Rolle lebt im Bot ueber `ROLE_EXPIRY_HOURS=168`. Der neue Port soll die Rolle am Termin ausrichten; wer nur Python nachbaut oder den alten Kommentar "48h" kopiert, entfernt die Rolle zu frueh.

Weitere Risiken:

- `GoalUpdate` setzt `completed_at=NULL` bei jedem Patch ohne `status="done"`.
- `notifications_ack` zaehlt Appointment-Acks ohne `rowcount`, Request-Acks aber idempotent per `rowcount`.
- `bot_session_id` wird vom Rust-Bot gesendet, aber von Python `SyncPayload` nicht genutzt.
- `scope != "mine"` bei `GET /appointments` bedeutet fuer jeden Coach "alle Termine".
- Admin/Caddy-Sub ist nicht numerisch; `_acting_coach_id` liefert dann `NULL`.

## Beruehrte DB-Tabellen

Website:

- `meta_users`
- `coaches`
- `coachees`
- `coaching_requests`
- `coaching_sessions`
- `coaching_goals`
- `coaching_milestones`
- `session_notes`
- `coaching_appointments`

Bot-Kontext fuer aktuelle HTTP-Bruecke:

- `coaching_requests`
- `coaching_sessions`
- `coaching_coach_rotation`
