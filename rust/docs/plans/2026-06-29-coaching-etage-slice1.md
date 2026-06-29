# Coaching-Etage Slice 1 - Sichtbarkeit, Web-Eingang, Plattform-Port und Auth

Datum: 2026-06-29  
Status: Implementierungsplan zur Freigabe nach Plan-Rework  
Scope: Planung fuer `Deadlock-Bots/rust` plus Frontend-Anpassungen in `Website/dl-coaching`

Gelesene Faktenbasis:

- Spec: `rust/docs/specs/2026-06-29-coaching-scrim-program-design.md`
- Vorlage: `rust/docs/plans/2026-06-29-coaching-etage-slice0.md`
- Inventare: `rust/docs/_work/slice1/map-coaching-platform.md`, `map-coaching-public.md`, `map-auth.md`, `map-auth-blast-and-schema.md`, `map-frontend.md`, `map-infra-rust.md`
- Kritik: `rust/docs/_work/slice1/plan-critique.md`; alle HIGH/MEDIUM/LOW-Befunde sind akzeptiert und in den betroffenen Tickets umgesetzt.

## Ziel

Slice 1 ersetzt die Coaching-Etage fuer Browser und Bot-internen Plattformfluss durch Rust, ohne den Python-Dienst `:8772` sofort abzuschalten. Der neue Dienst `dl-etage` laeuft auf `DL_ETAGE_PORT=8773`. Erst wenn `dl-etage` voll gebaut und verifiziert ist, wird Caddy fuer die Coaching-API umgebogen. Python `:8772` bleibt fuer Meta-Routen und als geplanter Fallback verfuegbar.

## Definition of Done

- `dl-etage` bedient Auth, public Coaching, Coaching-Platform und Scrim-Slice-1-Routen auf `127.0.0.1:8773`.
- `dl-etage` nutzt exakt dieselbe physische SQLite-Datei wie das live Python-Backend; Device/Inode und `PRAGMA database_list` sind vor Live-Bootstrap bewiesen.
- Login funktioniert ohne Zwangs-Re-Login: bestehende Python-Session-Cookies werden von Rust akzeptiert, und Rust-Cookies bleiben Python-kompatibel.
- Spieler sehen ihr Scrim-Team und das naechste Match.
- Neue Scrim-Anmeldungen kommen ueber ein Web-Formular in `scrim_participant` mit `source='web_form'`.
- Coaches sehen den Scrim-Pool mit Status-Filter. Slice 1 enthaelt keine Coach-Pool-Statusmutation.
- `ai_summary` und `ai_insights_json` bleiben als Legacy-DB-Spalten erhalten, werden aber von Rust nicht angenommen, nicht geschrieben und nicht zurueckgegeben.
- Bot-Coach-Sync, Notifications und Request-/Session-Mirror koennen hinter Feature-Flag in-process gegen die geteilte DB laufen. Die alten HTTP-Loops bleiben bis nach verifiziertem Live-Cutover fallbackfaehig.
- Coaching-Rolle wird im Bot am Termin ausgerichtet fuer eine Woche gehalten: mit Termin `scheduled_at + 7d`, ohne Termin `now + 7d`, bestehende spaetere Expiry wird nie verkuerzt.
- Caddy-Cutover ist der letzte Live-Schritt von Slice 1. Python `:8772` bleibt fuer Meta und den geplanten Fallback bestehen.
- Entfernung der alten HTTP-Bridge-Loops ist ein eigenes Post-Cutover-Ticket S1-11.

## Entschiedene Parameter

| Parameter | Entscheidung | Begruendung |
|---|---|---|
| Dienst | `dl-etage` | Eigener Blast-Radius und eigenes Deploy-Tempo fuer Coaching. |
| Port | `DL_ETAGE_PORT=8773`, neues Feld `dl_core::Ports::etage` | Strangler-Fig: Python `:8772` bleibt parallel. |
| Host | `DL_ETAGE_HOST`, Default `127.0.0.1` | Host ist dienstspezifisch, nicht Teil von `dl_core::Ports`. |
| Service | systemd User-Service `deadlock-etage-rust.service` | Analog `deadlock-bot-rust.service` und `deadlock-web-rust.service`. |
| DB | Geteilte SQLite; `dl-etage` muss dieselbe physische Datei wie live Python nutzen | Koexistenz mit Python bis Slice 3. Kein Live-Schema-Bootstrap vor bestandenem Device/Inode-Check. |
| FK-Enforcement | Fuer den Coaching-Workload keine strengere Semantik als Python; `dl-etage`-Verbindung setzt `PRAGMA foreign_keys=OFF` oder belegt, dass sie nicht enforced | Python setzt kein `PRAGMA foreign_keys=ON`; Legacy-Flows duerfen orphan-faehige Inserts behalten. |
| Caddy | Cutover zuletzt; spezifische Handles fuer Auth/Coaching/Scrim/Health zu `:8773`, Fallback fuer Meta zu `:8772` | Das Infra-Inventar zeigt Meta-Routen unter demselben Prefix. |
| AI-Felder | DB-Spalten bleiben, API/Service ignoriert und surfaced sie nicht | Paritaet minus AI laut Spec. |
| Discord-ID-Matrix | `discord_user_id`, `discord_channel_id`, Bot-IDs = SQLite `INTEGER`/Rust `i64`; `assigned_coach_id` = `TEXT`; interne IDs = `TEXT` | Verhindert token-vs-i64-Bugklasse ohne Python-DDL zu veraendern. |
| Clippy | Immer scoped: `cargo clippy -p <geaenderte-crate> [-p ...] -- -D warnings` | Kein workspace-weites Alt-Schuld-Reparieren. |
| User-sichtbare deutsche Texte | Codex setzt nur `"Platzhalter"` und meldet Datei:Zeile; Claude schreibt final | Gilt fuer Frontend-Strings, Embeds, DMs und UI-Fehlertexte. |

## Auth-Wire-Contract

| Bereich | Rust-Vertrag |
|---|---|
| Session-Cookie | Name `ddc_session` via `AUTH_COOKIE_NAME`; `HttpOnly`; `Path=/`; `SameSite=lax`; `Max-Age=2592000`; `Secure` nach Python-Auto-Logik; Domain-Auto `deutsche-deadlock-community.de` fuer DDC-Hosts/Subdomains; host-only sonst. |
| Pre-Auth-Cookie | Name `ddc_pre_auth` via `AUTH_PRE_AUTH_COOKIE_NAME`; gleiche Attribute wie Session; `Max-Age=600`; JWT `kind=pre_auth`, `state_id`, `next`, `iat`, `exp`, `iss`, kein `aud`. |
| Legacy-Cookie | `auth_token` beim Lesen akzeptieren und bei Login/Logout in Host-only- und Domain-Variante loeschen. |
| JWT | `HS256`; Secret-Fallback exakt `AUTH_SESSION_SECRET -> JWT_SECRET -> SESSIONS_ENCRYPTION_KEY`; Claims `sub`, `username`, `display_name`, `avatar_url`, `role`, `iss`, `aud`, `iat`, `exp`. Keine Live-Secret-Werte in Tests, Logs oder Docs. |
| Decode | Erst mit Aud/Iss, dann Fallback ohne Aud/Iss-Pflichtpruefung. Kein harter Fail wegen fehlender/abweichender `aud` oder `iss`, wenn Signatur und `exp` passen. |
| Rollen | `discord_roles` aus OAuth-Consume-Payload wird fuer Builds-/Coaching-Auth ignoriert. `role` kommt dynamisch aus `meta_users.role`; JWT-`role` ist nur Fallback. |
| Coach-Gate | `is_coach=true`, wenn `role == "admin"` oder aktive Zeile in `coaches` mit `discord_user_id=sub` und `status='active'`; nicht-numerische `sub` scheitert wie Python, ausser beim separaten loopback Forward-Auth-Admin. |
| Forward-Auth-Admin | `X-Admin-Validated: 1` nur von localhost/127.0.0.1/::1 erzeugt Admin-User `caddy-validated-admin`; Remote-Header ist wirkungslos. |
| `/api/auth/me` | Nicht eingeloggt: `{ "user": null }`. Eingeloggt: `{ "user": { "id", "username", "displayName", "avatarUrl", "role", "is_coach" } }`; Shape exakt wie Frontend-Vertrag. |
| OAuth Login | `GET /api/auth/discord/login` normalisiert `next`: leer -> Default-Basepath, absolute URLs, `//`, CR/LF/NUL, Pfade ohne `/`, `..` -> Fallback; Query/Fragment bleiben. Danach zentraler `POST /internal/v1/discord/initiate` mit `scope="identify"`, `requesting_service="builds"`, `metadata.site="builds"`, `redirect_after=<callback_url>`, `X-Internal-Token` aus der bestehenden Internal-Token-Kette. |
| OAuth Callback | `GET /api/auth/discord/callback` liest Pre-Auth fuer `next`, loescht Pre-Auth-Cookie, bevorzugt Query-`state_id` vor Cookie-`state_id`, vergleicht Query und Cookie nicht hart, ruft zentral `POST /internal/v1/discord/consume-result`, ignoriert `discord_roles`, upsertet `meta_users`, setzt Session, loescht `auth_token`, redirectet. Ohne/ungueltige Pre-Auth oder Consume-Fehler: Redirect ohne Session. |
| Internal-Token-Kette | Website -> Dashboard nutzt die dokumentierten Token-Namen ohne Werte auszugeben: `WEBSITE_INTERNAL_API_TOKEN`, `TURNIER_INTERNAL_API_TOKEN`, `MASTER_BROKER_TOKEN`, `MAIN_BOT_INTERNAL_TOKEN`, `TWITCH_INTERNAL_API_TOKEN`. |
| Cross-Runtime-Test | Python erzeugt mit Test-Secret ueber Python-Logik ein JWT, Rust akzeptiert es. Rust erzeugt ein Session-JWT, Python decodiert es mit Python-Fallback ohne Aud/Iss-Pflicht. |

## DB-Paritaetsvertrag

- Live-Datei-Identitaet ist blockierend: `stat -Lc '%d:%i %n'` fuer Python-DB-Pfad und `dl-etage`-DB-Pfad muss identische `device:inode` zeigen; zusaetzlich wird `PRAGMA database_list` fuer die tatsaechlich geoeffnete DB dokumentiert.
- Kein `bootstrap_schema()` gegen Live-Konfig, solange der Datei-Identitaets-Beweis nicht bestanden ist.
- `dl-etage` darf fuer die geteilte Website-Coaching-DB keine strengere FK-Enforcement-Semantik als Python einfuehren. Wenn `dl-db` beim Oeffnen global `foreign_keys=ON` setzt, muss der Coaching-Workload in `dl-etage` `PRAGMA foreign_keys=OFF` setzen oder nachweislich eine nicht-enforcende Verbindung nutzen.
- Legacy-orphan-faehige Inserts sind Paritaet, kein Bugfix: Goals/Notes ohne Existenzcheck, Match mit unbekanntem Coach und Admin/Caddy-Aktionen mit `coach_id=NULL` duerfen nicht durch neue FK-Enforcement-Semantik brechen.
- Python-DDL-Paritaet ist wichtiger als "schoenes" neues Schema: Typ-Affinity, Defaults, nullable CHECKs, fehlende explizite Indizes und `TEXT` vs `TIMESTAMP` exakt respektieren.
- `db.bootstrap_schema()` muss mit laufendem Python koexistieren: `CREATE TABLE IF NOT EXISTS`, `column_exists` vor ALTER, duplicate-column-Fehler nicht als normale Strategie.

## Ticket-DAG

### S1-01 - `dl-etage` Scaffolding und Port-Konfiguration

ID: S1-01  
Titel: `dl-etage` Scaffolding und Port-Konfiguration

Scope:

- `rust/Cargo.toml`
- `rust/bin/dl-etage/Cargo.toml`
- `rust/bin/dl-etage/src/main.rs`
- `rust/crates/dl-core/src/config.rs`
- optional `rust/bin/dl-etage/src/config.rs`

Blockiert durch: keine.

Definition of Done:

- `rust/Cargo.toml` enthaelt `bin/dl-etage` als Workspace-Member und die noetigen Workspace-Dependencies.
- Vor jedem `cargo build/test/clippy -p dl-etage` ist die Package-Sichtbarkeit bewiesen:
  ```bash
  cargo metadata --format-version=1 --no-deps
  cargo package --list -p dl-etage
  ```
- `dl_core::Ports` hat `etage` mit ENV `DL_ETAGE_PORT` und Default `8773`.
- `dl-etage` startet lokal auf `DL_ETAGE_HOST:DL_ETAGE_PORT`.
- `GET /api/health` liefert eine einfache OK-Response.
- Der Startpfad enthaelt den fruehen DB-Datei-Identitaets-Check als live-blockierende Vorbedingung: Python-DB-Pfad und `dl-etage`-DB-Pfad werden ohne Secret-Werte verglichen, und `device:inode` plus `PRAGMA database_list` koennen ausgegeben/dokumentiert werden.
- Fuer lokale Test-DBs darf `db.bootstrap_schema().await` idempotent laufen. Fuer Live-Konfig ist Bootstrap bis S1-02-Preflight blockiert.
- Verifikation:
  ```bash
  cargo build -p dl-core -p dl-etage
  cargo test -p dl-core
  cargo test -p dl-etage health
  DL_ETAGE_PORT=8773 cargo run -p dl-etage
  curl -fsS http://127.0.0.1:8773/api/health
  ```

Out-of-Scope:

- Keine Auth-Routen.
- Keine Caddy- oder systemd-Aenderung.
- Kein Python-Abschalten.
- Kein Live-Schema-Bootstrap vor S1-02.

Clippy-Scope:

```bash
cargo clippy -p dl-core -p dl-etage -- -D warnings
```

User-sichtbare Texte: Nein.

### S1-02 - Python-paritaeres Coaching-Schema in `dl-db`

ID: S1-02  
Titel: Python-paritaeres Coaching-Schema in `dl-db`

Scope:

- `rust/docs/db-schema.sql`
- `rust/crates/dl-db/src/lib.rs`
- `rust/crates/dl-db/tests/*`
- falls benoetigt `rust/crates/dl-squads/tests/*`

Blockiert durch: S1-01.

Definition of Done:

- Blockierender Live-Preflight vor jedem `bootstrap_schema()` gegen Live-Konfig:
  ```bash
  stat -Lc '%d:%i %n' "$PYTHON_DB_PATH" "$DL_ETAGE_DB_PATH"
  sqlite3 "$PYTHON_DB_PATH" 'PRAGMA database_list;'
  sqlite3 "$DL_ETAGE_DB_PATH" 'PRAGMA database_list;'
  ```
  Beide `stat`-Zeilen muessen dieselbe `device:inode` zeigen. Wenn nicht, wird Live-Bootstrap abgebrochen. `dl-etage` muss exakt dieselbe physische SQLite-Datei wie live Python nutzen.
- Die von `dl-etage` fuer Coaching genutzte Verbindung erzwingt keine FK-Semantik, die Python nicht hat. DoD-Check:
  ```sql
  PRAGMA foreign_keys;
  ```
  Erwartung fuer den Coaching-Workload: `0`, oder dokumentierter Beweis, dass die konkret genutzte Verbindung nicht enforced.
- Paritaetstests fuer legacy-orphan-faehige Inserts laufen mit der `dl-etage`-DB-Verbindung: Goal ohne existierenden `coachee_id`, Note ohne existierenden `session_id`/`coachee_id`, Public-Match mit unbekanntem `coach_id`.
- Golden-DDL-Liste enthaelt final diese 12 Tabellen: die 10 Coaching/Platform-Tabellen `coaches`, `coach_reviews`, `coach_applications`, `coachees`, `coaching_requests`, `coaching_sessions`, `coaching_surveys`, `coaching_goals`, `coaching_milestones`, `coaching_appointments` plus `session_notes` plus `meta_users`.
- Exakte Golden-DDL:
  ```sql
  CREATE TABLE IF NOT EXISTS meta_users (
      id TEXT PRIMARY KEY,
      username TEXT,
      display_name TEXT,
      avatar_url TEXT,
      role TEXT DEFAULT 'user',
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );

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
  );

  CREATE TABLE IF NOT EXISTS coach_reviews (
      id TEXT PRIMARY KEY,
      coach_id TEXT REFERENCES coaches(id),
      session_id TEXT,
      user_display_name TEXT,
      rating INTEGER CHECK(rating >= 0 AND rating <= 10),
      feedback_text TEXT,
      improved_areas TEXT,
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );

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
  );

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
  );

  CREATE TABLE IF NOT EXISTS coaching_surveys (
      id TEXT PRIMARY KEY,
      session_id TEXT REFERENCES coaching_sessions(id) UNIQUE,
      rating INTEGER CHECK(rating >= 0 AND rating <= 10),
      feedback_text TEXT,
      improved_areas TEXT,
      unresolved_items TEXT,
      would_recommend INTEGER,
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );

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
  );

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
  );

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
  );

  CREATE TABLE IF NOT EXISTS coaching_milestones (
      id TEXT PRIMARY KEY,
      goal_id TEXT REFERENCES coaching_goals(id),
      title TEXT NOT NULL,
      description TEXT,
      achieved INTEGER DEFAULT 0,
      achieved_at TIMESTAMP,
      sort_order INTEGER DEFAULT 0,
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );

  CREATE TABLE IF NOT EXISTS session_notes (
      id TEXT PRIMARY KEY,
      session_id TEXT,
      coachee_id TEXT REFERENCES coachees(id),
      coach_id TEXT REFERENCES coaches(id),
      content TEXT,
      visibility TEXT DEFAULT 'coach_only',
      created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
      updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
  );

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
  );
  ```
- Idempotente ALTER-Nachruestungen bleiben erhalten: `coaching_requests.assigned_coach_id TEXT`, `assigned_coach_username TEXT`, `reserved_until INTEGER`, `preferred_coach_id TEXT`, `notify_discord_at TIMESTAMP`, `bot_request_id INTEGER`; `coaching_sessions.coachee_id TEXT`, `bot_session_id TEXT`; `coaches.twitch_url TEXT`.
- ID-Typ-Matrix ist verbindlich:
  - `discord_user_id`, `discord_channel_id`, Bot-IDs: SQLite `INTEGER`, Rust `i64`.
  - `assigned_coach_id`: SQLite/Rust `TEXT`, auch wenn der Inhalt eine Discord-ID sein kann.
  - interne IDs wie `coaches.id`, `coachees.id`, Goals, Milestones, Notes, Sessions, Appointments: `TEXT`.
- `ai_summary` und `ai_insights_json` bleiben in `coaching_requests`, werden aber von keinem Rust-API-/Service-Pfad befuellt oder surfaced.
- `scrim_participant.heroes` wird idempotent als `TEXT` ergaenzt, falls nicht vorhanden.
- Pro Golden-Tabelle werden Snapshots verglichen:
  ```sql
  PRAGMA table_info(<table>);
  PRAGMA foreign_key_list(<table>);
  PRAGMA index_list(<table>);
  ```
  Erwartung: Python-DDL, PK/UNIQUE-Autoindizes, keine neuen expliziten Indizes ohne separate Freigabe.
- Idempotenztest fuehrt `bootstrap_schema()` zweimal gegen dieselbe Test-DB aus.
- Verifikation:
  ```bash
  cargo test -p dl-db
  cargo test -p dl-squads schema
  ```

Out-of-Scope:

- Keine Datenmigration oder Datenbereinigung.
- Keine neuen Indizes ohne separate Freigabe.
- Kein Droppen von AI-Spalten.
- Kein FK-Strengerziehen gegen die geteilte Website-Coaching-DB.

Clippy-Scope:

```bash
cargo clippy -p dl-db -p dl-squads -- -D warnings
```

User-sichtbare Texte: Nein.

### S1-03 - `dl-auth`: Cookie/JWT/OAuth wire-kompatibel

ID: S1-03  
Titel: `dl-auth`: Cookie/JWT/OAuth wire-kompatibel

Scope:

- `rust/Cargo.toml`
- `rust/crates/dl-auth/Cargo.toml`
- `rust/crates/dl-auth/src/*`
- `rust/bin/dl-etage/src/main.rs`
- `rust/bin/dl-etage/src/config.rs`
- `rust/crates/dl-webcore` nur falls fuer `DashboardClient` kleine Anpassungen noetig sind

Blockiert durch: S1-01, S1-02.

Definition of Done:

- `rust/Cargo.toml` enthaelt `crates/dl-auth` als Workspace-Member und `dl-etage` haengt sauber daran.
- Vor Tests/Clippy ist Package-Sichtbarkeit bewiesen:
  ```bash
  cargo metadata --format-version=1 --no-deps
  cargo package --list -p dl-auth
  ```
- `GET /api/auth/me`, `GET /api/auth/discord/login`, `GET /api/auth/discord/callback`, `POST /api/auth/logout` sind in `dl-etage` gemountet.
- Session-Cookie, Pre-Auth-Cookie, Delete-Varianten und Legacy `auth_token` sind Python-kompatibel.
- Cookie-Testmatrix:
  - Set-Header fuer host-only.
  - Set-Header fuer DDC-Domain `deutsche-deadlock-community.de`.
  - Logout/Delete fuer host-only Session, host-only Pre-Auth und host-only `auth_token`.
  - Logout/Delete fuer DDC-Domain Session, DDC-Domain Pre-Auth und DDC-Domain `auth_token`.
  - Mindestens ein Test mit non-default `AUTH_COOKIE_NAME`, `AUTH_PRE_AUTH_COOKIE_NAME`, `AUTH_SESSION_TTL_SECONDS`, `AUTH_PRE_AUTH_TTL_SECONDS`, `AUTH_COOKIE_DOMAIN`.
- JWT-Decode akzeptiert bestehende Python-Cookies ohne Aud/Iss-Zwang.
- Echter Cross-Runtime-Test mit isolierter Test-Env und Test-Secret:
  - Python erzeugt mit der Python-Auth-Bibliothek/-Logik ein HS256-Session-JWT; Rust akzeptiert es als `ddc_session`.
  - Rust erzeugt ein HS256-Session-JWT; Python decodiert es mit Python-Fallback ohne Aud/Iss-Pflicht.
  - Keine Live-Secrets werden gelesen, ausgegeben oder in Dateien geschrieben.
- OAuth-Relay-Vertrag aus `map-auth.md` ist umgesetzt und getestet:
  - `next`-Normalisierung exakt wie Python.
  - Pre-Auth-Cookie `ddc_pre_auth` mit `kind=pre_auth`, `state_id`, `next`, `iat`, `exp`, `iss`, ohne `aud`.
  - Login ruft zentral `/internal/v1/discord/initiate` mit `scope="identify"`, `requesting_service="builds"`, `metadata.site="builds"`, `redirect_after=<callback_url>`.
  - Callback bevorzugt Query-`state_id` vor Cookie-`state_id`.
  - Callback erzwingt keinen Query/Cookie-State-Gleichheitsvergleich; Einmaligkeit kommt ueber `/internal/v1/discord/consume-result`.
  - Fehlerfaelle "Callback ohne Pre-Auth", "ungueltige Pre-Auth", "fehlender State", "Consume-Fehler" redirecten ohne Session und loeschen Pre-Auth.
  - Internal-Token-Kette nutzt nur Namen, keine Wertausgabe.
- Rollenvertrag ist umgesetzt und getestet:
  - `discord_roles` aus dem OAuth-Consume-Payload wird fuer Builds-/Coaching-Auth ignoriert.
  - `role` wird pro Request dynamisch aus `meta_users.role` geladen; JWT-`role` nur Fallback.
  - `is_coach` ist `true` bei `role == "admin"` oder aktiver `coaches`-Zeile (`discord_user_id`, `status='active'`).
  - Positiver und negativer Test beweisen, dass gelieferte `discord_roles` die Rolle nicht aendern.
- Loopback-Forward-Auth-Admin ist implementiert und getestet:
  - `X-Admin-Validated: 1` von localhost erzeugt Admin-User.
  - Derselbe Header von nicht-lokalem Client wird ignoriert.
- `GET /api/auth/me` liefert exakt den Frontend-Shape; nicht eingeloggt bleibt `{ "user": null }`.
- Verifikation bevorzugt Axum-Router-Tests ohne externen Port:
  ```bash
  cargo test -p dl-auth
  cargo test -p dl-etage auth
  ```

Out-of-Scope:

- Keine Discord-Guild-Rollen fuer Admin/Coach.
- Keine serverseitige Session-DB.
- Keine Meta-Auth-Umstellung ausserhalb `/coaching`.

Clippy-Scope:

```bash
cargo clippy -p dl-auth -p dl-etage -- -D warnings
```

User-sichtbare Texte: Ja, falls UI/API-Fehlertexte neu formuliert werden. Codex setzt `"Platzhalter"` und meldet Datei:Zeile; Claude finalisiert.

### S1-04 - `dl-mentoring`: Public-Coaching-Routen aus `coaching.py`

ID: S1-04  
Titel: `dl-mentoring`: Public-Coaching-Routen aus `coaching.py`

Scope:

- `rust/Cargo.toml`
- `rust/crates/dl-mentoring/Cargo.toml`
- `rust/crates/dl-mentoring/src/model.rs`
- `rust/crates/dl-mentoring/src/store.rs`
- `rust/crates/dl-mentoring/src/public_routes.rs`
- `rust/crates/dl-mentoring/tests/*`
- `rust/bin/dl-etage/src/main.rs`

Blockiert durch: S1-02, S1-03.

Zu portierende Routen:

- `GET /api/coaching/coaches`
- `GET /api/coaching/coaches/{id}`
- `GET /api/coaching/coaches/{id}/reviews`
- `POST /api/coaching/coaches/profile`
- `POST /api/coaching/coaches/apply`
- `POST /api/coaching/requests`
- `GET /api/coaching/requests`
- `PATCH /api/coaching/requests/{id}/match`
- `POST /api/coaching/surveys`
- `GET /api/coaching/dashboard`
- `POST /api/coaching/sessions/{id}/end`
- `PATCH /api/coaching/admin/applications/{id}`

Definition of Done:

- `rust/Cargo.toml` enthaelt `crates/dl-mentoring` als Workspace-Member und `dl-etage` haengt sauber daran.
- Vor Tests/Clippy ist Package-Sichtbarkeit bewiesen:
  ```bash
  cargo metadata --format-version=1 --no-deps
  cargo package --list -p dl-mentoring
  ```
- Response-Shapes entsprechen `map-coaching-public.md`, mit bewusstem Rust-minus-AI:
  - `ai_summary` und `ai_insights_json` nicht im Request-Modell.
  - eingehende unbekannte AI-Felder werden ignoriert oder kontrolliert abgewiesen, aber nie gespeichert.
  - keine AI-Felder in Responses.
- `GET /api/coaching/requests` bleibt unauthentifiziert.
- Bot-Token-Auth akzeptiert `X-Internal-Token` und `X-Bot-Token` mit bestehender Env-Namenskette, ohne Secret-Werte auszugeben.
- Legacy-Paritaet wird namentlich getestet und nicht "repariert":
  - Mehrfaches `PATCH /api/coaching/requests/{id}/match` erzeugt mehrere Sessions.
  - Match mit unbekanntem Coach bleibt moeglich, sofern der Request existiert.
  - Doppelter `POST /api/coaching/surveys` laeuft gegen `coaching_surveys.session_id UNIQUE` und wird nicht idempotent gemacht.
  - Erneutes Admin-Approve kann gegen `coaches.discord_user_id UNIQUE` laufen.
  - `POST /api/coaching/sessions/{id}/end` liefert auch bei 0 betroffenen Zeilen `{ "status": "completed" }`.
- FK-Paritaet bleibt sichtbar: Tests laufen gegen die `dl-etage`-Coaching-Verbindung, die keine strengere FK-Enforcement-Semantik als Python hat.
- Auth-geschuetzte Routertests verwenden Test-Cookie oder pruefen den erwarteten `401`/`403`.
- Verifikation bevorzugt Axum-Router-Tests ohne externen Port:
  ```bash
  cargo test -p dl-mentoring public
  cargo test -p dl-etage public_coaching
  ```

Out-of-Scope:

- Keine neuen Coaching-Features.
- Keine Absicherung von `GET /requests`.
- Keine AI-Analyse.
- Keine Finalisierung deutscher User-Texte.

Clippy-Scope:

```bash
cargo clippy -p dl-mentoring -p dl-etage -- -D warnings
```

User-sichtbare Texte: Ja. Neue deutsche UI/API-Texte nur `"Platzhalter"` plus Datei:Zeile fuer Claude.

### S1-05 - `dl-mentoring`: Platform-Routen und In-Process-Service-Surface

ID: S1-05  
Titel: `dl-mentoring`: Platform-Routen und In-Process-Service-Surface

Scope:

- `rust/crates/dl-mentoring/src/platform_routes.rs`
- `rust/crates/dl-mentoring/src/notifications.rs`
- `rust/crates/dl-mentoring/src/coach_sync.rs`
- `rust/crates/dl-mentoring/src/service.rs`
- `rust/crates/dl-mentoring/tests/*`
- `rust/bin/dl-etage/src/main.rs`

Blockiert durch: S1-03, S1-04.

Exakte 24 Platform-Routen:

1. `POST /api/coaching/platform/sync`
2. `GET /api/coaching/platform/overview`
3. `GET /api/coaching/platform/queue`
4. `GET /api/coaching/platform/coachees`
5. `GET /api/coaching/platform/coachees/{coachee_id}`
6. `PATCH /api/coaching/platform/coachees/{coachee_id}`
7. `POST /api/coaching/platform/coachees/{coachee_id}/goals`
8. `PATCH /api/coaching/platform/goals/{goal_id}`
9. `DELETE /api/coaching/platform/goals/{goal_id}`
10. `POST /api/coaching/platform/goals/{goal_id}/milestones`
11. `PATCH /api/coaching/platform/milestones/{milestone_id}`
12. `DELETE /api/coaching/platform/milestones/{milestone_id}`
13. `POST /api/coaching/platform/coachees/{coachee_id}/notes`
14. `PATCH /api/coaching/platform/notes/{note_id}`
15. `DELETE /api/coaching/platform/notes/{note_id}`
16. `GET /api/coaching/platform/me`
17. `POST /api/coaching/platform/coaches/sync`
18. `POST /api/coaching/platform/appointments`
19. `GET /api/coaching/platform/appointments`
20. `PATCH /api/coaching/platform/appointments/{appointment_id}`
21. `GET /api/coaching/platform/notifications/due`
22. `POST /api/coaching/platform/notifications/ack`
23. `GET /api/coaching/platform/coaches/me`
24. `PATCH /api/coaching/platform/coaches/me`

Definition of Done:

- Jede der 24 Routen hat mindestens einen Axum-Router-Test.
- Auth-geschuetzte Tests definieren Test-Cookie/Test-Bot-Token oder pruefen den erwarteten `401`/`403`.
- `POST /platform/sync` schreibt `assigned_coach_id` als `TEXT` und befuellt `ai_summary`/`ai_insights_json` nie.
- `GET /platform/queue`, `GET /platform/me`, Coachee-Detail und alle Platform-Responses enthalten keine AI-Felder.
- `GET /notifications/due` behaelt den Python-Seiteneffekt: `request_created` kann `coachees` upserten.
- Queue-/Reservation-Paritaet bleibt: fremde Reservierungen bleiben aus der SQL-Queue, Freigabe laeuft im Bot.
- `GoalUpdate`-Eigenheit bleibt dokumentiert und getestet: `completed_at=NULL`, wenn Patch-Status nicht `"done"` ist, auch bei anderen Updates.
- Goals/Notes/Milestones behalten fehlende Existenz-/Ownership-Checks dort, wo Python sie nicht hat.
- `appointments` behalten `TEXT`-Zeitfelder und Cutoffs aus Python:
  - Coach-Terminliste: `scheduled_at >= now - 7 days`.
  - Spieler-`/me`: scheduled ab `now - 6h`, done/cancelled nur letzte 5.
- `scope != "mine"` bei `GET /appointments` bedeutet weiterhin "alle Termine" fuer jeden Coach.
- `POST /appointments` und `_acting_coach_id` behalten die Caddy/Admin-Eigenheit: nicht-numerische Admin-Sub kann `coach_id=NULL` erzeugen, sofern keine FK-Enforcement-Abweichung greift.
- Services fuer `coach_sync`, `notifications_due/ack` und Request-/Session-Mirror sind ohne HTTP verwendbar, damit `dl-community` sie in-process nutzen kann.
- Verifikation:
  ```bash
  cargo test -p dl-mentoring platform
  cargo test -p dl-etage platform
  ```
  DB-Test fuer Due-Side-Effect:
  ```sql
  SELECT id, discord_user_id FROM coachees WHERE discord_user_id=<test_id>;
  ```

Out-of-Scope:

- Keine Entfernung der Bot-Bridge in diesem Ticket.
- Keine Discord-DM-Textfinalisierung.
- Keine Meta-Routen.

Clippy-Scope:

```bash
cargo clippy -p dl-mentoring -p dl-etage -- -D warnings
```

User-sichtbare Texte: Ja. Neue Embeds/DMs/UI-Fehlertexte nur `"Platzhalter"` plus Datei:Zeile fuer Claude.

### S1-06 - `dl-squads`: Scrim-Slice-1-Routen unter `/api/scrim/*`

ID: S1-06  
Titel: `dl-squads`: Scrim-Slice-1-Routen unter `/api/scrim/*`

Scope:

- `rust/crates/dl-squads/src/model.rs`
- `rust/crates/dl-squads/src/store.rs`
- `rust/crates/dl-squads/src/routes.rs` oder aequivalenter Router-Modul
- `rust/crates/dl-squads/tests/*`
- `rust/bin/dl-etage/src/main.rs`
- `rust/docs/db-schema.sql` nur falls `heroes` nicht schon in S1-02 erledigt ist

Blockiert durch: S1-02, S1-03.

Routen:

- `GET /api/scrim/me`: eigenes Teilnehmerprofil, Team inklusive Mitglieder, naechstes Match.
- `GET /api/scrim/participants/me`: Formular-Prefill.
- `POST /api/scrim/participants/me`: Teilnehmer aus Web-Formular erstellen.
- `PUT /api/scrim/participants/me`: Teilnehmer idempotent upserten/aktualisieren.
- `GET /api/scrim/pool?status=<status>`: Coach-Pool mit optionalem Status-Filter.

Definition of Done:

- `scrim_participant.heroes` existiert idempotent als `TEXT` und wird als JSON-Array serialisiert.
- Web-Form schreibt `source='web_form'`; `discord_id` kommt aus der Session, nicht aus dem Body.
- Discord-IDs sind `i64`; keine Token/String-Verwechslung.
- `roles`, `heroes`, `availability` werden strukturiert angenommen und kontrolliert als TEXT/JSON gespeichert.
- Coach-Pool ist coach-gated ueber `dl-auth` und filtert nur nach erlaubten Statuswerten.
- Coach-Pool Slice 1 ist nur Sichtbarkeit plus Filter, keine Status-Mutation.
- `GET /api/scrim/me` liefert `participant: null`, `team: null`, `next_match: null`, wenn der User noch nicht im Pool ist.
- Auth-geschuetzte Checks sind Routertests: ohne Coach-Cookie erwartetes `401`/`403`, mit Test-Coach-Cookie erwarteter Erfolg.
- Verifikation:
  ```bash
  cargo test -p dl-squads
  cargo test -p dl-etage scrim
  ```
  DB-Check:
  ```sql
  SELECT discord_id, source, status, heroes FROM scrim_participant WHERE source='web_form';
  ```

Out-of-Scope:

- Kein Matching-Cockpit.
- Kein Team-Building-Algorithmus.
- Kein RSVP.
- Kein Coach-Status-Mutations-UI; falls noetig, separat freigeben.

Clippy-Scope:

```bash
cargo clippy -p dl-squads -p dl-etage -- -D warnings
```

User-sichtbare Texte: Ja. Frontend/API-Texte nur `"Platzhalter"` plus Datei:Zeile fuer Claude.

### S1-07 - `dl-bot`/`dl-community`: direkter DB-Pfad hinter Feature-Flag und Rollen-Haltedauer

ID: S1-07  
Titel: `dl-bot`/`dl-community`: direkter DB-Pfad hinter Feature-Flag und Rollen-Haltedauer

Scope:

- `rust/crates/dl-community/src/coaching.rs`
- `rust/crates/dl-community/src/coaching_requests.rs`
- `rust/bin/dl-bot/src/main.rs`
- `rust/crates/dl-mentoring/src/service.rs`
- Tests in `rust/crates/dl-community/tests/*` und `rust/crates/dl-mentoring/tests/*`

Blockiert durch: S1-05.

Definition of Done:

- Coach-Roster-Sync, Notifications und Request-/Session-Mirror koennen ueber `dl-mentoring` direkt gegen `dl-db` laufen.
- Der In-Process-Pfad ist hinter klarer Config/Feature-Flag geschaltet, z. B. `DL_COACHING_IN_PROCESS=1`.
- Die alten HTTP-Loops in `dl-community/src/coaching.rs` und `coaching_requests.rs` bleiben als fallbackfaehiger Pfad erhalten. Sie werden in S1-07 nicht entfernt.
- Fallback-Konfiguration ist testbar: Flag aus -> HTTP-Pfad, Flag an -> In-Process-Pfad.
- Es gibt kein Zeitfenster waehrend Bot-Restart/Cutover, in dem weder HTTP noch In-Process Notifications/Acks funktionieren.
- Direkter DB-Pfad ist gegen Test-DB verifiziert, bevor er live aktiviert wird.
- Bot-Mirror-/Service-DTOs nehmen kein `ai_summary` und kein `ai_insights_json` an oder ignorieren sie ausdruecklich.
- DB-Test: Sync/Mirror schreibt neue Requests mit `ai_summary IS NULL` und `ai_insights_json IS NULL`.
- Queue/Requests/Platform-Me Responses enthalten keine AI-Felder.
- Rollen-Haltedauer:
  - mit Appointment: `role_expires_at = scheduled_at + 7 days`;
  - ohne Termin: `role_expires_at = now + 7 days`;
  - bestehende spaetere Expiry wird nicht verkuerzt.
- Verifikation:
  ```bash
  cargo test -p dl-community coaching
  cargo test -p dl-mentoring notifications
  cargo test -p dl-mentoring mirror_ai_null
  rg "DL_COACHING_IN_PROCESS|WEBSITE_API_BASE|/coaching/platform/notifications|/coaching/platform/coaches/sync" rust/crates/dl-community/src rust/bin/dl-bot/src
  ```
  Erwartung fuer `rg`: Feature-Flag und fallbackfaehiger HTTP-Pfad sind sichtbar; Entfernung ist erst S1-11.

Out-of-Scope:

- Keine Caddy-Aenderung.
- Keine Discord-Textfinalisierung.
- Keine Entfernung der HTTP-Bridge-Loops.
- Keine komplette Entfernung von HTTP-Clients, die fuer andere Domains noch noetig sind.

Clippy-Scope:

```bash
cargo clippy -p dl-community -p dl-mentoring -- -D warnings
```

User-sichtbare Texte: Ja, falls DM/Embed-Texte beruehrt werden. Codex setzt `"Platzhalter"` und meldet Datei:Zeile fuer Claude.

### S1-08 - Frontend: Mein Scrim, Scrim-Anmeldung, Coach-Pool

ID: S1-08  
Titel: Frontend: Mein Scrim, Scrim-Anmeldung, Coach-Pool

Scope:

- `Website/dl-coaching/src/api/client.ts`
- `Website/dl-coaching/src/types/index.ts`
- `Website/dl-coaching/src/App.tsx`
- `Website/dl-coaching/src/components/Layout.tsx`
- neue Seiten:
  - `Website/dl-coaching/src/pages/MyScrimPage.tsx`
  - `Website/dl-coaching/src/pages/ScrimSignupPage.tsx`
  - `Website/dl-coaching/src/pages/ScrimPoolPage.tsx`
- optional vorhandene UI-Helfer in `Website/dl-coaching/src/components/*`

Blockiert durch: S1-03, S1-06.

Definition of Done:

- `scrim`-API-Client nutzt `BASE=/coaching/api` und `credentials: include`.
- Spielerroute zeigt Login-Gate und danach `GET /api/scrim/me`.
- Formularroute nutzt `GET /api/scrim/participants/me` und `POST`/`PUT /api/scrim/participants/me`.
- Coach-Pool ist `CoachOnly` und ruft `GET /api/scrim/pool?status=...`.
- Coach-Pool bietet nur Sichtbarkeit und Filter, keine Status-Mutation.
- Keine finalen deutschen User-Texte von Codex: alle neuen Texte sind `"Platzhalter"` und im Worker-Report mit Datei:Zeile gelistet.
- Verifikation:
  ```bash
  npm --prefix /home/naniadm/Documents/Website/dl-coaching run build
  rg "Platzhalter" /home/naniadm/Documents/Website/dl-coaching/src
  rg "/api/scrim" /home/naniadm/Documents/Website/dl-coaching/src
  ```

Out-of-Scope:

- Kein Matching-Cockpit.
- Keine Team-Verwaltung.
- Keine finalen deutschen Texte.

Clippy-Scope:

```text
Keine Rust-Crate geaendert. Kein cargo clippy. Frontend-Verifikation per npm build.
```

User-sichtbare Texte: Ja. Alle neuen Texte sind `"Platzhalter"` plus Datei:Zeile fuer Claude.

### S1-09 - Systemd-Wrapper und `dl-etage` Staging auf `:8773`

ID: S1-09  
Titel: Systemd-Wrapper und `dl-etage` Staging auf `:8773`

Scope:

- `scripts/run_dl_etage_service.sh`
- `~/.config/systemd/user/deadlock-etage-rust.service`
- `rust/bin/dl-etage/*` nur fuer servicebezogene Config-Defaults
- keine Caddy-Aenderung in diesem Ticket

Blockiert durch: S1-01, S1-03, S1-04, S1-05, S1-06.

Definition of Done:

- Wrapper folgt `run_dl_bot_service.sh`: Infisical-Konfig laden, systemd Credential nutzen, Secrets nur als Env exportieren, keine Werte ausgeben.
- Defaults:
  - `DL_ETAGE_HOST=127.0.0.1`
  - `DL_ETAGE_PORT=8773`
  - `RUST_LOG=info`
  - Auth-Callback defaultet auf `/coaching/api/auth/discord/callback`
- Vor Start gegen Live-Konfig ist der S1-02-Datei-Identitaets-Preflight bestanden und dokumentiert; keine Secret-Werte werden ausgegeben.
- Release-Binary startet unter `deadlock-etage-rust.service` und bleibt auf `127.0.0.1:8773`.
- Verifikation:
  ```bash
  cargo build -p dl-etage --release
  systemctl --user daemon-reload
  systemctl --user restart deadlock-etage-rust.service
  systemctl --user status deadlock-etage-rust.service --no-pager
  curl -fsS http://127.0.0.1:8773/api/health
  ```
  Artefakt-Check:
  ```bash
  grep -a "DL_ETAGE_PORT" /home/naniadm/Documents/Deadlock-Bots/rust/target/release/dl-etage
  grep -a "/api/scrim" /home/naniadm/Documents/Deadlock-Bots/rust/target/release/dl-etage
  ```

Out-of-Scope:

- Kein Caddy-Flip.
- Kein Abschalten von `deadlock-website-backend.service`.
- Keine Secret-Ausgabe.

Clippy-Scope:

```bash
cargo clippy -p dl-etage -- -D warnings
```

User-sichtbare Texte: Nein.

### S1-10 - Caddy-Cutover und Live-Verifikation

ID: S1-10  
Titel: Caddy-Cutover und Live-Verifikation

Scope:

- `/home/naniadm/Documents/Caddy/conf/Caddyfile`
- `scripts/run_dl_etage_service.sh` nur falls Cutover-ENV fehlt
- systemd User-Services:
  - `deadlock-etage-rust.service`
  - `deadlock-bot-rust.service`
  - `deadlock-website-backend.service` bleibt fuer Meta/Fallback auf `:8772`

Blockiert durch: S1-07, S1-08, S1-09.

Definition of Done:

- Aktive Caddy-Konfig wird vor Aenderung gesichert, z. B.:
  ```bash
  cp /home/naniadm/Documents/Caddy/conf/Caddyfile /home/naniadm/Documents/Caddy/conf/Caddyfile.pre-s1-10.$(date +%Y%m%d%H%M%S)
  ```
- `caddy validate` laeuft vor und nach der Aenderung:
  ```bash
  caddy validate --config /home/naniadm/Documents/Caddy/conf/Caddyfile
  ```
- Rollback-Kommandos sind im Ticket-Umsetzungsbericht dokumentiert, mindestens:
  ```bash
  cp <backup-caddyfile> /home/naniadm/Documents/Caddy/conf/Caddyfile
  caddy validate --config /home/naniadm/Documents/Caddy/conf/Caddyfile
  sudo -n systemctl reload caddy
  systemctl --user restart deadlock-website-backend.service
  systemctl --user restart deadlock-bot-rust.service
  ```
- Beweis, dass keine weitere aktive `:8772`-Route ausser dem geplanten `/coaching/api/*`-Fallback existiert:
  ```bash
  rg "127\\.0\\.0\\.1:8772|:8772" /home/naniadm/Documents/Caddy/conf/Caddyfile
  caddy adapt --config /home/naniadm/Documents/Caddy/conf/Caddyfile --pretty
  ```
- Pre-Cutover Live-curl gegen aktuelle Python-Upstreams, Status und kurze Body-Klasse dokumentieren:
  ```bash
  curl -i https://deutsche-deadlock-community.de/coaching/api/auth/me
  curl -i https://deutsche-deadlock-community.de/coaching/api/coaching/coaches
  curl -i https://deutsche-deadlock-community.de/coaching/api/health
  curl -i https://deutsche-deadlock-community.de/coaching/api/builds
  curl -i https://deutsche-deadlock-community.de/coaching/api/items
  curl -i https://deutsche-deadlock-community.de/coaching/api/heroes
  curl -i https://deutsche-deadlock-community.de/coaching/api/patchnotes
  curl -i https://deutsche-deadlock-community.de/coaching/api/tierlists
  curl -i https://deutsche-deadlock-community.de/coaching/api/history
  curl -i https://deutsche-deadlock-community.de/coaching/api/admin
  ```
  Erwartung: Auth/Coaching/Health funktionieren wie vor Cutover; Meta-Pfade zeigen den vorgefundenen Python-Status. `admin` darf ohne Auth `401`/`403` sein, darf aber nicht versehentlich auf `dl-etage`-404 wechseln.
- Default-Caddy-Schnitt:
  - `/coaching/api/auth/*` -> `127.0.0.1:8773`
  - `/coaching/api/coaching/*` -> `127.0.0.1:8773`
  - `/coaching/api/scrim/*` -> `127.0.0.1:8773`
  - `/coaching/api/health` -> `127.0.0.1:8773`
  - Fallback `/coaching/api/*` -> `127.0.0.1:8772`, bis Meta-Slice 3 oder ein sauberer Pre-Cutover-Beweis den breiten Flip freigibt.
- Services werden in sicherer Reihenfolge gestartet:
  1. `deadlock-etage-rust.service` restart und Health pruefen.
  2. `deadlock-bot-rust.service` restart; Feature-Flag fuer In-Process-Pfad nur aktivieren, wenn S1-07 verifiziert ist.
  3. Caddy reload.
- Post-Cutover Live-curl wiederholt dieselben Checks:
  - `/coaching/api/auth/me`, `/coaching/api/coaching/coaches`, `/coaching/api/health` gehen zu `:8773`.
  - `/coaching/api/{builds,items,heroes,patchnotes,tierlists,history,admin}` bleiben auf geplantem `:8772`-Fallback mit gleicher Statusklasse wie vorher.
  - Auth-geschuetzte Pfade ohne Cookie duerfen erwartetes `401`/`403` liefern; fuer positive Auth-Checks wird ein Test-Cookie oder eine bestehende Browser-Session ohne Secret-Ausgabe genutzt.
- Cookie-Paritaet live bewiesen:
  - vorhandenes Python-`ddc_session` funktioniert nach Caddy-Flip in Rust.
  - neuer Rust-Login setzt Domain/Path/SameSite/Secure/Max-Age wie Python.
  - Kein Nutzer wird durch reine Cookie-Abweichung ausgeloggt.
- Bridge-Verifikation:
  - In-Process-Pfad liefert Coach-Sync, Notifications und Acks gegen Live-DB.
  - HTTP-Fallback bleibt bis S1-11 schaltbar.
  - DB zeigt `notify_*` bzw. `notify_discord_at` nach Zustellung.

Out-of-Scope:

- Python `:8772` abschalten.
- Meta-Routen portieren.
- Slice-2-Matching.
- Entfernen der HTTP-Bridge-Loops.

Clippy-Scope:

```bash
cargo clippy -p dl-etage -p dl-auth -p dl-mentoring -p dl-squads -p dl-community -- -D warnings
```

User-sichtbare Texte: Nein, ausser Caddy-/Service-Fehlerseiten werden bewusst angepasst; dann `"Platzhalter"` plus Datei:Zeile.

### S1-11 - Post-Cutover Bridge-Removal

ID: S1-11  
Titel: Post-Cutover Bridge-Removal

Scope:

- `rust/crates/dl-community/src/coaching.rs`
- `rust/crates/dl-community/src/coaching_requests.rs`
- `rust/bin/dl-bot/src/main.rs`
- Tests in `rust/crates/dl-community/tests/*` und `rust/crates/dl-mentoring/tests/*`

Blockiert durch: S1-10.

Definition of Done:

- Live-Cutover aus S1-10 ist verifiziert, inklusive zugestellter und geackter Notifications sowie erfolgreichem Coach-Roster-Sync ueber den In-Process-Pfad.
- Produktive HTTP-Bridge-Loops fuer diese Pfade sind entfernt:
  - `POST /coaching/platform/coaches/sync`
  - `GET /coaching/platform/notifications/due`
  - `POST /coaching/platform/notifications/ack`
  - Request-/Session-Mirror gegen `/coaching/platform/sync`, soweit er nur die Website-Coaching-DB gespiegelt hat.
- Entfernte HTTP-Loop-Pfade haben Tests fuer den In-Process-Ersatz.
- `rg` beweist, dass keine produktiven Bridge-Loop-Aufrufe mehr existieren; reine Kommentare/Legacy-Tests nur mit klarer Markierung:
  ```bash
  rg "8772|WEBSITE_API_BASE|/coaching/platform/notifications|/coaching/platform/coaches/sync|/coaching/platform/sync" rust/crates/dl-community/src rust/bin/dl-bot/src
  ```
- Nach Bot-Restart gibt es keine Unterbrechung fuer Notifications/Acks.

Out-of-Scope:

- Kein Caddy-Umbau.
- Kein Python-Backend-Abschalten.
- Keine Meta-Portierung.

Clippy-Scope:

```bash
cargo clippy -p dl-community -p dl-mentoring -- -D warnings
```

User-sichtbare Texte: Ja, falls DM/Embed-Texte beruehrt werden. Codex setzt `"Platzhalter"` und meldet Datei:Zeile fuer Claude.

## Verifikationsstrategie

Pro Ticket:

- Neue Packages muessen vor Cargo-Kommandos als Workspace-Member sichtbar sein:
  ```bash
  cargo metadata --format-version=1 --no-deps
  cargo package --list -p <new-package>
  ```
- Build nur fuer geaenderte Rust-Crates:
  ```bash
  cargo build -p <crate> [-p <crate>]
  ```
- Clippy nur scoped:
  ```bash
  cargo clippy -p <crate> [-p <crate>] -- -D warnings
  ```
- Tests nur scoped:
  ```bash
  cargo test -p <crate> [test-filter]
  ```
- HTTP-Router-Verifikation bevorzugt Axum-Router-Tests ohne externen Port. Auth-geschuetzte Checks definieren Test-Cookie/Test-Bot-Token oder den erwarteten `401`/`403`.
- Externe `curl`-Checks sind nur fuer S1-01/S1-09 lokale Health-Starts und S1-10 Live-Cutover erlaubt; jedes solche Ticket nennt Start-/Restart-Kommandos.
- Frontend nur im `dl-coaching`-Projekt:
  ```bash
  npm --prefix /home/naniadm/Documents/Website/dl-coaching run build
  ```

Schluss-Checkliste live:

- Artefakt enthaelt die neue Etage-Konfiguration:
  ```bash
  grep -a "DL_ETAGE_PORT" rust/target/release/dl-etage
  grep -a "/api/scrim" rust/target/release/dl-etage
  ```
- `systemctl --user status deadlock-etage-rust.service` ist gesund.
- `curl` gegen lokale und oeffentliche Endpoints zeigt `dl-etage` fuer Auth/Coaching/Scrim/Health und Python-Fallback fuer Meta.
- DB-Checks zeigen:
  - Python-DB und `dl-etage`-DB sind identische Datei per `device:inode`.
  - `PRAGMA foreign_keys` der `dl-etage`-Coaching-Verbindung erzwingt keine strengere Semantik als Python.
  - Web-Form schreibt `scrim_participant.source='web_form'`.
  - `heroes` existiert und bleibt idempotent.
  - `ai_summary`/`ai_insights_json` bleiben bei neuen Requests `NULL` und tauchen nicht in JSON-Responses auf.
  - Notifications werden geackt.
- Cookie-Paritaet ist mit Test-Secret und live ohne Secret-Ausgabe bewiesen.
- Kein Zwangs-Re-Login: vorhandene Browser-Session bleibt gueltig.
- Caddy-Flip ist der letzte produktive Schritt vor dem optionalen Post-Cutover-Bridge-Removal S1-11.

## Festgeschriebene Defaults

1. Nur `Website/dl-coaching` unter `/coaching` wird in Slice 1 umgezogen. `Website/builds/frontend` bleibt Pre-Cutover-Risiko und wird vor breitem Auth-Cutover geprueft.
2. Caddy path-splittet Auth/Coaching/Scrim/Health zu `:8773` und laesst den Rest auf `:8772`. Breiter `/coaching/api/*`-Flip nur nach negativem Pre-Cutover-Beweis fuer Meta-Nutzung.
3. `dl-etage` nutzt denselben SQLite-Pfad wie Python ueber vorhandene Env/Wrapper-Konfiguration. Wenn `DEADLOCK_DB_PATH` und Python-`DB_PATH` auseinanderlaufen, ist das ein Blocker; der Wrapper muss auf die geteilte Website-DB zeigen.
4. `POST /api/scrim/participants/me` und `PUT /api/scrim/participants/me` werden beide angeboten. `PUT` ist der idempotente Frontend-Pfad; `POST` ist kompatibler Erstellpfad.
5. Coach-Pool in Slice 1 hat Sichtbarkeit mit Status-Filter, keine Statuswechsel.
6. Rollen-Haltedauer: ohne Appointment `now + 7 days`; mit Appointment `scheduled_at + 7 days`; bestehende spaetere Expiry nicht verkuerzen.
7. Health heisst `/api/health`, damit `/coaching/api/health` nach Caddy-Strip weiter funktioniert.
8. Finale systemd-Service-Bezeichnung ist `deadlock-etage-rust.service`.
9. Bestehende Python-Fehlertexte sind technisch relevante Paritaet, aber neue/veraenderte deutsche User-Texte bleiben `"Platzhalter"` fuer Claude-Finalisierung.
