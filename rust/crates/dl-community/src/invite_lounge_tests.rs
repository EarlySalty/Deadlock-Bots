use super::status::{
    brain_backoff, HeldAnswer, Notice, RequestStatus, SteamOutcome, SteamWait, ANSWER_QUEUE_LIMIT,
    CHECK_IN_SECONDS, RETRY_SECONDS,
};
use super::*;
use crate::answer_support::test_brain::{check_in_reply, ScriptedBrain, ScriptedReply};
use dl_brain::{
    BrainError, ConversationReply, InviteCheckInOutcome, InviteLoungeUpdate, InviteNotice,
    InviteStatus, NoticeTarget,
};
use std::collections::HashMap;

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use tokio::sync::Mutex;

const TEST_BRAIN_TIMEOUT: Duration = Duration::from_secs(120);

fn test_cutoff() -> Duration {
    claim_cutoff(TEST_BRAIN_TIMEOUT)
}

fn test_lease_millis() -> i64 {
    claim_lease_millis(test_cutoff())
}

const INVITE_LOUNGE_HINT_TEXT: &str = "Brain-Hinweis";
const NOW: i64 = 1_790_000_000;
const ONE_HOUR: i64 = 60 * 60;
const CHECK_IN_CHANNEL_ID: u64 = 1_464_736_918_951_432_222;
const BOT: u64 = 99;
const GUILD: u64 = 42;
const USER: u64 = 7;
const IMPERSONAL_REQUEST: &str = "Jo, kann man hier einen Invite bekommen, wenn man nett fragt?😅";
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
    actions_since: AtomicI64,
    progress: Mutex<SteamProgress>,
    handled: AtomicBool,
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
                    || !state.status.held_answers.is_empty()
            })
            .map(|(id, _)| *id)
            .collect())
    }
    async fn waiting_users(&self, created_since: i64) -> Result<Vec<u64>, String> {
        Ok(self
            .states
            .lock()
            .await
            .iter()
            .filter(|(_, state)| {
                state.request.as_ref().is_some_and(|request| {
                    request.phase == Phase::WaitingForCode && request.created_at >= created_since
                })
            })
            .map(|(id, _)| *id)
            .collect())
    }
    async fn outbox_users(&self) -> Result<Vec<u64>, String> {
        Ok(self
            .states
            .lock()
            .await
            .iter()
            .filter(|(_, state)| !state.outbox.is_empty())
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
    async fn actions_since(&self) -> Result<Option<i64>, String> {
        Ok(Some(self.actions_since.load(Ordering::SeqCst)))
    }
    async fn mark_actions_since(&self, now: i64) -> Result<(), String> {
        let _ = self
            .actions_since
            .compare_exchange(0, now, Ordering::SeqCst, Ordering::SeqCst);
        Ok(())
    }
    async fn steam_progress(&self, _code: &str, _since: i64) -> Result<SteamProgress, String> {
        Ok(*self.progress.lock().await)
    }
    async fn handled_since(&self, _code: &str, _since: i64) -> Result<bool, String> {
        Ok(self.handled.load(Ordering::SeqCst))
    }
}

#[derive(Default)]
struct RecordingPort {
    replies: Mutex<Vec<(u64, String)>>,
    history: Mutex<Vec<LoungeMessage>>,
    history_calls: AtomicUsize,
    identity_missing: AtomicBool,
    history_fails: AtomicBool,
    deleted: Mutex<HashSet<u64>>,
    edited: Mutex<HashMap<u64, String>>,
    presence_fails: AtomicBool,
    reply_fails: AtomicBool,
}

#[async_trait::async_trait]
impl InviteLoungeReplyPort for RecordingPort {
    fn bot_id(&self) -> Option<u64> {
        (!self.identity_missing.load(Ordering::SeqCst)).then_some(BOT)
    }
    fn channel_category(&self, _guild_id: u64, channel_id: u64) -> Option<u64> {
        Some(if channel_id == 100 {
            0
        } else {
            COMMUNITY_CATEGORY_ID
        })
    }
    async fn history(
        &self,
        _channel_id: u64,
        before: Option<u64>,
    ) -> Result<Vec<LoungeMessage>, String> {
        self.history_calls.fetch_add(1, Ordering::SeqCst);
        if self.history_fails.load(Ordering::SeqCst) {
            return Err("Verlauf nicht lesbar".into());
        }
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
        assert!([INVITE_LOUNGE_CHANNEL_ID, CHECK_IN_CHANNEL_ID].contains(&channel));
        if self.reply_fails.load(Ordering::SeqCst) {
            return Err("Antwort nicht zustellbar".into());
        }
        self.replies.lock().await.push((id, text.into()));
        Ok(id + 100_000)
    }
    async fn message_content(&self, _channel: u64, id: u64) -> Result<Option<String>, String> {
        if self.presence_fails.load(Ordering::SeqCst) {
            return Err("Discord nicht erreichbar".into());
        }
        if self.deleted.lock().await.contains(&id) {
            return Ok(None);
        }
        Ok(Some(
            self.edited
                .lock()
                .await
                .get(&id)
                .cloned()
                .unwrap_or_default(),
        ))
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
    reply: Mutex<Option<String>>,
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
        if let Some(reply) = self.reply.lock().await.clone() {
            return Ok(reply);
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
        brain: crate::answer_support::test_brain::ScriptedBrain::always(INVITE_LOUNGE_HINT_TEXT),
        store: store.clone(),
        port: port.clone(),
        invite: invite.clone(),
        guild_id: GUILD,
        lease_millis: test_lease_millis(),
        cutoff: test_cutoff(),
        clock_millis: Arc::new(|| NOW * 1000),
    };
    (watcher, store, port, invite)
}

fn setup_scripted() -> (
    InviteLoungeWatcher,
    Arc<MemoryStore>,
    Arc<RecordingPort>,
    Arc<RecordingInvite>,
    Arc<ScriptedBrain>,
) {
    setup_check_ins(Vec::new())
}

fn setup_check_ins(
    check_ins: Vec<ScriptedReply>,
) -> (
    InviteLoungeWatcher,
    Arc<MemoryStore>,
    Arc<RecordingPort>,
    Arc<RecordingInvite>,
    Arc<ScriptedBrain>,
) {
    let (mut watcher, store, port, invite) = setup();
    let brain = ScriptedBrain::always_with_check_ins("Brain-Status", check_ins);
    watcher.brain = brain.clone();
    (watcher, store, port, invite, brain)
}

fn notices(brain: &ScriptedBrain) -> Vec<(u64, InviteNotice)> {
    brain
        .requests
        .lock()
        .expect("Brain-Anfragen lesbar")
        .iter()
        .filter_map(|request| match &request.task {
            dl_brain::DiscordConversation::InviteNotice { notice } => {
                Some((request.user_id, notice.clone()))
            }
            _ => None,
        })
        .collect()
}

fn status_updates(brain: &ScriptedBrain) -> Vec<InviteLoungeUpdate> {
    notices(brain)
        .into_iter()
        .map(|(_, notice)| notice.update)
        .collect()
}

fn notice_answers(brain: &ScriptedBrain) -> Vec<Option<String>> {
    notices(brain)
        .into_iter()
        .map(|(_, notice)| notice.answer)
        .collect()
}

fn answer_update(outcome: InviteCheckInOutcome) -> InviteLoungeUpdate {
    InviteLoungeUpdate::CheckInAnswer { outcome }
}

fn check_in_texts(brain: &ScriptedBrain) -> Vec<String> {
    brain
        .requests
        .lock()
        .expect("Brain-Anfragen lesbar")
        .iter()
        .filter_map(|request| match &request.task {
            dl_brain::DiscordConversation::InviteCheckIn { message } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn assert_notices_request(brain: &ScriptedBrain, count: usize) {
    let notices = notices(brain);
    assert_eq!(notices.len(), count);
    assert!(notices.iter().all(|(user, notice)| *user == USER
        && notice.target
            == NoticeTarget::Reply {
                channel_id: CHECK_IN_CHANNEL_ID,
                message_id: 1
            }));
    let mut keys: Vec<_> = notices
        .iter()
        .map(|(_, notice)| &notice.idempotency_key)
        .collect();
    keys.dedup();
    assert_eq!(keys.len(), count);
}

fn capture_logs() -> (
    crate::answer_support::test_logging::LogCapture,
    tracing::subscriber::DefaultGuard,
) {
    let capture = crate::answer_support::test_logging::LogCapture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_ansi(false)
        .without_time()
        .with_target(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (capture, guard)
}

fn message(id: u64, at: i64, content: &str) -> LoungeMessage {
    LoungeMessage {
        guild_id: Some(GUILD),
        channel_id: CHECK_IN_CHANNEL_ID,
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

#[test]
fn impersonal_invite_requests_are_room_requests() {
    for content in [
        IMPERSONAL_REQUEST,
        "Kann man hier einen Invite bekommen?",
        "Könnte man bitte eine Einladung erhalten?",
        "Koennte man hier noch nen Invite kriegen?",
        "Darf man vielleicht einen Invite haben?",
        "kann man hier noch einen invite bekommen",
        "Kann man eine Einladung hier bekommen?",
        "Kann man hier ein Invite bekommen?",
        "Kann man hier Invite bekommen?",
        "Hallo zusammen, kann man hier einen Invite bekommen?",
        "Hey Leute, könnte man bitte eine Einladung erhalten?",
        "Moin! Kann man hier einen Invite bekommen?",
        "Guten Abend, darf man einen Invite haben?",
        "Hallo\nkann man hier einen Invite bekommen?",
        "Bin neu hier. Kann man einen Invite bekommen?",
    ] {
        assert_eq!(
            request_kind(content, BOT),
            Some(RequestKind::Room),
            "{content}"
        );
    }
}

#[test]
fn impersonal_invite_discussion_is_not_a_request() {
    for content in [
        "Man kann hier einen Invite bekommen, wenn man nett fragt.",
        "Hier kann man einen Invite bekommen, wenn man nett fragt.",
        "Bei Max kann man hier einen Invite bekommen.",
        "Bei Max\nkann man hier einen Invite bekommen.",
        "Mit etwas Geduld könnte man hier eine Einladung erhalten.",
        "Wenn man nett fragt, kann man hier einen Invite bekommen.",
        "Hallo zusammen, hier kann man einen Invite bekommen.",
        "Gut zu wissen. Bei Max kann man einen Invite bekommen.",
        "Die Frage lautet: Kann man hier einen Invite bekommen?",
        "Früher konnte man hier einen Invite bekommen.",
        "Ich habe einen Invite bekommen, danke!",
        "Kann man hier andere Leute einladen?",
        "Wie kann man einen Invite verschicken?",
        "Kann man Einladungen zurücknehmen?",
        "Kann man hier einen Invite verschenken?",
        "Kann man hier Hilfe bekommen? Der Invite ist schon da.",
        "Ich kann dich einladen, wenn du einen Invite bekommen willst.",
        "Warum kann man hier keinen Invite bekommen?",
    ] {
        assert_eq!(request_kind(content, BOT), None, "{content}");
    }
}

#[tokio::test]
async fn impersonal_request_without_code_gets_one_hint_without_dispatch() {
    let (watcher, store, port, invite) = setup();
    let event = message(1, NOW, IMPERSONAL_REQUEST);
    watcher
        .handle_message(event.clone(), NOW, None)
        .await
        .expect("request is recorded and hint is sent");
    watcher
        .handle_message(event, NOW, None)
        .await
        .expect("duplicate request is ignored");
    watcher
        .handle_message(message(2, NOW + 20, IMPERSONAL_REQUEST), NOW + 20, None)
        .await
        .expect("repeated request respects hint cooldown");
    assert_eq!(
        port.replies.lock().await.as_slice(),
        &[(1, INVITE_LOUNGE_HINT_TEXT.into())]
    );
    assert!(invite.calls.lock().await.is_empty());
    let request = store
        .load(USER)
        .await
        .expect("state can be read")
        .expect("request state exists")
        .request
        .expect("request is waiting for a code");
    assert_eq!(request.phase, Phase::WaitingForCode);
    assert!(!request.direct);
}

#[tokio::test]
async fn brain_ausfall_verbraucht_den_hinweis_nicht() {
    let (mut watcher, store, port, _invite) = setup();
    watcher.brain = crate::answer_support::test_brain::ScriptedBrain::failing();
    watcher
        .handle_message(message(1, NOW, IMPERSONAL_REQUEST), NOW, None)
        .await
        .expect("Brain-Ausfall ist kein Verarbeitungsfehler");
    let state = store
        .load(USER)
        .await
        .expect("state can be read")
        .expect("request state exists");
    assert!(!state.hint_sent);
    assert_eq!(state.last_hint_at, None);
    assert!(port.replies.lock().await.is_empty());

    watcher.brain =
        crate::answer_support::test_brain::ScriptedBrain::always(INVITE_LOUNGE_HINT_TEXT);
    watcher
        .handle_message(message(2, NOW + 20, IMPERSONAL_REQUEST), NOW + 20, None)
        .await
        .expect("Wiederholung bekommt den Hinweis");
    assert_eq!(
        port.replies.lock().await.as_slice(),
        &[(2, INVITE_LOUNGE_HINT_TEXT.into())]
    );
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
async fn schweigen_nach_der_nachfrage_loest_nie_eine_einladung_aus() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    for at in [
        NOW + CHECK_IN_SECONDS,
        NOW + ONE_HOUR,
        NOW + 6 * 60 * 60,
        NOW + 23 * 60 * 60,
        NOW + REQUEST_WINDOW_SECONDS,
    ] {
        watcher.poll_due(at).await.expect("Abfrage läuft");
    }
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(port.replies.lock().await.is_empty());
    let state = store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .expect("Bitte gespeichert");
    assert!(!state.status.invite_now);
    assert!(state.dispatch_started_at_millis.is_none());
}

#[tokio::test]
async fn raumbitte_ohne_code_fragt_erst_nach_dem_code_nach() {
    let (watcher, _, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(port.replies.lock().await.len(), 1);
    watcher
        .handle_message(message(2, NOW + 4000, "123456789"), NOW + 4000, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 4000 + CHECK_IN_SECONDS - 1)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(status_updates(&brain).is_empty());
    watcher
        .poll_due(NOW + 4000 + CHECK_IN_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(invite.calls.lock().await.is_empty());
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
async fn steam_ausfall_bleibt_still_offen_und_versucht_spaeter_erneut() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    invite.fail.store(true, Ordering::SeqCst);
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let state = store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .expect("Bitte gespeichert");
    assert!(state
        .last_result
        .expect("Ergebnis gespeichert")
        .contains("Fehlgeschlagen"));
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Pending
    );
    watcher
        .poll_due(NOW + RETRY_SECONDS - 1)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
    watcher
        .poll_due(NOW + RETRY_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 2);
    assert!(notices(&brain).is_empty());
    assert!(status_updates(&brain).is_empty());
    invite.fail.store(false, Ordering::SeqCst);
    watcher
        .poll_due(NOW + 2 * RETRY_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 3);
    assert_eq!(
        status_updates(&brain),
        vec![InviteLoungeUpdate::Status {
            status: InviteStatus::Sent
        }]
    );
    assert_notices_request(&brain, 1);
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn fa_raus_wird_einmal_gemeldet_und_danach_die_einladung() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    *invite.reply.lock().await =
        Some("📨 Freundschaftsanfrage ist raus. Sag ihm, er soll sie auf Steam annehmen.".into());
    store.progress.lock().await.open = true;
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    for at in [NOW + 61, NOW + 200, NOW + 400] {
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    assert_eq!(
        status_updates(&brain),
        vec![InviteLoungeUpdate::Status {
            status: InviteStatus::FriendshipMissing
        }]
    );
    *store.progress.lock().await = SteamProgress {
        outcome: Some(SteamOutcome::Sent),
        invited: true,
        open: false,
    };
    watcher
        .poll_due(NOW + 600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + 900)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::Status {
                status: InviteStatus::FriendshipMissing
            },
            InviteLoungeUpdate::Status {
                status: InviteStatus::Sent
            }
        ]
    );
    assert_notices_request(&brain, 2);
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn nachfrage_nach_dreissig_minuten_und_nein_loest_die_einladung_aus() {
    let (watcher, _, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS - 1)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(notices(&brain).is_empty());
    for at in [NOW + CHECK_IN_SECONDS, NOW + CHECK_IN_SECONDS + 60] {
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(invite.calls.lock().await.is_empty());
    let answer = message(2, NOW + 1900, "nein, noch nichts");
    watcher
        .handle_message(answer, NOW + 1900, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            InviteLoungeUpdate::Status {
                status: InviteStatus::Sent
            }
        ]
    );
    assert_eq!(check_in_texts(&brain), ["nein, noch nichts"]);
    assert_eq!(notice_answers(&brain), [None, None]);
}

#[tokio::test]
async fn ja_auf_die_nachfrage_bestaetigt_ohne_neue_einladung() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::Arrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let answer = message(2, NOW + 1900, "ja, ist angekommen");
    watcher
        .handle_message(answer, NOW + 1900, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + ONE_HOUR)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            answer_update(InviteCheckInOutcome::Arrived)
        ]
    );
    assert_eq!(check_in_texts(&brain), ["ja, ist angekommen"]);
    assert_eq!(
        store
            .load(USER)
            .await
            .expect("Zustand lesbar")
            .expect("Bitte gespeichert")
            .request
            .expect("Bitte vorhanden")
            .phase,
        Phase::Attempted
    );
}

#[tokio::test]
async fn nachfrage_liest_auch_schlichte_folgenachrichten() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::Arrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(message(2, NOW + 1900, "ja"), NOW + 1900, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .handle_message(
            message(3, NOW + 1901, "ja wirklich"),
            NOW + 1901,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(check_in_texts(&brain), ["ja"]);
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            answer_update(InviteCheckInOutcome::Arrived)
        ]
    );
    let state = store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .expect("Bitte gespeichert");
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Attempted
    );
}

#[tokio::test]
async fn unklare_antwort_bleibt_still_und_die_folgeantwort_wird_gelesen() {
    let (watcher, _, _, invite, brain) = setup_check_ins(vec![
        check_in_reply(InviteCheckInOutcome::Unclear),
        check_in_reply(InviteCheckInOutcome::Arrived),
    ]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let unclear = message(2, NOW + 1900, "wie meinst du das?");
    watcher
        .handle_message(unclear, NOW + 1900, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    let follow_up = message(3, NOW + 1901, "ja, angekommen");
    watcher
        .handle_message(follow_up, NOW + 1901, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        check_in_texts(&brain),
        ["wie meinst du das?", "ja, angekommen"]
    );
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            answer_update(InviteCheckInOutcome::Arrived)
        ]
    );
}

#[tokio::test]
async fn brain_fehler_bei_der_antwort_verbraucht_sie_nicht() {
    let (mut watcher, store, _, invite, brain) = setup_check_ins(vec![
        Err(BrainError::Backend("scripted".into())),
        check_in_reply(InviteCheckInOutcome::Arrived),
    ]);
    let clock = moving_clock(&mut watcher);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    clock.store((NOW + 1900) * 1000, Ordering::SeqCst);
    watcher
        .handle_message(
            message(2, NOW + 1900, "ja, ist da"),
            NOW + 1900,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    let later = NOW + 1900 + brain_backoff(1) / 1000;
    let held = stored(&store).await.status.held_answers;
    assert_eq!(
        held,
        vec![HeldAnswer {
            message_id: 2,
            content: "ja, ist da".into(),
            retry_at_millis: later * 1000,
            attempts: 1,
        }]
    );
    clock.store(later * 1000, Ordering::SeqCst);
    watcher
        .handle_message(message(3, later, "hallo?"), later, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(check_in_texts(&brain), ["ja, ist da", "ja, ist da"]);
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            answer_update(InviteCheckInOutcome::Arrived)
        ]
    );
}

#[tokio::test]
async fn beantwortete_raumbitte_bekommt_keine_nachfrage() {
    let (watcher, _, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut answer = message(2, NOW + 60, "Schick ich dir gleich");
    answer.author_id = 8;
    answer.reply_message_id = Some(1);
    port.history.lock().await.push(answer);
    for at in [NOW + CHECK_IN_SECONDS, NOW + ONE_HOUR] {
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    assert!(invite.calls.lock().await.is_empty());
    assert!(status_updates(&brain).is_empty());
    assert!(notices(&brain).is_empty());
}

#[tokio::test]
async fn bereits_eingeladen_laut_steam_bestaetigt_ohne_neue_einladung() {
    let (watcher, store, _, invite, brain) = setup_scripted();
    store.progress.lock().await.invited = true;
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    for at in [NOW + CHECK_IN_SECONDS, NOW + ONE_HOUR] {
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        status_updates(&brain),
        vec![InviteLoungeUpdate::Status {
            status: InviteStatus::AlreadyHasGame
        }]
    );
    assert_notices_request(&brain, 1);
}

#[tokio::test]
async fn eigene_bot_antwort_haelt_die_einladung_nicht_auf() {
    let (watcher, _, port, invite, _) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut own = message(2, NOW + CHECK_IN_SECONDS, "Ist die Einladung angekommen?");
    own.author_id = BOT;
    own.is_bot = true;
    own.reply_message_id = Some(1);
    port.history.lock().await.push(own);
    answer_no_after_check_in(&watcher).await;
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn fremde_bot_antwort_haelt_die_einladung_nicht_auf() {
    let (watcher, _, port, invite, _) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let mut other = message(2, NOW + 60, "Glückwunsch zum neuen Level!");
    other.author_id = 77;
    other.is_bot = true;
    other.reply_message_id = Some(1);
    port.history.lock().await.push(other);
    answer_no_after_check_in(&watcher).await;
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn unlesbarer_verlauf_haelt_die_einladung_nicht_auf() {
    let (watcher, _, port, invite, _) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    port.history_fails.store(true, Ordering::SeqCst);
    answer_no_after_check_in(&watcher).await;
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn von_hand_erledigte_bitte_bleibt_ohne_meldung() {
    let (watcher, store, _, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    store.handled.store(true, Ordering::SeqCst);
    store.progress.lock().await.invited = true;
    for at in [NOW + CHECK_IN_SECONDS, NOW + ONE_HOUR] {
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    assert!(invite.calls.lock().await.is_empty());
    assert!(status_updates(&brain).is_empty());
    assert!(notices(&brain).is_empty());
}

#[tokio::test]
async fn bitten_vor_dem_deploy_loesen_nichts_aus() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    store.actions_since.store(NOW, Ordering::SeqCst);
    watcher
        .handle_message(
            message(1, NOW - 10, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + ONE_HOUR)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert!(status_updates(&brain).is_empty());
    assert!(port.replies.lock().await.is_empty());
    watcher
        .handle_message(
            message(2, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn neustart_wiederholt_keinen_begonnenen_versuch() {
    let (watcher, store, port, invite) = setup();
    watcher
        .record_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
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
        .phase = Phase::Dispatching;
    assert!(store
        .compare_exchange(USER, Some(&previous), &claimed)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag"));
    let restarted = InviteLoungeWatcher {
        brain: crate::answer_support::test_brain::ScriptedBrain::always(INVITE_LOUNGE_HINT_TEXT),
        store,
        port,
        invite: invite.clone(),
        guild_id: GUILD,
        lease_millis: test_lease_millis(),
        cutoff: test_cutoff(),
        clock_millis: Arc::new(|| NOW * 1000),
    };
    restarted
        .poll_due(NOW + 60)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    restarted
        .dispatch_due(USER, NOW + 60)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert!(invite.calls.lock().await.is_empty());
    assert!(previous.request.is_some_and(|request| request.direct));
}

#[tokio::test]
async fn zwei_dispatcher_erhalten_nur_einen_versuch() {
    let (watcher, _, _, invite) = setup();
    watcher
        .record_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    invite.hold.store(true, Ordering::SeqCst);
    let watcher = Arc::new(watcher);
    let first = watcher.clone();
    let running = tokio::spawn(async move { first.dispatch_due(USER, NOW).await });
    invite.started.notified().await;
    watcher
        .dispatch_due(USER, NOW)
        .await
        .expect("Zweiter Versuch bleibt harmlos");
    invite.release.notify_one();
    running
        .await
        .expect("Erster Versuch endet")
        .expect("Erster Versuch läuft");
    assert_eq!(invite.calls.lock().await.len(), 1);
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
async fn direkte_bitte_in_jedem_kanal_der_community_kategorie_wird_bearbeitet() {
    let (watcher, _, _, invite) = setup();
    let mut event = message(1, NOW, "Kannst du mich einladen? 123456789");
    event.channel_id = 101;
    watcher
        .handle_message(event, NOW, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(
        invite.calls.lock().await.as_slice(),
        &[(BOT, GUILD, "123456789".into(), USER)]
    );
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
        brain: crate::answer_support::test_brain::ScriptedBrain::always(INVITE_LOUNGE_HINT_TEXT),
        store: store.clone(),
        port: port.clone(),
        invite: invite.clone(),
        guild_id: GUILD,
        lease_millis: test_lease_millis(),
        cutoff: test_cutoff(),
        clock_millis: Arc::new(|| NOW * 1000),
    };
    let pending = LoungeState {
        friend_code: Some("123456789".into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW - REQUEST_WINDOW_SECONDS - 1,
            direct: false,
            phase: Phase::Pending,
            reposts: Vec::new(),
            earlier: Vec::new(),
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
async fn postgres_offene_admin_einladung_zaehlt_als_erledigt() {
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("Testdatenbank");
    let store = PgLoungeStore {
        pool: db.pool().clone(),
    };
    let steam_id = steam_id64("123456789").expect("gültiger Freundescode");
    assert!(!store
        .handled_since("123456789", NOW)
        .await
        .expect("Abfrage läuft"));
    sqlx::query(
        "INSERT INTO steam.invite_requests (steam_id64, account_id, admin_id, created_at)
         VALUES ($1, 123456789, 7, $2)",
    )
    .bind(steam_id)
    .bind(NOW + 5)
    .execute(db.pool())
    .await
    .expect("Testzeile");
    assert!(store
        .handled_since("123456789", NOW)
        .await
        .expect("Abfrage läuft"));
    assert!(!store
        .handled_since("123456789", NOW + 10)
        .await
        .expect("Abfrage läuft"));
    sqlx::query(
        "INSERT INTO steam.bot_event_log (occurred_at, event_type, decision, steam_id)
         VALUES (to_timestamp($2), 'playtest_invite', 'sent', $1)",
    )
    .bind(steam_id)
    .bind((NOW + 20) as f64)
    .execute(db.pool())
    .await
    .expect("Testzeile");
    assert!(store
        .handled_since("123456789", NOW + 10)
        .await
        .expect("Abfrage läuft"));
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
            reposts: Vec::new(),
            earlier: Vec::new(),
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
async fn dank_des_mitglieds_schliesst_die_raumbitte_still() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::Arrived)]);
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
    for at in [NOW + CHECK_IN_SECONDS, NOW + ONE_HOUR] {
        watcher.poll_due(at).await.expect("Abfrage läuft");
    }
    assert!(invite.calls.lock().await.is_empty());
    assert!(notices(&brain).is_empty());
    let state = store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .expect("Bitte gespeichert");
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Attempted
    );
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

#[test]
fn wie_frage_und_ich_bitte_ohne_fragezeichen_sind_raumbitten() {
    for content in [
        "yo leute wie genau bekomm ich ein invite",
        "Freunde, ich benötige einen invite zum Playtest. Wäre jemand so nett?",
    ] {
        assert_eq!(
            request_kind(content, BOT),
            Some(RequestKind::Room),
            "{content}"
        );
    }
    assert_eq!(
        request_kind(
            "Du musst von jemand eingeladen werden der des Game hat",
            BOT
        ),
        None
    );
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
            reposts: Vec::new(),
            earlier: Vec::new(),
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
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag")
        .user_id;
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
            .handle_message(
                message(1, NOW - REQUEST_WINDOW_SECONDS - 1, content),
                NOW,
                Some(false),
            )
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
                    created_at: NOW - REQUEST_WINDOW_SECONDS - 1,
                    direct,
                    phase: Phase::Pending,
                    reposts: Vec::new(),
                    earlier: Vec::new(),
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
async fn raumbitte_mit_echtem_audit_meldet_nur_bereits_eingeladen() {
    let (watcher, store, port, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    invite.audit.store(true, Ordering::SeqCst);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    answer_no_after_check_in(&watcher).await;
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
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            InviteLoungeUpdate::Status {
                status: InviteStatus::AlreadyHasGame
            }
        ]
    );
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
    assert!(is_fresh(
        NOW - REQUEST_WINDOW_SECONDS,
        NOW,
        REQUEST_WINDOW_SECONDS
    ));
    assert!(!is_fresh(
        NOW - REQUEST_WINDOW_SECONDS - 1,
        NOW,
        REQUEST_WINDOW_SECONDS
    ));
    assert!(!is_fresh(NOW + 1, NOW, REQUEST_WINDOW_SECONDS));
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
            let due = NOW + if direct { 0 } else { CHECK_IN_SECONDS };
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
            assert_eq!(invite.calls.lock().await.len(), usize::from(direct));
            if !direct {
                let state = stored(&store).await;
                assert_eq!(state.status.notices, vec![Notice::CheckIn]);
                assert_eq!(state.outbox.len(), 1);
            }
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
            let max_age = REQUEST_WINDOW_SECONDS;
            store.states.lock().await.insert(
                USER,
                LoungeState {
                    friend_code: Some("123456789".into()),
                    request: Some(Request {
                        message_id: 1,
                        created_at: NOW - max_age,
                        direct,
                        phase: Phase::Pending,
                        reposts: Vec::new(),
                        earlier: Vec::new(),
                    }),
                    status: RequestStatus {
                        invite_now: !direct,
                        ..Default::default()
                    },
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
            if point == WaitPoint::Load {
                assert!(state.dispatch_started_at_millis.is_none());
            }
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
            message(1, NOW - REQUEST_WINDOW_SECONDS, "Kannst du mich einladen?"),
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
async fn gleicher_code_schiebt_die_nachfrage_nicht_auf() {
    let (watcher, _, _, invite, brain) = setup_scripted();
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
            message(2, NOW + 300, "Kann mich jemand einladen? 123456789"),
            NOW + 300,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn nicht_pruefbare_bitte_haelt_die_einladung_zurueck() {
    let (mut watcher, store, port, invite) = setup();
    let clock = moving_clock(&mut watcher);
    port.presence_fails.store(true, Ordering::SeqCst);
    direct_request(&watcher).await;
    assert!(invite.calls.lock().await.is_empty());
    let state = stored(&store).await;
    assert_eq!(state.status.check_at, Some(NOW + RETRY_SECONDS));
    assert_eq!(state.request.expect("Bitte bleibt").phase, Phase::Pending);
    watcher.poll_due(NOW + 60).await.expect("Abfrage läuft");
    assert!(invite.calls.lock().await.is_empty());
    port.presence_fails.store(false, Ordering::SeqCst);
    clock.store((NOW + RETRY_SECONDS) * 1000, Ordering::SeqCst);
    watcher
        .poll_due(NOW + RETRY_SECONDS)
        .await
        .expect("Abfrage läuft");
    watcher
        .poll_due(NOW + RETRY_SECONDS + 15)
        .await
        .expect("Abfrage läuft");
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn nein_in_bittenform_wird_als_antwort_gewertet() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    watcher
        .handle_message(
            message(2, at, "nein, kann mich jemand einladen?"),
            at,
            Some(false),
        )
        .await
        .expect("Antwort wird gelesen");
    assert_eq!(check_in_texts(&brain), ["nein, kann mich jemand einladen?"]);
    assert_eq!(
        stored(&store)
            .await
            .request
            .expect("Bitte bleibt")
            .message_id,
        1
    );
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn neu_gepostete_bitte_behaelt_die_nachfrage() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    port.deleted.lock().await.insert(1);
    watcher
        .handle_message(
            message(2, NOW + 300, "Kann mich jemand einladen? 123456789"),
            NOW + 300,
            Some(false),
        )
        .await
        .expect("Neue Bitte wird gelesen");
    let request = stored(&store).await.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 1);
    assert_eq!(request.reposts, vec![2]);
    for at in [NOW + CHECK_IN_SECONDS, NOW + CHECK_IN_SECONDS + 15] {
        watcher.poll_due(at).await.expect("Nachfrage läuft");
    }
    let request = stored(&store).await.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 2);
    assert_eq!(request.created_at, NOW);
    assert!(request.reposts.is_empty());
    assert_eq!(request.earlier, vec![1]);
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(notices(&brain).iter().all(|(_, notice)| notice.target
        == NoticeTarget::Reply {
            channel_id: CHECK_IN_CHANNEL_ID,
            message_id: 2
        }));
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn nein_nach_einladung_von_hand_sendet_keine_anfrage() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    store.handled.store(true, Ordering::SeqCst);
    let at = NOW + CHECK_IN_SECONDS + 60;
    watcher
        .handle_message(message(2, at, "nein"), at, Some(false))
        .await
        .expect("Nein wird gelesen");
    watcher.poll_due(at + 60).await.expect("Abfrage läuft");
    assert_eq!(check_in_texts(&brain), ["nein"]);
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
}

#[tokio::test]
async fn abgleich_findet_waehrend_der_pause_nachgereichten_code() {
    let (mut watcher, store, port, invite, _) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    port.edited
        .lock()
        .await
        .insert(1, "Kann mich jemand einladen? 123456789".into());
    let at = NOW + 600;
    clock.store(at * 1000, Ordering::SeqCst);
    assert_eq!(
        watcher
            .reread_waiting_requests(at)
            .await
            .expect("Abgleich läuft"),
        vec![USER]
    );
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("123456789"));
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Pending
    );
    assert!(watcher
        .reread_waiting_requests(at + 1)
        .await
        .expect("Abgleich läuft")
        .is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn geliehene_antwort_wird_nicht_doppelt_gewertet() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::Arrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let (started, release) = brain.gate_first_call();
    let at = NOW + CHECK_IN_SECONDS + 60;
    let watcher = Arc::new(watcher);
    let judging = watcher.clone();
    let task = tokio::spawn(async move {
        judging
            .handle_message(message(2, at, "ja"), at, Some(false))
            .await
    });
    started.notified().await;
    watcher.poll_due(at + 15).await.expect("Abfrage läuft");
    assert_eq!(check_in_texts(&brain), ["ja"]);
    release.notify_one();
    task.await
        .expect("Antwort-Task endet")
        .expect("Antwort wird gewertet");
    assert_eq!(check_in_texts(&brain), ["ja"]);
    assert!(stored(&store).await.status.held_answers.is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

fn moving_clock(watcher: &mut InviteLoungeWatcher) -> Arc<AtomicI64> {
    let clock = Arc::new(AtomicI64::new(NOW * 1000));
    let reader = clock.clone();
    watcher.clock_millis = Arc::new(move || reader.load(Ordering::SeqCst));
    clock
}

async fn direct_request(watcher: &InviteLoungeWatcher) {
    watcher
        .handle_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
}

async fn outbox(store: &MemoryStore) -> Vec<OutboxEntry> {
    store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .map(|state| state.outbox)
        .unwrap_or_default()
}

#[tokio::test]
async fn statuswechsel_geht_genau_einmal_mit_allen_fakten_an_den_brain() {
    let (watcher, store, port, _, brain) = setup_scripted();
    store.progress.lock().await.invited = true;
    direct_request(&watcher).await;
    watcher
        .poll_due(NOW + 600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_notices_request(&brain, 1);
    let (_, notice) = notices(&brain).remove(0);
    assert_eq!(notice.idempotency_key, "invite-lounge:1:invite");
    assert!(notice.already_invited);
    assert_eq!(notice.answer, None);
    assert!(outbox(&store).await.is_empty());
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn ohne_kanal_geht_die_meldung_als_dm() {
    let (watcher, store, _, _, brain) = setup_scripted();
    let state = LoungeState {
        outbox: vec![OutboxEntry::notice(1, None, Notice::Invite, false)],
        ..LoungeState::default()
    };
    assert!(store
        .compare_exchange(USER, None, &state)
        .await
        .expect("Zustand schreibbar"));
    watcher
        .deliver_outbox(USER)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    let notices = notices(&brain);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].1.target, NoticeTarget::DirectMessage);
    assert!(outbox(&store).await.is_empty());
}

#[tokio::test]
async fn fehlgeschlagene_meldung_bleibt_liegen_und_geht_genau_einmal_raus() {
    let (mut watcher, store, _, _, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    brain.script_notices(vec![Err(BrainError::Backend("Broker weg".into()))]);
    direct_request(&watcher).await;
    let queued = outbox(&store).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].attempts, 1);
    watcher
        .poll_due(NOW + 30)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(notices(&brain).len(), 1);
    clock.store((NOW + 61) * 1000, Ordering::SeqCst);
    for at in [NOW + 61, NOW + 3600] {
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    let keys: Vec<_> = notices(&brain)
        .into_iter()
        .map(|(_, notice)| notice.idempotency_key)
        .collect();
    assert_eq!(keys, ["invite-lounge:1:invite", "invite-lounge:1:invite"]);
    assert!(outbox(&store).await.is_empty());
}

#[tokio::test]
async fn abgelehnte_meldung_wird_verworfen() {
    let (mut watcher, store, _, _, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    brain.script_notices(vec![Ok(ConversationReply::Rejected)]);
    direct_request(&watcher).await;
    clock.store((NOW + 3600) * 1000, Ordering::SeqCst);
    watcher
        .poll_due(NOW + 3600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(notices(&brain).len(), 1);
    assert!(outbox(&store).await.is_empty());
}

#[tokio::test]
async fn bereits_zugestellte_antwort_wird_nicht_doppelt_gesendet() {
    let (mut watcher, store, port, _, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    brain.script_notices(vec![Err(BrainError::Backend("Zeitüberschreitung".into()))]);
    direct_request(&watcher).await;
    let mut delivered = message(5, NOW + 1, "<@7> Einladung ist raus.");
    delivered.author_id = BOT;
    delivered.is_bot = true;
    delivered.reply_message_id = Some(1);
    port.history.lock().await.extend([delivered]);
    clock.store((NOW + 120) * 1000, Ordering::SeqCst);
    watcher
        .poll_due(NOW + 120)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(notices(&brain).len(), 1);
    assert!(outbox(&store).await.is_empty());
}

#[tokio::test]
async fn nachfrage_auch_ausserhalb_von_deadlock_invite() {
    let (watcher, _, _, invite, brain) = setup_scripted();
    let mut request = message(1, NOW, "Kann mich jemand einladen? 123456789");
    request.channel_id = INVITE_LOUNGE_CHANNEL_ID;
    watcher
        .handle_message(request, NOW, Some(false))
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS + 60)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn nicht_verfuegbares_brain_verliert_keine_meldung() {
    let (mut watcher, store, _, _, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    let failures = (0..13)
        .map(|_| {
            Err(BrainError::Backend(
                "discord_invite_lounge unavailable".into(),
            ))
        })
        .collect();
    brain.script_notices(failures);
    direct_request(&watcher).await;
    for hour in 1..=12 {
        let at = NOW + hour * 3600;
        clock.store(at * 1000, Ordering::SeqCst);
        watcher
            .poll_due(at)
            .await
            .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    }
    assert_eq!(outbox(&store).await.len(), 1);
    clock.store((NOW + 13 * 3600) * 1000, Ordering::SeqCst);
    watcher
        .poll_due(NOW + 13 * 3600)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(notices(&brain).len(), 14);
    assert!(outbox(&store).await.is_empty());
}

#[tokio::test]
async fn fremde_bot_antwort_zaehlt_nicht_als_zugestellte_meldung() {
    let (mut watcher, store, port, _, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    brain.script_notices(vec![Err(BrainError::Backend("Zeitüberschreitung".into()))]);
    direct_request(&watcher).await;
    let mut hint = message(5, NOW + 1, "Den Freundescode findest du in Steam.");
    hint.author_id = BOT;
    hint.is_bot = true;
    hint.reply_message_id = Some(1);
    port.history.lock().await.extend([hint]);
    clock.store((NOW + 120) * 1000, Ordering::SeqCst);
    watcher
        .poll_due(NOW + 120)
        .await
        .expect("Testzustand und Testports erfüllen den erwarteten Vertrag");
    assert_eq!(notices(&brain).len(), 2);
    assert!(outbox(&store).await.is_empty());
}

async fn answer_no_after_check_in(watcher: &InviteLoungeWatcher) {
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    watcher
        .handle_message(message(9, at, "nein, noch nichts"), at, Some(false))
        .await
        .expect("Antwort wird gelesen");
}

fn open_request_state(phase: Phase, wait: Option<SteamWait>) -> LoungeState {
    LoungeState {
        friend_code: Some("123456789".into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW,
            direct: false,
            phase,
            reposts: Vec::new(),
            earlier: Vec::new(),
        }),
        status: RequestStatus {
            channel_id: Some(CHECK_IN_CHANNEL_ID),
            notices: vec![Notice::CheckIn, Notice::FriendRequest],
            wait,
            check_at: wait.map(|_| NOW + 2 * ONE_HOUR),
            check_in_at: Some(NOW + CHECK_IN_SECONDS),
            ..Default::default()
        },
        last_seen: 1,
        dispatch_started_at_millis: Some((NOW + ONE_HOUR) * 1000),
        ..Default::default()
    }
}

async fn stored(store: &MemoryStore) -> LoungeState {
    store
        .load(USER)
        .await
        .expect("Zustand lesbar")
        .expect("Zustand gespeichert")
}

#[tokio::test]
async fn spaetes_ja_schliesst_die_bitte_in_jedem_offenen_zustand() {
    for (phase, wait) in [
        (Phase::Pending, Some(SteamWait::Delivery)),
        (Phase::Pending, Some(SteamWait::Retry)),
        (Phase::Pending, None),
        (Phase::Dispatching, None),
    ] {
        let (watcher, store, _, invite, brain) =
            setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::Arrived)]);
        let mut state = open_request_state(phase, wait);
        state.outbox.push(OutboxEntry::notice(
            1,
            Some(CHECK_IN_CHANNEL_ID),
            Notice::NotAccepted,
            false,
        ));
        store.states.lock().await.insert(USER, state);
        let at = NOW + ONE_HOUR + 60;
        watcher
            .handle_message(message(5, at, "Jo habe ich schon !"), at, Some(false))
            .await
            .expect("Späte Antwort wird gelesen");
        watcher
            .poll_due(NOW + 3 * ONE_HOUR)
            .await
            .expect("Abfrage läuft");
        let state = stored(&store).await;
        assert_eq!(
            state.request.expect("Bitte vorhanden").phase,
            Phase::Attempted
        );
        assert!(state.status.wait.is_none());
        assert!(state.status.check_at.is_none());
        assert!(state.outbox.is_empty());
        assert_eq!(check_in_texts(&brain), ["Jo habe ich schon !"]);
        assert_eq!(
            status_updates(&brain),
            vec![answer_update(InviteCheckInOutcome::Arrived)]
        );
        assert!(invite.calls.lock().await.is_empty());
    }
}

#[tokio::test]
async fn geloeschte_bitte_bekommt_weder_nachfrage_noch_einladung() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    port.deleted.lock().await.insert(1);
    for at in [NOW + CHECK_IN_SECONDS, NOW + ONE_HOUR] {
        watcher.poll_due(at).await.expect("Abfrage läuft");
    }
    assert!(invite.calls.lock().await.is_empty());
    assert!(notices(&brain).is_empty());
    let state = stored(&store).await;
    assert!(state.request.is_none());
    assert_eq!(
        state.last_result.as_deref(),
        Some("Bitte gelöscht; still beendet")
    );
    let at = NOW + ONE_HOUR + 10;
    watcher
        .handle_message(
            message(3, at, "Kann mich jemand einladen? 123456789"),
            at,
            Some(false),
        )
        .await
        .expect("Neue Bitte wird angelegt");
    let request = stored(&store).await.request.expect("Neue Bitte vorhanden");
    assert_eq!(request.message_id, 3);
    assert_eq!(request.phase, Phase::Pending);
}

#[tokio::test]
async fn geloeschte_bitte_im_steam_warten_endet_still() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    let mut state = open_request_state(Phase::Pending, Some(SteamWait::Delivery));
    state.outbox.push(OutboxEntry::notice(
        1,
        Some(CHECK_IN_CHANNEL_ID),
        Notice::NotAccepted,
        false,
    ));
    state.outbox[0].next_at_millis = (NOW + 3 * ONE_HOUR) * 1000;
    store.states.lock().await.insert(USER, state);
    port.deleted.lock().await.insert(1);
    watcher
        .poll_due(NOW + 2 * ONE_HOUR)
        .await
        .expect("Abfrage läuft");
    assert!(invite.calls.lock().await.is_empty());
    assert!(notices(&brain).is_empty());
    let state = stored(&store).await;
    assert!(state.request.is_none());
    assert!(state.outbox.is_empty());
    assert!(store
        .pending_users()
        .await
        .expect("Offene Bitten lesbar")
        .is_empty());
}

#[tokio::test]
async fn meldung_zu_geloeschter_bitte_wird_still_verworfen() {
    let (mut watcher, store, port, _, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    brain.script_notices(vec![Err(BrainError::Backend("Broker weg".into()))]);
    direct_request(&watcher).await;
    assert_eq!(outbox(&store).await.len(), 1);
    port.deleted.lock().await.insert(1);
    clock.store((NOW + 61) * 1000, Ordering::SeqCst);
    watcher.poll_due(NOW + 61).await.expect("Abfrage läuft");
    assert_eq!(notices(&brain).len(), 1);
    let state = stored(&store).await;
    assert!(state.outbox.is_empty());
    assert!(state.request.is_none());
}

#[tokio::test]
async fn nicht_pruefbare_bitte_haelt_die_nachfrage_bis_discord_antwortet() {
    let (mut watcher, store, port, invite, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    port.presence_fails.store(true, Ordering::SeqCst);
    clock.store((NOW + CHECK_IN_SECONDS) * 1000, Ordering::SeqCst);
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Prüffehler hält den Takt nicht an");
    assert!(status_updates(&brain).is_empty());
    let state = stored(&store).await;
    assert_eq!(state.outbox.len(), 1);
    assert!(state.request.is_some());
    port.presence_fails.store(false, Ordering::SeqCst);
    let later = NOW + CHECK_IN_SECONDS + brain_backoff(1) / 1000;
    clock.store(later * 1000, Ordering::SeqCst);
    watcher.poll_due(later).await.expect("Abfrage läuft");
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn antwort_ohne_brain_urteil_wird_spaeter_erneut_gewertet() {
    let (mut watcher, store, _, invite, brain) = setup_check_ins(vec![
        Err(BrainError::Backend("Brain weg".into())),
        check_in_reply(InviteCheckInOutcome::Arrived),
    ]);
    let clock = moving_clock(&mut watcher);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    let at = NOW + CHECK_IN_SECONDS + 120;
    clock.store(at * 1000, Ordering::SeqCst);
    watcher
        .handle_message(message(2, at, "hab ich schon"), at, Some(false))
        .await
        .expect("Antwort wird gehalten");
    let retry_at = at * 1000 + brain_backoff(1);
    assert_eq!(
        stored(&store).await.status.held_answers,
        vec![HeldAnswer {
            message_id: 2,
            content: "hab ich schon".into(),
            retry_at_millis: retry_at,
            attempts: 1,
        }]
    );
    clock.store(retry_at - 1, Ordering::SeqCst);
    watcher.poll_due(at + 30).await.expect("Abfrage läuft");
    assert_eq!(check_in_texts(&brain), ["hab ich schon"]);
    assert!(status_updates(&brain).is_empty());
    clock.store(retry_at, Ordering::SeqCst);
    watcher.poll_due(at + 61).await.expect("Abfrage läuft");
    assert_eq!(check_in_texts(&brain), ["hab ich schon", "hab ich schon"]);
    let state = stored(&store).await;
    assert!(state.status.held_answers.is_empty());
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Attempted
    );
    assert!(notices(&brain).is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn bearbeitete_bitte_uebernimmt_den_nachgereichten_code() {
    let (mut watcher, store, _, invite, brain) = setup_scripted();
    let clock = moving_clock(&mut watcher);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen?"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    assert_eq!(
        stored(&store).await.request.expect("Bitte vorhanden").phase,
        Phase::WaitingForCode
    );
    let edited_at = NOW + 600;
    clock.store(edited_at * 1000, Ordering::SeqCst);
    let edit = dl_discord::MessageEdit {
        guild_id: Some(GUILD),
        channel_id: CHECK_IN_CHANNEL_ID,
        message_id: 1,
        author_id: USER,
        content: "Kann mich jemand einladen? 123456789".into(),
    };
    assert_eq!(
        watcher
            .record_edit(&edit)
            .await
            .expect("Bearbeitung lesbar"),
        Some(USER)
    );
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("123456789"));
    let request = state.request.expect("Bitte vorhanden");
    assert_eq!(request.phase, Phase::Pending);
    assert_eq!(request.created_at, edited_at);
    watcher
        .poll_due(edited_at + CHECK_IN_SECONDS)
        .await
        .expect("Abfrage läuft");
    assert_eq!(status_updates(&brain), vec![InviteLoungeUpdate::CheckIn]);
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn spaetes_ja_waehrend_des_versands_bleibt_geschlossen() {
    let (watcher, store, _, invite, brain) = setup_check_ins(vec![
        check_in_reply(InviteCheckInOutcome::NotArrived),
        check_in_reply(InviteCheckInOutcome::Arrived),
    ]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    watcher
        .record_message(message(2, at, "nein"), at, Some(false))
        .await
        .expect("Nein wird gelesen");
    invite.hold.store(true, Ordering::SeqCst);
    let watcher = Arc::new(watcher);
    let dispatching = watcher.clone();
    let task = tokio::spawn(async move { dispatching.dispatch_due(USER, at).await });
    invite.started.notified().await;
    assert_eq!(
        stored(&store).await.request.expect("Bitte vorhanden").phase,
        Phase::Dispatching
    );
    watcher
        .handle_message(message(3, at + 5, "doch, ja hab ich"), at + 5, Some(false))
        .await
        .expect("Späte Antwort wird gelesen");
    invite.release.notify_one();
    task.await
        .expect("Versand-Task endet")
        .expect("Versand endet ohne Fehler");
    watcher.poll_due(at + 60).await.expect("Abfrage läuft");
    let state = stored(&store).await;
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Attempted
    );
    assert!(state.status.wait.is_none());
    assert!(state.outbox.is_empty());
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert_eq!(
        status_updates(&brain),
        vec![
            InviteLoungeUpdate::CheckIn,
            answer_update(InviteCheckInOutcome::Arrived)
        ]
    );
}

#[tokio::test]
async fn nein_mit_dem_freundescode_loest_die_einladung_aus() {
    let (watcher, _, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    watcher
        .handle_message(
            message(2, at, "nein, mein Code ist 123456789"),
            at,
            Some(false),
        )
        .await
        .expect("Antwort wird gelesen");
    assert_eq!(check_in_texts(&brain), ["nein, mein Code ist 123456789"]);
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn gehaltene_antwort_geht_bei_einer_folgenachricht_nicht_verloren() {
    let (mut watcher, store, _, invite, brain) = setup_check_ins(vec![
        Err(BrainError::Backend("Brain weg".into())),
        check_in_reply(InviteCheckInOutcome::NotArrived),
        check_in_reply(InviteCheckInOutcome::Unclear),
    ]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    let clock = moving_clock(&mut watcher);
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    clock.store(at * 1000, Ordering::SeqCst);
    watcher
        .handle_message(message(2, at, "nein, hab ich nicht"), at, Some(false))
        .await
        .expect("Antwort wird gehalten");
    assert_eq!(stored(&store).await.status.held_answers.len(), 1);
    assert!(invite.calls.lock().await.is_empty());
    watcher
        .handle_message(message(3, at + 5, "hallo?"), at + 5, Some(false))
        .await
        .expect("Folgenachricht wird gelesen");
    assert_eq!(check_in_texts(&brain), ["nein, hab ich nicht"]);
    assert_eq!(stored(&store).await.status.held_answers.len(), 2);
    let later = at + brain_backoff(1) / 1000;
    clock.store(later * 1000, Ordering::SeqCst);
    watcher.poll_due(later).await.expect("Abfrage läuft");
    assert_eq!(
        check_in_texts(&brain),
        ["nein, hab ich nicht", "nein, hab ich nicht", "hallo?"]
    );
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert!(stored(&store).await.status.held_answers.is_empty());
}

#[tokio::test]
async fn veraltete_bitte_im_versand_nimmt_keine_antwort_mehr_an() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::Arrived)]);
    store
        .states
        .lock()
        .await
        .insert(USER, open_request_state(Phase::Dispatching, None));
    let at = NOW + REQUEST_WINDOW_SECONDS + 10;
    watcher
        .handle_message(message(5, at, "hab ich schon"), at, Some(false))
        .await
        .expect("Nachricht wird gelesen");
    assert!(check_in_texts(&brain).is_empty());
    assert!(stored(&store).await.status.held_answers.is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn gehaltene_antwort_im_versand_wird_beim_naechsten_takt_gewertet() {
    let (mut watcher, store, _, invite, brain) = setup_check_ins(vec![
        Err(BrainError::Backend("Brain weg".into())),
        check_in_reply(InviteCheckInOutcome::Arrived),
    ]);
    let clock = moving_clock(&mut watcher);
    store
        .states
        .lock()
        .await
        .insert(USER, open_request_state(Phase::Dispatching, None));
    let at = NOW + ONE_HOUR + 60;
    clock.store(at * 1000, Ordering::SeqCst);
    watcher
        .handle_message(message(5, at, "hab ich schon"), at, Some(false))
        .await
        .expect("Antwort wird gehalten");
    assert_eq!(stored(&store).await.status.held_answers.len(), 1);
    clock.store(at * 1000 + brain_backoff(1), Ordering::SeqCst);
    watcher.poll_due(at + 61).await.expect("Abfrage läuft");
    assert_eq!(check_in_texts(&brain), ["hab ich schon", "hab ich schon"]);
    let state = stored(&store).await;
    assert!(state.status.held_answers.is_empty());
    assert_eq!(
        state.request.expect("Bitte vorhanden").phase,
        Phase::Attempted
    );
    assert!(invite.calls.lock().await.is_empty());
}

#[test]
fn volle_antwortschlange_nimmt_keine_weitere_antwort_auf() {
    let mut state = open_request_state(Phase::Pending, None);
    for message_id in 10..10 + ANSWER_QUEUE_LIMIT as u64 {
        queue_held_answer(
            &mut state,
            USER,
            HeldAnswer::fresh(message_id, "nein".into()),
        );
    }
    assert_eq!(state.status.held_answers.len(), ANSWER_QUEUE_LIMIT);
    let overflow = 10 + ANSWER_QUEUE_LIMIT as u64;
    let (capture, guard) = capture_logs();
    queue_held_answer(
        &mut state,
        USER,
        HeldAnswer::fresh(overflow, "hallo?".into()),
    );
    drop(guard);
    assert!(capture.text().contains(
        "Invite-Lounge verwirft Antwort, weil schon zu viele Antworten auf ein Brain-Urteil warten"
    ));
    assert_eq!(state.status.held_answers.len(), ANSWER_QUEUE_LIMIT);
    assert!(state
        .status
        .held_answers
        .iter()
        .all(|held| held.message_id != overflow));
    assert_eq!(state.last_seen, overflow);
}

#[test]
fn neuer_code_in_der_antwort_braucht_platz_in_der_schlange() {
    let mut state = open_request_state(Phase::Pending, None);
    state.dispatch_started_at_millis = None;
    assert!(takes_answer_code(&state, 99));
    for message_id in 10..10 + ANSWER_QUEUE_LIMIT as u64 {
        queue_held_answer(
            &mut state,
            USER,
            HeldAnswer::fresh(message_id, "nein".into()),
        );
    }
    assert!(!takes_answer_code(&state, 99));
    assert!(takes_answer_code(&state, 10));
}

#[tokio::test]
async fn geloeschte_direkte_bitte_bekommt_keine_einladung() {
    let (watcher, store, port, invite) = setup();
    port.deleted.lock().await.insert(1);
    direct_request(&watcher).await;
    assert!(invite.calls.lock().await.is_empty());
    let state = stored(&store).await;
    assert!(state.request.is_none());
    assert_eq!(
        state.last_result.as_deref(),
        Some("Bitte gelöscht; still beendet")
    );
}

#[tokio::test]
async fn nein_in_bittenform_mit_gleichem_code_loest_die_einladung_aus() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    let text = "Nein, kann mich jemand einladen? 123456789";
    watcher
        .handle_message(message(2, at, text), at, Some(false))
        .await
        .expect("Antwort wird gelesen");
    assert_eq!(check_in_texts(&brain), [text]);
    let calls = invite.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].2, "123456789");
    assert_eq!(
        stored(&store)
            .await
            .request
            .expect("Bitte bleibt")
            .message_id,
        1
    );
}

#[tokio::test]
async fn neuer_code_in_der_antwort_geht_an_den_neuen_code() {
    let (watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    let (capture, guard) = capture_logs();
    watcher
        .handle_message(
            message(2, at, "nein, mein Code ist 987654321"),
            at,
            Some(false),
        )
        .await
        .expect("Antwort wird gelesen");
    drop(guard);
    assert!(capture
        .text()
        .contains("Invite-Lounge übernimmt neuen Freundescode aus der Antwort"));
    assert_eq!(check_in_texts(&brain), ["nein, mein Code ist 987654321"]);
    let calls = invite.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].2, "987654321");
    assert_eq!(
        stored(&store).await.friend_code.as_deref(),
        Some("987654321")
    );
}

#[tokio::test]
async fn neuer_code_nach_der_einladung_startet_die_bitte_neu() {
    let (watcher, store, _, _) = setup();
    store.states.lock().await.insert(
        USER,
        open_request_state(Phase::Pending, Some(SteamWait::Delivery)),
    );
    let at = NOW + 2 * ONE_HOUR;
    let (capture, guard) = capture_logs();
    let recorded = watcher
        .record_message(
            message(2, at, "nein, mein Code ist 987654321"),
            at,
            Some(false),
        )
        .await
        .expect("Nachricht wird gelesen");
    drop(guard);
    assert_eq!(recorded.map(|recorded| recorded.user_id), Some(USER));
    assert!(capture.text().contains("reason=\"neuer Freundescode\""));
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("987654321"));
    let request = state.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 2);
    assert_eq!(request.created_at, at);
    assert_eq!(request.phase, Phase::Pending);
    assert!(state.status.wait.is_none());
    assert!(!state.status.asked());
    assert!(state.status.held_answers.is_empty());
}

#[tokio::test]
async fn neuer_code_vor_der_nachfrage_startet_die_bitte_neu() {
    let (watcher, store, _, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    let (capture, guard) = capture_logs();
    watcher
        .handle_message(
            message(2, NOW + 300, "mein Code ist 987654321"),
            NOW + 300,
            Some(false),
        )
        .await
        .expect("Neuer Code wird gelesen");
    drop(guard);
    assert!(capture.text().contains("reason=\"neuer Freundescode\""));
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("987654321"));
    let request = state.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 2);
    assert_eq!(request.created_at, NOW + 300);
    assert_eq!(request.phase, Phase::Pending);
    assert!(state.status.held_answers.is_empty());
    assert!(check_in_texts(&brain).is_empty());
    assert!(invite.calls.lock().await.is_empty());
}

#[tokio::test]
async fn doppelte_bitte_behaelt_den_anker_und_hilfe_von_hand_zaehlt() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .handle_message(
            message(2, NOW + 300, "Kann mich jemand einladen? 123456789"),
            NOW + 300,
            Some(false),
        )
        .await
        .expect("Doppelte Bitte wird gelesen");
    let request = stored(&store).await.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 1);
    assert_eq!(request.reposts, vec![2]);
    let mut staff = message(3, NOW + 400, "schick ich dir gleich");
    staff.author_id = 8;
    staff.reply_message_id = Some(1);
    *port.history.lock().await = vec![
        message(1, NOW, "Kann mich jemand einladen? 123456789"),
        message(2, NOW + 300, "Kann mich jemand einladen? 123456789"),
        staff,
    ];
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Abfrage läuft");
    assert!(notices(&brain).is_empty());
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        stored(&store).await.request.expect("Bitte bleibt").phase,
        Phase::Attempted
    );
}

#[tokio::test]
async fn gehaltenes_nein_folgt_der_neu_geposteten_bitte() {
    let (mut watcher, store, port, invite, brain) = setup_check_ins(vec![
        Err(BrainError::Backend("scripted".into())),
        check_in_reply(InviteCheckInOutcome::NotArrived),
    ]);
    let clock = moving_clock(&mut watcher);
    brain.script_notices(vec![Err(BrainError::Backend("Broker weg".into()))]);
    let repost = "Kann mich jemand einladen? 123456789";
    watcher
        .handle_message(message(1, NOW, repost), NOW, Some(false))
        .await
        .expect("Bitte wird angelegt");
    let asked = NOW + CHECK_IN_SECONDS;
    clock.store(asked * 1000, Ordering::SeqCst);
    watcher.poll_due(asked).await.expect("Nachfrage läuft");
    assert_eq!(notices(&brain).len(), 1);
    let answered = asked + 10;
    clock.store(answered * 1000, Ordering::SeqCst);
    watcher
        .handle_message(message(2, answered, "nein"), answered, Some(false))
        .await
        .expect("Antwort wird gehalten");
    assert_eq!(stored(&store).await.status.held_answers.len(), 1);
    port.deleted.lock().await.insert(1);
    let reposted = answered + 5;
    clock.store(reposted * 1000, Ordering::SeqCst);
    watcher
        .handle_message(message(3, reposted, repost), reposted, Some(false))
        .await
        .expect("Neu gepostete Bitte wird gelesen");
    let request = stored(&store).await.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 1);
    assert_eq!(request.reposts, vec![3]);
    let retried = asked + 60;
    clock.store(retried * 1000, Ordering::SeqCst);
    watcher.poll_due(retried).await.expect("Meldung läuft");
    let request = stored(&store).await.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 3);
    assert_eq!(
        notices(&brain).last().map(|(_, notice)| notice.target),
        Some(NoticeTarget::Reply {
            channel_id: CHECK_IN_CHANNEL_ID,
            message_id: 3
        })
    );
    let judged = answered + brain_backoff(1) / 1000;
    clock.store(judged * 1000, Ordering::SeqCst);
    watcher
        .poll_due(judged)
        .await
        .expect("Antwort wird gewertet");
    assert_eq!(check_in_texts(&brain), ["nein", "nein", repost]);
    let calls = invite.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].2, "123456789");
}

fn claimed_notice(state: &mut LoungeState, message_id: u64) -> OutboxEntry {
    let mut entry = OutboxEntry::notice(
        message_id,
        Some(CHECK_IN_CHANNEL_ID),
        Notice::CheckIn,
        false,
    );
    entry.claim(NOW * 1000);
    state.outbox = vec![entry.clone()];
    entry
}

#[test]
fn meldung_zur_geloeschten_bitte_folgt_der_neu_geposteten() {
    let mut state = open_request_state(Phase::Pending, None);
    if let Some(request) = state.request.as_mut() {
        request.reposts = vec![5];
    }
    let claimed = claimed_notice(&mut state, 1);
    let outcome = settle_notice(&mut state, &claimed, NoticeResult::Gone, NOW * 1000);
    assert_eq!(outcome, Some(GoneOutcome::Followed { old: 1, new: 5 }));
    let request = state.request.clone().expect("Bitte bleibt");
    assert_eq!(request.message_id, 5);
    assert_eq!(request.created_at, NOW);
    assert!(request.reposts.is_empty());
    assert_eq!(request.earlier, vec![1]);
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.outbox[0].request_message_id, 5);
    assert_eq!(state.outbox[0].claimed_at_millis, None);

    let mut moved = open_request_state(Phase::Pending, None);
    if let Some(request) = moved.request.as_mut() {
        request.message_id = 5;
    }
    let stale = claimed_notice(&mut moved, 1);
    let before = moved.request.clone();
    let outcome = settle_notice(&mut moved, &stale, NoticeResult::Gone, NOW * 1000);
    assert_eq!(outcome, Some(GoneOutcome::Unchanged));
    assert_eq!(moved.request, before);
    assert!(moved.outbox.is_empty());
}

#[test]
fn geloeschte_bitte_im_versand_folgt_der_neu_geposteten() {
    let mut state = open_request_state(Phase::Dispatching, None);
    if let Some(request) = state.request.as_mut() {
        request.reposts = vec![5];
    }
    let claimed = claimed_notice(&mut state, 1);
    let outcome = settle_notice(&mut state, &claimed, NoticeResult::Gone, NOW * 1000);
    assert_eq!(outcome, Some(GoneOutcome::Followed { old: 1, new: 5 }));
    let request = state.request.clone().expect("Bitte bleibt");
    assert_eq!(request.message_id, 5);
    assert_eq!(request.phase, Phase::Dispatching);
    assert_eq!(request.earlier, vec![1]);
    assert_eq!(state.outbox.len(), 1);
    assert_eq!(state.outbox[0].request_message_id, 5);
}

#[tokio::test]
async fn fehlgeschlagene_hinweis_antwort_gibt_den_hinweis_frei() {
    let (watcher, store, port, _) = setup();
    port.reply_fails.store(true, Ordering::SeqCst);
    watcher
        .handle_message(message(1, NOW, IMPERSONAL_REQUEST), NOW, None)
        .await
        .expect("Antwortfehler ist kein Verarbeitungsfehler");
    let state = stored(&store).await;
    assert!(!state.hint_sent);
    assert_eq!(state.last_hint_at, None);
    assert!(port.replies.lock().await.is_empty());
}

#[tokio::test]
async fn haengendes_brain_urteil_endet_nach_der_frist_und_bleibt_gehalten() {
    let (mut watcher, store, _, invite, brain) =
        setup_check_ins(vec![check_in_reply(InviteCheckInOutcome::NotArrived)]);
    watcher.cutoff = Duration::from_millis(50);
    let clock = moving_clock(&mut watcher);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    let at = NOW + CHECK_IN_SECONDS + 60;
    clock.store(at * 1000, Ordering::SeqCst);
    let _gate = brain.gate_first_call();
    let (capture, guard) = capture_logs();
    watcher
        .handle_message(message(2, at, "nein"), at, Some(false))
        .await
        .expect("Antwort wird gelesen");
    drop(guard);
    assert!(capture
        .text()
        .contains("Invite-Lounge bricht das Brain-Urteil nach der Frist ab"));
    let held = stored(&store).await.status.held_answers;
    assert_eq!(held.len(), 1);
    assert_eq!(held[0].attempts, 1);
    assert!(invite.calls.lock().await.is_empty());
    let later = at + brain_backoff(1) / 1000;
    clock.store(later * 1000, Ordering::SeqCst);
    watcher.poll_due(later).await.expect("Abfrage läuft");
    watcher.poll_due(later + 15).await.expect("Abfrage läuft");
    assert_eq!(check_in_texts(&brain), ["nein", "nein"]);
    assert_eq!(invite.calls.lock().await.len(), 1);
}

#[tokio::test]
async fn haengende_meldung_endet_nach_der_frist_und_bleibt_in_der_ablage() {
    let (mut watcher, store, _, _, brain) = setup_scripted();
    watcher.cutoff = Duration::from_millis(50);
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    let _gate = brain.gate_first_call();
    let (capture, guard) = capture_logs();
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Nachfrage läuft");
    drop(guard);
    assert!(capture
        .text()
        .contains("Invite-Lounge bricht die Meldung nach der Frist ab"));
    let queued = outbox(&store).await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].attempts, 1);
}

#[test]
fn laufender_versand_bleibt_ueber_das_steam_zeitlimit_hinaus_stehen() {
    let started_millis = (NOW + ONE_HOUR) * 1000;
    for (elapsed_millis, replaced) in [
        (
            dl_bridges::steam::INVITE_FORWARD_TIMEOUT_SECS as i64 * 1000 + 30_000,
            false,
        ),
        (ACTIVE_DISPATCH_MILLIS, true),
    ] {
        let mut state = open_request_state(Phase::Dispatching, None);
        let now_millis = started_millis + elapsed_millis;
        let event = message(5, now_millis / 1000, "Kannst du mich einladen? 123456789");
        update_state(
            &mut state,
            &event,
            Some(RequestKind::Direct),
            Some("123456789".into()),
            false,
            now_millis / 1000,
            now_millis,
        );
        let request = state.request.expect("Bitte bleibt");
        assert_eq!(request.message_id == 5, replaced, "{elapsed_millis}");
    }
}

#[cfg(feature = "testing")]
mod pg_store {
    use super::*;
    use dl_central_db::testing::test_pool;

    const LEGACY_FIELD: &str = "\"check_in_message_id\":42,";

    fn legacy_text(canonical: &str, check_in_at: i64) -> String {
        let anchor = format!("\"check_in_at\":{check_in_at},");
        let end = canonical
            .find(&anchor)
            .expect("check_in_at steht vor einem weiteren Feld")
            + anchor.len();
        format!("{}{LEGACY_FIELD}{}", &canonical[..end], &canonical[end..])
    }

    #[tokio::test]
    async fn altzeile_wird_beim_laden_umgeschrieben_und_bleibt_schreibbar() {
        let db = test_pool().await.expect("Testdatenbank");
        let store = PgLoungeStore {
            pool: db.pool().clone(),
        };
        let mut state = open_request_state(Phase::Pending, None);
        state.status.invite_now = true;
        let canonical = serde_json::to_string(&state).expect("Zustand serialisierbar");
        let legacy = legacy_text(&canonical, NOW + CHECK_IN_SECONDS);
        assert_ne!(legacy, canonical);
        assert_eq!(legacy.replacen(LEGACY_FIELD, "", 1), canonical);
        let key = cooldown_key(USER);
        kv::set(db.pool(), STATE_KV_NS, &key, &legacy)
            .await
            .expect("Altzeile geschrieben");
        let loaded = store
            .load(USER)
            .await
            .expect("Zustand lesbar")
            .expect("Zustand vorhanden");
        let stored = kv::get(db.pool(), STATE_KV_NS, &key)
            .await
            .expect("Zeile lesbar")
            .expect("Zeile vorhanden");
        assert_eq!(loaded, state);
        assert_eq!(stored, canonical);
        let mut next = loaded.clone();
        next.last_seen = 7;
        assert!(store
            .compare_exchange(USER, Some(&loaded), &next)
            .await
            .expect("Schreiben läuft"));
    }

    #[tokio::test]
    async fn gehaltene_antwort_im_versand_zaehlt_als_offen() {
        let db = test_pool().await.expect("Testdatenbank");
        let store = PgLoungeStore {
            pool: db.pool().clone(),
        };
        let quiet = open_request_state(Phase::Dispatching, None);
        let mut held = open_request_state(Phase::Dispatching, None);
        held.status.held_answers = vec![HeldAnswer::fresh(5, "hab ich schon".into())];
        let mut waiting = open_request_state(Phase::WaitingForCode, None);
        waiting.friend_code = None;
        let mut stale = waiting.clone();
        if let Some(request) = stale.request.as_mut() {
            request.created_at = NOW - REQUEST_WINDOW_SECONDS - 1;
        }
        for (user_id, state) in [
            (USER, &held),
            (USER + 1, &quiet),
            (USER + 2, &waiting),
            (USER + 3, &stale),
        ] {
            assert!(store
                .compare_exchange(user_id, None, state)
                .await
                .expect("Zustand geschrieben"));
        }
        assert_eq!(
            store.pending_users().await.expect("Offene lesbar"),
            vec![USER]
        );
        assert_eq!(
            store
                .waiting_users(NOW - REQUEST_WINDOW_SECONDS)
                .await
                .expect("Wartende lesbar"),
            vec![USER + 2]
        );
    }
}

#[tokio::test]
async fn neuer_code_waehrend_des_versands_startet_die_bitte_neu() {
    let (watcher, store, _, invite, _) = setup_scripted();
    watcher
        .record_message(
            message(1, NOW, "Kannst du mich einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    invite.hold.store(true, Ordering::SeqCst);
    let watcher = Arc::new(watcher);
    let dispatching = watcher.clone();
    let task = tokio::spawn(async move { dispatching.dispatch_due(USER, NOW).await });
    invite.started.notified().await;
    watcher
        .record_message(
            message(2, NOW + 5, "Kannst du mich einladen? 987654321"),
            NOW + 5,
            Some(false),
        )
        .await
        .expect("Neuer Code wird gelesen");
    let state = stored(&store).await;
    let request = state.request.expect("Bitte vorhanden");
    assert_eq!(request.message_id, 2);
    assert_eq!(request.phase, Phase::Pending);
    assert!(request.direct);
    assert_eq!(state.friend_code.as_deref(), Some("987654321"));
    assert_eq!(state.dispatch_started_at_millis, None);
    invite.hold.store(false, Ordering::SeqCst);
    invite.release.notify_one();
    task.await
        .expect("Versand-Task endet")
        .expect("Versand endet ohne Fehler");
    let state = stored(&store).await;
    assert_eq!(
        state.request.as_ref().map(|request| request.phase),
        Some(Phase::Pending)
    );
    assert!(state.outbox.is_empty());
    watcher
        .dispatch_due(USER, NOW + 20)
        .await
        .expect("Neuer Versand läuft");
    let codes: Vec<String> = invite
        .calls
        .lock()
        .await
        .iter()
        .map(|call| call.2.clone())
        .collect();
    assert_eq!(codes, ["123456789", "987654321"]);
}

#[tokio::test]
async fn neuer_code_bei_direkter_bitte_im_steam_warten_geht_an_den_neuen_code() {
    let (watcher, store, _, invite) = setup();
    let mut state = open_request_state(Phase::Pending, Some(SteamWait::Delivery));
    if let Some(request) = state.request.as_mut() {
        request.direct = true;
    }
    store.states.lock().await.insert(USER, state);
    let at = NOW + 2 * ONE_HOUR;
    watcher
        .handle_message(message(2, at, "mein Code ist 987654321"), at, Some(false))
        .await
        .expect("Neuer Code wird gelesen");
    let codes: Vec<String> = invite
        .calls
        .lock()
        .await
        .iter()
        .map(|call| call.2.clone())
        .collect();
    assert_eq!(codes, ["987654321"]);
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("987654321"));
    let request = state.request.expect("Bitte vorhanden");
    assert_eq!(request.message_id, 2);
    assert!(request.direct);
}

#[tokio::test]
async fn hilfe_an_einer_kopie_der_bitte_zaehlt() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .expect("Bitte wird angelegt");
    watcher
        .handle_message(
            message(2, NOW + 300, "Kann mich jemand einladen? 123456789"),
            NOW + 300,
            Some(false),
        )
        .await
        .expect("Kopie wird gelesen");
    let mut staff = message(3, NOW + 400, "schick ich dir gleich");
    staff.author_id = 8;
    staff.reply_message_id = Some(2);
    *port.history.lock().await = vec![
        message(1, NOW, "Kann mich jemand einladen? 123456789"),
        message(2, NOW + 300, "Kann mich jemand einladen? 123456789"),
        staff,
    ];
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Abfrage läuft");
    assert!(notices(&brain).is_empty());
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        stored(&store).await.request.expect("Bitte bleibt").phase,
        Phase::Attempted
    );
}

#[tokio::test]
async fn hilfe_an_der_geloeschten_ersten_bitte_zaehlt_nach_dem_folgen() {
    let (watcher, store, port, invite, brain) = setup_scripted();
    let mut state = open_request_state(Phase::Pending, None);
    state.status.notices.clear();
    state.status.check_in_at = None;
    state.dispatch_started_at_millis = None;
    if let Some(request) = state.request.as_mut() {
        request.message_id = 5;
        request.earlier = vec![1];
    }
    store.states.lock().await.insert(USER, state);
    let mut staff = message(6, NOW + 400, "schick ich dir gleich");
    staff.author_id = 8;
    staff.reply_message_id = Some(1);
    *port.history.lock().await = vec![
        message(5, NOW + 300, "Kann mich jemand einladen? 123456789"),
        staff,
    ];
    watcher
        .poll_due(NOW + CHECK_IN_SECONDS)
        .await
        .expect("Abfrage läuft");
    assert!(notices(&brain).is_empty());
    assert!(invite.calls.lock().await.is_empty());
    assert_eq!(
        stored(&store).await.request.expect("Bitte bleibt").phase,
        Phase::Attempted
    );
}

#[test]
fn geloeschte_kopien_werden_der_reihe_nach_verfolgt_und_zuletzt_verworfen() {
    let mut state = open_request_state(Phase::Pending, None);
    if let Some(request) = state.request.as_mut() {
        request.reposts = vec![2, 3];
    }
    assert_eq!(
        on_gone(&mut state, 1),
        GoneOutcome::Followed { old: 1, new: 3 }
    );
    assert_eq!(on_gone(&mut state, 1), GoneOutcome::Unchanged);
    assert_eq!(
        on_gone(&mut state, 3),
        GoneOutcome::Followed { old: 3, new: 2 }
    );
    let request = state.request.clone().expect("Bitte bleibt");
    assert_eq!(request.earlier, vec![1, 3]);
    assert_eq!(request.created_at, NOW);
    assert_eq!(on_gone(&mut state, 2), GoneOutcome::Forgotten);
    assert!(state.request.is_none());
}

#[test]
fn kopien_der_bitte_bleiben_begrenzt_und_ohne_doppelte() {
    let mut state = open_request_state(Phase::Pending, None);
    for id in [2, 3, 3, 4, 5, 6, 7, 1] {
        assert!(note_repost(
            &mut state,
            &message(id, NOW + 10, "x 123456789")
        ));
    }
    let request = state.request.expect("Bitte bleibt");
    assert_eq!(request.reposts, vec![3, 4, 5, 6, 7]);
}

#[tokio::test]
async fn geloeschte_wartende_bitte_wird_beim_abgleich_still_verworfen() {
    let (watcher, store, port, _) = setup();
    let mut state = open_request_state(Phase::WaitingForCode, None);
    state.friend_code = None;
    state.status.notices.clear();
    store.states.lock().await.insert(USER, state);
    port.deleted.lock().await.insert(1);
    let (capture, guard) = capture_logs();
    let changed = watcher
        .reread_waiting_requests(NOW + 60)
        .await
        .expect("Abgleich läuft");
    drop(guard);
    assert!(changed.is_empty());
    let state = stored(&store).await;
    assert!(state.request.is_none());
    assert_eq!(
        state.last_result.as_deref(),
        Some("Bitte gelöscht; still beendet")
    );
    assert!(capture
        .text()
        .contains("Invite-Lounge verwirft gelöschte Bitte still"));
}

#[tokio::test]
async fn hinweisfreigabe_haelt_trotz_zwischenzeitlicher_schreibung() {
    let (watcher, store, port, _) = setup();
    port.reply_fails.store(true, Ordering::SeqCst);
    let recorded = watcher
        .record_message(message(1, NOW, IMPERSONAL_REQUEST), NOW, None)
        .await
        .expect("Bitte wird angelegt")
        .expect("Bitte gespeichert");
    let hint = recorded.hint.expect("Hinweis beansprucht");
    let previous = stored(&store).await;
    let mut touched = previous.clone();
    touched.last_seen = 2;
    assert!(store
        .compare_exchange(USER, Some(&previous), &touched)
        .await
        .expect("Schreiben läuft"));
    watcher.send_hint(hint).await.expect("Hinweis endet");
    let state = stored(&store).await;
    assert!(!state.hint_sent);
    assert_eq!(state.last_hint_at, None);
    assert_eq!(state.last_seen, 2);
}

#[tokio::test]
async fn verlaufsfehler_vor_dem_hinweis_verliert_die_bitte_nicht() {
    let (watcher, store, port, _) = setup();
    port.history_fails.store(true, Ordering::SeqCst);
    watcher
        .handle_message(message(1, NOW, IMPERSONAL_REQUEST), NOW, None)
        .await
        .expect("Verlaufsfehler ist kein Verarbeitungsfehler");
    let state = stored(&store).await;
    assert_eq!(
        state.request.map(|request| request.phase),
        Some(Phase::WaitingForCode)
    );
    assert!(state.hint_sent);
    assert_eq!(port.replies.lock().await.len(), 1);
}

#[tokio::test]
async fn abgelaufene_leihe_wendet_kein_zweites_urteil_an() {
    let (watcher, store, _, _) = setup();
    let mut state = open_request_state(Phase::Pending, None);
    state.dispatch_started_at_millis = None;
    let mut stale = HeldAnswer::fresh(5, "nein".into());
    stale.lease(NOW * 1000, test_lease_millis());
    let mut current = stale.clone();
    current.lease(NOW * 1000 + test_lease_millis(), test_lease_millis());
    state.status.held_answers = vec![current];
    store.states.lock().await.insert(USER, state.clone());
    let settled = watcher
        .store_answer(
            USER,
            CHECK_IN_CHANNEL_ID,
            &stale,
            Some(InviteCheckInOutcome::NotArrived),
            false,
            NOW + 60,
        )
        .await
        .expect("Speichern läuft");
    assert!(settled);
    assert_eq!(stored(&store).await, state);
}

#[tokio::test]
async fn zahl_in_einem_anderen_kanal_aendert_die_offene_bitte_nicht() {
    let (watcher, store, port, invite) = setup();
    let mut state = open_request_state(Phase::Pending, Some(SteamWait::Delivery));
    if let Some(request) = state.request.as_mut() {
        request.direct = true;
    }
    store.states.lock().await.insert(USER, state);
    let at = NOW + 2 * ONE_HOUR;
    let mut chat = message(2, at, "war das Match 45678912 verbuggt?");
    chat.channel_id = 555;
    watcher
        .handle_message(chat, at, Some(false))
        .await
        .expect("Nachricht wird gelesen");
    assert!(invite.calls.lock().await.is_empty());
    assert!(port.replies.lock().await.is_empty());
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("123456789"));
    let request = state.request.expect("Bitte bleibt");
    assert_eq!(request.message_id, 1);
    assert_eq!(state.status.channel_id, Some(CHECK_IN_CHANNEL_ID));
}

#[tokio::test]
async fn raumbitte_im_versand_mit_neuem_code_laedt_erst_nach_einem_nein() {
    let (watcher, store, _, invite) = setup();
    let mut state = open_request_state(Phase::Dispatching, None);
    state.status = RequestStatus::for_channel(CHECK_IN_CHANNEL_ID);
    store.states.lock().await.insert(USER, state);
    watcher
        .handle_message(
            message(2, NOW + 5, "Kann mich jemand einladen? 987654321"),
            NOW + 5,
            Some(false),
        )
        .await
        .expect("Neuer Code wird gelesen");
    watcher
        .dispatch_due(USER, NOW + 20)
        .await
        .expect("Takt läuft");
    assert!(invite.calls.lock().await.is_empty());
    let state = stored(&store).await;
    assert_eq!(state.friend_code.as_deref(), Some("987654321"));
    assert_eq!(state.dispatch_started_at_millis, None);
    let request = state.request.expect("Bitte vorhanden");
    assert_eq!(request.message_id, 2);
    assert_eq!(request.phase, Phase::Pending);
    assert!(!request.direct);
}

#[tokio::test]
async fn direkte_bitte_mit_neuem_code_im_laufenden_versand_bleibt_direkt() {
    let (watcher, store, _, _) = setup();
    let mut state = open_request_state(Phase::Dispatching, None);
    state.status = RequestStatus::for_channel(CHECK_IN_CHANNEL_ID);
    state.dispatch_started_at_millis = Some(NOW * 1000);
    store.states.lock().await.insert(USER, state);
    watcher
        .record_message(
            message(2, NOW + 5, "Kannst du mich einladen? 987654321"),
            NOW + 5,
            Some(false),
        )
        .await
        .expect("Direkte Bitte wird gelesen");
    let state = stored(&store).await;
    let request = state.request.expect("Bitte vorhanden");
    assert_eq!(request.message_id, 2);
    assert!(request.direct);
    assert_eq!(state.friend_code.as_deref(), Some("987654321"));
}
