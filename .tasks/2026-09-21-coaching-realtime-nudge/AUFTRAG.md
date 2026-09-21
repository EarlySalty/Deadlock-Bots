# Auftrag: Coaching-Benachrichtigungen realtime (Nudge statt 60-s-Poll)

status: aktiv (2026-09-21)

## Ziel

Der Discord-Bot holt fällige Coaching-Benachrichtigungen heute per 60-s-Poll
(`GET /coaching/platform/notifications/due`). Ein Termin/ eine Anfrage soll
sofort nach dem Anlegen in Discord ankommen. Die Coaching-Plattform (Website-
Backend, 127.0.0.1:8772) stößt die Zustellung deshalb per Nudge auf dem
bestehenden Master-Broker (:8770) an; der 60-s-Poll bleibt als Fallback.

Zwei Pakete, ein Interface-Vertrag:

- **Paket A (Deadlock-Bots):** Nudge-Endpunkt am Master-Broker + Wake des
  Notification-Loops.
- **Paket B (Website, builds/backend-rust):** Best-effort-Nudge nach jeder
  Mutation, die eine fällige Notification erzeugt.

## Interface-Vertrag (bindend für beide Pakete)

- Route: `POST /internal/master/v1/coaching/notifications-nudge`
- Auth: wie alle Broker-Routen, Header `X-Internal-Token` (Token-Kette
  MASTER_BROKER_TOKEN, siehe BrokerState.token).
- Request-Body: leer oder `{"reason": "<kurz>"}` — Handler liest ihn nicht aus.
- Response: `200 {"ok": true}`. Der Nudge ist inhärent idempotent, kein
  Idempotency-Key, kein Envelope nötig.
- Wirkung Bot-Seite: ausstehendes `process_notifications()` läuft sofort;
  der 60-s-Timer bleibt unverändert als Fallback bestehen.

## Arbeitsschritte Paket A (Deadlock-Bots, Worktree coaching-nudge-bots-20260921)

1. `dl-broker`: Feld `coaching_wake: Arc<tokio::sync::Notify>` an
   `BrokerState` hängen (oder gleichwertige Übergabe), neue Route
   `/internal/master/v1/coaching/notifications-nudge` mit Handler, der
   `notify_one()` ruft und `{"ok": true}` antwortet.
2. `dl-community::coaching::spawn`: Notification-Task (heute
   `loop { process_notifications(); sleep(60s) }`) auf
   `tokio::select!` zwischen 60-s-Sleep und `Notify` umbauen; Einlaufen des
   Nudels → sofort `process_notifications()`.
3. `dl-bot/src/main.rs`: `Notify` anlegen, an `BrokerState` und an
   `coaching::spawn` durchreichen.
4. Tests: Handler (Auth + 200 + Wake) und Loop-Wake (Notify beendet den Sleep
   sofort) im bestehenden Teststil der Crates.

## Arbeitsschritte Paket B (Website, Worktree coaching-nudge-website-20260921)

1. Kleinen Best-effort-Helper ergänzen (neues Modul neben `discord_broker.rs`
   oder dort eingliedern): `POST {MASTER_BROKER_BASE}/internal/master/v1/
   coaching/notifications-nudge` mit `X-Internal-Token` aus der bestehenden
   Token-Kette; feuer-und-vergessen (tokio::spawn), Fehler nur `tracing::warn!`.
2. Nudge nach den drei Mutationen feuern, die eine fällige Notification
   erzeugen:
   - `create_appointment` → 'created'
   - `update_appointment` bei Statuswechsel auf 'cancelled' → 'cancelled'
   - `create_coaching_request` (website-originated, website_request_id gesetzt)
     → 'request_created'
3. Die User-Antwort darf durch den Nudge nie fehlschlagen oder langsamer
   werden: kein `.await` auf den HTTP-Aufruf im Request-Pfad.
4. Test: Helper baut URL/Header korrekt (Verdrahtungstest im Stil der
   bestehenden Broker-Client-Tests).

## Fundstellen (Vorcheck, Stand 2026-09-21)

Bot (Deadlock-Bots @ bb03deb5):
- `rust/crates/dl-community/src/coaching.rs:20-21`: `ROLE_SYNC_INTERVAL` 600 s,
  `NOTIFICATION_INTERVAL` 60 s.
- `rust/crates/dl-community/src/coaching.rs:498-504`: Notification-Task als
  Sleep-Loop — Umbaupunkt fürs select!.
- `rust/crates/dl-community/src/coaching.rs:113-134`: `WebsiteClient::from_env`,
  Token-Kette TWITCH_INTERNAL_API_TOKEN → MASTER_BROKER_TOKEN →
  COACHING_BOT_TOKEN, Base WEBSITE_API_BASE.
- `rust/crates/dl-community/src/coaching.rs:339-405`: `process_notifications`
  (due holen, DMs/Karten zustellen, acks schreiben).
- `rust/crates/dl-broker/src/lib.rs:62-72`: `BrokerState` + `SharedBroker`;
  `lib.rs:142-260` Router mit Pfadschema `/internal/master/v1/...`
  (kebab-case, z. B. `send-dm`), `lib.rs:30` `TOKEN_HEADER = "X-Internal-Token"`.
- `rust/bin/dl-bot/src/main.rs:1258-1273`: Master-Broker :8770 Setup;
  `main.rs:1558-1568`: `CoachingSync`-Bau + `coaching::spawn`.

Website-Backend (Website @ b271acc, `builds/backend-rust`):
- `src/routes/platform.rs:994-1073`: `notifications_due` — fällige Notifications
  sind Zustand abgeleitet (notify_*_at IS NULL, Reminder-Fenster 2 h).
- `src/routes/platform.rs:838` `create_appointment`, `:928` + `:946`
  `update_appointment` (Statuswechsel), `:1075` `notifications_ack`.
- `src/routes/coaching.rs:291` + `:364` `create_coaching_request` (INSERT
  coaching.requests).
- `src/discord_broker.rs:301,350,368,389`: bestehende Broker-Client-Muster
  (Base `{master_broker_base}/internal/master/v1/discord/...`, Token).
- `src/config.rs:94,174-177`: `master_broker_base`
  (Default `http://127.0.0.1:8770`), `master_broker_token`.

## Was nicht angefasst wird

- Coach-Roster-Sync (600 s + Rollen-Event-Resync) bleibt wie er ist.
- Zustelllogik (`process_notifications`), DM-Texte, Karten, Acks bleiben
  inhaltlich unverändert.
- Der 60-s-Poll bleibt als Fallback stehen (kein Entfernen, kein Absenken).
- Keine Änderungen an Caddy, an der Bot-Umgebung oder an Python-Altlasten.
- Kein Refactoring außerhalb der genannten Stellen, kein fmt über den Diff raus.

## Fertig-Kriterium

- Paket A: `cargo test -p dl-broker -p dl-community` grün, `cargo build` des
  Bins fehlerfrei; Handler existiert unter dem Vertrag und weckt den Loop
  (Test beweist das Wake-Verhalten).
- Paket B: `cargo test` (backend-rust) grün, Build fehlerfrei; Nudge feuert
  nach den drei Mutationen, Request-Pfad wartet nie auf ihn.
- Beide: nur die genannten Dateien im Diff, kein Fremd-Commit.

## Deploy-Weg

Nach Merge durch den Orchestrator: dl-bot-Release-Build + Service-Restart
(Akte deploy-restart-selbstdienst), Website-Backend über den bestehenden
backend-rust-Deploy-Weg. Live-Prüfung mit echtem Termin gegen den Owner als
Coachee — gehört nicht zum Worker-Auftrag.

## Rahmen

- Du bist der einzige Thread für dieses Paket. Keine Unter-Threads oder
  Unter-Agenten spawnen.
- Keine Code-Kommentare schreiben, Code erklärt sich selbst.
- Nur den eigenen Branch pushen, nie main.
- Auftrag größer als beschrieben: Bump-up-Nachricht an den Intent-Thread, dann
  stoppen.
