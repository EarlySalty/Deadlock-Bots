# Reaction-Roles (generisch) + Dashboard-Verwaltung — Design & Implementierungs-Spec

**Datum:** 2026-06-28
**Branch:** `rust/reaction-roles-2026-06-28` (Basis: `origin/rust/parity-reconcile-welle2-2026-06-28`)
**Sprache:** Rust only (kein Python-Cog). Handler in `dl-bot`/`dl-discord`/`dl-community`, Verwaltung in `dl-dashboard` + `service/static/dashboard.html`.

## 1. Ziel & Kontext

Eine Discord-Ankündigung ("Geplante Scrimmatches mit einem festen Coach") kam gut an (16× ✅). Daraus wird ein **generisches Reaction-Role-System**:

- User reagiert mit einem konfigurierten Emoji auf eine konfigurierte Nachricht → bekommt eine Rolle **und** (optional) eine einmalige DM.
- Entfernt der User die Reaktion → Rolle wird wieder entzogen (per-Mapping schaltbar). Die Rolle bleibt damit ein **exakter Live-Roster** der Interessierten.
- Bestehende Reaktionen (die schon-16) werden **einmalig nachverarbeitet** (Backfill: Rolle + DM).
- Verwaltung der Mappings über eine **Admin-UI im Dashboard** (kein Chat-Befehl).

Das Feature wird erst user-sichtbar, wenn `dl-bot` mit `DL_BOT_GATEWAY=1` live ist (Cutover, separater Workstream B). Diese Spec deckt **nur** das Feature (Workstream A).

## 2. Scope

**In Scope:** Tabellen, Reaction-Event-Handler (add/remove), Rollen-Vergabe/-Entzug, einmalige DM, Backfill, Dashboard-CRUD (Backend + Frontend), Rust-Tests, Seed des ersten (Scrim-)Mappings.

**Out of Scope:** Scrim-Programm-Mechanik (Teams, Termine, Coach-Zuordnung). Message-Delete-/`reaction_remove_all`-Behandlung. Custom-Emoji-Animationen. Jegliche Python-Änderung.

## 3. Erstes Mapping (Seed-Daten)

| Feld | Wert |
|---|---|
| `guild_id` | (Guild der Ankündigung — aus `source_channel`/Config ermitteln; Codex: Guild-ID aus bestehender Config/Adapter `first_guild` oder Dashboard-Form) |
| `source_channel_id` | `1371952264620806214` (Channel, in dem die Ankündigung steht — für Backfill-Fetch) |
| `message_id` | `1520838367611322478` |
| `emoji` | `✅` (Unicode `white_check_mark`) |
| `role_id` | `1520849762851618817` |
| `dm_enabled` | `true` |
| `remove_on_unreact` | `true` |
| `backfill_pending` | `true` (für die 16) |
| `dm_text` | siehe §9 |

Der Seed erfolgt über die neue Dashboard-UI (dogfooding), Backfill-Haken gesetzt. Alternativ Insert-Statement im Seed-Test/Doku dokumentieren. **Wichtig:** Mapping + DM-Text müssen final sein, BEVOR `backfill_pending=1` gesetzt wird (sonst bekommen die 16 eine halbfertige DM).

## 4. Datenmodell (in `dl-db` `SCHEMA_DUMP` einfügen)

Beide Tabellen in die `SCHEMA_DUMP`-Konstante in `rust/crates/dl-db/src/lib.rs` aufnehmen (bootstrap_schema macht sie idempotent via `CREATE TABLE IF NOT EXISTS`). Spalten-/Index-Namen exakt so:

```sql
CREATE TABLE reaction_role_mappings(
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  guild_id          INTEGER NOT NULL,
  source_channel_id INTEGER NOT NULL,
  message_id        INTEGER NOT NULL,
  emoji             TEXT    NOT NULL,
  role_id           INTEGER NOT NULL,
  dm_enabled        INTEGER NOT NULL DEFAULT 0,
  dm_text           TEXT,
  remove_on_unreact INTEGER NOT NULL DEFAULT 1,
  backfill_pending  INTEGER NOT NULL DEFAULT 0,
  active            INTEGER NOT NULL DEFAULT 1,
  created_at        INTEGER NOT NULL,
  updated_at        INTEGER NOT NULL
);
CREATE UNIQUE INDEX ux_reaction_role_mappings_msg_emoji
  ON reaction_role_mappings(message_id, emoji);

CREATE TABLE reaction_role_dm_log(
  mapping_id INTEGER NOT NULL,
  user_id    INTEGER NOT NULL,
  sent_at    INTEGER NOT NULL,
  PRIMARY KEY (mapping_id, user_id)
);
```

`dm_log` ist die Anti-Doppel-DM-Spur und überlebt Neustarts/Backfill. Booleans als INTEGER 0/1 (rusqlite-idiomatisch, konsistent mit Bestand).

## 5. Emoji-Kanonisierung

Serenity liefert `ReactionType`. Lookup-Schlüssel = kanonischer String:
- **Unicode:** der Literal-String (z. B. `✅`).
- **Custom:** `name:id` (z. B. `pog:123456789012345678`).

Eine Funktion `canonical_emoji(&ReactionType) -> String` (in dl-community-Modul, getestet). Beim Speichern aus dem Dashboard ebenfalls kanonisch ablegen (Unicode-Emoji oder `name:id`).

## 6. Domänen-Logik (neues Modul `dl-community::reaction_roles`)

Struct `ReactionRoleService { db: Arc<dl_db::Db>, port: Arc<dyn ReactionRolePort> }`.

**Port-Trait** (testbar mit Mock; Real-Impl in dl-bot über `DiscordAdapter`):
```rust
#[async_trait]
pub trait ReactionRolePort: Send + Sync {
    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> Result<(), PortErr>;
    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> Result<(), PortErr>;
    async fn send_dm(&self, user_id: u64, content: &str) -> Result<(), DmErr>; // DmErr unterscheidet permanent (Forbidden/UserNotFound/DmOpenFailed) vs transient
    async fn reaction_users(&self, channel_id: u64, message_id: u64, emoji: &ReactionType, after: Option<u64>) -> Result<Vec<u64>, PortErr>; // paginiert, max 100/Seite
}
```
> `DiscordAdapter` hat bereits `add_role`/`remove_role`/`send_dm` (adapter.rs:369/415/272). `reaction_users` ggf. als neue Adapter-Methode über serenity `http.get_reaction_users(channel, message, reaction_type, limit, after)` ergänzen.

**`handle_reaction_add(guild_id, channel_id, message_id, user_id, emoji, is_bot)`:**
1. `is_bot` → return (Bots ignorieren).
2. Mapping per `(message_id, canonical_emoji)` lesen, nur `active=1`. Keins → return.
3. `port.add_role(guild_id, user_id, role_id)` (Discord ist idempotent bei bereits vorhandener Rolle). Fehler loggen, nicht abbrechen.
4. Wenn `dm_enabled` UND kein `dm_log`-Eintrag `(mapping_id, user_id)`:
   - `port.send_dm(user_id, dm_text)`.
   - Bei Erfolg ODER **permanentem** Fehler (DMs zu / User weg) → `dm_log` schreiben (kein Retry-Loop).
   - Bei **transientem** Fehler → `dm_log` NICHT schreiben (späterer Retry möglich).

**`handle_reaction_remove(guild_id, channel_id, message_id, user_id, emoji, is_bot)`:**
1. `is_bot` → return.
2. Mapping lesen (active=1). Keins → return.
3. Wenn `remove_on_unreact` → `port.remove_role(...)`. `dm_log` NICHT anfassen (DM bleibt einmalig; Re-Reaktion löst keine neue DM aus).

## 7. Backfill

`run_pending_backfills()` auf dem Service, als One-Shot-`spawn` in `dl-bot/main.rs` nach Adapter-Init (REST genügt — `get_reaction_users` ist REST). Für jedes Mapping mit `backfill_pending=1 AND active=1`:
1. Reaktoren paginiert über `port.reaction_users(source_channel_id, message_id, emoji, after)` holen (Schleife bis < 100 zurückkommt).
2. Pro Nicht-Bot-User dieselbe Logik wie `handle_reaction_add` (Rolle + DM-once). Zwischen DM-Sends ein kleiner Delay (~1–2 s) gegen Rate-Limits.
3. Nach vollständigem Durchlauf `backfill_pending=0` setzen.
4. Schlägt der Fetch fehl (Message gelöscht / falscher Channel) → Fehler loggen, `backfill_pending` belassen (fixbar + Retry beim nächsten Start), aber pro Start nur **einmal** versuchen.

## 8. Gateway-Verdrahtung (dl-discord + dl-bot)

1. **Intent:** In `rust/crates/dl-discord/src/gateway.rs` (Block bei Zeile 361–367) `GatewayIntents::GUILD_MESSAGE_REACTIONS` ergänzen. (Nicht-privilegiert, kein Portal-Toggle nötig; `GUILD_MEMBERS` ist bereits aktiv.)
2. **EventHandler:** In `impl EventHandler for Handler` (gateway.rs:56) zwei Methoden ergänzen:
   - `async fn reaction_add(&self, ctx, add: Reaction)`
   - `async fn reaction_remove(&self, ctx, removed: Reaction)`
   Beide rufen — analog zu bestehenden Events (`guild_member_update` etc.) — einen vom `Handler` gehaltenen Reaction-Role-Port/Dispatcher auf. `Reaction` liefert `guild_id`, `channel_id`, `message_id`, `user_id` (bei remove ggf. `Option<UserId>` → wenn `None`, ist es kein verwertbares Event, dann return), `emoji`. Bot-Check: User über Cache/REST auflösen oder `add.member`/`user_id` gegen Bot prüfen; wenn nicht ermittelbar, konservativ verarbeiten.
3. **Glue:** Real-Port-Impl in `dl-bot` (modglue.rs oder neue `reactionglue.rs`) über `DiscordAdapter`. Service-Konstruktion + Port in `main.rs` wie die anderen Module (`db.clone()`, `adapter.clone()`), Handler bekommt den Port, `run_pending_backfills()` als `spawn`.

> **Codex:** Lies `gateway.rs` (Handler-Struct + `build_client`-Aufruf in main.rs) um zu sehen, wie bestehende Events ihre Ports halten, und repliziere exakt dieses Muster — nicht erfinden.

## 9. DM-Text (final — NICHT ändern, exakt so einsetzen)

```
Hey! 👋 Schön, dass du beim Scrim-Coaching dabei bist.

Kurz worum's geht: Wir (Leo & deniz) stellen feste Teams mit einem festen Coach zusammen, spielen regelmäßig Showmatches gegeneinander und setzen uns zwischendurch zusammen, um an euren Punkten zu arbeiten — Schritt für Schritt besser werden, als Team.

Damit wir die Teams gut zusammenbekommen, schreib uns am besten direkt in <#1520842755037855975>:
• deinen aktuellen Rang
• deine bevorzugte Lane/Rolle
• wann du grob Zeit hast (Wochentag/Uhrzeit)

Wir melden uns bei dir, sobald die Gruppen stehen. Bis gleich! 🎮
```
`<#1520842755037855975>` rendert als klickbarer Channel-Link (Empfänger teilt den Server). Umlaute exakt erhalten (ä/ö/ü, kein ASCII-Ersatz). Als benannte Konstante ablegen, nicht inline streuen.

## 10. Dashboard-CRUD (dl-dashboard)

Neues Modul `rust/crates/dl-dashboard/src/reaction_roles.rs`, Muster = `crate::deadlock` (Heroes). Routen in `web.rs` (Router bei Zeile ~252), hinter demselben Admin-Gate (`decide_access`, Owner `662995601738170389` / Mod-Rolle `1337516255846735875`) und `security_headers`:

- `GET  /api/reaction-roles` → Liste aller Mappings (JSON-Array).
- `POST /api/reaction-roles` → Upsert. Body: `{ id?, guild_id, source_channel_id, message_id, emoji, role_id, dm_enabled, dm_text, remove_on_unreact, backfill }`. Bei `backfill=true` → `backfill_pending=1` setzen. `updated_at`/`created_at` server-seitig (Sekunden seit Epoch).
- `DELETE /api/reaction-roles/{id}` → Mapping + zugehörige `dm_log`-Zeilen löschen.

Validierung an der Grenze: IDs als u64 parsebar, emoji nicht leer, role_id ≠ 0. Keine Übervalidierung.

## 11. Frontend (service/static/dashboard.html)

Neue Admin-Section analog zur Heroes-Section. Tabelle (bestehende Mappings: Message-ID, Emoji, Rolle, DM an/aus, Toggle, Status) + Formular (alle Felder aus §10) + „Backfill bestehende Reaktionen"-Checkbox + Speichern/Löschen. Vanilla-JS-Fetch gegen §10-Routen, kein npm-Build.

**Deutsche Labels (final — exakt so):**
- Section-Titel: `Reaction-Rollen`
- Spalten: `Nachricht`, `Emoji`, `Rolle`, `DM`, `Bei Reaktion-Entfernen Rolle entziehen`, `Status`
- Formular-Felder: `Quell-Channel-ID`, `Nachrichten-ID`, `Emoji`, `Rollen-ID`, `DM senden`, `DM-Text`, `Rolle bei Entfernen entziehen`, `Bestehende Reaktionen nachverarbeiten`
- Buttons: `Speichern`, `Löschen`, `Neu`
- Hinweis am DM-Text-Feld: `Tipp: <#KANAL-ID> wird zum klickbaren Link.`
- Warnung bei gesetztem Backfill-Haken: `Achtung: Backfill verschickt echte DMs an alle, die bereits reagiert haben.`

## 12. Tests (A7 — schließt die „0-Tests"-Lücke fürs Feature)

Mit `tempfile`-Db (dev-dep vorhanden) + Mock-`ReactionRolePort` (zeichnet Calls auf):
- `canonical_emoji` Unicode vs Custom.
- `handle_reaction_add`: vergibt Rolle; DM nur wenn `dm_enabled` und noch kein `dm_log`; zweiter Add → keine zweite DM.
- Permanent-DM-Fehler → `dm_log` gesetzt (kein Retry); transienter Fehler → kein `dm_log`.
- `handle_reaction_remove`: entzieht Rolle nur bei `remove_on_unreact`; rührt `dm_log` nicht an.
- `run_pending_backfills`: iteriert paginierte Mock-Reaktoren, vergibt Rolle + DM-once, setzt `backfill_pending=0`.
- Bot-User wird in add/remove/backfill ignoriert.

## 13. Build & Verifikation (Definition of Done)

```bash
cd rust
cargo build --release --bin dl-bot
cargo clippy --all-targets -- -D warnings      # --all-targets, damit Test-Warnungen auffallen
cargo test -p dl-community -p dl-db -p dl-dashboard
cargo fmt --all                                # vor Commit, nicht vergessen
```
- Build grün, clippy 0 Warnungen (inkl. Tests), neue Tests grün, `cargo fmt` sauber.
- Live-Beweis (im Cutover, Workstream B): nach Boot `reaction_role_mappings` + `reaction_role_dm_log` in `data/deadlock.sqlite3` vorhanden; ein Test-✅ vergibt Rolle + DM; Entfernen entzieht Rolle.

## 14. Constraints (verbindlich)

- **Nur dieses Feature.** Keine fremden Dateien anfassen, kein Scope-Wachstum, keine unrelated Refactors.
- **Keine erfundenen APIs.** serenity-0.12-/rusqlite-Signaturen vor Nutzung im Code verifizieren.
- Kein `.unwrap()`/`.expect()` in Produktionspfaden; Fehler via `thiserror`/`anyhow` + `tracing`.
- Deutsche Strings exakt aus dieser Spec übernehmen (Umlaute korrekt), als benannte Konstanten. Keine eigenen User-Texte erfinden.
- Idiomatisch, kleine Funktionen, tiefe Module. Bestehende Patterns spiegeln (deadlock.rs, modglue.rs), nichts neu erfinden.
