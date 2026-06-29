# Slice 1: Infrastruktur + Rust-Vorlagen fuer dl-etage

Stand: 2026-06-29. Bestandsaufnahme read-only; keine Secret-Werte gelesen.

## Kurzfazit

- Aktive Caddy-Datei: `/home/naniadm/Documents/Caddy/conf/Caddyfile`.
- Heute zeigt nur ein Caddy-Upstream auf das Python-Coaching/Meta-Backend `127.0.0.1:8772`: `deutsche-deadlock-community.de` + `/coaching/api/*`, mit `uri strip_prefix /coaching`.
- Oeffentliche Coaching-API-Formen sind dadurch:
  - `/coaching/api/auth/*` -> upstream `/api/auth/*`
  - `/coaching/api/coaching/*` -> upstream `/api/coaching/*`
  - `/coaching/api/health` -> upstream `/api/health`
- Die Coaching-SPA selbst ist statisch aus `/home/naniadm/Documents/Website/dl-coaching/dist`; sie proxyt nicht auf `:8772`.
- Es gibt keinen aktiven Caddy-Block `handle /api/auth/*`, `handle /api/coaching/*` oder generisches `handle /api/*` auf `:8772`.
- Python `:8772` enthaelt aber neben Auth/Coaching auch Meta-Router (`/api/builds`, `/api/items`, `/api/heroes`, `/api/patchnotes`, `/api/tierlists`, plus `/api/history`, `/api/admin`). Bei einem Caddy-Cutover darf der grobe `/coaching/api/*`-Block deshalb nicht blind komplett auf dl-etage zeigen, solange diese Routen nicht portiert oder separat auf Python gehalten sind.

## A) Caddy

### Aktive Bloecke auf `:8772`

Host: `deutsche-deadlock-community.de`.

```caddy
handle /coaching/api/* {
	uri strip_prefix /coaching
	reverse_proxy 127.0.0.1:8772 {
		header_up X-Forwarded-Proto https
		header_up Host deutsche-deadlock-community.de
	}
}
```

Effekt:

| Public Path | Upstream |
| --- | --- |
| `/coaching/api/auth/*` | `http://127.0.0.1:8772/api/auth/*` |
| `/coaching/api/coaching/*` | `http://127.0.0.1:8772/api/coaching/*` |
| `/coaching/api/health` | `http://127.0.0.1:8772/api/health` |
| `/coaching/api/{builds,items,heroes,patchnotes,tierlists,history,admin}/*` | `http://127.0.0.1:8772/api/{...}/*`, falls ein Client diese Pfade unter `/coaching/api` nutzt |

Die SPA:

```caddy
@coaching_app path /coaching /coaching/*
handle @coaching_app {
	uri strip_prefix /coaching
	root * /home/naniadm/Documents/Website/dl-coaching/dist
	try_files {path} {path}/index.html /index.html
	file_server
}
```

Zugehoerige CSP-Ausnahme:

```caddy
@coaching_paths path /coaching /coaching/*
header @coaching_paths {
	defer
	X-Frame-Options "DENY"
	Content-Security-Policy "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; img-src 'self' data: https://cdn.discordapp.com; font-src 'self' data: https://fonts.gstatic.com; connect-src 'self' https://discord.com https://discordapp.com; base-uri 'none'; frame-ancestors 'none'"
}
```

### Andere relevante API-Bloecke, nicht `:8772`

| Host/Pfad | Upstream/Handler | Bedeutung |
| --- | --- | --- |
| `deutsche-deadlock-community.de /api/public/*` | `127.0.0.1:8766` | altes Dashboard/Public API |
| `deutsche-deadlock-community.de /builds/api/*` | strip `/builds` -> `127.0.0.1:8771` | Rust `dl-tierlist`/Builds-Backend |
| `deutsche-deadlock-community.de /builds/auth/*` | strip `/builds` -> `127.0.0.1:8771` | Rust Tierlist-Auth |
| `deutsche-deadlock-community.de /builds/*` | statisch `Website/dl-tierlist/dist` | Builds/Tierlist SPA |
| `deutsche-deadlock-community.de` Fallback | `404` | kein generisches `/api/*` |

### Python-Router hinter `:8772`

`Website/builds/backend/app/main.py` mountet:

| Upstream Prefix | Router |
| --- | --- |
| `/api/auth` | Discord Login/Callback, `/me`, Logout |
| `/api/coaching` | Coaching Public/Profile/Requests/Surveys/Admin Applications |
| `/api/coaching/platform` | Coach-/Coachee-Plattform, Sync, Notifications, Appointments |
| `/api/builds` | Meta Builds |
| `/api/items` | Meta Items |
| `/api/heroes` | Meta Heroes |
| `/api/patchnotes` | Meta Patchnotes |
| `/api/tierlists` | Meta Tierlists |
| `/api/history` | Tierlist-History |
| `/api/admin` | Admin Reports/Votes/Users |
| `/api/health` | Health |

Die Coaching-SPA baut ihre Basis aus Vite `base: '/coaching/'`:

- `AuthContext.tsx`: `/coaching/api/auth`
- `src/api/client.ts`: `/coaching/api` + Pfade wie `/auth/me`, `/coaching/coaches`, `/coaching/platform/...`
- Vite-Dev-Proxy: `/coaching/api` -> `http://localhost:8772`, Rewrite zu `/api`

### Cutover-Konsequenz

Sauberer Cutover: spezifisch routen, nicht den ganzen Block pauschal umbiegen:

```caddy
handle /coaching/api/auth/* {
	uri strip_prefix /coaching
	reverse_proxy 127.0.0.1:<DL_ETAGE_PORT> {
		header_up X-Forwarded-Proto https
		header_up Host deutsche-deadlock-community.de
	}
}

handle /coaching/api/coaching/* {
	uri strip_prefix /coaching
	reverse_proxy 127.0.0.1:<DL_ETAGE_PORT> {
		header_up X-Forwarded-Proto https
		header_up Host deutsche-deadlock-community.de
	}
}

handle /coaching/api/* {
	uri strip_prefix /coaching
	reverse_proxy 127.0.0.1:8772 {
		header_up X-Forwarded-Proto https
		header_up Host deutsche-deadlock-community.de
	}
}
```

So bleiben Meta-Routen (`builds/items/heroes/patchnotes/tierlists`, ggf. `history/admin/health`) bei Python, bis Slice 3 sie portiert oder explizit entfernt.

## B) systemd

### `deadlock-website-backend.service` (Python `:8772`)

Fragment: `/home/naniadm/.config/systemd/user/deadlock-website-backend.service`  
Drop-in: `/home/naniadm/.config/systemd/user/deadlock-website-backend.service.d/20-creds.conf`

Unit-Muster:

```ini
[Service]
Type=simple
WorkingDirectory=/home/naniadm/Documents/Website
ExecStart=/usr/bin/bash -lc '/home/naniadm/Documents/Website/scripts/run_builds_backend.sh'
Restart=always
RestartSec=5
LoadCredential=infisical-token:/home/naniadm/.config/infisical-tokens/infisical-token-bots
```

Wrapper: `/home/naniadm/Documents/Website/scripts/run_builds_backend.sh`

Mechanismus:

- `ROOT_DIR` = `/home/naniadm/Documents/Website`
- `BACKEND_DIR` = `Website/builds/backend`
- `INFISICAL_CONFIG_FILE` defaultet auf `$HOME/.config/deadlock-bots/infisical.conf`
- diese Config enthaelt nicht-geheime Infisical-Verbindungsparameter und wird per `source` exportiert
- systemd-Credential `infisical-token` wird aus `$CREDENTIALS_DIRECTORY/infisical-token` in `INFISICAL_SERVICE_TOKEN` geladen
- Secrets werden via `INFISICAL_EXPORT_SCRIPT` (default `/home/naniadm/Documents/Deadlock-Bots/scripts/export_infisical_env.py`) als Shell-Exports erzeugt und per `eval` gesetzt
- Infisical-Retry ueber `INFISICAL_RETRY_DELAY` und `INFISICAL_MAX_ATTEMPTS`
- setzt `PYTHONUNBUFFERED=1`
- setzt `AUTH_PUBLIC_CALLBACK_URL=https://deutsche-deadlock-community.de/coaching/api/auth/discord/callback`
- startet:

```bash
exec "$PYTHON_BIN" -m uvicorn app.main:app \
  --host "${WEBSITE_BACKEND_HOST:-127.0.0.1}" \
  --port "${WEBSITE_BACKEND_PORT:-8772}"
```

ENV-/Secret-Namen, die im Mechanismus oder Python-Code relevant sind (nur Namen):

- Bootstrap/Loader: `INFISICAL_CONFIG_FILE`, `INFISICAL_EXPORT_SCRIPT`, `INFISICAL_SERVICE_TOKEN`, `INFISICAL_RETRY_DELAY`, `INFISICAL_MAX_ATTEMPTS`, `PYTHON_BIN`
- Bindung: `WEBSITE_BACKEND_HOST`, `WEBSITE_BACKEND_PORT`
- DB: `DB_PATH`
- Auth/Sessions: `AUTH_PUBLIC_CALLBACK_URL`, `AUTH_COOKIE_NAME`, `AUTH_PRE_AUTH_COOKIE_NAME`, `AUTH_SESSION_TTL_SECONDS`, `AUTH_PRE_AUTH_TTL_SECONDS`, `AUTH_SESSION_AUDIENCE`, `AUTH_SESSION_ISSUER`, `AUTH_COOKIE_DOMAIN`, `AUTH_DDC_COOKIE_DOMAIN`, `AUTH_COOKIE_PATH`, `AUTH_COOKIE_SAMESITE`, `AUTH_COOKIE_SECURE`, `AUTH_INSECURE_COOKIE`, `AUTH_SESSION_SECRET`, `JWT_SECRET`, `SESSIONS_ENCRYPTION_KEY`
- Dashboard/Auth-Relay: `DASHBOARD_INTERNAL_API_BASE`, `WEBSITE_INTERNAL_API_TOKEN`, `TURNIER_INTERNAL_API_TOKEN`, `MAIN_BOT_INTERNAL_TOKEN`, `TWITCH_INTERNAL_API_TOKEN`
- Coaching Bot/API: `TWITCH_INTERNAL_API_TOKEN`, `MASTER_BROKER_TOKEN`, `COACHING_BOT_TOKEN`

### `deadlock-bot-rust.service` als Vorlage

Fragment: `/home/naniadm/.config/systemd/user/deadlock-bot-rust.service`

```ini
[Service]
Type=simple
WorkingDirectory=/home/naniadm/Documents/Deadlock-Bots
LoadCredential=infisical-token:/home/naniadm/.config/infisical-tokens/infisical-token-bots
ExecStart=/usr/bin/bash -lc '/home/naniadm/Documents/Deadlock-Bots/scripts/run_dl_bot_service.sh'
Restart=on-failure
RestartSec=5
```

Wrapper: `/home/naniadm/Documents/Deadlock-Bots/scripts/run_dl_bot_service.sh`

Mechanismus:

- `ROOT_DIR` = `/home/naniadm/Documents/Deadlock-Bots`
- `CONFIG_FILE` defaultet auf `$HOME/.config/deadlock-bots/infisical.conf`
- source der nicht-geheimen Infisical-Konfiguration
- systemd-Credential `infisical-token` -> `INFISICAL_SERVICE_TOKEN`
- Secrets via `scripts/export_infisical_env.py --format shell` und `eval`
- Defaults:
  - `DL_BOT_GATEWAY=1`
  - `DL_BOT_COMMAND_SYNC=0`
  - `RUST_LOG=info`
  - `WEBSITE_API_BASE=http://127.0.0.1:8772/api`
- Startet Release-Binary:

```bash
cd "$ROOT_DIR"
exec "$ROOT_DIR/rust/target/release/dl-bot"
```

Vorlage fuer `dl-etage`:

- neue User-Unit analog, `WorkingDirectory=/home/naniadm/Documents/Deadlock-Bots`
- `LoadCredential` identisch
- eigener Wrapper, z. B. `/home/naniadm/Documents/Deadlock-Bots/scripts/run_dl_etage_service.sh`
- Wrapper laedt Infisical wie `run_dl_bot_service.sh`
- setzt nur Etage-spezifische Defaults (`RUST_LOG`, `DL_ETAGE_HOST`, `DL_ETAGE_PORT`, ggf. Auth-Callback)
- startet `rust/target/release/dl-etage`
- `Restart=on-failure`, `RestartSec=5`

## C) Rust-Vorlagen

### `bin/dl-web`

Dateien:

- `/home/naniadm/Documents/Deadlock-Bots/rust/bin/dl-web/src/main.rs`
- `/home/naniadm/Documents/Deadlock-Bots/rust/bin/dl-web/Cargo.toml`

Muster:

- `dl_core::observability::init_tracing("info")`
- `dl_core::Config::from_env()`
- `dl_webcore::WebConfig::from_env()`
- `dl_db::Db::open(&cfg.db_path)`
- einmalig shared Clients bauen (`DashboardClient`)
- pro Dienst eigener `TcpListener::bind(...)`
- `axum::serve(...)` pro Listener
- `tokio::select!` wartet auf alle Server oder `ctrl_c`

Aktuell bindet ein Prozess drei HTTP-Server:

- Tierlist: `web_cfg.tierlist_host` + `cfg.ports.tierlist_public`
- Public-Stats: `web_cfg.stats_host` + `cfg.ports.public_stats`
- Dashboard: `DASHBOARD_HOST` + `cfg.ports.dashboard`

`Cargo.toml` zeigt die erwartete Binary-Struktur: eigene `bin/*`, Workspace-Dependencies auf `dl-core`, `dl-db`, Domain-Crates und `dl-webcore`.

### `bin/dl-bot`

Datei: `/home/naniadm/Documents/Deadlock-Bots/rust/bin/dl-bot/src/main.rs`

Startmuster:

- Single-Instance PID-Lock
- `Config::from_env`
- `Db::open`
- `db.bootstrap_schema().await`
- Discord-REST-Adapter mit `DISCORD_TOKEN`
- InteractionRouter/Dispatcher bauen
- interne HTTP-Server:
  - Broker: `MASTER_BROKER_HOST` + `cfg.ports.master_broker`
  - Changelog: `127.0.0.1` + `cfg.ports.changelog_api`
- Gateway-Loops nur wenn `DL_BOT_GATEWAY=1`
- `tokio::select!` auf Broker, Changelog, Restart-Kanal, Ctrl-C

Coaching-Anbindung:

- `WebsiteClient::from_env(...)` erzeugt aktuell den HTTP-Client zur Python-Website-API.
- `CoachingRequests` bekommt diesen Client als optionalen `CoachingWebsiteSyncClient` fuer Request/Session-Mirror.
- Innerhalb des Gateway-Blocks startet:
  - `dl_community::coaching::spawn(sync, &dispatcher)` fuer Rollen-/Notification-Loops
  - `dl_community::coaching_requests::spawn(...)` fuer Coaching-Survey/Poll/Voice-Ende-Listener

Das ist der Hebel fuer dl-etage: die Traits in `dl-community::coaching` koennen statt eines HTTP-Clients eine in-process Store/Service-Implementierung aus einer neuen Etage-Domain-Crate bekommen.

### `crates/dl-db`

Datei: `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-db/src/lib.rs`

Muster:

- `Db::open(path)` oeffnet nur existierende DB-Dateien; kein stilles Neuanlegen im Prod-Pfad.
- `Db::open_creating(path)` ist fuer Tests/Werkzeuge.
- WAL-Pragmas und `foreign_keys=ON` beim Oeffnen.
- `write(...)`: ein serialisierter Writer hinter Mutex + `spawn_blocking`.
- `read(...)`: frische Read-only-Verbindung pro Read.
- `bootstrap_schema()`:
  - liest `docs/db-schema.sql` ueber `include_str!`
  - ersetzt `CREATE TABLE/INDEX/TRIGGER` durch `IF NOT EXISTS`
  - fuehrt idempotent `execute_batch` aus
  - haengt explizite `ensure_*`-Nachmigrationen an
- Beispiel `ensure_coaching_request_mirror_schema()`:
  - `column_exists(...)` via `PRAGMA table_info`
  - `ALTER TABLE ... ADD COLUMN` nur wenn Spalte fehlt
  - Index `CREATE UNIQUE INDEX IF NOT EXISTS ...`

### `dl-squads` / `scrim_*`

Dateien:

- `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-squads/src/{lib.rs,store.rs,seed.rs,model.rs}`
- DDL: `/home/naniadm/Documents/Deadlock-Bots/rust/docs/db-schema.sql`

Wichtig: `dl-squads` legt das Schema nicht selbst an. Es nutzt `Db` und schreibt/liest gegen Tabellen, die der zentrale `Db::bootstrap_schema()` aus `docs/db-schema.sql` erzeugt.

Aktuelle Tabellen im Schema-Dump:

- `scrim_participant`
- `scrim_team`
- `scrim_team_member`
- `scrim_match`

Store-Muster:

- `upsert_participant_by_discord`
- `upsert_participant_by_name`
- `create_team`
- `add_team_member`
- `list_pool`
- `create_match`
- `set_participant_status`

Seed-Import:

- `import_seed_roster_json`
- `import_seed_roster`
- normalisiert Namen, erzeugt Teilnehmer, Teams, Teammitglieder und Matches idempotent ueber Store-Funktionen.

### `crates/dl-community/src/coaching.rs`

Aktuelle HTTP-Bruecke:

- `WebsiteClient::from_env` nutzt Token-Kette `TWITCH_INTERNAL_API_TOKEN` -> `MASTER_BROKER_TOKEN` -> `COACHING_BOT_TOKEN`.
- `WEBSITE_API_BASE` defaultet im Code auf `https://deutsche-deadlock-community.de/api`; der Service-Wrapper setzt bewusst `http://127.0.0.1:8772/api`.
- Header: `X-Internal-Token` und `X-Bot-Token`.

Gepollte/aufgerufene Endpunkte:

| Funktion | Methode/Pfad relativ zu `WEBSITE_API_BASE` |
| --- | --- |
| `sync_coaching` | `POST /coaching/platform/sync` |
| `sync_coaches` | `POST /coaching/platform/coaches/sync` |
| `due_notifications` | `GET /coaching/platform/notifications/due` |
| `ack_notifications` | `POST /coaching/platform/notifications/ack` mit `items` |
| `ack_request_created_notifications` | `POST /coaching/platform/notifications/ack` mit `request_ids` |

Loops:

- `ROLE_SYNC_INTERVAL = 600s`: Coach-Rolle -> Coach-Roster -> Website syncen; leere Roster werden nicht gesendet.
- Coach-Rollen-Events: Dispatcher-Subscription mit `ROLE_DEBOUNCE = 5s`, danach ausserplanmaessiger Roster-Sync.
- `NOTIFICATION_INTERVAL = 60s`: faellige Notifications holen, DMs senden oder `request_created` an `RequestNotificationSink` posten, danach ack.

Bot-Loops, die daran haengen:

- `dl-bot` startet `dl_community::coaching::spawn(...)` im Gateway-Block.
- `CoachingRequests` wird als `request_sink` uebergeben; `request_created` wird nicht per DM, sondern an den Request-Sink gepostet und dann geackt.
- `CoachingRequests` nutzt den gleichen Website-Client optional als `CoachingWebsiteSyncClient` fuer Request/Session-Mirror.

### `crates/dl-core`

Datei: `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-core/src/config.rs`

Aktuelle Config:

- DB:
  - `DEADLOCK_DB_PATH`
  - `DEADLOCK_DB_DIR`
  - Default `data/deadlock.sqlite3`
- Ports:
  - `DASHBOARD_PORT` default `8766`
  - `PUBLIC_STATS_PORT` default `8768`
  - `MASTER_BROKER_PORT` default `8770`
  - `TIERLIST_PUBLIC_PORT` default `8771`
  - `CHANGELOG_API_PORT` default `8899`

Fuer dl-etage passt ein neuer Port hier hinein, z. B.:

- Feld `etage: u16`
- ENV `DL_ETAGE_PORT`
- Default fuer Parallelbetrieb als neuer Port (konkret noch festzulegen; naheliegend ist ein freier Port neben `8772`)

Host-Konfiguration sollte nicht in `dl-core::Ports`, sondern in einer Etage-spezifischen Config bleiben, analog `WebConfig` (`DL_ETAGE_HOST`, default `127.0.0.1`).

### `crates/dl-webcore`

Dateien:

- `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-webcore/src/config.rs`
- `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-webcore/src/dashboard.rs`
- `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-webcore/src/session.rs`
- `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-webcore/src/envelope.rs`
- `/home/naniadm/Documents/Deadlock-Bots/rust/crates/dl-webcore/src/client_ip.rs`

Wiederverwendbar fuer dl-etage:

- Ja: `DashboardClient` fuer Discord-OAuth-Relay und Sessionvalidierung gegen zentrale interne Dashboard-API.
- Ja: `SessionCodec`, falls dl-etage eigene kompatible HMAC-Cookies ausgeben/lesen soll.
- Ja: `envelope` und `client_ip` als kleine HTTP-Helfer.
- Eingeschraenkt: `WebConfig` ist heute auf Public-Stats/Tierlist zugeschnitten (`PUBLIC_STATS_*`, `TIERLIST_PUBLIC_HOST`, `DL_TIERLIST_REFRESH`). Fuer dl-etage besser eigene `EtageConfig` bauen und nur einzelne Muster uebernehmen.

## Empfehlung fuer dl-etage

Sauberste Struktur:

1. Neues Domain-Crate `crates/dl-etage` fuer Coaching/Auth/Etage-Stores und Services.
2. Neues Binary `bin/dl-etage` als Axum-Server:
   - `Config::from_env()`
   - `EtageConfig::from_env()`
   - `Db::open(&cfg.db_path)`
   - ggf. `db.bootstrap_schema().await` nur wenn dieser Prozess Schema-Owner sein soll
   - `TcpListener::bind(format!("{}:{}", etage_host, cfg.ports.etage))`
   - Router mit `/api/auth/*` und `/api/coaching/*` (oder intern ohne `/api`, aber Caddy strippt nur `/coaching`)
3. `dl-core::Ports` um `etage` erweitern (`DL_ETAGE_PORT`).
4. systemd-User-Service analog `deadlock-bot-rust.service`, eigener Wrapper und Release-Binary `rust/target/release/dl-etage`.
5. Caddy-Cutover spezifisch:
   - `/coaching/api/auth/*` -> dl-etage
   - `/coaching/api/coaching/*` -> dl-etage
   - Rest `/coaching/api/*` -> Python `:8772`, bis Meta in Slice 3 geklaert ist
6. Bot-Bridge nach Cutover nicht als HTTP-Loop auf dl-etage belassen, sondern Traits aus `dl-community::coaching` durch eine Store/Service-Implementierung aus `dl-etage` bedienen. Dann nutzen Bot und Webserver dieselbe Rust-Domainlogik; HTTP bleibt nur fuer Browser/Caddy.

Reuse-Bewertung:

- `dl-web` als Bootstrap-Vorlage: ja.
- `dl-webcore::DashboardClient`: ja.
- `dl-webcore::WebConfig`: nein, nur Muster uebernehmen.
- `dl-community::coaching::WebsiteClient`: kurzfristig fuer Parallelbetrieb ja; Zielzustand ist ersetzen durch in-process Trait-Implementierung.
