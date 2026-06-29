# Public Coaching Router Inventory

Quelle: `/home/naniadm/Documents/Website/builds/backend/app/routers/coaching.py` (708 Zeilen), plus `database.py`, `auth.py`, `main.py`, `coaching_platform.py` und Frontend-Typ-Crosscheck.

## Mount-Prefix

- FastAPI mountet `coaching.router` in `app/main.py:32` mit Prefix `/api/coaching`.
- `coaching_platform.router` ist in `app/main.py:33` ebenfalls unter `/api/coaching` gemountet.
- Die Coaching-Frontend-App laeuft unter `/coaching/`; ihr API-Base ist `/coaching/api`, das lokal/proxyseitig auf `/api` umgeschrieben wird. Effektiv wird z. B. `/coaching/api/coaching/coaches` zu Backend `/api/coaching/coaches`.

## Gemeinsame Helper und Auth

- `get_db()` aus `app/database.py`: gibt einen `_DBProxy` auf die globale `aiosqlite`-Connection zurueck; `close()` ist ein No-op. Alle Router rufen trotzdem `await db.close()` im `finally`.
- `require_authenticated_user(request)` aus `auth.py`: liest Session-Cookie/JWT oder lokalen Forward-Auth-Admin-Header und liefert Dict mit `id`, `username`, `displayName`, `avatarUrl`, `role`, `sub`. Ohne User: 401.
- `require_admin_user(request)`: `require_authenticated_user`, dann `role == "admin"`, sonst 403.
- `require_bot_token(request)` in `coaching.py`: akzeptiert Header `X-Internal-Token` oder `X-Bot-Token`; vergleicht gegen den ersten gesetzten internen Token aus den bekannten Env-Namen. Ohne konfigurierte Token-Quelle: 503; falscher Header: 401.
- `_authenticate_coaching_request(request)`: wenn Bot-Token-Header vorhanden ist, Bot-Auth und Identitaet aus Body; sonst Website-Session und Identitaet aus Session.

## Pydantic-Modelle

### CoachProfileBase

- `display_name: str`
- `bio: Optional[str] = None`
- `specialties: list[str] = []`
- `availability: dict = {}`

### CoachProfileCreate

Erbt `CoachProfileBase` plus:

- `discord_user_id: int`
- `discord_username: str`
- `avatar_url: Optional[str] = None`

### CoachProfile Response

Exakte Python-Felder:

- `id: str`
- `display_name: str`
- `discord_username: str`
- `avatar_url: Optional[str]`
- `bio: Optional[str]`
- `specialties: list[str]`
- `availability: dict`
- `status: str`
- `avg_rating: float`
- `total_reviews: int`
- `total_sessions: int`
- `twitch_url: Optional[str] = None`

Mapping: `_coach_from_row` parst `specialties_json` zu `specialties`, `availability_json` zu `availability`; fehlerhafte JSON-Werte werden zu `[]`/`{}`. `display_name` faellt auf `discord_username` zurueck. `avg_rating`, `total_reviews`, `total_sessions` fallen auf `0` zurueck. `twitch_url` wird nur gesetzt, wenn die Spalte im Row-Objekt existiert.

### CoachReview Response

- `id: str`
- `coach_id: str`
- `user_display_name: str`
- `rating: int`
- `feedback_text: Optional[str]`
- `improved_areas: Optional[str]`
- `created_at: str`

Mapping: `user_display_name` faellt auf `"Anonymous"` zurueck.

### CoachingRequestCreate

Aktueller Python-Input:

- `id: Optional[str] = None`
- `display_name: Optional[str] = None` (im Router nicht verwendet)
- `discord_user_id: Optional[int] = None`
- `discord_username: Optional[str] = None`
- `rank: str`
- `subrank: str = ""`
- `hero: Optional[str] = None`
- `games_played: Optional[str] = None`
- `hours_played: Optional[str] = None`
- `availability: Optional[str] = None`
- `current_problems: Optional[str] = None`
- `preferred_coach_id: Optional[str] = None`
- `ai_summary: Optional[str] = None` (Rust-Port: entfernen/ignorieren)
- `ai_insights_json: Optional[str] = None` (Rust-Port: entfernen/ignorieren)

### CoachingRequest Response

Aktuelle Python-Felder:

- `id: str`
- `discord_username: str`
- `rank: str`
- `subrank: str`
- `hero: Optional[str]`
- `games_played: Optional[str]`
- `hours_played: Optional[str]`
- `availability: Optional[str]`
- `current_problems: Optional[str]`
- `ai_summary: Optional[str]` (Rust-Port: nicht mehr zurueckgeben)
- `status: str`
- `created_at: str`

Gewolltes Rust-minus-AI-Response-Modell:

- `id: str`
- `discord_username: str`
- `rank: str`
- `subrank: str`
- `hero: Option<String>`
- `games_played: Option<String>`
- `hours_played: Option<String>`
- `availability: Option<String>`
- `current_problems: Option<String>`
- `status: String`
- `created_at: String`

### CoachApplicationCreate

- `discord_user_id: int`
- `discord_username: str`
- `display_name: str`
- `application_text: str`
- `experience_text: str`
- `rank: str`
- `specialties: list[str] = []`
- `availability: dict = {}`

### SurveySubmit

- `session_id: str`
- `rating: int`
- `feedback_text: Optional[str] = None`
- `improved_areas: Optional[str] = None`
- `unresolved_items: Optional[str] = None`
- `would_recommend: Optional[bool] = None`

## Datenbankschema im Scope

`database.py` legt diese Tabellen/Spalten an:

- `coaches`: `id`, `discord_user_id`, `discord_username`, `display_name`, `avatar_url`, `bio`, `specialties_json`, `availability_json`, `status`, `avg_rating`, `total_reviews`, `total_sessions`, `created_at`, `updated_at`; per ALTER zusaetzlich `twitch_url`.
- `coach_reviews`: `id`, `coach_id`, `session_id`, `user_display_name`, `rating`, `feedback_text`, `improved_areas`, `created_at`.
- `coaching_requests`: `id`, `discord_user_id`, `discord_username`, `rank`, `subrank`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `preferred_coach_id`, `ai_summary`, `ai_insights_json`, `status`, `notify_discord_at`, `created_at`, `updated_at`; per ALTER zusaetzlich `assigned_coach_id`, `assigned_coach_username`, `reserved_until`, `bot_request_id`.
- `coaching_sessions`: `id`, `request_id`, `coach_id`, `discord_user_id`, `discord_username`, `discord_channel_id`, `status`, `scheduled_at`, `started_at`, `completed_at`, `created_at`; per ALTER zusaetzlich `coachee_id`, `bot_session_id`.
- `coaching_surveys`: `id`, `session_id`, `rating`, `feedback_text`, `improved_areas`, `unresolved_items`, `would_recommend`, `created_at`.
- `coach_applications`: `id`, `discord_user_id`, `discord_username`, `display_name`, `application_text`, `experience_text`, `rank`, `specialties_json`, `availability_json`, `status`, `reviewed_by`, `reviewed_at`, `created_at`, `updated_at`.

## Routeninventar

### 1. `GET /api/coaching/coaches`

- Auth: keine.
- Query: `specialty: Optional[str] = None`, `min_rating: Optional[float] = None`.
- Request-Model: keins.
- Response: `list[CoachProfile]`.
- SQL:
  - Basis: `SELECT * FROM coaches WHERE status='active'`
  - Wenn `specialty`: `AND specialties_json LIKE ?`, Param `"%{specialty}%"`
  - Wenn `min_rating`: `AND avg_rating >= ?`
  - Order: `ORDER BY avg_rating DESC, total_sessions DESC`
- Logik/Seiteneffekte: read-only; JSON-Felder werden geparst; nur aktive Coaches.
- Paritaetsnotiz: `if min_rating:` ignoriert `0.0` als Filter.

### 2. `GET /api/coaching/coaches/{coach_id}`

- Auth: keine.
- Path: `coach_id: str`.
- Request-Model: keins.
- Response: `CoachProfile`.
- SQL: `SELECT * FROM coaches WHERE id=?`.
- Logik/Seiteneffekte: 404 wenn nicht gefunden; kein `status='active'`-Filter, also auch inaktive Profile koennen per ID kommen.

### 3. `GET /api/coaching/coaches/{coach_id}/reviews`

- Auth: keine.
- Path: `coach_id: str`.
- Request-Model: keins.
- Response: `list[CoachReview]`.
- SQL: `SELECT * FROM coach_reviews WHERE coach_id=? ORDER BY created_at DESC`.
- Logik/Seiteneffekte: keine Coach-Existenzpruefung; bei unbekanntem Coach leere Liste.

### 4. `POST /api/coaching/coaches/profile`

- Auth: `require_authenticated_user(request)`.
- Identitaet: `user_data["sub"]` muss als String exakt `profile.discord_user_id` entsprechen; sonst 403.
- Request-Model: `CoachProfileCreate`.
- Response: `CoachProfile`.
- SQL:
  - Existenz: `SELECT id FROM coaches WHERE discord_user_id=?`
  - Wenn vorhanden:
    - `UPDATE coaches SET display_name=?, bio=?, specialties_json=?, availability_json=?, avatar_url=?, updated_at=CURRENT_TIMESTAMP WHERE id=?`
  - Sonst:
    - `INSERT INTO coaches (id, discord_user_id, discord_username, display_name, avatar_url, bio, specialties_json, availability_json, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'active')`
  - Rueckgabe: `SELECT * FROM coaches WHERE id=?`
- Logik/Seiteneffekte: erstellt oder aktualisiert Coach-Profil; `specialties` und `availability` werden als JSON-String gespeichert; neue Coaches sind sofort `active`; ID via `secrets.token_urlsafe(16)`.
- Kein Discord-/Notification-Effekt.

### 5. `POST /api/coaching/coaches/apply`

- Auth: `require_authenticated_user(request)`.
- Identitaet: `user_data["sub"]` muss `application.discord_user_id` entsprechen; sonst 403.
- Request-Model: `CoachApplicationCreate`.
- Response ohne response_model:
  - Neu/erneut nach rejected: `{ "id": str, "status": "pending", "message": "Application submitted" }`
  - Bereits pending/approved: `{ "id": str, "status": str, "message": "Application already submitted" }`
- SQL:
  - `SELECT id, status FROM coach_applications WHERE discord_user_id=?`
  - Wenn existing und `status in ("approved", "pending")`: kein Write.
  - Wenn existing rejected:
    - `UPDATE coach_applications SET application_text=?, experience_text=?, rank=?, specialties_json=?, availability_json=?, status='pending', updated_at=CURRENT_TIMESTAMP WHERE id=?`
  - Sonst:
    - `INSERT INTO coach_applications (id, discord_user_id, discord_username, display_name, application_text, experience_text, rank, specialties_json, availability_json, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending')`
- Logik/Seiteneffekte: `rejected -> pending` Reapply; `pending`/`approved` sind idempotent; keine Coach-Zeile bis Admin-Approval.

### 6. `POST /api/coaching/requests`

- Auth: `_authenticate_coaching_request(request)`.
- Identitaet:
  - Bot: Header `X-Internal-Token` oder `X-Bot-Token`; Body muss `discord_user_id` und `discord_username` enthalten, sonst 422.
  - Website-Session: `discord_user_id` aus `user_data["sub"]`, Username aus `user_data["username"]` oder `user_data["displayName"]`; Body-Discord-Felder werden ignoriert.
- Request-Model: `CoachingRequestCreate`.
- Response: `CoachingRequest`.
- SQL:
  - Schema-Check: `PRAGMA table_info(coaching_requests)`.
  - Insert-Spalten aktuell:
    - `id`, `discord_user_id`, `discord_username`, `rank`, `subrank`, `hero`, `games_played`, `hours_played`, `availability`, `current_problems`, `ai_summary`, `ai_insights_json`, `status`
    - plus `preferred_coach_id`, wenn Body gesetzt und Spalte existiert.
  - SQL dynamisch: `INSERT INTO coaching_requests (<insert_columns>) VALUES (<placeholders>)`.
- Logik/Seiteneffekte:
  - `request_id = req.id or token_urlsafe(16)`.
  - `rank = req.rank or ""`, `subrank = req.subrank or ""`.
  - Status immer `pending`.
  - Response wird aus Input gebaut, nicht aus DB neu gelesen; `created_at=datetime.utcnow().isoformat()`, nicht DB-Default.
  - `display_name` wird nicht benutzt.
  - `notify_discord_at` bleibt DB-Default `NULL`; `coaching_platform.notifications_due` erzeugt spaeter daraus `request_created` Bot-Notification, `notifications_ack` setzt dann `notify_discord_at`.
- Rust-minus-AI:
  - Request-Struct ohne `ai_summary`/`ai_insights_json`.
  - Insert ohne beide Spalten; DB-Spalten bleiben `NULL`.
  - Response ohne `ai_summary`.

### 7. `GET /api/coaching/requests`

- Auth: keine, trotz Kommentar "internal/bot use".
- Query: `status: Optional[str] = None`.
- Request-Model: keins.
- Response: `list[CoachingRequest]`.
- SQL:
  - Basis: `SELECT * FROM coaching_requests`
  - Wenn `status`: `WHERE status=?`
  - Order: `ORDER BY created_at DESC`
- Logik/Seiteneffekte: read-only; `_request_from_row` reduziert `SELECT *` auf das Response-Modell. `discord_user_id`, `preferred_coach_id`, `assigned_*`, `reserved_until`, `notify_discord_at`, `ai_insights_json` werden nicht zurueckgegeben.
- Rust-minus-AI: `ai_summary` nicht mappen und nicht zurueckgeben.

### 8. `PATCH /api/coaching/requests/{request_id}/match`

- Auth: `Depends(require_bot_token)`.
- Identitaet: keine User-Identitaet, nur gueltiger interner Token.
- Path: `request_id: str`.
- Query/Form-Parameter laut FastAPI-Signatur:
  - `coach_id: str` required
  - `discord_channel_id: Optional[int] = None`
- Request-Model: keins.
- Response: `{ "status": "matched", "session_id": str }`.
- SQL:
  - `SELECT * FROM coaching_requests WHERE id=?`
  - `INSERT INTO coaching_sessions (id, request_id, coach_id, discord_user_id, discord_username, discord_channel_id, status) VALUES (?, ?, ?, ?, ?, ?, 'active')`
  - `UPDATE coaching_requests SET status='matched', updated_at=CURRENT_TIMESTAMP WHERE id=?`
- Logik/Seiteneffekte:
  - 404 wenn Request fehlt.
  - Kein Check, ob Coach existiert/active ist.
  - Keine Idempotenz: mehrfacher Match kann mehrere Sessions fuer denselben Request erzeugen.
  - Request-Status `pending/analyzed/... -> matched`.
  - Session-Status neu `active`.
  - `discord_channel_id` wird nur gespeichert; keine direkte Discord-Aktion.

### 9. `POST /api/coaching/surveys`

- Auth: `Depends(require_bot_token)`.
- Identitaet: keine User-Identitaet, nur interner Token.
- Request-Model: `SurveySubmit`.
- Response: `{ "status": "stored", "survey_id": str }`.
- SQL:
  - `SELECT * FROM coaching_sessions WHERE id=?`
  - `INSERT INTO coaching_surveys (id, session_id, rating, feedback_text, improved_areas, unresolved_items, would_recommend) VALUES (?, ?, ?, ?, ?, ?, ?)`
  - `UPDATE coaching_sessions SET status='completed', completed_at=CURRENT_TIMESTAMP WHERE id=?`
  - Stats:
    - `SELECT AVG(rating) as avg, COUNT(*) as cnt FROM coaching_surveys WHERE session_id IN (SELECT id FROM coaching_sessions WHERE coach_id=?)`
  - Public Review:
    - `INSERT INTO coach_reviews (id, coach_id, session_id, user_display_name, rating, feedback_text, improved_areas) VALUES (?, ?, ?, ?, ?, ?, ?)`
  - Coach Stats:
    - `UPDATE coaches SET avg_rating=?, total_reviews=?, total_sessions=total_sessions+1, updated_at=CURRENT_TIMESTAMP WHERE id=?`
- Logik/Seiteneffekte:
  - 404 wenn Session fehlt.
  - `would_recommend`: `True -> 1`, `False -> 0`, `None -> NULL`.
  - Session `active -> completed`.
  - Erstellt anonymisierte Public Review: `Coachee #<sha256(discord_user_id+coach_id)[:6]>`, bei fehlender ID `"Anonym"`.
  - Aktualisiert Coach-Rating/Review-Count und inkrementiert `total_sessions`, aber nur wenn `stats["avg"]` truthy ist; ein Durchschnitt von `0` wuerde das Update ueberspringen.
  - `coaching_surveys.session_id` ist UNIQUE; Duplicate Survey erzeugt DB-Fehler.

### 10. `GET /api/coaching/dashboard`

- Auth: `require_authenticated_user(request)`.
- Identitaet: `int(user_data["sub"])` wird gegen `coaches.discord_user_id` gematcht.
- Request-Model: keins.
- Response ohne response_model:
  - Wenn kein Coach: `{ "profile": null, "sessions": [], "reviews": [], "applications": [] }`
  - Wenn Coach: `{ "profile": CoachProfile, "sessions": list[raw coaching_sessions row], "reviews": list[CoachReview] }`
- SQL:
  - `SELECT * FROM coaches WHERE discord_user_id=?`
  - `SELECT * FROM coaching_sessions WHERE coach_id=? ORDER BY created_at DESC LIMIT 10`
  - `SELECT * FROM coach_reviews WHERE coach_id=? ORDER BY created_at DESC LIMIT 5`
- Raw Session-Felder (Schema + ALTER): `id`, `request_id`, `coach_id`, `discord_user_id`, `discord_username`, `discord_channel_id`, `status`, `scheduled_at`, `started_at`, `completed_at`, `created_at`, optional `coachee_id`, optional `bot_session_id`.
- Logik/Seiteneffekte:
  - Read-only.
  - `applications`-Key existiert nur in der No-Coach-Antwort, nicht in der normalen Coach-Antwort.
  - Nicht-numerisches `sub` kann bei `int(user_data["sub"])` einen ungefangenen Fehler ausloesen.

### 11. `POST /api/coaching/sessions/{session_id}/end`

- Auth: `Depends(require_bot_token)`.
- Identitaet: keine User-Identitaet, nur interner Token.
- Path: `session_id: str`.
- Request-Model: keins.
- Response: `{ "status": "completed" }`.
- SQL: `UPDATE coaching_sessions SET status='completed', completed_at=CURRENT_TIMESTAMP WHERE id=?`.
- Logik/Seiteneffekte:
  - Session-Status wird auf `completed` gesetzt.
  - Keine Existenzpruefung; auch bei 0 betroffenen Zeilen kommt `{"status":"completed"}`.
  - Kein Survey, kein Review, kein Coach-Rating-Update, kein Request-Status-Update.

### 12. `PATCH /api/coaching/admin/applications/{application_id}`

- Auth: `require_admin_user(request)`.
- Identitaet: Admin-Session; `user_data["sub"]` wird als `reviewed_by` gespeichert.
- Path: `application_id: str`.
- Query: `status: str` required; erlaubt nur `"approved"` oder `"rejected"`, sonst 400.
- Request-Model: keins.
- Response: `{ "status": "approved" | "rejected" }`.
- SQL:
  - `SELECT * FROM coach_applications WHERE id=?`
  - Wenn `status == "approved"`:
    - `INSERT INTO coaches (id, discord_user_id, discord_username, display_name, specialties_json, availability_json, status) VALUES (?, ?, ?, ?, ?, ?, 'active')`
  - Immer:
    - `UPDATE coach_applications SET status=?, reviewed_by=?, reviewed_at=CURRENT_TIMESTAMP WHERE id=?`
- Logik/Seiteneffekte:
  - 404 wenn Application fehlt.
  - Approved erstellt sofort aktiven Coach aus Application-Daten.
  - Kein Check auf bereits existierende Coach-Zeile; wegen `coaches.discord_user_id UNIQUE` kann ein erneutes Approve/Parallelzustand einen DB-Fehler werfen.
  - Rejected erstellt keinen Coach.

## AI-Entfernung: alle produktiven Backend-Stellen

Ziel fuer Rust: AI-Felder duerfen wegen geteilter SQLite-Spalten existieren, aber nicht angenommen, nicht geschrieben und nicht zurueckgegeben werden. Serde kann unbekannte JSON-Felder standardmaessig ignorieren; falls ein strikter Extractor genutzt wird, explizit Kompatibilitaet entscheiden.

| Datei:Zeile | Feld | Art | Rust-Massnahme |
|---|---|---|---|
| `app/database.py:216` | `ai_summary` | DB-Spalte in `coaching_requests` | Spalte/Migration darf bleiben; Public-Port nicht lesen/schreiben/surfacen. |
| `app/database.py:217` | `ai_insights_json` | DB-Spalte in `coaching_requests` | Spalte/Migration darf bleiben; Public-Port nicht lesen/schreiben/surfacen. |
| `app/routers/coaching.py:73` | `ai_summary` | Input `CoachingRequestCreate` | Aus Rust-Request-Model entfernen; eingehende unbekannte Felder ignorieren, nicht speichern. |
| `app/routers/coaching.py:74` | `ai_insights_json` | Input `CoachingRequestCreate` | Aus Rust-Request-Model entfernen; eingehende unbekannte Felder ignorieren, nicht speichern. |
| `app/routers/coaching.py:87` | `ai_summary` | Response `CoachingRequest` | Aus Rust-Response entfernen. |
| `app/routers/coaching.py:162` | `ai_summary` | `_request_from_row` DB -> Response | Nicht aus Row mappen; altes DB-Material bleibt verborgen. |
| `app/routers/coaching.py:432` | `ai_summary`, `ai_insights_json` | Insert-Spalten `POST /requests` | Beide Spalten aus Insert-Liste entfernen. |
| `app/routers/coaching.py:437` | `ai_summary`, `ai_insights_json` | Insert-Werte `POST /requests` | Keine Werte binden; DB bleibt `NULL`/alt. |
| `app/routers/coaching.py:464` | `ai_summary` | direkte `POST /requests` Response | Feld nicht in Response bauen. |
| `app/routers/coaching_platform.py:113` | `ai_summary` | Bot-Sync-Input `SyncPayload` | Fuer Rust-Port/Plattform-Sync entfernen oder als ignoriertes Kompatibilitaetsfeld behandeln; nicht persistieren. |
| `app/routers/coaching_platform.py:176` | `ai_summary` | Bot-Sync Update-Spalte | Assignment aus `UPDATE coaching_requests` entfernen. |
| `app/routers/coaching_platform.py:182` | `ai_summary` | Bot-Sync Update-Wert | Wert nicht binden. |
| `app/routers/coaching_platform.py:190` | `ai_summary` | Bot-Sync Insert-Spalte | Spalte aus `INSERT coaching_requests` entfernen. |
| `app/routers/coaching_platform.py:196` | `ai_summary` | Bot-Sync Insert-Wert | Wert nicht binden. |

Frontend-/Test-Crosscheck:

- `Website/builds/frontend/src/api/client.ts:143` und `:208` typisieren `ai_summary`; `CoachDashboardPage.tsx:59-60` zeigt es an. Das scheint generierter/alter Build zu sein.
- `Website/dl-coaching/src/api/client.ts:64-76` und `:141-159` enthalten in den aktuellen Source-Typen kein `ai_summary` mehr. Das passt zur gewollten AI-Entfernung.
- `Website/builds/backend/tests/test_appointments.py:148-149` legt Test-Schema mit beiden DB-Spalten an; fuer Rust-Kompatibilitaet koennen Schema-Spalten weiter existieren.

## Beziehung zu `coaching_platform.py`

Beide Router teilen `/api/coaching` und dieselbe SQLite. Wichtig fuer 1:1-Port:

- `coaches`
  - Public-Router schreibt in `POST /coaches/profile`, `PATCH /admin/applications/{id}` und aktualisiert Stats in `POST /surveys`.
  - Platform-Router schreibt in `_upsert_coach`, `/platform/coaches/sync`, `/platform/coaches/me`.
  - `auth.require_coach_user` prueft `coaches.discord_user_id` + `status='active'`.
- `coaching_requests`
  - Public-Router erstellt in `POST /requests`, liest in `GET /requests`, setzt `status='matched'` in `PATCH /requests/{id}/match`.
  - Platform-Router spiegelt Bot-Zustand in `/platform/sync`: Status, Assignment, Reservation, optional Session.
  - Platform-Router liest Queue (`status='analyzed'`) und Notification-Due (`status IN ('pending','analyzed','open','new')`), ack setzt `notify_discord_at`.
  - AI-Altspalten werden aktuell von Public und Platform beschrieben; Rust soll sie nicht mehr beschreiben.
- `coaching_sessions`
  - Public-Router erstellt active Session beim Match; markiert completed in Survey und Session-End.
  - Platform-Router spiegelt Bot-Sessions in `/platform/sync`, liest sie fuer Overview, Coachee-Detail und My-Coaching.
- `coach_reviews`
  - Nur Public-Survey erzeugt Public Reviews; Public Coach-Review und Dashboard lesen sie.
- `coaching_surveys`
  - Public-Survey schreibt; Public-Survey nutzt sie zur Rating-Aggregation.
- `coach_applications`
  - Public Apply schreibt/aktualisiert; Public Admin Review liest/aktualisiert und erstellt daraus `coaches`.
- `coachees`
  - Platform-Router erzeugt/aktualisiert Coachees bei `/platform/sync` und sogar in `GET /platform/notifications/due` fuer noch nicht bestaetigte Requests. Public-Router schreibt diese Tabelle nicht direkt.

Notification-/Discord-Kopplung:

- Public `POST /requests` loest nicht direkt Discord aus, hinterlaesst aber `notify_discord_at=NULL`; `coaching_platform.py` gibt diese Requests in `/platform/notifications/due` als `type="request_created"` an den Bot aus. Bot bestaetigt ueber `/platform/notifications/ack`.
- Public `PATCH /requests/{id}/match` speichert optional `discord_channel_id`, versendet aber selbst nichts.
- Survey/Session-End haben keine direkte Notification-Queue-Wirkung.

## Paritaetsrisiken

1. `GET /requests` ist unauthentifiziert und gibt alle Requests mit `discord_username` und Problemtext aus. Falls Rust hier absichert, ist es ein API-Vertragsbruch; falls Rust offen bleibt, ist es ein bewusst zu tragendes Legacy-Risiko.
2. AI-Entfernung muss an allen Schreib- und Readback-Pfaden passieren, nicht nur im Public-Request-Model. Sonst koennen alte `ai_summary`-Werte ueber `GET /requests` oder Platform-Queue wieder auftauchen.
3. Mehrere Endpunkte sind nicht idempotent oder validieren Fremdschluessel nicht: Match kann Mehrfach-Sessions erzeugen, Admin-Approve kann gegen `coaches.discord_user_id UNIQUE` laufen, Survey-Duplikate treffen `coaching_surveys.session_id UNIQUE`.
4. Dashboard-Response ist inkonsistent: No-Coach enthaelt `applications`, Coach-Fall nicht. Raw `sessions` haengen vom SQLite-Schema/ALTER-Stand ab.
5. `POST /requests` gibt `created_at` aus App-Zeit zurueck, nicht aus DB; ein Rust-Port, der nach Insert refetcht, kann sichtbare Timestamp-Abweichungen erzeugen.

