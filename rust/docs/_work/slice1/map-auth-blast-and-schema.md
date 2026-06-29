# Slice 1: Auth-Blast-Radius und Coaching-Schema

Stand: 2026-06-29. Arbeitsmodus: Read-only Bestandsaufnahme, keine Git-Befehle, keine Secret-Werte gelesen.

## Kurzantworten

- Auth ueber `/coaching` hinaus genutzt: **JA**, aber nur in der alten/gebuendelten `Website/builds/frontend`-Source: dort haengt der globale `AuthProvider` an `/api/auth/*` und die App enthaelt neben Coaching auch Home, Heroes, Tierlists, Patchnotes, History, Feedback und Admin. Im Python-Backend selbst werden `require_authenticated_user`, `require_admin_user` und `require_coach_user` nur von `/api/coaching/*`-Routern konsumiert.
- Dokumentierte Tabellen: **10/10** (`coachees`, `coaches`, `coach_applications`, `coach_reviews`, `coaching_requests`, `coaching_appointments`, `coaching_goals`, `coaching_milestones`, `coaching_sessions`, `coaching_surveys`).
- DB-Paritaets-Fallen: **JA**. Rust muss Typstrings, Defaults, NULL-Verhalten, doppelte idempotente ALTERs, `TEXT` vs `TIMESTAMP`, `INTEGER` Discord-IDs, nullable CHECK-Felder und fehlende explizite Indizes exakt nachbilden.

## Suchumfang

- Python-Backend: `/home/naniadm/Documents/Website/builds/backend/app/` inklusive aller Router.
- Frontends: `dl-coaching`, `dl-landing`, `dl-tierlist`, `dl-patch`, `dl-activity`, `deco-elevator-new`, plus `builds/frontend` als Dashboard/Legacy-Frontend-Source.
- Ausgeschlossen bei Source-Suche: `.git`, `node_modules`, `dist`, `.next`, `build`, `__pycache__`.
- Migrationen/SQL-Dateien fuer die geforderten Coaching-Tabellen: keine gefunden. Die DDL kommt aus `Website/builds/backend/app/database.py`.

## A) Auth-Blast-Radius

### Backend: Auth-Provider und Router-Mount

| Datei | Treffer | Zweck | Scope/Cutover-Folge |
|---|---:|---|---|
| `Website/builds/backend/app/main.py:24` | `app.include_router(auth.router, prefix="/api/auth")` | Mountet Python-Auth auf `/api/auth`. | Muss beim Umzug nach Rust dl-etage ersetzt/proxy-kompatibel bleiben. |
| `Website/builds/backend/app/routers/auth.py:16` | `ddc_session` | Session-Cookie-Name, Default via `AUTH_COOKIE_NAME`. | Rust muss denselben Cookie lesen/setzen/loeschen oder Cutover bricht eingeloggte Clients. |
| `Website/builds/backend/app/routers/auth.py:186-213` | `/api/auth/discord/callback` | Ermittelt public base path fuer den OAuth-Callback. | Pfad-/Prefix-Verhalten ist kritisch, weil Caddy `/coaching` strippt. |
| `Website/builds/backend/app/routers/auth.py:368-443` | `get_current_user_optional`, `require_authenticated_user`, `require_admin_user`, `require_coach_user` | Decodiert Cookie/JWT, laedt Rolle aus `meta_users`, prueft Admin/Coach. | Rust muss `me`, Role-Lookup und Coach-Gate gleich liefern. |
| `Website/builds/backend/app/routers/auth.py:446-550` | `/discord/login`, `/discord/callback`, `/me`, `/logout` | Delegierter Discord-OAuth-Flow, Session-Cookie setzen/loeschen, Me-Endpoint. | Direkter Cutover-Pfad fuer `/api/auth/*`. |
| `Website/scripts/run_builds_backend.sh:62-67` | Callback und Port 8772 | Setzt `AUTH_PUBLIC_CALLBACK_URL` auf `/coaching/api/auth/discord/callback`, startet Python auf `127.0.0.1:8772`. | Belegt aktuellen Python-Origin und `/coaching`-Callback-Prefix. |

### Backend: Konsumenten der Python-Auth-Helper

Andere Backend-Router (`admin.py`, `builds.py`, `heroes.py`, `history.py`, `items.py`, `patchnotes.py`, `tierlists.py`) hatten keine Treffer auf `require_authenticated_user`, `require_admin_user`, `require_coach_user`, `get_current_user(_optional)` oder `ddc_session`.

| Datei | Treffer | Zweck | Scope/Cutover-Folge |
|---|---:|---|---|
| `Website/builds/backend/app/routers/coaching.py:14` | Import `require_admin_user`, `require_authenticated_user` | Auth-Gates fuer klassische Coaching-Routen. | Nur `/api/coaching/*`. |
| `Website/builds/backend/app/routers/coaching.py:215-219` | `_authenticate_coaching_request` -> `require_authenticated_user` wenn kein Bot-Token | Website-User duerfen Coaching-Requests selbst anlegen; Bot nutzt internen Token. | Nur Coaching-Anfrage. |
| `Website/builds/backend/app/routers/coaching.py:299-355` | Coach-Profil und Bewerbung brauchen `require_authenticated_user` | Profil darf nur fuer eigene Discord-ID erstellt werden; Bewerbung ebenso. | Nur Coaching. |
| `Website/builds/backend/app/routers/coaching.py:605-608` | Coach-Dashboard braucht `require_authenticated_user` | Privates altes Coach-Dashboard. | Nur Coaching-Dashboard. |
| `Website/builds/backend/app/routers/coaching.py:667-674` | Bewerbungsreview braucht `require_admin_user` | Admin approve/reject fuer Coach-Bewerbungen. | Nur Coaching-Admin unter `/api/coaching/admin/*`. |
| `Website/builds/backend/app/routers/coaching_platform.py:23-25` | Import `require_authenticated_user`, `require_coach_user` | Neue Plattform nutzt Coach-/User-Gates. | Nur `/api/coaching/platform/*`. |
| `Website/builds/backend/app/routers/coaching_platform.py:238-296` | `require_coach_user` | Overview, Queue, Coachee-Liste. | Coach-Bereich. |
| `Website/builds/backend/app/routers/coaching_platform.py:327-578` | `require_coach_user` | Coachee-Detail, Profil-Update, Goals, Milestones, Notes. | Coach-Bereich. |
| `Website/builds/backend/app/routers/coaching_platform.py:590-592` | `require_authenticated_user` | Spieler sieht eigene Coaching-Sicht. | User-Bereich `/coaching/me`. |
| `Website/builds/backend/app/routers/coaching_platform.py:748-822` | `require_coach_user` | Termine erstellen/listen/aendern. | Coach-Bereich. |
| `Website/builds/backend/app/routers/coaching_platform.py:1042-1077` | `require_coach_user` | Coach-Eigenprofil lesen/aendern. | Coach-Bereich. |

### Frontend: `/api/auth/*`-Konsumenten

| Datei | Treffer | Zweck | Scope/Cutover-Folge |
|---|---:|---|---|
| `Website/dl-coaching/src/context/AuthContext.tsx:14-47` | `${BASE_URL}/api/auth/me`, `/discord/login`, `/logout`, `credentials: include` | Zentrale Auth fuer die Coaching-Etage. | Nur `dl-coaching`; bricht Login, Logout, User-Status, Coach/Admin-Flags. |
| `Website/dl-coaching/src/api/client.ts:3-24` | Alle API-Requests mit `credentials: include`; Auth-Wrapper `me`/`logout`. | Cookie wird fuer alle Coaching-API-Calls mitgeschickt. | Nur `dl-coaching`. |
| `Website/dl-coaching/src/api/client.ts:280-323` | Coaching-Plattform-Client | Ruft auth-geschuetzte `/coaching/platform/*`-Endpunkte. | Nur `dl-coaching`, Coach-/User-Bereich. |
| `Website/dl-coaching/src/components/Layout.tsx:3-85` | `useAuth`, Login/Logout, Coach-Navigation | Sichtbarkeit von `Mein Coaching` und Coach-Bereich. | Nur `dl-coaching`. |
| `Website/dl-coaching/src/pages/CoachingRequestPage.tsx:24-43,50-62` | `useAuth`, `login`, `coaching.createRequest` | Coaching-Anfrage verlangt Website-Login. | Nur `dl-coaching`. |
| `Website/dl-coaching/src/pages/MyCoachingPage.tsx` | `useAuth` | Eigene Coaching-Sicht. | Nur `dl-coaching`. |
| `Website/dl-coaching/src/pages/CoachDashboardPage.tsx`, `CoachOverviewPage.tsx`, `CoacheeDetailPage.tsx` | `useAuth`, `isCoach` | Coach-only UI-Gates. | Nur `dl-coaching`. |
| `Website/builds/frontend/src/context/AuthContext.tsx:14-47` | `${BASE_URL}/api/auth/me`, `/discord/login`, `/logout` | Legacy/gebuendelter AuthProvider fuer ganze App. | **Nicht nur Coaching**: betrifft auch Dashboard/Admin/Tierlists/Feedback, falls dieses Frontend deployed ist. |
| `Website/builds/frontend/src/App.tsx:23-46` | Routen unter globalem Layout | App enthaelt Home, Heroes, Tierlists, Patchnotes, History, Feedback, Admin und Coaching. | **Blast-Radius ueber `/coaching` hinaus** in dieser Source. |
| `Website/builds/frontend/src/components/Layout.tsx:1-90` | `useAuth`, Login/Logout, `isAdmin`, `isCoach` | Header/Navigation fuer ganze Legacy-App. | **Dashboard/Landing-artige Meta-App + Coaching**, nicht nur Coaching. |
| `Website/builds/frontend/src/pages/AdminPage.tsx:1-31` | `useAuth`, `isAdmin` | Admin-Panel wird clientseitig ueber `/api/auth/me`-Rolle gated. | **Admin/Dashboard**, nicht `/coaching`. |
| `Website/builds/frontend/src/pages/FeedbackPage.tsx`, `TierListsPage.tsx` | `useAuth` | User-Kontext fuer Feedback/Tierlists. | **Nicht nur Coaching**. |

### Frontend: Auth-aehnliche Treffer, aber nicht Python `/api/auth`/`ddc_session`

| Datei | Treffer | Zweck | Scope/Cutover-Folge |
|---|---:|---|---|
| `Website/dl-tierlist/src/admin.js:3-4,56-58,805-807,966-973,1249-1261` | `/auth/discord/login`, `/api/admin/me`, `credentials: include` | Tierlist-Admin prueft eigene Admin-Session ueber `/api/admin/me`, Login geht zu `/auth/discord/login`. | Tierlist-Admin, aber kein `/api/auth/*`-Konsument und kein `ddc_session`-Treffer. |
| `Website/dl-tierlist/admin/index.html:30` | `/builds/auth/discord/login?next=/builds/admin/` | Statischer Login-Link fuer Tierlist-Admin. | Tierlist-Admin, anderer Auth-Pfad. |
| `Website/dl-tierlist/API-CONTRACT.md:8-9` | `master_dash_session`, `/auth/discord/login`, `/auth/discord/callback` | Dokumentiert separaten Deadlock-Bots Auth-Flow. | Nicht Python `/api/auth`; relevant nur falls Proxy-Pfade beim Cutover vereinheitlicht werden. |
| `Website/dl-activity/src/activity.js:33-35,152-174` | `VITE_AUTH_BASE`, `/auth/discord/login`, `/auth/discord/logout`, `/api/public/me` | Aktivitaetsseite nutzt separaten Auth-/Public-Me-Flow. | Nicht Python `/api/auth`; kein `ddc_session`-Treffer. |

### Frontend: Negativtreffer

- `Website/dl-landing`: keine Treffer fuer `api/auth`, `/auth/discord`, `ddc_session`, `useAuth`, `AuthProvider`, `require_authenticated_user`, `get_current_user`.
- `Website/dl-patch`: keine Treffer.
- `Website/deco-elevator-new`: keine Treffer.

### Cutover-Fazit Auth

1. Wenn nur die aktuelle `dl-coaching`-Etage an Rust dl-etage umzieht, liegt der direkte `/api/auth/*`-Blast-Radius bei Coaching-Login, Coaching-Anfrage, Coach-Bereich, Spieler-`/me` und Coaching-Admin.
2. Wenn `Website/builds/frontend` noch deployed oder als Dashboard relevant ist, ist der Blast-Radius groesser: Admin, Tierlists, Feedback und globale Header-Auth haengen ebenfalls an `/api/auth/*`.
3. `dl-tierlist` und `dl-activity` muessen separat betrachtet werden: sie nutzen Auth, aber nicht die hier gesuchte Python-`/api/auth`/`ddc_session`-Schiene.

## B) DB-DDL fuer Rust Schema-on-Boot

Quelle: `Website/builds/backend/app/database.py:170-369`.

Keine expliziten `CREATE INDEX`-Statements fuer die zehn Tabellen gefunden. Es existieren nur SQLite-Autoindizes aus `PRIMARY KEY` und `UNIQUE` Constraints.

### Globale ALTER-Nachruestungen

Diese Statements laufen nach den `CREATE TABLE IF NOT EXISTS`-Bloebecken und werden bei Fehlern geschluckt. Einige Spalten (`preferred_coach_id`, `notify_discord_at`) existieren in neuen Tabellen bereits im `CREATE TABLE`; fuer alte DBs bleiben die ALTERs relevant.

```sql
ALTER TABLE coaching_requests ADD COLUMN assigned_coach_id TEXT;
ALTER TABLE coaching_requests ADD COLUMN assigned_coach_username TEXT;
ALTER TABLE coaching_requests ADD COLUMN reserved_until INTEGER;
ALTER TABLE coaching_requests ADD COLUMN preferred_coach_id TEXT;
ALTER TABLE coaching_requests ADD COLUMN notify_discord_at TIMESTAMP;
ALTER TABLE coaching_sessions ADD COLUMN coachee_id TEXT;
ALTER TABLE coaching_requests ADD COLUMN bot_request_id INTEGER;
ALTER TABLE coaching_sessions ADD COLUMN bot_session_id TEXT;
ALTER TABLE coaches ADD COLUMN twitch_url TEXT;
```

### `coaches`

```sql
CREATE TABLE IF NOT EXISTS coaches (
    id TEXT PRIMARY KEY,
    discord_user_id INTEGER UNIQUE NOT NULL,
    discord_username TEXT,
    display_name TEXT,
    avatar_url TEXT,
    bio TEXT,
    specialties_json TEXT DEFAULT '[]',
    availability_json TEXT DEFAULT '{}',
    status TEXT DEFAULT 'active',
    avg_rating REAL DEFAULT 0,
    total_reviews INTEGER DEFAULT 0,
    total_sessions INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `discord_user_id` | `INTEGER` | ja | - | `UNIQUE` |
| `discord_username` | `TEXT` | nein | - | - |
| `display_name` | `TEXT` | nein | - | - |
| `avatar_url` | `TEXT` | nein | - | - |
| `bio` | `TEXT` | nein | - | - |
| `specialties_json` | `TEXT` | nein | `'[]'` | JSON-Text |
| `availability_json` | `TEXT` | nein | `'{}'` | JSON-Text |
| `status` | `TEXT` | nein | `'active'` | - |
| `avg_rating` | `REAL` | nein | `0` | - |
| `total_reviews` | `INTEGER` | nein | `0` | - |
| `total_sessions` | `INTEGER` | nein | `0` | - |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `updated_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `twitch_url` | `TEXT` | nein | - | per ALTER |

### `coach_reviews`

```sql
CREATE TABLE IF NOT EXISTS coach_reviews (
    id TEXT PRIMARY KEY,
    coach_id TEXT REFERENCES coaches(id),
    session_id TEXT,
    user_display_name TEXT,
    rating INTEGER CHECK(rating >= 0 AND rating <= 10),
    feedback_text TEXT,
    improved_areas TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `coach_id` | `TEXT` | nein | - | `REFERENCES coaches(id)` |
| `session_id` | `TEXT` | nein | - | - |
| `user_display_name` | `TEXT` | nein | - | - |
| `rating` | `INTEGER` | nein | - | `CHECK(rating >= 0 AND rating <= 10)` |
| `feedback_text` | `TEXT` | nein | - | - |
| `improved_areas` | `TEXT` | nein | - | - |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

### `coaching_requests`

```sql
CREATE TABLE IF NOT EXISTS coaching_requests (
    id TEXT PRIMARY KEY,
    discord_user_id INTEGER NOT NULL,
    discord_username TEXT,
    rank TEXT NOT NULL DEFAULT '',
    subrank TEXT NOT NULL DEFAULT '',
    hero TEXT,
    games_played TEXT,
    hours_played TEXT,
    availability TEXT,
    current_problems TEXT,
    preferred_coach_id TEXT,
    ai_summary TEXT,
    ai_insights_json TEXT,
    status TEXT DEFAULT 'pending',
    notify_discord_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `discord_user_id` | `INTEGER` | ja | - | Discord-ID als Integer |
| `discord_username` | `TEXT` | nein | - | - |
| `rank` | `TEXT` | ja | `''` | - |
| `subrank` | `TEXT` | ja | `''` | - |
| `hero` | `TEXT` | nein | - | - |
| `games_played` | `TEXT` | nein | - | - |
| `hours_played` | `TEXT` | nein | - | - |
| `availability` | `TEXT` | nein | - | - |
| `current_problems` | `TEXT` | nein | - | - |
| `preferred_coach_id` | `TEXT` | nein | - | auch per ALTER fuer Alt-DBs |
| `ai_summary` | `TEXT` | nein | - | **AI legacy: bleibt in DB, Rust nicht befuellen/surfacen** |
| `ai_insights_json` | `TEXT` | nein | - | **AI legacy: bleibt in DB, Rust nicht befuellen/surfacen** |
| `status` | `TEXT` | nein | `'pending'` | - |
| `notify_discord_at` | `TIMESTAMP` | nein | - | auch per ALTER fuer Alt-DBs |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `updated_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `assigned_coach_id` | `TEXT` | nein | - | per ALTER; im Sync als Discord-ID-String genutzt |
| `assigned_coach_username` | `TEXT` | nein | - | per ALTER |
| `reserved_until` | `INTEGER` | nein | - | per ALTER; Unix-Timestamp |
| `bot_request_id` | `INTEGER` | nein | - | per ALTER |

### `coaching_sessions`

```sql
CREATE TABLE IF NOT EXISTS coaching_sessions (
    id TEXT PRIMARY KEY,
    request_id TEXT REFERENCES coaching_requests(id),
    coach_id TEXT REFERENCES coaches(id),
    discord_user_id INTEGER NOT NULL,
    discord_username TEXT,
    discord_channel_id INTEGER,
    status TEXT DEFAULT 'active',
    scheduled_at TIMESTAMP,
    started_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `request_id` | `TEXT` | nein | - | `REFERENCES coaching_requests(id)` |
| `coach_id` | `TEXT` | nein | - | `REFERENCES coaches(id)` |
| `discord_user_id` | `INTEGER` | ja | - | Discord-ID als Integer |
| `discord_username` | `TEXT` | nein | - | - |
| `discord_channel_id` | `INTEGER` | nein | - | - |
| `status` | `TEXT` | nein | `'active'` | - |
| `scheduled_at` | `TIMESTAMP` | nein | - | - |
| `started_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `completed_at` | `TIMESTAMP` | nein | - | - |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `coachee_id` | `TEXT` | nein | - | per ALTER |
| `bot_session_id` | `TEXT` | nein | - | per ALTER |

### `coaching_surveys`

```sql
CREATE TABLE IF NOT EXISTS coaching_surveys (
    id TEXT PRIMARY KEY,
    session_id TEXT REFERENCES coaching_sessions(id) UNIQUE,
    rating INTEGER CHECK(rating >= 0 AND rating <= 10),
    feedback_text TEXT,
    improved_areas TEXT,
    unresolved_items TEXT,
    would_recommend INTEGER,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `session_id` | `TEXT` | nein | - | `REFERENCES coaching_sessions(id) UNIQUE` |
| `rating` | `INTEGER` | nein | - | `CHECK(rating >= 0 AND rating <= 10)` |
| `feedback_text` | `TEXT` | nein | - | - |
| `improved_areas` | `TEXT` | nein | - | - |
| `unresolved_items` | `TEXT` | nein | - | - |
| `would_recommend` | `INTEGER` | nein | - | Boolean als `0/1/NULL` |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

### `coach_applications`

```sql
CREATE TABLE IF NOT EXISTS coach_applications (
    id TEXT PRIMARY KEY,
    discord_user_id INTEGER UNIQUE NOT NULL,
    discord_username TEXT,
    display_name TEXT,
    application_text TEXT,
    experience_text TEXT,
    rank TEXT,
    specialties_json TEXT DEFAULT '[]',
    availability_json TEXT DEFAULT '{}',
    status TEXT DEFAULT 'pending',
    reviewed_by TEXT,
    reviewed_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `discord_user_id` | `INTEGER` | ja | - | `UNIQUE` |
| `discord_username` | `TEXT` | nein | - | - |
| `display_name` | `TEXT` | nein | - | - |
| `application_text` | `TEXT` | nein | - | - |
| `experience_text` | `TEXT` | nein | - | - |
| `rank` | `TEXT` | nein | - | - |
| `specialties_json` | `TEXT` | nein | `'[]'` | JSON-Text |
| `availability_json` | `TEXT` | nein | `'{}'` | JSON-Text |
| `status` | `TEXT` | nein | `'pending'` | - |
| `reviewed_by` | `TEXT` | nein | - | - |
| `reviewed_at` | `TIMESTAMP` | nein | - | - |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `updated_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

### `coachees`

```sql
CREATE TABLE IF NOT EXISTS coachees (
    id TEXT PRIMARY KEY,
    discord_user_id INTEGER UNIQUE NOT NULL,
    discord_username TEXT,
    display_name TEXT,
    rank TEXT,
    main_heroes_json TEXT DEFAULT '[]',
    current_focus TEXT,
    notes TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `discord_user_id` | `INTEGER` | ja | - | `UNIQUE` |
| `discord_username` | `TEXT` | nein | - | - |
| `display_name` | `TEXT` | nein | - | - |
| `rank` | `TEXT` | nein | - | - |
| `main_heroes_json` | `TEXT` | nein | `'[]'` | JSON-Text |
| `current_focus` | `TEXT` | nein | - | - |
| `notes` | `TEXT` | nein | - | Coach-intern; Plattform gibt sie in `/platform/me` bewusst nicht aus |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `updated_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

### `coaching_goals`

```sql
CREATE TABLE IF NOT EXISTS coaching_goals (
    id TEXT PRIMARY KEY,
    coachee_id TEXT REFERENCES coachees(id),
    coach_id TEXT REFERENCES coaches(id),
    session_id TEXT,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT DEFAULT 'open',
    sort_order INTEGER DEFAULT 0,
    target_date TEXT,
    completed_at TIMESTAMP,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `coachee_id` | `TEXT` | nein | - | `REFERENCES coachees(id)` |
| `coach_id` | `TEXT` | nein | - | `REFERENCES coaches(id)` |
| `session_id` | `TEXT` | nein | - | Keine FK |
| `title` | `TEXT` | ja | - | - |
| `description` | `TEXT` | nein | - | - |
| `status` | `TEXT` | nein | `'open'` | - |
| `sort_order` | `INTEGER` | nein | `0` | - |
| `target_date` | `TEXT` | nein | - | - |
| `completed_at` | `TIMESTAMP` | nein | - | - |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `updated_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

### `coaching_milestones`

```sql
CREATE TABLE IF NOT EXISTS coaching_milestones (
    id TEXT PRIMARY KEY,
    goal_id TEXT REFERENCES coaching_goals(id),
    title TEXT NOT NULL,
    description TEXT,
    achieved INTEGER DEFAULT 0,
    achieved_at TIMESTAMP,
    sort_order INTEGER DEFAULT 0,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `goal_id` | `TEXT` | nein | - | `REFERENCES coaching_goals(id)` |
| `title` | `TEXT` | ja | - | - |
| `description` | `TEXT` | nein | - | - |
| `achieved` | `INTEGER` | nein | `0` | Boolean als `0/1` |
| `achieved_at` | `TIMESTAMP` | nein | - | - |
| `sort_order` | `INTEGER` | nein | `0` | - |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

### `coaching_appointments`

```sql
CREATE TABLE IF NOT EXISTS coaching_appointments (
    id TEXT PRIMARY KEY,
    coach_id TEXT REFERENCES coaches(id),
    coachee_id TEXT REFERENCES coachees(id),
    scheduled_at TEXT NOT NULL,
    duration_minutes INTEGER DEFAULT 60,
    title TEXT,
    note TEXT,
    status TEXT DEFAULT 'scheduled',
    notify_created_at TEXT,
    notify_reminder_at TEXT,
    notify_cancelled_at TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)
```

| Spalte | Typ | NOT NULL | DEFAULT | Constraints/Notizen |
|---|---|---:|---|---|
| `id` | `TEXT` | nein | - | `PRIMARY KEY` |
| `coach_id` | `TEXT` | nein | - | `REFERENCES coaches(id)` |
| `coachee_id` | `TEXT` | nein | - | `REFERENCES coachees(id)` |
| `scheduled_at` | `TEXT` | ja | - | ISO-8601 Text |
| `duration_minutes` | `INTEGER` | nein | `60` | - |
| `title` | `TEXT` | nein | - | - |
| `note` | `TEXT` | nein | - | - |
| `status` | `TEXT` | nein | `'scheduled'` | Werte im Code: `scheduled`, `done`, `cancelled` |
| `notify_created_at` | `TEXT` | nein | - | ISO-Text |
| `notify_reminder_at` | `TEXT` | nein | - | ISO-Text |
| `notify_cancelled_at` | `TEXT` | nein | - | ISO-Text |
| `created_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |
| `updated_at` | `TIMESTAMP` | nein | `CURRENT_TIMESTAMP` | - |

## Paritaets-Fallen fuer Rust

1. **Idempotenz:** Python fuehrt `CREATE TABLE IF NOT EXISTS` aus und danach blind `ALTER TABLE ADD COLUMN`, Fehler werden ignoriert. Rust sollte bei geteilter SQLite-DB entweder `PRAGMA table_info` pruefen oder duplicate-column-Fehler gezielt ignorieren.
2. **Doppelte Spalten im Bootpfad:** `preferred_coach_id` und `notify_discord_at` stehen im `CREATE TABLE coaching_requests` und werden trotzdem per ALTER versucht. Das ist Absicht fuer Alt-DBs.
3. **Typ-Affinity exakt halten:** `coaching_appointments.scheduled_at` und Notification-Stempel sind `TEXT`, waehrend viele andere Zeitfelder `TIMESTAMP` sind. Nicht vereinheitlichen.
4. **Discord-IDs:** `discord_user_id` ist in mehreren Tabellen `INTEGER`, aber `assigned_coach_id` ist `TEXT` und wird im Sync als Discord-ID-String gespeichert, nicht als FK auf `coaches.id`.
5. **Defaults/NOT NULL:** `coaching_requests.rank` und `subrank` sind `TEXT NOT NULL DEFAULT ''`; Inserts duerfen dort kein `NULL` schreiben.
6. **AI-Spalten:** `ai_summary` und `ai_insights_json` bleiben in `coaching_requests`. Rust soll sie nicht droppen; laut Port-Vorgabe nicht befuellen/surfacen.
7. **Keine expliziten Indizes:** Es gibt nur PK/UNIQUE-Autoindizes. Rust sollte fuer Koexistenz keine zusaetzlichen Indizes erzwingen, ausser separat freigegeben.
8. **Foreign Keys ohne ON DELETE und Python setzt kein `PRAGMA foreign_keys=ON`:** Rust sollte nicht ploetzlich strengere FK-Semantik aktivieren, wenn das laufende Python daneben andere Annahmen hat.
9. **CHECKs nullable:** Ratings in `coach_reviews` und `coaching_surveys` haben `CHECK(rating >= 0 AND rating <= 10)`, aber kein `NOT NULL`.
10. **JSON-Spalten sind plain TEXT:** `specialties_json`, `availability_json`, `main_heroes_json` haben Text-Defaults und keine JSON-Constraint.
