use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use tokio::sync::Mutex;

const NOW: i64 = 1_790_000_000;
const BOT: u64 = 99;
const GUILD: u64 = 42;
const USER: u64 = 7;
const AUDIT_REPLY: &str = "ℹ️ Dieser Steam-Account wurde schon eingeladen, am <t:1700000000:D>. Hier ist nichts mehr zu tun.";
const RUNNING_REPLY: &str = "⏳ Für diesen Steam-Account läuft gerade schon eine Einladung. Ich schicke sie nicht doppelt, sonst zählt sie zweimal gegen das Tageslimit.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaitPoint {
    Load,
    Claim,
    Finish,
    Hint,
}

struct ClockAdvance {
    point: WaitPoint,
    clock: Arc<AtomicI64>,
    at: i64,
}

#[derive(Default)]
struct MemoryStore {
    states: Mutex<HashMap<u64, LoungeState>>,
    known: Mutex<HashMap<u64, String>>,
    legacy_hints: Mutex<HashMap<u64, i64>>,
    backfilled: AtomicBool,
    advance: Mutex<Option<ClockAdvance>>,
}

impl MemoryStore {
    async fn after_wait(&self, point: WaitPoint) {
        let mut advance = self.advance.lock().await;
        if advance
            .as_ref()
            .is_some_and(|advance| advance.point == point)
        {
            let advance = advance.take().expect("passender Uhrschritt ist vorhanden");
            advance.clock.store(advance.at, Ordering::SeqCst);
        }
    }
}

#[async_trait::async_trait]
impl LoungeStore for MemoryStore {
    async fn load(&self, id: u64) -> Result<Option<LoungeState>, String> {
        let state = self.states.lock().await.get(&id).cloned();
        self.after_wait(WaitPoint::Load).await;
        Ok(state)
    }
    async fn compare_exchange(
        &self,
        id: u64,
        previous: Option<&LoungeState>,
        next: &LoungeState,
    ) -> Result<bool, String> {
        let mut states = self.states.lock().await;
        if states.get(&id) != previous {
            return Ok(false);
        }
        states.insert(id, next.clone());
        drop(states);
        match next.request.as_ref().map(|request| request.phase) {
            Some(Phase::Dispatching) => self.after_wait(WaitPoint::Claim).await,
            Some(Phase::Attempted) => self.after_wait(WaitPoint::Finish).await,
            Some(Phase::WaitingForCode) => self.after_wait(WaitPoint::Hint).await,
            _ => {}
        }
        Ok(true)
    }
    async fn pending_users(&self) -> Result<Vec<u64>, String> {
        Ok(self
            .states
            .lock()
            .await
            .iter()
            .filter(|(_, state)| {
                state
                    .request
                    .as_ref()
                    .is_some_and(|request| request.phase == Phase::Pending)
            })
            .map(|(id, _)| *id)
            .collect())
    }
    async fn known_code(&self, id: u64) -> Result<Option<String>, String> {
        Ok(self.known.lock().await.get(&id).cloned())
    }
    async fn last_hint_at(&self, id: u64) -> Result<Option<i64>, String> {
        Ok(self.legacy_hints.lock().await.get(&id).copied())
    }
    async fn backfill_done(&self) -> Result<bool, String> {
        Ok(self.backfilled.load(Ordering::SeqCst))
    }
    async fn mark_backfill_done(&self) -> Result<(), String> {
        self.backfilled.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingPort {
    replies: Mutex<Vec<(u64, String)>>,
    history: Mutex<Vec<LoungeMessage>>,
    history_calls: AtomicUsize,
    identity_missing: AtomicBool,
}

#[async_trait::async_trait]
impl InviteLoungeReplyPort for RecordingPort {
    fn bot_id(&self) -> Option<u64> {
        (!self.identity_missing.load(Ordering::SeqCst)).then_some(BOT)
    }
    async fn history(&self, before: Option<u64>) -> Result<Vec<LoungeMessage>, String> {
        self.history_calls.fetch_add(1, Ordering::SeqCst);
        let mut messages: Vec<_> = self
            .history
            .lock()
            .await
            .iter()
            .filter(|message| before.is_none_or(|id| message.message_id < id))
            .cloned()
            .collect();
        messages.sort_by_key(|message| std::cmp::Reverse(message.message_id));
        messages.truncate(100);
        Ok(messages)
    }
    async fn reply_text(&self, channel: u64, id: u64, text: &str) -> Result<u64, String> {
        assert_eq!(channel, INVITE_LOUNGE_CHANNEL_ID);
        self.replies.lock().await.push((id, text.into()));
        Ok(id + 100_000)
    }
}

#[derive(Default)]
struct RecordingInvite {
    calls: Mutex<Vec<(u64, u64, String, u64)>>,
    fail: AtomicBool,
    audit: AtomicBool,
    running_task: AtomicBool,
    sends: AtomicUsize,
    hold: AtomicBool,
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    advance: Mutex<Option<(Arc<AtomicI64>, i64)>>,
}

#[async_trait::async_trait]
impl InviteEventPort for RecordingInvite {
    async fn invite(&self, bot: u64, guild: u64, code: &str, user: u64) -> Result<String, String> {
        self.calls
            .lock()
            .await
            .push((bot, guild, code.into(), user));
        if self.hold.load(Ordering::SeqCst) {
            self.started.notify_one();
            self.release.notified().await;
        }
        if let Some((clock, at)) = self.advance.lock().await.take() {
            clock.store(at, Ordering::SeqCst);
        }
        if self.audit.load(Ordering::SeqCst) {
            return Ok(AUDIT_REPLY.into());
        }
        if self.running_task.load(Ordering::SeqCst) {
            return Ok(RUNNING_REPLY.into());
        }
        if self.fail.load(Ordering::SeqCst) {
            return Err("Steam nicht erreichbar".into());
        }
        self.sends.fetch_add(1, Ordering::SeqCst);
        Ok("🎮 Ihr seid schon Steam-Freunde, ich hab die Einladung direkt angestoßen.\nAntwort von Steam: OK".into())
    }
}

fn setup() -> (
    InviteLoungeWatcher,
    Arc<MemoryStore>,
    Arc<RecordingPort>,
    Arc<RecordingInvite>,
) {
    let store = Arc::new(MemoryStore::default());
    let port = Arc::new(RecordingPort::default());
    let invite = Arc::new(RecordingInvite::default());
    let watcher = InviteLoungeWatcher {
        store: store.clone(),
        port: port.clone(),
        invite: invite.clone(),
        guild_id: GUILD,
        clock_millis: Arc::new(|| NOW * 1000),
    };
    (watcher, store, port, invite)
}
fn message(id: u64, at: i64, content: &str) -> LoungeMessage {
    LoungeMessage {
        guild_id: Some(GUILD),
        channel_id: INVITE_LOUNGE_CHANNEL_ID,
        message_id: id,
        author_id: USER,
        content: content.into(),
        created_at: at,
        created_at_millis: at * 1000,
        is_bot: false,
        reply_message_id: None,
    }
}

#[tokio::test]
async fn direkte_bitte_sofort_ohne_neulingsgrenze_und_ohne_tipp() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(
        invite.calls.lock().await.as_slice(),
        &[(BOT, GUILD, "123456789".into(), USER)]
    );
    assert!(port.replies.lock().await.is_empty());
}

#[test]
fn direkte_bitte_mit_umgangssprache_und_tippfehlern() {
    for content in [
        "kannste mich auch einladen",
        "kanst du mich einladn",
        "kannsrt du mich einladden?",
        "<@99> bitte mich einladen",
        "<@!99> kannst mich auch inviten?",
    ] {
        assert_eq!(
            request_kind(content, BOT),
            Some(RequestKind::Direct),
            "{content}"
        );
    }
    assert_eq!(
        request_kind("kann mich jemand einladen?", BOT),
        Some(RequestKind::Room)
    );
}

#[tokio::test]
async fn direkte_bitte_nutzt_bekannten_code() {
    let (watcher, store, port, invite) = setup();
    store.known.lock().await.insert(USER, "123456789".into());
    watcher
        .handle_message(
            message(1, NOW, "Kannste mich auch einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert!(!port
        .replies
        .lock()
        .await
        .iter()
        .any(|(_, text)| text == INVITE_LOUNGE_HINT_TEXT));
}

#[tokio::test]
async fn direkte_bitte_fragt_einmal_code_und_fortsetzung_geht_sofort() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(
            message(2, NOW + 20, "Kannste mich auch einladen?"),
            NOW + 20,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(port.replies.lock().await.len(), 1);
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    let mut code = message(3, NOW + 40, "Mein Freundescode: 123456789");
    code.reply_message_id = Some(1);
    watcher
        .handle_message(code, NOW + 40, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert_eq!(port.replies.lock().await.len(), 1);
    assert_eq!(port.replies.lock().await[0].0, 1);
}

#[tokio::test]
async fn raumbitte_mit_code_wartet_genau_eine_stunde() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 3599)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(port.replies.lock().await.is_empty());
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    watcher
        .poll_due(NOW + 3600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 7200)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn raumbitte_ohne_code_startet_stunde_erst_bei_code() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(port.replies.lock().await[0].1, INVITE_LOUNGE_HINT_TEXT);
    watcher
        .handle_message(message(2, NOW + 4000, "123456789"), NOW + 4000, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 7599)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    watcher
        .poll_due(NOW + 7600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn raumbitte_ohne_code_verwendet_keinen_alten_code() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(message(1, NOW - 100, "123456789"), NOW - 100, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(
            message(2, NOW, "Kann mich jemand einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 8000)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    assert_eq!(port.replies.lock().await.len(), 1);
}

#[tokio::test]
async fn fremder_nutzer_kann_code_fortsetzung_nicht_ausloesen() {
    let (watcher, _, _, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut other = message(2, NOW + 1, "123456789");
    other.author_id = 8;
    watcher
        .handle_message(other, NOW + 1, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 8000)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn audit_und_dispatch_task_blocken_und_antwort_bleibt_erhalten() {
    for (audit, expected) in [(true, AUDIT_REPLY), (false, RUNNING_REPLY)] {
        let (watcher, store, port, invite) = setup();
        invite.audit.store(audit, Ordering::SeqCst);
        invite.running_task.store(!audit, Ordering::SeqCst);
        watcher
            .handle_message(
                message(1, NOW, "Kannst du mich einladen? 123456789"),
                NOW,
                Some(false),
            )
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
        watcher
            .poll_due(NOW + 3600)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
        assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
        assert_eq!(invite.calls.lock().await.len(), 1);
        assert!(port.replies.lock().await.is_empty());
        assert_eq!(
            store
                .load(USER)
                .await
                .expect("Testzustand lesbar")
                .expect("Bitte wurde gespeichert")
                .last_result
                .as_deref(),
            Some(expected)
        );
    }
}

#[tokio::test]
async fn fehler_bleibt_gespeichert_neue_direkte_bitte_erlaubt_einen_versuch() {
    let (watcher, store, _, invite) = setup();
    invite.fail.store(true, Ordering::SeqCst);
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 3600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 7200)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(
            message(2, NOW + 7200, "Kann mich jemand einladen? 123456789"),
            NOW + 7200,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert!(store
        .load(USER)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .last_result
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .contains("Fehlgeschlagen"));
    let retry = message(3, NOW + 7300, "Kannste mich auch einladen?");
    watcher
        .handle_message(retry.clone(), NOW + 7300, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(retry, NOW + 7300, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 12_000)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 2);
}

#[tokio::test]
async fn neustart_wiederholt_keinen_begonnenen_versuch() {
    let (watcher, store, port, invite) = setup();
    watcher
        .record_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let previous = store
        .load(USER)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut claimed = previous.clone();
    claimed
        .request
        .as_mut()
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .phase = Phase::Attempted;
    assert!(store
        .compare_exchange(USER, Some(&previous), &claimed)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    let restarted = InviteLoungeWatcher {
        store,
        port,
        invite: invite.clone(),
        guild_id: GUILD,
        clock_millis: Arc::new(|| NOW * 1000),
    };
    restarted
        .poll_due(NOW + 8000)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn zwei_dispatcher_erhalten_nur_einen_versuch() {
    let (watcher, _, _, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let (first, second) = tokio::join!(
        watcher.dispatch_due(USER, NOW + 3600),
        watcher.dispatch_due(USER, NOW + 3600)
    );
    first.expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    second.expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn angebote_dank_und_andere_kanaele_loesen_nichts_aus() {
    let (watcher, _, port, invite) = setup();
    for (index, content) in [
        "ich lad dich ein 123456789",
        "Ich kann dich einladen, brauchst du einen Invite? 123456789",
        "Danke für die Einladung! 123456789",
    ]
    .iter()
    .enumerate()
    {
        watcher
            .handle_message(message(index as u64 + 1, NOW, content), NOW, Some(false))
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    let mut other = message(20, NOW, "Kannst du mich einladen? 123456789");
    other.channel_id = 100;
    watcher
        .handle_message(other, NOW, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut bot = message(21, NOW, "Kannst du mich einladen? 123456789");
    bot.is_bot = true;
    watcher
        .handle_message(bot, NOW, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(port.replies.lock().await.is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn profil_link_braucht_zahlencode() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(
            message(
                1,
                NOW,
                "Kannst du mich einladen? https://steamcommunity.com/id/123456789",
            ),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(port.replies.lock().await[0].1, INVITE_LOUNGE_HINT_TEXT);
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        friend_code("https://steamcommunity.com/id/example 123456789"),
        Some("123456789".into())
    );
}

#[tokio::test]
async fn vorhandener_tipp_und_alter_cooldown_verhindern_zweiten_hinweis() {
    let (watcher, store, port, _) = setup();
    let source = message(1, NOW, "Kannst du mich einladen?");
    let mut tip = message(2, NOW + 1, INVITE_LOUNGE_HINT_TEXT);
    tip.author_id = BOT;
    tip.is_bot = true;
    tip.reply_message_id = Some(1);
    port.history.lock().await.extend([source.clone(), tip]);
    watcher
        .handle_message(source, NOW + 2, None)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(port.replies.lock().await.is_empty());
    store.legacy_hints.lock().await.insert(8, NOW);
    let mut other = message(3, NOW + 3, "Kannst du mich einladen?");
    other.author_id = 8;
    watcher
        .handle_message(other, NOW + 3, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn rueckblick_sieben_tage_einmal_mit_juengeren_warteauftraegen() {
    let (watcher, store, port, invite) = setup();
    let mut old = message(
        1,
        NOW - NEWCOMER_MAX_JOIN_SECONDS - 1,
        "Kann mich jemand einladen? 111111111",
    );
    old.author_id = 8;
    let mature = message(2, NOW - 3600, "Kann mich jemand einladen? 123456789");
    let mut young = message(3, NOW - 3599, "Kann mich jemand einladen? 222222222");
    young.author_id = 9;
    young.guild_id = None;
    port.history.lock().await.extend([young, mature, old]);
    watcher
        .backfill(NOW)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert!(port.replies.lock().await.is_empty());
    assert!(store
        .backfill_done()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    let fetched = port.history_calls.load(Ordering::SeqCst);
    watcher
        .backfill(NOW)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(port.history_calls.load(Ordering::SeqCst), fetched);
    watcher
        .poll_due(NOW + 1)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn rueckblick_bewahrt_direkte_code_fortsetzung_und_vorhandenen_tipp() {
    let (watcher, _, port, invite) = setup();
    let source = message(1, NOW - 120, "Kannst du mich einladen?");
    let mut tip = message(2, NOW - 100, INVITE_LOUNGE_HINT_TEXT);
    tip.author_id = BOT;
    tip.is_bot = true;
    tip.reply_message_id = Some(1);
    let code = message(3, NOW - 60, "123456789");
    port.history.lock().await.extend([code, tip, source]);
    watcher
        .backfill(NOW)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert!(port.replies.lock().await.is_empty());
    watcher
        .handle_message(
            message(4, NOW, "Kannst du mich einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn rueckblick_paginiert_ueber_mehr_als_hundert_nachrichten() {
    let (watcher, _, port, invite) = setup();
    let mut history = vec![message(
        1,
        NOW - 7200,
        "Kann mich jemand einladen? 123456789",
    )];
    for id in 2..=205 {
        history.push(message(id, NOW - 100, "Guten Morgen"));
    }
    port.history.lock().await.extend(history);
    watcher
        .backfill(NOW)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(port.history_calls.load(Ordering::SeqCst) >= 3);
    assert!(invite.calls.lock().await.is_empty());
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn fehlende_bot_id_erzeugt_keinen_menschlichen_admin() {
    let (watcher, store, port, invite) = setup();
    port.identity_missing.store(true, Ordering::SeqCst);
    assert!(watcher.backfill(NOW).await.is_err());
    assert!(!store
        .backfill_done()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    assert!(watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false)
        )
        .await
        .is_err());
    assert!(invite.calls.lock().await.is_empty());
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn postgres_alte_bitte_wird_dauerhaft_ohne_versand_beendet() {
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("Testdatenbank");
    let store = Arc::new(PgLoungeStore {
        pool: db.pool().clone(),
    });
    let port = Arc::new(RecordingPort::default());
    let invite = Arc::new(RecordingInvite::default());
    let watcher = InviteLoungeWatcher {
        store: store.clone(),
        port: port.clone(),
        invite: invite.clone(),
        guild_id: GUILD,
        clock_millis: Arc::new(|| NOW * 1000),
    };
    let pending = LoungeState {
        friend_code: Some("123456789".into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW - 7200,
            direct: false,
            phase: Phase::Pending,
        }),
        ..Default::default()
    };
    assert!(store
        .compare_exchange(USER, None, &pending)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    watcher
        .poll_due(NOW)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(
        store
            .load(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .request
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .phase,
        Phase::Expired
    );
    assert!(store
        .pending_users()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .is_empty());
    assert!(invite.calls.lock().await.is_empty());
    assert!(port.replies.lock().await.is_empty());
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn postgres_zustand_claim_und_bekannter_freundescode() {
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("Testdatenbank");
    let store = PgLoungeStore {
        pool: db.pool().clone(),
    };
    let mut state = LoungeState {
        friend_code: Some("123456789".into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW,
            direct: false,
            phase: Phase::Pending,
        }),
        ..Default::default()
    };
    assert!(store
        .compare_exchange(USER, None, &state)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    assert!(!store
        .compare_exchange(USER, None, &state)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    assert_eq!(
        store
            .pending_users()
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"),
        vec![USER]
    );
    let previous = state.clone();
    state
        .request
        .as_mut()
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .phase = Phase::Attempted;
    let (a, b) = tokio::join!(
        store.compare_exchange(USER, Some(&previous), &state),
        store.compare_exchange(USER, Some(&previous), &state)
    );
    assert_eq!(
        usize::from(a.expect("Testzustand und Testports erfüllen den erwarteten Vertrag"))
            + usize::from(b.expect("Testzustand und Testports erfüllen den erwarteten Vertrag")),
        1
    );
    assert!(store
        .pending_users()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .is_empty());
    assert_eq!(
        store
            .load(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"),
        Some(state)
    );
    assert!(!store
        .backfill_done()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    store
        .mark_backfill_done()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(store
        .backfill_done()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    kv::set(
        db.pool(),
        COOLDOWN_KV_NS,
        &cooldown_key(USER),
        &NOW.to_string(),
    )
    .await
    .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(
        store
            .last_hint_at(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"),
        Some(NOW)
    );
    sqlx::query("INSERT INTO core.users (discord_id) VALUES ($1)")
        .bind(USER as i64)
        .execute(db.pool())
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    sqlx::query("INSERT INTO core.steam_links (discord_id, steam_id, primary_account) VALUES ($1, '76561198083722517', true)").bind(USER as i64).execute(db.pool()).await.expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(
        store
            .known_code(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"),
        Some("123456789".into())
    );
}

#[tokio::test]
async fn direkte_bitte_waehrend_dispatch_startet_keinen_zweiten_versuch() {
    let (watcher, store, _, invite) = setup();
    invite.hold.store(true, Ordering::SeqCst);
    let initial = watcher.handle_message(
        message(1, NOW, "Kannst du mich einladen? 123456789"),
        NOW,
        Some(false),
    );
    let during = async {
        invite.started.notified().await;
        watcher
            .handle_message(
                message(2, NOW + 1, "Kannst du mich auch einladen?"),
                NOW + 1,
                Some(false),
            )
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
        assert_eq!(invite.calls.lock().await.len(), 1);
        invite.release.notify_one();
    };
    let (result, ()) = tokio::join!(initial, during);
    result.expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let state = store
        .load(USER)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(state.last_seen, 2);
    assert_eq!(
        state
            .request
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .phase,
        Phase::Attempted
    );
    assert!(state
        .last_result
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .starts_with("🎮"));
}

#[tokio::test]
async fn dank_ist_kein_audit_beleg_und_stoppt_keine_raumbitte() {
    let (watcher, _, _, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(
            message(2, NOW + 100, "Danke für die Einladung!"),
            NOW + 100,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 3600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}

#[test]
fn raumbitten_mit_bitte_sind_keine_direkte_bot_bitte() {
    for content in [
        "Kann mich bitte einer einladen? 123456789",
        "Kann mich bitte irgendjemand einladen? 123456789",
        "Kann mich bitte jemand einladen? 123456789",
        "Bitte kann mich wer einladen? 123456789",
    ] {
        assert_eq!(
            request_kind(content, BOT),
            Some(RequestKind::Room),
            "{content}"
        );
    }
}

#[tokio::test]
async fn gepufferte_bitte_waehrend_gescheitertem_versand_startet_keinen_versuch() {
    let (watcher, store, _, invite) = setup();
    let state = LoungeState {
        last_seen: 1,
        friend_code: Some("123456789".into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW,
            direct: true,
            phase: Phase::Attempted,
        }),
        dispatch_started_at_millis: Some(NOW * 1000 + 700),
        last_dispatch_finished_at_millis: Some((NOW + 120) * 1000 + 999),
        last_result: Some("Fehlgeschlagen".into()),
        ..Default::default()
    };
    store.states.lock().await.insert(USER, state);
    let buffered = message(2, NOW + 120, "Kannst du mich auch einladen?");
    watcher
        .handle_message(buffered, NOW + 121, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    watcher
        .handle_message(
            message(3, NOW + 122, "Kannste mich auch einladen?"),
            NOW + 122,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn laufender_versand_blockiert_keine_bitte_eines_anderen_nutzers() {
    let (watcher, _, _, invite) = setup();
    let watcher = Arc::new(watcher);
    invite.hold.store(true, Ordering::SeqCst);
    watcher
        .record_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut tasks = tokio::task::JoinSet::new();
    spawn_dispatch(&mut tasks, watcher.clone(), USER);
    invite.started.notified().await;
    let mut second = message(2, NOW, "Kannst du mich einladen? 222222222");
    second.author_id = 8;
    let user_id = watcher
        .record_message(second, NOW, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    invite.hold.store(false, Ordering::SeqCst);
    spawn_dispatch(&mut tasks, watcher, user_id);
    tasks
        .join_next()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    invite.release.notify_one();
    tasks
        .join_next()
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 2);
    let calls = invite.calls.lock().await;
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].3, USER);
    assert_eq!(calls[1].3, 8);
}

#[tokio::test]
async fn geaenderter_code_im_rueckblick_bleibt_ohne_auftrag() {
    let (watcher, _, port, invite) = setup();
    port.history.lock().await.extend([
        message(1, NOW - 7200, "Kann mich jemand einladen? 123456789"),
        message(2, NOW - 300, "Kann mich jemand einladen? 222222222"),
    ]);
    watcher
        .backfill(NOW)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    watcher
        .poll_due(NOW + 3299)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    watcher
        .poll_due(NOW + 3300)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn verspätete_gateway_nachricht_erzeugt_weder_hinweis_noch_versand() {
    for content in [
        "Kannst du mich einladen?",
        "Kannst du mich einladen? 123456789",
        "Kann mich jemand einladen? 123456789",
    ] {
        let (watcher, store, port, invite) = setup();
        watcher
            .handle_message(message(1, NOW - 7200, content), NOW, Some(false))
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
        assert!(store
            .load(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .is_none());
        assert!(port.replies.lock().await.is_empty());
        assert!(invite.calls.lock().await.is_empty());
    }
}

#[tokio::test]
async fn alter_warteauftrag_verfällt_ohne_steam_oder_antwort() {
    for direct in [true, false] {
        let (watcher, store, port, invite) = setup();
        store.states.lock().await.insert(
            USER,
            LoungeState {
                friend_code: Some("123456789".into()),
                request: Some(Request {
                    message_id: 1,
                    created_at: NOW - 7200,
                    direct,
                    phase: Phase::Pending,
                }),
                ..Default::default()
            },
        );
        watcher
            .poll_due(NOW)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
        watcher
            .poll_due(NOW + 1)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
        assert_eq!(
            store
                .load(USER)
                .await
                .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
                .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
                .request
                .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
                .phase,
            Phase::Expired
        );
        assert!(port.replies.lock().await.is_empty());
        assert!(invite.calls.lock().await.is_empty());
    }
}

#[tokio::test]
async fn raumbitte_mit_echtem_audit_erzeugt_keine_späte_statusantwort() {
    let (watcher, store, port, invite) = setup();
    invite.audit.store(true, Ordering::SeqCst);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + ROOM_WAIT_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    assert_eq!(
        store
            .load(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .last_result
            .as_deref(),
        Some(AUDIT_REPLY)
    );
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn langsamer_direkter_versand_speichert_ergebnis_ohne_späte_antwort() {
    let (mut watcher, store, port, invite) = setup();
    let clock = Arc::new(AtomicI64::new(NOW * 1000));
    let reader = clock.clone();
    watcher.clock_millis = Arc::new(move || reader.load(Ordering::SeqCst));
    *invite.advance.lock().await = Some((clock, (NOW + 7200) * 1000));
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert_eq!(
        store
            .load(USER)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .request
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
            .phase,
        Phase::Attempted
    );
    assert!(port.replies.lock().await.is_empty());
}

#[test]
fn frischefenster_hat_eine_obere_und_untere_grenze() {
    assert!(is_fresh(NOW - FRESHNESS_SECONDS, NOW, FRESHNESS_SECONDS));
    assert!(!is_fresh(
        NOW - FRESHNESS_SECONDS - 1,
        NOW,
        FRESHNESS_SECONDS
    ));
    assert!(!is_fresh(NOW + 1, NOW, FRESHNESS_SECONDS));
}

#[tokio::test]
async fn rueckblick_und_gateway_erhalten_die_bitte_in_beiden_reihenfolgen() {
    for history_first in [true, false] {
        for direct in [true, false] {
            let (watcher, store, port, invite) = setup();
            let content = if direct {
                "Kannst du mich einladen? 123456789"
            } else {
                "Kann mich jemand einladen? 123456789"
            };
            let source = message(10, NOW, content);
            port.history.lock().await.push(source.clone());
            if history_first {
                watcher
                    .record_history(NOW)
                    .await
                    .expect("Rückblick beobachtet die frische Bitte");
                assert_eq!(
                    store
                        .load(USER)
                        .await
                        .expect("Zustand lesbar")
                        .expect("Historischer Code gespeichert")
                        .last_seen,
                    0
                );
            }
            watcher
                .record_message(source.clone(), NOW, Some(false))
                .await
                .expect("Gateway legt die Live-Bitte an");
            if !history_first {
                watcher
                    .record_history(NOW)
                    .await
                    .expect("Rückblick bewahrt die Live-Bitte");
            }
            let state = store
                .load(USER)
                .await
                .expect("Zustand lesbar")
                .expect("Live-Bitte gespeichert");
            assert_eq!(state.last_seen, source.message_id);
            assert_eq!(
                state.request.expect("Live-Bitte vorhanden").phase,
                Phase::Pending
            );
            let due = NOW + if direct { 0 } else { ROOM_WAIT_SECONDS };
            watcher
                .dispatch_due(USER, due)
                .await
                .expect("Live-Bitte wird einmal ausgeführt");
            watcher
                .record_message(source, NOW, Some(false))
                .await
                .expect("Gateway-Duplikat ist harmlos");
            watcher
                .dispatch_due(USER, due)
                .await
                .expect("Erledigter Auftrag bleibt erledigt");
            assert_eq!(invite.calls.lock().await.len(), 1);
            assert!(port.replies.lock().await.is_empty());
        }
    }
}

#[tokio::test]
async fn ablauf_waehrend_load_oder_claim_verhindert_jeden_versand() {
    for direct in [true, false] {
        for point in [WaitPoint::Load, WaitPoint::Claim] {
            let (mut watcher, store, port, invite) = setup();
            let clock = Arc::new(AtomicI64::new(NOW * 1000));
            let reader = clock.clone();
            watcher.clock_millis = Arc::new(move || reader.load(Ordering::SeqCst));
            let max_age = FRESHNESS_SECONDS + if direct { 0 } else { ROOM_WAIT_SECONDS };
            store.states.lock().await.insert(
                USER,
                LoungeState {
                    friend_code: Some("123456789".into()),
                    request: Some(Request {
                        message_id: 1,
                        created_at: NOW - max_age,
                        direct,
                        phase: Phase::Pending,
                    }),
                    ..Default::default()
                },
            );
            *store.advance.lock().await = Some(ClockAdvance {
                point,
                clock,
                at: (NOW + 1) * 1000,
            });
            watcher
                .dispatch_due(USER, NOW)
                .await
                .expect("Abgelaufene Bitte wird abgeschlossen");
            let state = store
                .load(USER)
                .await
                .expect("Zustand lesbar")
                .expect("Bitte gespeichert");
            assert_eq!(
                state.request.expect("Bitte vorhanden").phase,
                Phase::Expired
            );
            assert!(store
                .pending_users()
                .await
                .expect("Offene Bitten lesbar")
                .is_empty());
            assert!(invite.calls.lock().await.is_empty());
            assert!(port.replies.lock().await.is_empty());
        }
    }
}

#[tokio::test]
async fn ablauf_waehrend_hinweis_claim_unterdrueckt_den_hinweis() {
    let (mut watcher, store, port, invite) = setup();
    let clock = Arc::new(AtomicI64::new(NOW * 1000));
    let reader = clock.clone();
    watcher.clock_millis = Arc::new(move || reader.load(Ordering::SeqCst));
    *store.advance.lock().await = Some(ClockAdvance {
        point: WaitPoint::Hint,
        clock,
        at: (NOW + 1) * 1000,
    });
    watcher
        .handle_message(
            message(1, NOW - FRESHNESS_SECONDS, "Kannst du mich einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Verspäteter Hinweis wird unterdrückt");
    assert!(port.replies.lock().await.is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn ablauf_waehrend_abschluss_speichert_status_ohne_oeffentliche_antwort() {
    let (mut watcher, store, port, invite) = setup();
    let clock = Arc::new(AtomicI64::new(NOW * 1000));
    let reader = clock.clone();
    watcher.clock_millis = Arc::new(move || reader.load(Ordering::SeqCst));
    *store.advance.lock().await = Some(ClockAdvance {
        point: WaitPoint::Finish,
        clock,
        at: (NOW + 7200) * 1000,
    });
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Echter Versandstatus wird gespeichert");
    assert_eq!(invite.calls.lock().await.len(), 1);
    let state = store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .expect("Bitte gespeichert");
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Attempted
    );
    assert!(state
        .last_result
        .expect("Versandstatus gespeichert")
        .starts_with("🎮"));
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn gleicher_code_schiebt_die_stunde_nicht_auf() {
    let (watcher, _, _, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(message(2, NOW + 300, "123456789"), NOW + 300, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 3600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}
