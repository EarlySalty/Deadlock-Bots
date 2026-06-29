# Slice 1 Auth Map: Website Builds Discord OAuth + JWT Session

Quelle:
- `/home/naniadm/Documents/Website/builds/backend/app/routers/auth.py`
- `/home/naniadm/Documents/Website/builds/backend/app/database.py`
- `/home/naniadm/Documents/Website/builds/backend/app/main.py`
- zentrale delegierte OAuth-Quelle: `/home/naniadm/Documents/Deadlock-Bots/service/dashboard.py` und `service/db.py`
- Frontend-Verbrauch: `/home/naniadm/Documents/Website/builds/frontend/src/context/AuthContext.tsx`, `src/api/client.ts`, `src/types/index.ts`

Keine Secret-Werte gelesen. Nur ENV-/Keyring-Namen und Codepfade inventarisiert.

## Public route prefix

`auth.router` wird mit Prefix `/api/auth` gemountet.

Effektive Builds-Auth-Routen:
- `GET /api/auth/discord/login`
- `GET /api/auth/discord/callback`
- `GET /api/auth/me`
- `POST /api/auth/logout`

Bei einem Caddy/Base-Path wie `/builds` nutzt das Frontend `import.meta.env.BASE_URL` und ruft effektiv `/builds/api/auth/...` auf. `auth.py` leitet aus der FastAPI-Callback-URL den Default-Redirect-Pfad ab.

## GET /discord/login

Lokaler Handler: `discord_login(request, next=None)`.

Ablauf:
1. Baut `callback_url` via `_build_callback_url(request)`.
2. Normalisiert `next` zu einem relativen lokalen Pfad:
   - leer -> `_default_redirect_path(request)`
   - absolute URLs, `//...`, CR/LF/NUL, Pfade ohne fuehrendes `/`, Pfadsegmente `..` -> Fallback
   - Query und Fragment bleiben erhalten
3. Ruft den zentralen Dashboard-OAuth-Dienst:
   - URL: `{DASHBOARD_INTERNAL_API_BASE}/internal/v1/discord/initiate`
   - Methode: `POST`
   - Header: `X-Internal-Token: <resolved internal token>`, `Content-Type: application/json`
   - Body:
     ```json
     {
       "scope": "identify",
       "redirect_after": "<callback_url>",
       "requesting_service": "builds",
       "metadata": {"site": "builds"}
     }
     ```
4. Erwartet JSON-Felder:
   - `authorize_url`
   - `state_id`
5. Antwort an Browser:
   - `302` auf `authorize_url`
   - setzt Pre-Auth-Cookie mit signiertem JWT.

OAuth-Scopes:
- Website-Builds sendet exakt `identify`.
- Der zentrale Dienst kann andere Scopes, aber fuer diese Schicht wird nicht `guilds.members.read` angefordert.

Callback/Redirect-URI:
- Website-Callback fuer `redirect_after`:
  - wenn `AUTH_PUBLIC_CALLBACK_URL` gesetzt ist: exakt dieser Wert
  - sonst: `{X-Forwarded-Proto oder request.url.scheme}://{X-Forwarded-Host oder Host oder request.url.netloc}{url_for(discord_callback).path}`
  - Pfad ist durch Router-Mount typischerweise `/api/auth/discord/callback` oder mit Base-Path `/builds/api/auth/discord/callback`.
- Discord-OAuth-Redirect-URI im zentralen Dienst:
  - hartkodiert in `Deadlock-Bots/service/dashboard.py`: `https://deutsche-deadlock-community.de/callback/discord`
  - kein ENV-Name fuer diese URI im aktuellen Code.

Pre-Callback-Cookie:
- Name: `AUTH_PRE_AUTH_COOKIE_NAME` oder Default `ddc_pre_auth`
- Wert: JWT `HS256` mit Claims:
  - `state_id`
  - `next`
  - `kind`: `pre_auth`
  - `iss`: `AUTH_SESSION_ISSUER` oder Default `ddc-auth`
  - `iat`
  - `exp`
- Kein `aud` Claim im Pre-Auth-JWT.
- TTL/Max-Age: `AUTH_PRE_AUTH_TTL_SECONDS` oder Default `600`.
- Cookie-Attribute wie Session-Cookie, siehe Cookie-Abschnitt.

Authorize-URL aus zentralem Dienst:
- Basis: `https://discord.com/api/v10/oauth2/authorize`
- Query:
  - `client_id`: `DISCORD_OAUTH_CLIENT_ID` aus ENV/Keyring, sonst Bot-Application-ID als Fallback
  - `redirect_uri`: `https://deutsche-deadlock-community.de/callback/discord`
  - `response_type`: `code`
  - `scope`: vom Website-Body, hier `identify`
  - `state`: zentral erzeugter State

State-Erzeugung/Speicherung:
- Zentrale Erzeugung: `secrets.token_urlsafe(32)` im Dashboard.
- Persistenz: Tabelle `oauth_states` in `Deadlock-Bots/service/db.py`:
  - `state`
  - `provider`: `discord`
  - `flow_type`: `delegated:builds`
  - `requesting_service`: `builds`
  - `redirect_after`: Website-Callback-URL
  - `created_at`
  - `expires_at`
  - `used`
  - `metadata`: JSON mit `redirect_uri`, `scope`, `site`
- Default-TTL im zentralen DB-Helper: `DEADLOCK_OAUTH_STATE_TTL_SECONDS` oder Default `21600`.
- Dashboard-Init akzeptiert `redirect_after` nur fuer:
  - `https://deutsche-deadlock-community.de`
  - `https://admin.deutsche-deadlock-community.de`
  - localhost/127.0.0.1/::1 mit http oder https
- Lokale Website-CSRF-Bindung: `state_id` wird zusaetzlich im signierten `ddc_pre_auth` Cookie gespeichert.

Wichtige Kompatibilitaets-/Security-Details:
- Callback verwendet `state_id` aus Query bevorzugt. Nur wenn Query fehlt, nimmt er `state_id` aus dem Pre-Auth-Cookie.
- Es gibt keinen expliziten Vergleich `query.state_id == pre_auth.state_id`.
- Einmaligkeit/Gueltigkeit wird durch den zentralen `/consume-result`-Aufruf und `oauth_states.used` erzwungen.

## GET /discord/callback

Lokaler Handler: `discord_callback(request, state_id=None)`.

Ablauf:
1. `next_path` startet mit `_default_redirect_path(request)`.
2. Liest `ddc_pre_auth` und decodiert es als JWT.
3. Wenn Pre-Auth gueltig ist, wird `next` daraus normalisiert und als Redirect-Ziel genutzt.
4. Erstellt direkt `RedirectResponse(next_path, 302)`.
5. Loescht `ddc_pre_auth` in Host-only- und ggf. Domain-Variante.
6. Ermittelt `state_id_value`:
   - zuerst Query-Parameter `state_id`
   - sonst `pre_auth.state_id`
7. Wenn kein State vorhanden: Redirect ohne Session.
8. Ruft zentral:
   - URL: `{DASHBOARD_INTERNAL_API_BASE}/internal/v1/discord/consume-result`
   - Methode: `POST`
   - Body: `{"state_id": "<state_id>"}`
9. Bei Fehlern/ungueltigem Payload: Redirect ohne Session.
10. Erwartete Felder:
    - `discord_id`
    - `discord_name`
    - optional `discord_avatar`
    - `discord_roles` wird vom zentralen Dienst geliefert, aber von `auth.py` ignoriert.
11. Upsert in lokale Website-DB `meta_users`.
12. Erstellt Session-JWT.
13. Setzt `ddc_session`.
14. Loescht Legacy-Cookie `auth_token`.
15. Redirect zu `next_path`.

Zentraler Discord Token-Exchange:
- Endpoint: `POST https://discord.com/api/v10/oauth2/token`
- Content-Type: `application/x-www-form-urlencoded`
- Params/Form:
  - `client_id`
  - `client_secret`
  - `grant_type=authorization_code`
  - `code`
  - `redirect_uri`
- Client-ID/Secret:
  - ENV `DISCORD_OAUTH_CLIENT_ID`, `DISCORD_OAUTH_CLIENT_SECRET` werden zuerst versucht
  - sonst Keyring-Service `DeadlockBot` mit denselben Namen
  - Client-ID fallbackt auf `bot.application_id`, Secret hat keinen Fallback

Zentraler Userinfo-Fetch:
- Endpoint: `GET https://discord.com/api/v10/users/@me`
- Header: `Authorization: Bearer <access_token>`
- Auswertung:
  - `id` -> Discord-ID
  - `global_name` bevorzugt als Anzeigename
  - sonst `username`
  - Avatar-URL: `https://cdn.discordapp.com/avatars/{id}/{avatar}.{gif|png}`

Zentraler Guild-Member/Roles-Fetch:
- Keine REST-URL in diesem Codepfad.
- Nutzt Discord-Bot/Gateway-Objekte:
  - optional `guild_id` aus State-Metadata
  - sonst konfigurierte `self._discord_auth_guild_ids`
  - wenn leer: alle `bot.guilds`
  - pro Guild: `guild.get_member(discord_user_id)`, fallback `guild.fetch_member(discord_user_id)`
- Rollen:
  - sammelt `member.roles`
  - filtert Guild-`@everyone`-Rolle (Role-ID == Guild-ID)
  - Rueckgabe als Liste von Role-ID-Strings
- Fuer Builds-Website:
  - `auth.py` uebergibt keine `guild_id`, nur `metadata.site=builds`
  - `auth.py` ignoriert `discord_roles` vollstaendig.

Post-Login-Redirect:
- Kein `FRONTEND_URL`-ENV in dieser Schicht.
- Browser wird nach erfolgreichem oder fehlgeschlagenem Callback auf `next_path` redirectet.
- `next_path` kommt aus Login-Query `next` oder Default aus Base-Path:
  - Callback-Pfad endet auf `/api/auth/discord/callback`
  - Default ist der oeffentliche Base-Path davor, mit trailing slash
  - Beispiel ohne Base-Path: `/`
  - Beispiel mit `/builds/api/auth/discord/callback`: `/builds/`

## JWT: create_jwt(...)

Algorithmus:
- `HS256`

Secret-ENV-Reihenfolge:
1. `AUTH_SESSION_SECRET`
2. `JWT_SECRET`
3. `SESSIONS_ENCRYPTION_KEY`

Wenn beim Erstellen kein Secret vorhanden ist:
- `HTTPException 503`, Detail `Auth session secret is not configured`

Session-JWT-Claims:
```json
{
  "sub": "<discord_user_id>",
  "username": "<discord_name>",
  "display_name": "<display_name or username>",
  "avatar_url": "<avatar_url or null>",
  "role": "<role>",
  "iss": "<AUTH_SESSION_ISSUER default ddc-auth>",
  "aud": "<AUTH_SESSION_AUDIENCE default ddc-web>",
  "iat": "<unix seconds>",
  "exp": "<unix seconds>"
}
```

TTL:
- `AUTH_SESSION_TTL_SECONDS`
- Default: `30 * 24 * 60 * 60` = `2592000` Sekunden = 30 Tage.

Decode-Kompatibilitaet:
- `decode_jwt` gibt `None` zurueck, wenn kein Token oder kein Secret.
- Erste Validierung:
  - Algorithmus `HS256`
  - Audience `AUTH_SESSION_AUDIENCE`
  - Issuer `AUTH_SESSION_ISSUER`
- Fallback:
  - decodiert mit `HS256` ohne Audience/Issuer-Pruefung.
- Bei jeder Exception: `None`.

Wichtig fuer Rust:
- Bestehende Browser-Cookies koennen nur ohne Re-Login weiterlaufen, wenn Rust denselben Secret-Fallback, `HS256`, Numeric-Date-Claims und den Audience/Issuer-Fallback akzeptiert.

## Session cookie

Primaerer Name:
- `AUTH_COOKIE_NAME` oder Default `ddc_session`

Legacy-Name, der beim Lesen akzeptiert und bei Login/Logout geloescht wird:
- `auth_token`

Set-Cookie bei erfolgreichem Callback:
- Name: `ddc_session` (sofern `AUTH_COOKIE_NAME` nicht ueberschreibt)
- Wert: Session-JWT
- `Max-Age`: `AUTH_SESSION_TTL_SECONDS`, Default `2592000`
- `HttpOnly`: true
- `Secure`:
  - wenn `AUTH_COOKIE_SECURE` gesetzt: truthy (`1`, `true`, `yes`, `on`) -> true, sonst false
  - sonst wenn `AUTH_INSECURE_COOKIE` truthy: false
  - sonst bei Host `127.0.0.1`, `localhost`, `::1`: false
  - sonst true nur wenn `X-Forwarded-Proto` oder request scheme `https`
- `SameSite`: `AUTH_COOKIE_SAMESITE` oder Default `lax`
- `Path`: `AUTH_COOKIE_PATH` oder Default `/`
- `Domain`:
  - wenn `AUTH_COOKIE_DOMAIN` gesetzt: normalisiert, lower-case, fuehrender Punkt entfernt
  - sonst wenn Request-Host exakt `AUTH_DDC_COOKIE_DOMAIN` oder Subdomain davon: `AUTH_DDC_COOKIE_DOMAIN`
  - `AUTH_DDC_COOKIE_DOMAIN` Default: `deutsche-deadlock-community.de`
  - sonst kein Domain-Attribut, also host-only
- `Expires`: beim Setzen nicht explizit gesetzt, nur `Max-Age`.

Pre-Auth-Cookie:
- Name: `AUTH_PRE_AUTH_COOKIE_NAME` oder Default `ddc_pre_auth`
- Attribute identisch zu Session-Cookie
- `Max-Age`: `AUTH_PRE_AUTH_TTL_SECONDS`, Default `600`

Delete-Verhalten:
- `_delete_cookie_variants` loescht immer Host-only.
- Wenn `_cookie_domain(request)` eine Domain liefert, loescht es zusaetzlich mit Domain.
- Genutzte Delete-Parameter aus auth.py: `path` und optional `domain`; Starlette setzt dabei Expiration/Max-Age fuer Loeschung.

## Lokale DB-Abhaengigkeiten

`app.database.get_db()` nutzt SQLite:
- ENV `DB_PATH`
- Default: `Website/builds/backend/deadlock.db`
- Row-Factory: `aiosqlite.Row`

Auth-relevante Tabellen:
- `meta_users`
  - `id TEXT PRIMARY KEY`
  - `username`
  - `display_name`
  - `avatar_url`
  - `role TEXT DEFAULT 'user'`
- `coaches`
  - `discord_user_id INTEGER UNIQUE NOT NULL`
  - `status TEXT DEFAULT 'active'`

Login-Upsert:
- Wenn `meta_users.id = discord_id` existiert:
  - liest bestehende `role`
  - aktualisiert `username`, `display_name`, `avatar_url`
- Wenn nicht:
  - inserts mit `role='user'`
- Das JWT bekommt diese lokale Rolle.

Rollen werden bei jedem Request dynamisch aus `meta_users.role` nachgeladen:
- Payload-`role` ist nur Fallback, wenn keine DB-Rolle existiert.

## Dependency/helper behavior

### get_current_user_optional(request)

Spezialfall Forward-Auth:
- Wenn Header `X-Admin-Validated == "1"` und Client-IP/Host in `127.0.0.1`, `::1`, `localhost`:
  - gibt Admin-User zurueck:
    ```json
    {
      "id": "caddy-validated-admin",
      "username": "<X-Admin-User or admin>",
      "displayName": "<same>",
      "avatarUrl": null,
      "role": "admin",
      "sub": "caddy-validated-admin"
    }
    ```

Normalfall:
1. Liest Cookie:
   - zuerst `SESSION_COOKIE_NAME`
   - dann Legacy `auth_token`
2. Decodiert JWT.
3. Wenn Decode fehlschlaegt, `sub` fehlt oder `username` fehlt: `None`.
4. Laedt `role` aus `meta_users`; Fallback ist JWT-Claim `role` oder `user`.
5. Rueckgabe:
   ```json
   {
     "id": "<sub>",
     "username": "<username>",
     "displayName": "<display_name/displayName/username>",
     "avatarUrl": "<avatar_url/avatarUrl or null>",
     "role": "<role>",
     "sub": "<sub>"
   }
   ```

Nicht enthalten:
- kein `roles` Array
- kein `is_admin`
- kein `is_coach` in der Dependency-User-Dict-Shape

### require_authenticated_user(request)

- Ruft `get_current_user_optional`.
- Wenn kein User: `HTTP 401`, Detail `Not authenticated`.
- Sonst Rueckgabe der User-Dict-Shape oben.

### require_admin_user(request)

- Erst `require_authenticated_user`.
- Wenn `user["role"] != "admin"`: `HTTP 403`, Detail `Admin only`.
- Sonst Rueckgabe User.

### require_coach_user(request)

- Erst `require_authenticated_user`.
- Wenn `role == "admin"`: erlaubt.
- Sonst `_is_active_coach(user["sub"])`.
- `_is_active_coach`:
  - castet `user_id` zu int
  - Query: `SELECT 1 FROM coaches WHERE discord_user_id=? AND status='active'`
  - Exception -> false
- Wenn false: `HTTP 403`, Detail `Coach only`.
- Sonst Rueckgabe User.

## GET /me

Handler: `me(request)`.

Nicht eingeloggt:
```json
{"user": null}
```

Eingeloggt:
```json
{
  "user": {
    "id": "<user.id>",
    "username": "<user.username>",
    "displayName": "<user.displayName>",
    "avatarUrl": "<user.avatarUrl or null>",
    "role": "<user.role>",
    "is_coach": "<bool>"
  }
}
```

`is_coach`:
- true wenn `role == "admin"`
- sonst true wenn aktive `coaches`-Zeile fuer `sub`

Frontend erwartet:
- `User.id`
- `User.username`
- `User.displayName`
- `User.avatarUrl`
- `User.role` (`user`, `admin`, `builder` im TS-Typ)
- optional `User.is_coach`

Frontend berechnet:
- `isAdmin = user?.role === "admin"`
- `isCoach = user?.role === "admin" || !!user?.is_coach`

## POST /logout

Handler: `logout(request)`.

Antwort:
- HTTP `204`
- leerer Body

Geloescht:
- Session-Cookie `SESSION_COOKIE_NAME`, Default `ddc_session`
- Pre-Auth-Cookie `PRE_AUTH_COOKIE_NAME`, Default `ddc_pre_auth`
- Legacy-Cookie `auth_token`
- jeweils Host-only und, wenn passend, Domain-Variante.

Keine serverseitige Session-DB wird geloescht, weil die Website-Session stateless JWT ist.

## Rollenmodell fuer Builds-Auth

Authenticated:
- Jeder Request mit gueltigem Session-JWT und gueltigem `sub` + `username`.

Admin:
- Primaer lokale DB-Rolle `meta_users.role == "admin"`.
- Alternativ nur fuer trusted loopback Forward-Auth: `X-Admin-Validated: 1`.
- Keine Discord-Admin-Role-ID in `auth.py`.
- Keine Discord-Guild-Permission-Pruefung in `auth.py`.

Coach:
- Admin ist automatisch Coach.
- Sonst lokale Website-DB: aktive Zeile in `coaches` fuer Discord-ID.
- Keine Discord-Coach-Role-ID in `auth.py`.

Discord-Rollen:
- Zentraler Dienst kann Role-IDs liefern.
- Builds-Auth ignoriert sie fuer Role/Coach/Admin.

Hardcodierte zentrale Dashboard-Adminwerte, nicht fuer Builds-Role-Entscheidung:
- Dashboard-Owner-User-ID: `662995601738170389`
- Dashboard-Moderator-Role-ID: `1337518124647579661`
- Diese gelten fuer `/auth/discord/login` des Admin-Dashboards, nicht fuer `/api/auth/discord/login` der Builds-Website.

## ENV-/Keyring-Vertrag fuer Rust

| Name | Zweck | Aktueller Default/Verhalten |
|---|---|---|
| `AUTH_COOKIE_NAME` | Primaerer Website-Session-Cookie-Name | `ddc_session` |
| `AUTH_PRE_AUTH_COOKIE_NAME` | Pre-Auth-State-Cookie vor Discord-Callback | `ddc_pre_auth` |
| `AUTH_SESSION_TTL_SECONDS` | Max-Age und JWT-exp fuer Website-Session | `2592000` |
| `AUTH_PRE_AUTH_TTL_SECONDS` | Max-Age und JWT-exp fuer Pre-Auth-State | `600` |
| `AUTH_SESSION_AUDIENCE` | JWT `aud` und strikte Decode-Pruefung | `ddc-web` |
| `AUTH_SESSION_ISSUER` | JWT `iss` und strikte Decode-Pruefung | `ddc-auth` |
| `AUTH_SESSION_SECRET` | bevorzugtes HS256-JWT-Secret | kein Default, required zum Signieren |
| `JWT_SECRET` | Fallback-HS256-JWT-Secret | kein Default |
| `SESSIONS_ENCRYPTION_KEY` | zweiter Fallback fuer JWT-Secret | kein Default |
| `AUTH_COOKIE_DOMAIN` | explizite Cookie-Domain, uebersteuert DDC-Autodomain | leer -> Auto |
| `AUTH_DDC_COOKIE_DOMAIN` | Auto-Domain fuer DDC-Host/Subdomains | `deutsche-deadlock-community.de` |
| `AUTH_COOKIE_PATH` | Cookie-Path fuer Session und Pre-Auth | `/` |
| `AUTH_COOKIE_SAMESITE` | Cookie-SameSite fuer Session und Pre-Auth | `lax` |
| `AUTH_COOKIE_SECURE` | explizites Secure-Flag | unset -> Auto |
| `AUTH_INSECURE_COOKIE` | erzwingt unsichere Cookies, wenn `AUTH_COOKIE_SECURE` unset | false |
| `AUTH_PUBLIC_CALLBACK_URL` | externe Website-Callback-URL fuer `redirect_after` zum zentralen OAuth-Dienst | unset -> aus Request-Forwarded-Headern bauen |
| `DASHBOARD_INTERNAL_API_BASE` | Basis-URL des zentralen Dashboard-OAuth-Dienstes | `http://127.0.0.1:8766` |
| `WEBSITE_INTERNAL_API_TOKEN` | erster Token-Kandidat fuer Website -> Dashboard `X-Internal-Token` | kein Default; Dashboard akzeptiert diesen Namen aktuell nicht direkt |
| `TURNIER_INTERNAL_API_TOKEN` | weiterer Token-Kandidat; zentraler Dienst akzeptiert ihn | kein Default |
| `MASTER_BROKER_TOKEN` | zentraler Dienst akzeptiert ihn als Turnier/Internal-Token-Fallback | kein Default |
| `MAIN_BOT_INTERNAL_TOKEN` | weiterer Token-Kandidat; zentraler Dienst akzeptiert ihn ueber Turnier-Token-Fallback | kein Default |
| `TWITCH_INTERNAL_API_TOKEN` | weiterer Token-Kandidat; zentraler Dienst akzeptiert ihn | kein Default |
| `DB_PATH` | SQLite-DB fuer `meta_users` und `coaches` | `Website/builds/backend/deadlock.db` |
| `DISCORD_OAUTH_CLIENT_ID` | zentraler Discord-OAuth-Client-ID-ENV/Keyring-Name | ENV zuerst, sonst Keyring `DeadlockBot`, sonst Bot-Application-ID |
| `DISCORD_OAUTH_CLIENT_SECRET` | zentraler Discord-OAuth-Client-Secret-ENV/Keyring-Name | ENV zuerst, sonst Keyring `DeadlockBot`, kein Fallback |
| `DEADLOCK_OAUTH_STATE_TTL_SECONDS` | zentrale DB-TTL fuer delegated OAuth-States in `oauth_states` | `21600` |
| `MASTER_DASHBOARD_OAUTH_STATE_TTL_SEC` | zentrale in-memory Dashboard-OAuth-State-TTL fuer Admin-Login, nicht DB-State-TTL der delegated Builds-States | Default `21600` |

Nicht vorhanden in dieser Builds-Auth-Schicht:
- kein `FRONTEND_URL` fuer Post-Login-Redirect
- kein `DISCORD_GUILD_ID` in `auth.py`
- kein Coach/Admin-Discord-Role-ID-ENV in `auth.py`
- kein `roles` Claim im Website-JWT

## Rust-Cutover-Kompatibilitaetsrisiko

Groesstes Risiko ohne Re-Login: bestehende Browser haben ein `ddc_session`-JWT, das mit Python-`jose` als `HS256` signiert wurde und ueber Domain/Path/SameSite/Secure sichtbar ist. Rust muss exakt kompatibel sein bei:
- Cookie-Name `ddc_session`
- Cookie-Domain-Logik (`deutsche-deadlock-community.de` fuer DDC-Subdomains, sonst host-only)
- Cookie-Path `/`
- Secret-Fallback-Reihenfolge `AUTH_SESSION_SECRET` -> `JWT_SECRET` -> `SESSIONS_ENCRYPTION_KEY`
- JWT-Algorithmus `HS256`
- Claim-Namen `sub`, `username`, `display_name`, `avatar_url`, `role`, `iss`, `aud`, `iat`, `exp`
- Decode-Fallback ohne Audience/Issuer-Pruefung
- dynamischem Nachladen der Rolle aus `meta_users`

Wenn einer dieser Punkte abweicht, bleiben Cookies zwar im Browser, werden aber von Rust nicht gelesen oder nicht akzeptiert, was faktisch alle Nutzer ausloggt.
