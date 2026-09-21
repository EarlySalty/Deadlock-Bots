# Briefing Worker B — Coaching-Nudge, Website-Backend (Paket B)

status: aktiv (2026-09-21)

Du arbeitest Paket B des Auftrags
`/home/nathanael/repos/Deadlock-Bots/.tasks/2026-09-21-coaching-realtime-nudge/AUFTRAG.md`.
Lies diese Datei zuerst vollständig; der Interface-Vertrag dort ist bindend.
Paket A (Discord-Bot) läuft parallel in einem eigenen Thread — halte dich exakt
an den Vertrag, damit A und B zusammenpassen. Dein Arbeitsbereich ist
`builds/backend-rust` im Website-Repo.

## 1. Referenzauszüge (wörtlich, Stand b271acc)

Fällige Notifications sind abgeleiteter Zustand,
`builds/backend-rust/src/routes/platform.rs:994-1029` (Auszug):

```rust
pub async fn notifications_due(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<Value>> {
    require_bot_token(&headers)?;
    let now = Utc::now();
    let in_2h = now + Duration::hours(2);
    let rows = sqlx::query(
        "SELECT 'created' AS type, a.id AS appointment_id, co.discord_user_id, \
                COALESCE(co.display_name, co.discord_username) AS coachee_display, \
                COALESCE(c.display_name, c.discord_username) AS coach_display, \
                a.scheduled_at, a.duration_minutes, a.title, a.note \
         FROM coaching.appointments a \
         JOIN coaching.coachees co ON a.coachee_id=co.id \
         JOIN coaching.coaches c ON a.coach_id=c.id \
         WHERE a.status='scheduled' AND a.notify_created_at IS NULL \
         UNION ALL \
         SELECT 'reminder' AS type, ... \
         WHERE a.status='scheduled' AND a.notify_reminder_at IS NULL \
           AND a.notify_created_at IS NOT NULL AND a.scheduled_at BETWEEN $1 AND $2 \
         UNION ALL \
         SELECT 'cancelled' AS type, ... \
         WHERE a.status='cancelled' AND a.notify_cancelled_at IS NULL \
           AND a.notify_created_at IS NOT NULL",
    )
```

Mutationstellen: `builds/backend-rust/src/routes/platform.rs:838`
`create_appointment` (INSERT `coaching.appointments` in `platform.rs:866`),
`platform.rs:928` `update_appointment` mit Status-Lese `platform.rs:946`
(`SELECT coach_id, status FROM coaching.appointments WHERE id=$1`).
Requests entstehen in `builds/backend-rust/src/routes/coaching.rs:291`
`create_coaching_request`, INSERT in `coaching.rs:364` (INSERT INTO
`coaching.requests`, `website_request_id` in `coaching.rs:338-353`).

Bestehendes Broker-Client-Muster, `builds/backend-rust/src/discord_broker.rs`
(Auszüge):

```rust
            base: cfg.master_broker_base.clone(),
```
```rust
        format!(
            "{}/internal/master/v1/discord/send-dm",
            self.base
        ),
```

Config, `builds/backend-rust/src/config.rs:94,174-177`:

```rust
    pub master_broker_base: String,
```
```rust
            master_broker_base: env_or("MASTER_BROKER_BASE", "http://127.0.0.1:8770"),
            master_broker_token: env_or(...),
```

## 2. Scope-Zaun

Exakt dieser Auftrag: ein Best-effort-Nudge-Helfer (neues kleines Modul neben
`discord_broker.rs` oder dort eingliedern) und dessen Aufruf nach den drei
Mutationen `create_appointment`, `update_appointment` (nur Statuswechsel auf
'cancelled'), `create_coaching_request` (nur website-originated,
website_request_id gesetzt). Kein Refactoring, kein fmt über den Diff hinaus,
keine Änderungen an `notifications_due`/`notifications_ack`, an der
Zustelllogik, am Schema oder an Caddy. Ignoriere andere Doku im Repo. Reminder
bekommen bewusst keinen Nudge (2-h-Fenster, Fallback-Poll reicht).

## 3. Branch und Commit-Regeln

- Worktree: `/home/nathanael/.worktrees/coaching-nudge-website-20260921`
- Branch: `feat/coaching-notifications-nudge-20260921` (Basis b271acc =
  main). Haupt-Checkout `/home/nathanael/repos/Website` nicht anfassen, nie
  dort committen.
- Commit und Push auf den eigenen Branch sind erlaubt, nie main.

## 4. Beweisziel

`cargo test` für `builds/backend-rust` grün und Build fehlerfrei im Worktree.
Bewiesen wird: der Helper sendet `POST {MASTER_BROKER_BASE}/internal/master/v1/
coaching/notifications-nudge` mit Header `X-Internal-Token` aus der
MASTER_BROKER_TOKEN-Kette (Verdrahtungstest im Stil der bestehenden
discord_broker-Tests, Base `http://127.0.0.1:8770`), er läuft feuer-und-
vergessen (der Request-Pfad awaitet ihn nicht, ein Fehler erzeugt nur
`tracing::warn!`), und er ist genau an den drei Mutationen verdrahtet.
Schreib keine Test-/Faktor-Zeremonie über den Beweis hinaus.

## 5. Intent-Thread und Bump-up

Melde dich mit
`[Bump-up] Paket B: Grund: ... Erledigt: ... Worktree: ... Offen: ...`
an den Intent-Thread `7581f506-c812-4c9e-bafe-b85c9deff6cd` und stoppe danach.
Bei einer Rückfrage vor Abschluss gilt dasselbe Format mit Grund „Rückfrage".

## Rahmen

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder
  Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben, Code erklärt sich selbst.
- Nur den eigenen Branch pushen, nie main.
