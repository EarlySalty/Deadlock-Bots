# Slice 1 Frontend Map - `Website/dl-coaching`

Stand: 2026-06-29. Analyse read-only; einzige Schreibdatei ist dieses Dokument.

## Quellen

Gelesen:

- `Website/dl-coaching/src/App.tsx`
- `Website/dl-coaching/src/api/client.ts`
- `Website/dl-coaching/src/context/AuthContext.tsx`
- `Website/dl-coaching/src/types/index.ts`
- `Website/dl-coaching/src/pages/CoachDashboardPage.tsx`
- `Website/dl-coaching/src/pages/CoachDetailPage.tsx`
- `Website/dl-coaching/src/pages/CoacheeDetailPage.tsx`
- `Website/dl-coaching/src/pages/CoachesPage.tsx`
- `Website/dl-coaching/src/pages/CoachingRequestPage.tsx`
- `Website/dl-coaching/src/pages/CoachOverviewPage.tsx`
- `Website/dl-coaching/src/pages/MyCoachingPage.tsx`
- ergänzend: `src/main.tsx`, `src/components/ui.tsx`, `src/components/Layout.tsx`, `vite.config.ts`, `package.json`

## Routing

`vite.config.ts` setzt `base: '/coaching/'`. `src/main.tsx` nutzt `BrowserRouter basename={import.meta.env.BASE_URL.replace(/\/$/, '')}`.

| React-Route | Effektiver Pfad | Seite |
|---|---:|---|
| `/` | `/coaching/` | `CoachesPage` |
| `/anfrage` | `/coaching/anfrage` | `CoachingRequestPage` |
| `/coaches/:id` | `/coaching/coaches/:id` | `CoachDetailPage` |
| `/dashboard` | `/coaching/dashboard` | `CoachDashboardPage` |
| `/overview` | `/coaching/overview` | `CoachOverviewPage` |
| `/coachees/:id` | `/coaching/coachees/:id` | `CoacheeDetailPage` |
| `/me` | `/coaching/me` | `MyCoachingPage` |

## API-/Auth-Basis

Es gibt keine eigene `VITE_API_*`- oder `VITE_AUTH_*`-Konfiguration.

| Zweck | Quelle | Name | Ergebnis im aktuellen Vite-Build |
|---|---|---|---|
| Router-Basename | Vite built-in | `import.meta.env.BASE_URL` | `/coaching/` laut `vite.config.ts` |
| API-Basis | `src/api/client.ts` | `BASE` | `${BASE_URL ohne Slash}/api`, also `/coaching/api` |
| Auth-Basis | `src/context/AuthContext.tsx` | `authBase` | `${BASE_URL ohne Slash}/api/auth`, also `/coaching/api/auth` |
| Dev-Proxy | `vite.config.ts` | kein ENV-Name | `/coaching/api` -> `http://localhost:8772/api` |

Gefundene `.env*`-Dateien im Verzeichnis `Website/dl-coaching`: keine.

Alle API-Requests aus `src/api/client.ts` senden `credentials: 'include'` und `Content-Type: application/json`. `AuthContext` verwendet ebenfalls Cookie-Credentials.

## AuthContext

Globale Auth-Aktionen:

| Aktion | Methode + Pfad | Verhalten | Shape |
|---|---|---|---|
| Session prüfen | `GET /api/auth/me` | beim Mount des `AuthProvider` | `{ user: User | null }` |
| Login | Browser-Redirect zu `/api/auth/discord/login?next=...` | `next` ist aktuelle Route inkl. Query/Hash | kein JSON |
| Logout | `POST /api/auth/logout` | setzt lokalen `user` auf `null` | Response wird ignoriert |

`User` aus `src/types/index.ts`:

```ts
{
  id: string
  username: string
  displayName: string
  avatarUrl: string
  role: 'user' | 'admin' | 'builder'
  is_coach?: boolean
}
```

`isCoach` ist im Frontend `user.role === 'admin' || !!user.is_coach`.

## Seiteninventar

Die Pfade in dieser Tabelle sind HTTP-Pfade inklusive `/api`; der Client-Wrapper bekommt intern nur den Teil nach `/api`.

| Seite | Route | Auth | API-Endpunkte | Erwartete Shapes |
|---|---|---|---|---|
| `CoachesPage` | `/` | öffentlich | `GET /api/coaching/coaches` | `CoachProfile[]` |
| `CoachDetailPage` | `/coaches/:id` | öffentlich | `GET /api/coaching/coaches/:id`; `GET /api/coaching/coaches/:id/reviews` | `CoachProfile`; `CoachReview[]` |
| `CoachingRequestPage` | `/anfrage` | Login für Formular; ohne Login nur Auth-CTA | `POST /api/coaching/requests` | Request: `CreateCoachingRequest`; Response: `CoachingRequest` |
| `CoachDashboardPage` | `/dashboard` | Coach-only | `GET /api/coaching/platform/appointments?scope=mine`; `GET /api/coaching/platform/queue`; `GET /api/coaching/platform/coachees`; `GET /api/coaching/platform/coaches/me`; `POST /api/coaching/platform/appointments`; `PATCH /api/coaching/platform/appointments/:id`; `PATCH /api/coaching/platform/coaches/me` | `{ appointments: Appointment[] }`; `{ requests: PlatformQueueRequest[] }`; `{ coachees: CoacheeListItem[] }`; `CoachSelf`; `{ id: string }` bei Appointment-Create; Patch-Responses ignoriert |
| `CoachOverviewPage` | `/overview` | Coach-only | `GET /api/coaching/platform/overview` | `{ coaches: PlatformCoachStat[], recent_sessions: PlatformRecentSession[] }` |
| `CoacheeDetailPage` | `/coachees/:id` | Coach-only | `GET /api/coaching/platform/coachees/:id`; `PATCH /api/coaching/platform/coachees/:id`; `POST /api/coaching/platform/coachees/:coacheeId/goals`; `PATCH /api/coaching/platform/goals/:goalId`; `DELETE /api/coaching/platform/goals/:goalId`; `POST /api/coaching/platform/goals/:goalId/milestones`; `PATCH /api/coaching/platform/milestones/:milestoneId`; `DELETE /api/coaching/platform/milestones/:milestoneId`; `POST /api/coaching/platform/appointments`; `PATCH /api/coaching/platform/appointments/:id`; `POST /api/coaching/platform/coachees/:coacheeId/notes`; `PATCH /api/coaching/platform/notes/:noteId`; `DELETE /api/coaching/platform/notes/:noteId` | `CoacheeDetail`; Patch/Delete-Responses ignoriert; Creates liefern `{ id: string }` |
| `MyCoachingPage` | `/me` | Login erforderlich; kein Coach-only | `GET /api/coaching/platform/me` | `MyCoaching` |

## Relevante Daten-Shapes

`CoachProfile`:

```ts
{
  id: string
  display_name: string
  discord_username: string
  avatar_url: string | null
  bio: string | null
  specialties: string[]
  availability: Record<string, string>
  status: string
  avg_rating: number
  total_reviews: number
  total_sessions: number
  twitch_url?: string | null
}
```

`CoachReview`:

```ts
{
  id: string
  coach_id: string
  user_display_name: string
  rating: number
  feedback_text: string | null
  improved_areas: string | null
  created_at: string
}
```

`CreateCoachingRequest`:

```ts
{
  display_name?: string
  rank: string
  subrank?: string
  hero?: string
  games_played?: string
  hours_played?: string
  availability?: string
  current_problems: string
  preferred_coach_id?: string
}
```

`CoachingRequest`:

```ts
{
  id: string
  discord_username: string
  rank: string
  subrank: string
  hero: string | null
  games_played: string | null
  hours_played: string | null
  availability: string | null
  current_problems: string | null
  status: string
  created_at: string
}
```

`PlatformQueueRequest`:

```ts
{
  id: string
  discord_user_id: number
  discord_username: string | null
  rank: string | null
  subrank: string | null
  hero: string | null
  games_played: string | null
  hours_played: string | null
  availability: string | null
  current_problems: string | null
  status: string
  assigned_coach_id: string | null
  assigned_coach_username: string | null
  reserved_until: number | null
  created_at: string
  reserved_for_me: boolean
  is_open: boolean
}
```

`CoacheeDetail`:

```ts
{
  profile: CoacheeProfile
  goals: Goal[]
  notes: SessionNote[]
  sessions: PlatformSession[]
  appointments?: Appointment[]
}
```

`MyCoaching`:

```ts
{
  profile: CoacheeProfile | null
  goals: Goal[]
  notes: SessionNote[]
  sessions: PlatformSession[]
  appointments?: Appointment[]
}
```

`Appointment`:

```ts
{
  id: string
  coach_id: string | null
  coachee_id: string | null
  scheduled_at: string
  duration_minutes: number
  title: string | null
  note: string | null
  status: 'scheduled' | 'done' | 'cancelled' | string
  coachee_display?: string | null
  coach_display?: string | null
  created_at?: string
  updated_at?: string
}
```

## Bestehende wiederverwendbare Patterns

- API-Wrapper in `src/api/client.ts`: neues `scrim`-Objekt neben `coaching` und `coachingPlatform`.
- React Query: `useQuery`, `useMutation`, Query-Key-Konventionen, Invalidierung via `useQueryClient`.
- Auth-Gates: `useAuth()`, `PageSpinner`, `CoachOnly`.
- Login-Gate aus `CoachingRequestPage` und `MyCoachingPage`.
- Form-Layout aus `CoachingRequestPage`.
- Coach-Tabs aus `CoachDashboardPage`; fuer Scrim-Pool als dritter Tab erweiterbar.
- Listen/Karten aus `CoachDashboardPage` (`QueueCard`, `CoacheeRow`) fuer Pool-Eintraege.
- Tabellenlayout aus `CoachOverviewPage` fuer Coach-Pool.
- naechster-Termin-Banner und Timeline-Pattern aus `MyCoachingPage` fuer "Mein Team + naechstes Match".
- Komponenten: `SectionHead`, `EmptyState`, `PageSpinner`, `CoachOnly`, `Avatar`, `SessionStatusBadge`.
- Datumshelfer: `fmtDate`, `fmtDateTime`, `parseUtc`, `isUpcoming`.

## Scrim-Datenbasis

Gelesene Rust-/Schema-Basis:

- `Deadlock-Bots/rust/docs/specs/2026-06-29-coaching-scrim-program-design.md`
- `Deadlock-Bots/rust/docs/db-schema.sql`
- `Deadlock-Bots/rust/crates/dl-squads/src/store.rs`

Aktuelle Tabellen:

```sql
scrim_participant(
  id, discord_id, display_name, rank, rank_source, rank_verified,
  roles, availability, status, source, created_at, updated_at
)

scrim_team(
  id, name, coach, discord_role_id, discord_channel_id, created_at
)

scrim_team_member(
  team_id, participant_id, role, is_captain, is_bench
)

scrim_match(
  id, team_a_id, team_b_id, when_text, scheduled_at, status, created_at
)
```

Wichtig: Die Spec nennt `heroes`, das aktuelle Schema hat aber keine `heroes`-Spalte. Fuer ein "volles Profil" muss Slice 1 entweder eine Schema-Erweiterung fuer `heroes` bekommen oder das Feld bewusst aus der ersten Web-Form-Version ausklammern.

## Slice-1 Vorschlag: neue Frontend-Teile

Ich wuerde Scrim fachlich unter `/api/scrim/...` fuehren, nicht unter `/api/coaching/...`: Die Daten liegen in `dl-squads`/`scrim_*`, und 1:1-Coaching bleibt sauber getrennt.

### 1. Spieler-Sicht: "Mein Team + naechstes Match"

Frontend:

- neue Seite: `src/pages/MyScrimPage.tsx`
- Route: `/scrim/me` effektiv `/coaching/scrim/me`
- Navigation: fuer eingeloggte User neben `/me` als "Mein Scrim" oder im Spielerbereich
- Auth: Login erforderlich, kein Coach-only

API:

| Methode + Pfad | Zweck | Auth | Shape |
|---|---|---|---|
| `GET /api/scrim/me` | Aktuellen User per Session-Discord-ID im Scrim-Pool finden, Team und naechstes Match laden | logged-in | `{ participant: ScrimParticipant | null, team: ScrimTeamWithMembers | null, next_match: ScrimMatchWithTeams | null }` |

Empfohlene Response-Shapes:

```ts
type ScrimParticipantStatus =
  | 'new'
  | 'profile_complete'
  | 'admitted'
  | 'assigned'
  | 'bench'
  | 'waitlist'
  | 'left'

interface ScrimParticipant {
  id: number
  discord_id: number | null
  display_name: string
  rank: string | null
  rank_source: 'self' | 'steam' | string
  rank_verified: boolean
  roles: string[] | string | null
  availability: Record<string, unknown> | string | null
  status: ScrimParticipantStatus | string
  source: 'discord_reaction' | 'web_form' | 'seed' | string
  created_at: string
  updated_at: string
}

interface ScrimTeamWithMembers {
  id: number
  name: string
  coach: string | null
  discord_role_id: number | null
  discord_channel_id: number | null
  members: Array<ScrimParticipant & {
    team_role: string | null
    is_captain: boolean
    is_bench: boolean
  }>
}

interface ScrimMatchWithTeams {
  id: number
  team_a: Pick<ScrimTeamWithMembers, 'id' | 'name'>
  team_b: Pick<ScrimTeamWithMembers, 'id' | 'name'> | null
  when_text: string | null
  scheduled_at: string | null
  status: 'planned' | string
  created_at: string
}
```

Wiederverwendung:

- `MyCoachingPage`: Login-Gate, "naechster Termin"-Banner, Timeline-/Kartenlogik.
- `SessionStatusBadge` oder kleines Scrim-Status-Badge mit gleicher visueller API.
- `SectionHead`, `EmptyState`, `PageSpinner`.

### 2. Web-Formular: Pool-Eintrag mit vollem Profil

Frontend:

- neue Seite: `src/pages/ScrimSignupPage.tsx`
- Route: `/scrim/anmeldung` effektiv `/coaching/scrim/anmeldung`
- Auth: empfohlen logged-in, damit `discord_id` aus Session kommt und Discord-Reaktions-Teilnehmer sauber geupsertet werden.
- Verhalten: bestehendes Profil laden, Formular vorfuellen, Submit als Upsert.

API:

| Methode + Pfad | Zweck | Auth | Shape |
|---|---|---|---|
| `GET /api/scrim/participants/me` | vorhandenen Pool-Eintrag fuer Formular-Prefill laden | logged-in | `{ participant: ScrimParticipant | null }` |
| `PUT /api/scrim/participants/me` | Pool-Eintrag des eingeloggten Users upserten; Backend setzt `discord_id`, `source='web_form'`, `status='profile_complete'` | logged-in | Request: `ScrimParticipantForm`; Response: `ScrimParticipant` |

Empfohlener Request:

```ts
interface ScrimParticipantForm {
  display_name?: string
  rank: string
  rank_source?: 'self'
  roles: string[]
  availability: Record<string, unknown>
  heroes?: string[]
}
```

Backend-Hinweis:

- `roles` und `availability` sind im aktuellen Schema `TEXT`; API sollte sie als JSON-Arrays/-Objekte annehmen und serverseitig serialisieren.
- `heroes` braucht eine Schema-Entscheidung. Nicht als ad-hoc-String in ein fremdes Feld quetschen.

Wiederverwendung:

- `CoachingRequestPage`: Login-Gate, Success-`EmptyState`, Formularstruktur, Submit-Mutation.
- `AuthContext.user`: Display-Name und Avatar fuer Form-Header.
- `input-field`, `btn-amber`, `btn-ghost` CSS-Klassen.

### 3. Coach-Sicht: Scrim-Pool mit Status-Filter

Frontend:

- neue Seite: `src/pages/ScrimPoolPage.tsx`
- Route: `/scrim/pool` effektiv `/coaching/scrim/pool`
- Auth: Coach-only
- Navigation: `CoachTabs` um `scrim-pool` erweitern oder separater Scrim-Tab im Coach-Bereich.

API:

| Methode + Pfad | Zweck | Auth | Shape |
|---|---|---|---|
| `GET /api/scrim/pool?status=...` | Pool mit optionalem Status-Filter laden | Coach-only | `{ participants: ScrimParticipant[] }` |
| `PATCH /api/scrim/participants/:id/status` | Statuswechsel, z. B. `waitlist -> admitted`, `admitted -> assigned/bench/left` | Coach-only | Request: `{ status: ScrimParticipantStatus }`; Response: `ScrimParticipant` oder `{ ok: true }` |

Statusfilter:

```ts
['new', 'profile_complete', 'admitted', 'assigned', 'bench', 'waitlist', 'left']
```

Wiederverwendung:

- `CoachDashboardPage`: Coach-Guard, `CoachTabs`, Card-Listen, Suchfeld-Pattern, React-Query-Invalidierung.
- `CoachOverviewPage`: Tabellenlayout fuer dichte Pool-Ansicht.
- `SectionHead`, `EmptyState`, `PageSpinner`, `CoachOnly`.

## Vorgeschlagene Endpoint-Liste gesamt

Minimal fuer Slice 1:

```text
GET   /api/scrim/me
GET   /api/scrim/participants/me
PUT   /api/scrim/participants/me
GET   /api/scrim/pool?status=<status>
PATCH /api/scrim/participants/:id/status
```

Optional, falls das Frontend Match-/Teamdetails separat nachladen soll statt alles ueber `/api/scrim/me` zu bekommen:

```text
GET /api/scrim/teams/:id
GET /api/scrim/teams/:id/matches?upcoming=1
```

Meine Empfehlung: fuer Slice 1 bei `GET /api/scrim/me` aggregieren, damit die Spieler-Seite nur einen Request braucht.

## Offene Punkte / Risiken

- Kein `VITE_*` fuer API/Auth vorhanden; ein spaeterer Rust-Port muss entweder weiter unter `/coaching/api` laufen oder die Vite-Konfiguration/API-Basis bewusst umbauen.
- `heroes` ist in der Scrim-Spec enthalten, aber nicht im aktuellen `scrim_participant`-Schema.
- `dl-squads` hat Store-Funktionen fuer Pool/Seed/Team/Match, aber in den gelesenen Treffern noch keine Website-HTTP-Routen unter `/api/scrim`.
- `WORKFLOW.md` wurde gelesen, aber wegen der expliziten Read-only-Regel dieser Worker-Aufgabe nicht aktualisiert.
