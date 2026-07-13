# Voice Pair Guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Die Discord-Nutzer `887664726421671976` und `279971744964542464` können regulär nie in denselben Voice-Channel verbinden; die Sperre gilt serverweit, bidirektional und ohne Disconnect.

**Architecture:** Ein kleiner `dl-voice::voice_pair_guard`-Subscriber verarbeitet die bestehenden normalisierten Voice-Events. Er speichert vor jedem temporären Deny den vorherigen Member-Overwrite in Postgres, setzt nur das `CONNECT`-Bit und restauriert den exakten Ausgangszustand bei Leave/Move oder Reconciliation. Die bestehenden TempVoice-Permission-Schreibpfade komponieren ihre gewünschte Basisberechtigung mit einem aktiven Pair-Lock, sodass Owner-Unban den Guard nicht aufheben kann.

**Tech Stack:** Rust, Tokio, Serenity 0.12.5, SQLx/PostgreSQL, bestehender `dl-discord::Dispatcher` und `dl-voice`-Workspace.

## Global Constraints

- Guild-ID: `1289721245281292288`.
- Geschützte Nutzer: `887664726421671976` und `279971744964542464` mit identischen Rechten.
- Schutz gilt für jeden Voice-Channel, nicht nur TempVoice.
- Kein Disconnect und kein automatisches Verschieben.
- Move setzt zuerst den neuen Lock und restauriert danach den alten.
- Eine aktive Pair-Sperre hat Vorrang vor TempVoice-Owner-Ban und -Unban.
- Fremde Allow-/Deny-Bits eines Member-Overwrites bleiben unverändert.
- Keine neue Dependency und keine konfigurierbare Regel-Engine.
- Die fremde, unstaged Formatänderung in `rust/bin/dl-bot/src/scrimglue.rs` bleibt unangetastet.

---

### Task 1: Persistenter Guard-Kern

**Files:**
- Create: `rust/crates/dl-central-db/migrations/2026071262_voice_pair_guard_locks.sql`
- Create: `rust/crates/dl-voice/src/voice_pair_guard.rs`

**Interfaces:**
- Consumes: `dl_discord::VoiceEvent`, `sqlx::PgPool`, vorhandene `glue::merge_connect_overwrite`-Semantik.
- Produces: `VoicePairGuardStore`, `VoicePairGuard`, `VoicePairPort`, `counterpart(user_id) -> Option<u64>`, `handle_event(VoiceEvent)`, `reconcile(guild_id)` und `spawn(...)`.

- [ ] **Step 1: Migration für restaurierbare Locks schreiben**

```sql
CREATE TABLE IF NOT EXISTS voice.voice_pair_guard_locks (
    guild_id BIGINT NOT NULL,
    channel_id BIGINT NOT NULL,
    blocked_user_id BIGINT NOT NULL,
    previous_allow BIGINT NOT NULL,
    previous_deny BIGINT NOT NULL,
    had_overwrite BOOLEAN NOT NULL,
    PRIMARY KEY (guild_id, channel_id, blocked_user_id)
);
```

- [ ] **Step 2: Failing Tests für beide Richtungen und Kanalwechsel schreiben**

Tests im `#[cfg(test)]`-Modul von `voice_pair_guard.rs`:

```rust
struct TestFixture {
    guard: Arc<VoicePairGuard>,
    port: Arc<MockVoicePairPort>,
    store: Arc<MemoryVoicePairStore>,
}

async fn setup_guard() -> (Arc<VoicePairGuard>, Arc<MockVoicePairPort>);
async fn setup_guard_with_previous(
    previous: Option<(u64, u64)>,
) -> (Arc<VoicePairGuard>, Arc<MockVoicePairPort>);
async fn setup_guard_with_lock(
    channel_id: u64,
    blocked_user_id: u64,
    previous: Option<(u64, u64)>,
) -> (Arc<VoicePairGuard>, Arc<MockVoicePairPort>);
async fn guarded_lane() -> TestFixture;
```

```rust
#[tokio::test]
async fn join_sperrt_den_gegenpart_auf_jedem_voice_channel() {
    let (guard, port) = setup_guard().await;
    guard.handle_event(VoiceEvent::Join {
        guild_id: GUILD_ID,
        user_id: USER_A,
        channel_id: 42,
    }).await;
    assert_eq!(port.connect_state(42, USER_B).await, Some(false));
}

#[tokio::test]
async fn beide_nutzer_haben_identische_guard_rechte() {
    let (guard, port) = setup_guard().await;
    guard.handle_event(VoiceEvent::Join {
        guild_id: GUILD_ID,
        user_id: USER_B,
        channel_id: 43,
    }).await;
    assert_eq!(port.connect_state(43, USER_A).await, Some(false));
}

#[tokio::test]
async fn move_sperrt_zuerst_neu_und_restauriert_dann_alt() {
    let (guard, port) = setup_guard().await;
    guard.handle_event(VoiceEvent::Move {
        guild_id: GUILD_ID,
        user_id: USER_A,
        from_channel_id: 42,
        to_channel_id: 43,
    }).await;
    assert_eq!(port.channels_written().await, vec![43, 42]);
}

#[tokio::test]
async fn leave_restauriert_allow_deny_oder_fehlendes_overwrite_exakt() {
    for previous in [None, Some((1 << 10, 0)), Some((0, 1 << 20))] {
        let (guard, port) = setup_guard_with_previous(previous).await;
        guard.handle_event(VoiceEvent::Leave {
            guild_id: GUILD_ID,
            user_id: USER_A,
            channel_id: 42,
        }).await;
        assert_eq!(port.member_overwrite(42, USER_B).await, previous);
    }
}

#[tokio::test]
async fn unbeteiligte_guilds_und_nutzer_bleiben_unveraendert() {
    let (guard, port) = setup_guard().await;
    guard.handle_event(VoiceEvent::Join {
        guild_id: 1,
        user_id: 2,
        channel_id: 3,
    }).await;
    assert!(port.channels_written().await.is_empty());
}
```

- [ ] **Step 3: RED verifizieren**

Run: `cargo test -p dl-voice voice_pair_guard -- --nocapture`

Expected: FAIL, weil Modul und Guard-Verhalten noch fehlen; kein Compilefehler aus den bestehenden Crates.

- [ ] **Step 4: Minimalen Guard implementieren**

Die Implementierung verwendet feste Konstanten und genau eine Gegenpart-Funktion:

```rust
pub const GUILD_ID: u64 = 1_289_721_245_281_292_288;
pub const USER_A: u64 = 887_664_726_421_671_976;
pub const USER_B: u64 = 279_971_744_964_542_464;

pub fn counterpart(user_id: u64) -> Option<u64> {
    match user_id {
        USER_A => Some(USER_B),
        USER_B => Some(USER_A),
        _ => None,
    }
}
```

`Join` speichert den vorhandenen Member-Overwrite vor dem ersten Deny. `Move` ruft Lock-Neu vor Restore-Alt auf. `Leave` restauriert die gespeicherten Bits oder löscht nur dann das Member-Overwrite, wenn vorher keines existierte. `Update` ist ein No-op. Datenbankzeilen werden erst nach erfolgreicher Restaurierung gelöscht.

- [ ] **Step 5: GREEN verifizieren**

Run: `cargo test -p dl-voice voice_pair_guard -- --nocapture`

Expected: alle neuen Guard-Kerntests PASS.

- [ ] **Step 6: Commit und Push**

```bash
git add rust/crates/dl-central-db/migrations/2026071262_voice_pair_guard_locks.sql rust/crates/dl-voice/src/voice_pair_guard.rs
git commit -m "feat: persistenten Voice-Pair-Guard ergänzen" -m "Co-authored-by: GPT 5.6 sol <gpt@local>"
git push
```

### Task 2: Discord-Overwrites sicher komponieren

**Files:**
- Modify: `rust/crates/dl-voice/src/glue.rs`
- Modify: `rust/crates/dl-voice/src/tempvoice/engine.rs`
- Modify: `rust/crates/dl-voice/src/tempvoice/interface.rs`
- Test: bestehende Unit-Tests in denselben Dateien

**Interfaces:**
- Consumes: aktive Lock-Zeilen aus `VoicePairGuardStore` und `merge_connect_overwrite(existing_allow, existing_deny, connect)`.
- Produces: zielgenaues Lesen/Setzen/Löschen eines Member-Overwrites sowie effektive `CONNECT`-Auflösung für Einzel- und Batch-Schreibpfade.

- [ ] **Step 1: Failing Tests für Overwrite-Erhalt und Guard-Priorität schreiben**

```rust
#[test]
fn connect_aenderung_erhaelt_alle_fremden_permission_bits() {
    let view = 1 << 10;
    assert_eq!(merge_connect_overwrite(view, 0, Some(false)), Some((view, 1 << 20)));
}

#[tokio::test]
async fn aktiver_guard_haelt_deny_trotz_owner_unban() {
    let fixture = guarded_lane().await;
    fixture.port.set_member_connect(GUILD_ID, 42, USER_B, None).await.unwrap();
    assert_eq!(fixture.port.connect_state(42, USER_B).await, Some(false));
}

#[tokio::test]
async fn owner_unban_aktualisiert_waehrend_lock_nur_den_restore_zustand() {
    let fixture = guarded_lane().await;
    fixture.port.set_member_connect(GUILD_ID, 42, USER_B, None).await.unwrap();
    assert_eq!(fixture.store.restore_connect(GUILD_ID, 42, USER_B).await, None);
}

#[tokio::test]
async fn normaler_owner_ban_bleibt_nach_guard_leave_erhalten() {
    let fixture = guarded_lane().await;
    fixture.port.set_member_connect(GUILD_ID, 42, USER_B, Some(false)).await.unwrap();
    fixture.guard.restore(GUILD_ID, 42, USER_B).await.unwrap();
    assert_eq!(fixture.port.connect_state(42, USER_B).await, Some(false));
}
```

- [ ] **Step 2: RED verifizieren**

Run: `cargo test -p dl-voice connect_aenderung aktiver_guard owner_unban normaler_owner_ban -- --nocapture`

Expected: FAIL wegen fehlender Guard-Komposition.

- [ ] **Step 3: Einzel- und Batch-Pfade zentral komponieren**

Vor jedem normalen `set_member_connect` oder `apply_member_connect_batch` gilt:

```rust
let effective = if store.is_locked(guild_id, channel_id, user_id).await? {
    store.update_restore_connect(guild_id, channel_id, user_id, requested).await?;
    Some(false)
} else {
    requested
};
```

Die Discord-Schreibfunktion nutzt den vorhandenen `merge_connect_overwrite`-Helper mit den aktuellen Allow-/Deny-Bits und `ChannelId::create_permission`; wenn danach keine Bits übrig bleiben, nutzt sie `ChannelId::delete_permission`. Guard-eigene Lock-/Restore-Aufrufe verwenden einen expliziten Raw-Pfad, damit sie den gespeicherten Basiszustand nicht überschreiben.

- [ ] **Step 4: GREEN und bestehende TempVoice-Ban-Tests verifizieren**

Run: `cargo test -p dl-voice owner_bans_landen_als_overwrites -- --nocapture && cargo test -p dl-voice voice_pair_guard -- --nocapture`

Expected: alle Guard- und vorhandenen Owner-Ban-Tests PASS.

- [ ] **Step 5: Commit und Push**

```bash
git add rust/crates/dl-voice/src/glue.rs rust/crates/dl-voice/src/tempvoice/engine.rs rust/crates/dl-voice/src/tempvoice/interface.rs
git commit -m "fix: Voice-Sondersperre vor Owner-Rechten priorisieren" -m "Co-authored-by: GPT 5.6 sol <gpt@local>"
git push
```

### Task 3: Subscriber, Startup-Reconciliation und Bot-Verdrahtung

**Files:**
- Modify: `rust/crates/dl-voice/src/voice_pair_guard.rs`
- Modify: `rust/crates/dl-voice/src/lib.rs`
- Modify: `rust/bin/dl-bot/src/main.rs`

**Interfaces:**
- Consumes: `Dispatcher::subscribe_voice()`, `Dispatcher::subscribe_gateway()`, `GatewayEvent::CacheReady`, Cache-Voice-States und den Guard aus Task 1.
- Produces: laufenden Guard-Subscriber mit Reconciliation nach CacheReady und nach `RecvError::Lagged`.

- [ ] **Step 1: Failing Reconciliation-Tests schreiben**

```rust
#[tokio::test]
async fn reconcile_setzt_fehlenden_lock_fuer_bereits_verbundenen_nutzer() {
    let (guard, port) = setup_guard().await;
    port.set_voice_channel(GUILD_ID, USER_A, Some(42)).await;
    guard.reconcile(GUILD_ID).await;
    assert_eq!(port.connect_state(42, USER_B).await, Some(false));
}

#[tokio::test]
async fn reconcile_restauriert_stale_locks_und_entfernt_geloeschte_kanaele() {
    let (guard, port) = setup_guard_with_lock(42, USER_B, None).await;
    port.set_voice_channel(GUILD_ID, USER_A, None).await;
    guard.reconcile(GUILD_ID).await;
    assert_eq!(port.member_overwrite(42, USER_B).await, None);
    assert!(guard.store().list_locks(GUILD_ID).await.unwrap().is_empty());
}

#[tokio::test]
async fn guard_ruft_niemals_disconnect_oder_move_auf() {
    let (guard, port) = setup_guard().await;
    guard.handle_event(VoiceEvent::Join {
        guild_id: GUILD_ID,
        user_id: USER_A,
        channel_id: 42,
    }).await;
    assert_eq!(port.member_move_calls().await, 0);
}
```

- [ ] **Step 2: RED verifizieren**

Run: `cargo test -p dl-voice reconcile -- --nocapture`

Expected: FAIL wegen fehlender Cache-Reconciliation.

- [ ] **Step 3: Subscriber und Main-Wiring implementieren**

`spawn` hält je einen Voice- und Gateway-Receiver. Nach `CacheReady` für die Ziel-Guild wird `reconcile` ausgeführt. Bei `Lagged` wird geloggt und ebenfalls reconciled. `main.rs` baut Store, Discord-Port und Guard aus dem vorhandenen `central_pool`, `adapter` und `dispatcher`; `lib.rs` exportiert `pub mod voice_pair_guard;`.

- [ ] **Step 4: GREEN verifizieren**

Run: `cargo test -p dl-voice reconcile -- --nocapture && cargo check -p dl-bot`

Expected: Reconciliation-Tests PASS und `dl-bot` kompiliert.

- [ ] **Step 5: Commit und Push**

```bash
git add rust/crates/dl-voice/src/voice_pair_guard.rs rust/crates/dl-voice/src/lib.rs rust/bin/dl-bot/src/main.rs
git commit -m "feat: Voice-Pair-Guard im Bot starten" -m "Co-authored-by: GPT 5.6 sol <gpt@local>"
git push
```

### Task 4: Nutzerverhalten dokumentieren und vollständig verifizieren

**Files:**
- Modify: `CHANGELOG.md`
- Modify: `docs/voice-features.md`

**Interfaces:**
- Consumes: implementiertes und getestetes Verhalten aus Tasks 1–3.
- Produces: interne technische Doku und neutralen Changelog-Eintrag ohne Nutzer-IDs.

- [ ] **Step 1: Doku und Changelog aktualisieren**

`docs/voice-features.md` beschreibt serverweite bidirektionale CONNECT-Sperre, Restore bei Leave/Move, Restart-Reconciliation und die gleichzeitige-Join-Grenze. `CHANGELOG.md` erhält oben einen neuen Eintrag im bestehenden `## #N — Titel`-Format: Problem → Änderung → aktuelles Verhalten; keine Nutzer-IDs, Datei-/Funktionsnamen oder internen Stackdetails.

- [ ] **Step 2: Vollständige Verifikation ausführen**

Run:

```bash
cargo fmt --all -- --check
cargo test -p dl-voice
cargo test -p dl-bot
cargo clippy -p dl-voice -p dl-bot --all-targets -- -D warnings
cargo build --release --workspace
```

Expected: alle Befehle Exit 0, keine Testfehler und keine Clippy-Warnungen.

- [ ] **Step 3: Finalen Diff gegen Spec prüfen**

Run: `git diff --check && git status --short && git diff --stat HEAD~3`

Expected: nur geplante Guard-, Migration-, Wiring-, Test- und Doku-Dateien plus die klar getrennte fremde unstaged `scrimglue.rs`-Formatänderung.

- [ ] **Step 4: Commit und Push**

```bash
git add CHANGELOG.md docs/voice-features.md
git commit -m "docs: Voice-Zugriffsschutz dokumentieren" -m "Co-authored-by: GPT 5.6 sol <gpt@local>"
git push
```

- [ ] **Step 5: Merge, Deploy und Live-Beweis**

Nach bestandenem Merge-Kritiker: Feature-Branch nach `main` mergen und pushen, `cargo build --release --workspace`, betroffene systemd-User-Services neu starten und PID-Wechsel, `/proc/<pid>/exe` sowie fehlerfreies Journal belegen. Erst danach Branch und Worktree bereinigen.
