# Briefing Worker A — Coaching-Nudge, Bot-Seite (Paket A)

status: aktiv (2026-09-21)

Du arbeitest Paket A des Auftrags
`/home/nathanael/repos/Deadlock-Bots/.tasks/2026-09-21-coaching-realtime-nudge/AUFTRAG.md`.
Lies diese Datei zuerst vollständig; der Interface-Vertrag dort ist bindend.
Paket B (Website-Backend) läuft parallel in einem eigenen Thread — halte dich
exakt an den Vertrag, damit A und B zusammenpassen.

## 1. Referenzauszüge (wörtlich, Stand bb03deb5)

Notification-Loop, `rust/crates/dl-community/src/coaching.rs:498-504`:

```rust
    let notification_task = tokio::spawn(async move {
        loop {
            sync.process_notifications().await;
            tokio::time::sleep(NOTIFICATION_INTERVAL).await;
        }
    });
    vec![role_task, role_event_task, notification_task]
```

Intervalle, `rust/crates/dl-community/src/coaching.rs:20-21`:

```rust
pub const ROLE_SYNC_INTERVAL: Duration = Duration::from_secs(600);
pub const NOTIFICATION_INTERVAL: Duration = Duration::from_secs(60);
```

Spawn-Signatur, `rust/crates/dl-community/src/coaching.rs:459-462`:

```rust
pub fn spawn(
    sync: Arc<CoachingSync>,
    dispatcher: &dl_discord::Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
```

Broker-State, `rust/crates/dl-broker/src/lib.rs:62-72`:

```rust
pub struct BrokerState {
    pub port: Arc<dyn DiscordPort>,
    pub channel_info: Arc<dyn ChannelInfoPort>,
    pub token: String,
    pub store: IdempotencyStore,
    pub channel_allowlist: Allowlist,
    pub guild_allowlist: Allowlist,
    pub role_allowlist: Allowlist,
}
pub type SharedBroker = Arc<BrokerState>;
```

Routenschema, `rust/crates/dl-broker/src/lib.rs:142-260` (Auszug):

```rust
pub fn router(state: SharedBroker) -> Router {
    Router::new()
        .route("/internal/master/v1/health", get(handlers::health))
        .route(
            "/internal/master/v1/discord/send-dm",
            post(handlers::send_dm),
        )
```

Auth-Header, `rust/crates/dl-broker/src/lib.rs:30-32`:

```rust
pub const TOKEN_HEADER: &str = "X-Internal-Token";
pub const IDEMPOTENCY_HEADER: &str = "X-Idempotency-Key";
pub const REQUEST_ID_HEADER: &str = "X-Request-Id";
```

Verkabelung im Bot, `rust/bin/dl-bot/src/main.rs:1258` („Master-Broker :8770 —
Token-Kette wie das Original", Broker-Bau darin, Adresse `main.rs:1273`) und
`main.rs:1558-1568`:

```rust
        match coaching_website.clone() {
            Some(website) => {
                let sync = Arc::new(dl_community::coaching::CoachingSync {
                    // ...
                    request_sink: Some(coaching_requests.clone()),
                });
                dl_community::coaching::spawn(sync, &dispatcher);
```

## 2. Scope-Zaun

Exakt dieser Auftrag: Nudge-Route + Wake-Feld am Broker, select!-Umbau des
Notification-Tasks, Verkabelung in main.rs, Tests. Kein Refactoring, kein fmt
über den Diff hinaus, keine Änderungen an Roster-Sync, Zustelllogik, DM-Texten,
Acks oder am 60-s-Fallback. Ignoriere andere Doku im Repo.

## 3. Branch und Commit-Regeln

- Worktree: `/home/nathanael/.worktrees/coaching-nudge-bots-20260921`
- Branch: `feat/coaching-notifications-nudge-20260921` (Basis bb03deb5 =
  main). Der Haupt-Checkout `/home/nathanael/repos/Deadlock-Bots` hat
  uncommittete Fremdänderungen — dort nichts anfassen, nie dort committen.
- Commit und Push auf den eigenen Branch sind erlaubt, nie main.

## 4. Beweisziel

`cargo test -p dl-broker -p dl-community` grün und `cargo build -p dl-bot`
fehlerfrei im Worktree. Die Tests beweisen: der neue Handler antwortet 200 mit
`{"ok": true}` (bei fehlendem/ falschem Token abgewiesen wie die anderen
Routen) und ruft `Notify::notify_one()`; der Notification-Task verlässt den
60-s-Sleep sofort, wenn der Notify gefeuert wird, und ruft
`process_notifications()` (Test mit Test-Client, der due/ack aufzeichnet).
Schreib keine Test-/Faktor-Zeremonie über den Beweis hinaus.

## 5. Intent-Thread und Bump-up

Melde dich mit
`[Bump-up] Paket A: Grund: ... Erledigt: ... Worktree: ... Offen: ...`
an den Intent-Thread `7581f506-c812-4c9e-bafe-b85c9deff6cd` und stoppe danach.
Bei einer Rückfrage vor Abschluss gilt dasselbe Format mit Grund „Rückfrage".

## Rahmen

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder
  Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben, Code erklärt sich selbst.
- Nur den eigenen Branch pushen, nie main.
