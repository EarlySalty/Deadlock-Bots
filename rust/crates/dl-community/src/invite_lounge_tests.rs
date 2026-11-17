use super::status::{SteamOutcome, CHECK_IN_SECONDS, NOTICE_MAX_AGE_MILLIS, RETRY_SECONDS};
use super::*;
use crate::answer_support::test_brain::{decided, ScriptedBrain, ScriptedReply};
use dl_brain::{
    BrainError, ConversationReply, InviteIntent, InviteLoungeUpdate, InviteNotice, InviteStatus,
    NoticeTarget,
};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
use tokio::sync::Mutex;

const NOW: i64 = 1_790_000_000;
const BOT: u64 = 99;
const GUILD: u64 = 42;
const USER: u64 = 7;
const OTHER: u64 = 8;
const CODE: &str = "123456789";
const AUDIT_REPLY: &str = "ℹ️ Dieser Steam-Account wurde schon eingeladen, am <t:1700000000:D>. Hier ist nichts mehr zu tun.";
const RUNNING_REPLY: &str = "⏳ Für diesen Steam-Account läuft gerade schon eine Einladung. Ich schicke sie nicht doppelt, sonst zählt sie zweimal gegen das Tageslimit.";

#[derive(Clone, Copy, PartialEq, Eq)]
enum WaitPoint {
    Load,
    Claim,
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
    advance: Mutex<Option<ClockAdvance>>,
    actions_since: AtomicI64,
    progress: Mutex<SteamProgress>,
    progress_queries: Mutex<Vec<(i64, i64)>>,
    handled: AtomicBool,
    opted_out: AtomicBool,
    opt_out_on_write: AtomicBool,
    hidden: Mutex<HashSet<u64>>,
    hidden_fails: AtomicBool,
    mark: Mutex<Option<u64>>,
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

    async fn state(&self) -> LoungeState {
        self.states
            .lock()
            .await
            .get(&USER)
            .cloned()
            .expect("Zustand gespeichert")
    }

    async fn put(&self, state: LoungeState) {
        self.states.lock().await.insert(USER, state);
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
    ) -> Result<Cas, String> {
        if self.opted_out.load(Ordering::SeqCst) || self.opt_out_on_write.load(Ordering::SeqCst) {
            return Ok(Cas::OptedOut);
        }
        let mut states = self.states.lock().await;
        if states.get(&id) != previous {
            return Ok(Cas::Stale);
        }
        states.insert(id, next.clone());
        drop(states);
        if next
            .request
            .as_ref()
            .is_some_and(|request| request.phase == Phase::Dispatching)
        {
            self.after_wait(WaitPoint::Claim).await;
        }
        Ok(Cas::Written)
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
                    .is_some_and(|request| request.phase.open())
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
    async fn opted_out(&self, _user_id: u64) -> bool {
        self.opted_out.load(Ordering::SeqCst)
    }
    async fn hidden_authors(&self, user_ids: &[u64]) -> Result<HashSet<u64>, String> {
        if self.hidden_fails.load(Ordering::SeqCst) {
            return Err("Datenschutzstand nicht lesbar".into());
        }
        let hidden = self.hidden.lock().await;
        Ok(user_ids
            .iter()
            .copied()
            .filter(|id| hidden.contains(id))
            .collect())
    }
    async fn catch_up_mark(&self) -> Result<Option<u64>, String> {
        Ok(*self.mark.lock().await)
    }
    async fn set_catch_up_mark(&self, message_id: u64) -> Result<(), String> {
        *self.mark.lock().await = Some(message_id);
        Ok(())
    }
    async fn invited(&self, _code: &str) -> Result<bool, String> {
        Ok(self.progress.lock().await.invited)
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
    async fn steam_progress(
        &self,
        _code: &str,
        since: i64,
        requested_at: i64,
    ) -> Result<SteamProgress, String> {
        self.progress_queries
            .lock()
            .await
            .extend([(since, requested_at)]);
        Ok(*self.progress.lock().await)
    }
    async fn handled_since(&self, _code: &str, _since: i64) -> Result<bool, String> {
        Ok(self.handled.load(Ordering::SeqCst))
    }
}

#[derive(Default)]
struct RecordingPort {
    history: Mutex<Vec<LoungeMessage>>,
    history_calls: AtomicUsize,
    identity_missing: AtomicBool,
    history_fails: AtomicBool,
}

#[async_trait::async_trait]
impl InviteLoungeReplyPort for RecordingPort {
    fn bot_id(&self) -> Option<u64> {
        (!self.identity_missing.load(Ordering::SeqCst)).then_some(BOT)
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

struct Lounge {
    watcher: InviteLoungeWatcher,
    store: Arc<MemoryStore>,
    port: Arc<RecordingPort>,
    invite: Arc<RecordingInvite>,
    brain: Arc<ScriptedBrain>,
}

fn watcher(
    store: Arc<dyn LoungeStore>,
    port: Arc<RecordingPort>,
    invite: Arc<RecordingInvite>,
    brain: Arc<ScriptedBrain>,
) -> InviteLoungeWatcher {
    InviteLoungeWatcher {
        brain,
        store,
        port,
        invite,
        guild_id: GUILD,
        clock_millis: Arc::new(|| NOW * 1000),
        locks: UserLocks::default(),
    }
}

fn setup() -> Lounge {
    let store = Arc::new(MemoryStore::default());
    let port = Arc::new(RecordingPort::default());
    let invite = Arc::new(RecordingInvite::default());
    let brain = Arc::new(ScriptedBrain::default());
    Lounge {
        watcher: watcher(store.clone(), port.clone(), invite.clone(), brain.clone()),
        store,
        port,
        invite,
        brain,
    }
}

impl Lounge {
    fn moving_clock(&mut self) -> Arc<AtomicI64> {
        let clock = Arc::new(AtomicI64::new(NOW * 1000));
        let reader = clock.clone();
        self.watcher.clock_millis = Arc::new(move || reader.load(Ordering::SeqCst));
        clock
    }

    fn decide(&self, replies: Vec<ScriptedReply>) {
        self.brain.script_turns(replies);
    }

    async fn post(&self, message: LoungeMessage, now: i64) -> Option<u64> {
        let context = self
            .port
            .history(message.channel_id, Some(message.message_id))
            .await
            .unwrap_or_default();
        self.port.history.lock().await.push(message.clone());
        let turn = self
            .watcher
            .handle_turn(&message, Context::Given(&context), now)
            .await
            .expect("Nachricht verarbeitet");
        let Turn::Stored(user_id) = turn else {
            return None;
        };
        self.watcher.follow_up(user_id, now).await;
        Some(user_id)
    }

    async fn poll(&self, now: i64) {
        self.watcher
            .poll_due(now)
            .await
            .expect("Warteaufträge geprüft");
    }

    async fn calls(&self) -> usize {
        self.invite.calls.lock().await.len()
    }

    async fn outbox(&self) -> Vec<OutboxEntry> {
        self.store
            .states
            .lock()
            .await
            .get(&USER)
            .map(|state| state.outbox.clone())
            .unwrap_or_default()
    }

    async fn direct_request(&self) {
        self.decide(vec![wants(InviteTurnAction::Invite, None)]);
        self.post(message(1, NOW, "Kannst du mich einladen? 123456789"), NOW)
            .await;
    }
}

fn wants(action: InviteTurnAction, text: Option<&str>) -> ScriptedReply {
    decided(InviteIntent::WantsInvite, action, text)
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
        system: false,
        reply_message_id: None,
    }
}

fn from(author_id: u64, mut message: LoungeMessage) -> LoungeMessage {
    message.author_id = author_id;
    message.is_bot = author_id == BOT;
    message
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

fn updates(brain: &ScriptedBrain) -> Vec<InviteLoungeUpdate> {
    notices(brain)
        .into_iter()
        .map(|(_, notice)| notice.update)
        .collect()
}

fn turns(brain: &ScriptedBrain) -> Vec<InviteTurn> {
    brain
        .requests
        .lock()
        .expect("Brain-Anfragen lesbar")
        .iter()
        .filter_map(|request| match &request.task {
            dl_brain::DiscordConversation::InviteTurn { turn } => Some(turn.clone()),
            _ => None,
        })
        .collect()
}

fn sent() -> InviteLoungeUpdate {
    InviteLoungeUpdate::Status {
        status: InviteStatus::Sent,
    }
}

fn pending(direct: bool) -> LoungeState {
    LoungeState {
        last_seen: 1,
        friend_code: Some(CODE.into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW,
            direct,
            phase: Phase::Pending,
        }),
        status: RequestStatus::for_channel(CHECK_IN_CHANNEL_ID),
        ..Default::default()
    }
}

#[tokio::test]
async fn lachs_antwort_im_fremden_check_in_wird_gefragt_und_nein_loest_die_einladung_aus() {
    let lounge = setup();
    lounge
        .store
        .put(LoungeState {
            last_seen: 2,
            friend_code: Some("381230343".into()),
            ..Default::default()
        })
        .await;
    let mut check_in = from(
        BOT,
        message(4, NOW - 60, "<@8> Hast du deine Einladung schon bekommen?"),
    );
    check_in.reply_message_id = Some(3);
    lounge.port.history.lock().await.extend([
        from(
            OTHER,
            message(3, NOW - 2000, "Kann mich wer einladen? 987654321"),
        ),
        check_in,
    ]);
    let mut answer = message(5, NOW, "ich habe auch noch nichts bekommen");
    answer.reply_message_id = Some(4);
    lounge.decide(vec![wants(
        InviteTurnAction::AskAlreadyInvited,
        Some("Hast du schon eine Einladung bekommen?"),
    )]);
    lounge.post(answer, NOW).await;
    let turn = turns(&lounge.brain).remove(0);
    assert_eq!(turn.message, "ich habe auch noch nichts bekommen");
    assert_eq!(
        turn.reply_to,
        Some(InviteChatLine {
            speaker: InviteSpeaker::Bot,
            text: "@jemand Hast du deine Einladung schon bekommen?".into()
        })
    );
    assert_eq!(turn.member.phase, InviteRequestPhase::None);
    assert!(turn.member.has_friend_code);
    assert!(!turn.member.code_in_message);
    assert_eq!(lounge.calls().await, 0);
    let (_, reply) = notices(&lounge.brain).remove(0);
    assert_eq!(reply.update, InviteLoungeUpdate::Reply);
    assert_eq!(
        reply.answer.as_deref(),
        Some("Hast du schon eine Einladung bekommen?")
    );
    assert_eq!(
        reply.target,
        NoticeTarget::Reply {
            channel_id: CHECK_IN_CHANNEL_ID,
            message_id: 5
        }
    );
    assert_eq!(reply.idempotency_key, "invite-lounge:5:reply");
    let state = lounge.store.state().await;
    assert!(state.status.asked());
    lounge.poll(NOW + 3 * CHECK_IN_SECONDS).await;
    assert_eq!(lounge.calls().await, 0);
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(message(6, NOW + 4000, "nein"), NOW + 4000)
        .await;
    let turn = turns(&lounge.brain).remove(1);
    assert_eq!(turn.member.phase, InviteRequestPhase::CheckInAsked);
    assert_eq!(
        turn.recent.last().map(|line| line.speaker),
        Some(InviteSpeaker::Member)
    );
    assert_eq!(
        lounge.invite.calls.lock().await.clone(),
        vec![(BOT, GUILD, "381230343".to_string(), USER)]
    );
    assert_eq!(
        updates(&lounge.brain),
        vec![InviteLoungeUpdate::Reply, sent()]
    );
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn greenlight_nachfrage_nach_dreissig_minuten_und_nein_ohne_antwortbezug_laedt_ein() {
    let lounge = setup();
    lounge.decide(vec![wants(InviteTurnAction::WaitForCommunity, None)]);
    lounge
        .post(message(1, NOW, "Kann mich wer einladen? 123456789"), NOW)
        .await;
    assert_eq!(
        lounge
            .store
            .state()
            .await
            .request
            .map(|request| request.phase),
        Some(Phase::Pending)
    );
    lounge.poll(NOW + CHECK_IN_SECONDS - 1).await;
    assert!(notices(&lounge.brain).is_empty());
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::CheckIn]);
    assert_eq!(lounge.calls().await, 0);
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(message(2, NOW + 1900, "nein, noch nicht"), NOW + 1900)
        .await;
    let turn = turns(&lounge.brain).remove(1);
    assert_eq!(turn.member.phase, InviteRequestPhase::CheckInAsked);
    assert_eq!(turn.member.last_update, Some(InviteLoungeUpdate::CheckIn));
    assert_eq!(lounge.calls().await, 1);
    assert_eq!(
        updates(&lounge.brain),
        vec![InviteLoungeUpdate::CheckIn, sent()]
    );
    assert_eq!(
        lounge
            .store
            .state()
            .await
            .request
            .map(|request| request.phase),
        Some(Phase::Attempted)
    );
}

#[tokio::test]
async fn ja_auf_die_nachfrage_schliesst_ohne_einladung() {
    for text in [Some("Super, viel Spaß!"), None] {
        let lounge = setup();
        lounge.store.put(pending(false)).await;
        lounge.poll(NOW + CHECK_IN_SECONDS).await;
        lounge.decide(vec![decided(
            InviteIntent::HasAccess,
            InviteTurnAction::Close,
            text,
        )]);
        lounge
            .post(message(2, NOW + 1900, "ja, hab es"), NOW + 1900)
            .await;
        lounge.poll(NOW + 7200).await;
        assert_eq!(lounge.calls().await, 0);
        let closing = match text {
            Some(_) => InviteLoungeUpdate::Reply,
            None => Notice::Confirmed.update(),
        };
        assert_eq!(
            updates(&lounge.brain),
            vec![InviteLoungeUpdate::CheckIn, closing]
        );
        let state = lounge.store.state().await;
        assert_eq!(
            state.request.map(|request| request.phase),
            Some(Phase::Attempted)
        );
        assert!(state.outbox.is_empty());
    }
}

#[tokio::test]
async fn normaler_chat_bleibt_still_und_merkt_sich_nur_die_gelesene_nachricht() {
    let lounge = setup();
    lounge
        .post(
            message(1, NOW, "Wer spielt heute Abend noch eine Runde?"),
            NOW,
        )
        .await;
    lounge.poll(NOW + 7200).await;
    lounge
        .post(
            message(1, NOW + 5, "Wer spielt heute Abend noch eine Runde?"),
            NOW + 5,
        )
        .await;
    assert_eq!(turns(&lounge.brain).len(), 1);
    assert!(notices(&lounge.brain).is_empty());
    assert_eq!(
        lounge.store.state().await,
        LoungeState {
            last_seen: 1,
            ..Default::default()
        }
    );
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn schweigen_loest_nie_eine_einladung_aus() {
    let lounge = setup();
    lounge.decide(vec![wants(InviteTurnAction::WaitForCommunity, None)]);
    lounge
        .post(message(1, NOW, "Kann mich wer einladen? 123456789"), NOW)
        .await;
    for at in [
        NOW + 3600,
        NOW + 6 * 24 * 3600,
        NOW + REQUEST_WINDOW_SECONDS + 1,
    ] {
        lounge.poll(at).await;
    }
    assert_eq!(lounge.calls().await, 0);
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::CheckIn]);
    assert_eq!(
        lounge
            .store
            .state()
            .await
            .request
            .map(|request| request.phase),
        Some(Phase::Expired)
    );
}

#[tokio::test]
async fn code_nachfrage_und_fortsetzung_mit_code() {
    let lounge = setup();
    lounge.decide(vec![
        wants(
            InviteTurnAction::AskFriendCode,
            Some("Schick mir kurz deinen Freundescode."),
        ),
        wants(InviteTurnAction::Invite, None),
    ]);
    lounge
        .post(message(1, NOW, "kannst du mich inviten"), NOW)
        .await;
    let state = lounge.store.state().await;
    assert_eq!(
        state.request.map(|request| request.phase),
        Some(Phase::WaitingForCode)
    );
    assert_eq!(lounge.calls().await, 0);
    lounge
        .post(message(2, NOW + 60, "123456789"), NOW + 60)
        .await;
    let turn = turns(&lounge.brain).remove(1);
    assert_eq!(turn.member.phase, InviteRequestPhase::WaitingForCode);
    assert!(turn.member.code_in_message);
    assert!(!turn.member.has_friend_code);
    assert_eq!(
        lounge.invite.calls.lock().await.clone(),
        vec![(BOT, GUILD, CODE.to_string(), USER)]
    );
    assert_eq!(
        updates(&lounge.brain),
        vec![InviteLoungeUpdate::Reply, sent()]
    );
}

#[tokio::test]
async fn bekannter_steam_link_reicht_fuer_die_einladung() {
    let lounge = setup();
    lounge.store.known.lock().await.insert(USER, CODE.into());
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(message(1, NOW, "Kannst du mich bitte einladen?"), NOW)
        .await;
    assert!(turns(&lounge.brain)[0].member.has_friend_code);
    assert_eq!(
        lounge.invite.calls.lock().await.clone(),
        vec![(BOT, GUILD, CODE.to_string(), USER)]
    );
}

#[tokio::test]
async fn bereits_eingeladene_bekommen_hilfe_statt_einladung() {
    let lounge = setup();
    lounge.store.progress.lock().await.invited = true;
    lounge.decide(vec![
        wants(InviteTurnAction::Invite, None),
        decided(
            InviteIntent::InviteProblem,
            InviteTurnAction::Help,
            Some("Schau in deiner Steam-Bibliothek unter Deadlock nach."),
        ),
    ]);
    lounge
        .post(message(1, NOW, "Kannst du mich einladen? 123456789"), NOW)
        .await;
    assert_eq!(lounge.store.state().await.last_seen, 1);
    lounge
        .post(
            message(2, NOW + 10, "ist nicht in meiner Bibliothek"),
            NOW + 10,
        )
        .await;
    assert!(turns(&lounge.brain)[0].member.already_invited);
    assert_eq!(lounge.calls().await, 0);
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::Reply]);
    let state = lounge.store.state().await;
    assert_eq!(state.last_seen, 2);
    assert_eq!(state.request, None);
}

#[tokio::test]
async fn entscheidung_waehrend_laufendem_versand_bleibt_still() {
    let lounge = setup();
    let mut state = pending(true);
    if let Some(request) = state.request.as_mut() {
        request.phase = Phase::Dispatching;
    }
    state.dispatch_started_at_millis = Some(NOW * 1000);
    lounge.store.put(state).await;
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(message(2, NOW + 5, "und jetzt? 123456789"), NOW + 5)
        .await;
    let state = lounge.store.state().await;
    assert_eq!(state.last_seen, 2);
    assert!(state.outbox.is_empty());
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn datenschutz_opt_out_geht_nicht_an_den_brain() {
    let lounge = setup();
    lounge.store.opted_out.store(true, Ordering::SeqCst);
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(message(1, NOW, "Kannst du mich einladen? 123456789"), NOW)
        .await;
    assert!(turns(&lounge.brain).is_empty());
    assert!(lounge.store.states.lock().await.is_empty());
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn brain_ausfall_verbraucht_die_nachricht_nicht() {
    let lounge = setup();
    lounge.store.put(pending(false)).await;
    lounge.decide(vec![Err(BrainError::Backend("weg".into()))]);
    let answer = message(2, NOW + 10, "nein, noch nicht");
    lounge.post(answer.clone(), NOW + 10).await;
    assert_eq!(lounge.store.state().await.last_seen, 1);
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge.post(answer, NOW + 20).await;
    assert_eq!(lounge.store.state().await.last_seen, 2);
    assert_eq!(lounge.calls().await, 1);
}

#[tokio::test]
async fn gesehene_nachricht_geht_kein_zweites_mal_an_den_brain() {
    let lounge = setup();
    lounge.store.put(pending(false)).await;
    lounge
        .post(message(1, NOW, "Kann mich wer einladen? 123456789"), NOW)
        .await;
    assert!(turns(&lounge.brain).is_empty());
}

#[tokio::test]
async fn fremde_kanaele_bots_alte_und_leere_nachrichten_loesen_nichts_aus() {
    let lounge = setup();
    lounge
        .store
        .actions_since
        .store(NOW - 100, Ordering::SeqCst);
    let mut other_channel = message(1, NOW, "Kannst du mich einladen? 123456789");
    other_channel.channel_id = 100;
    let mut other_guild = message(2, NOW, "Kannst du mich einladen? 123456789");
    other_guild.guild_id = Some(1);
    let bot = from(BOT, message(3, NOW, "Kannst du mich einladen? 123456789"));
    let mut foreign_bot = from(OTHER, message(4, NOW, "Kannst du mich einladen? 123456789"));
    foreign_bot.is_bot = true;
    for event in [
        other_channel,
        other_guild,
        bot,
        foreign_bot,
        message(5, NOW, "   "),
        message(6, NOW - 200, "vor dem Deploy, 123456789"),
    ] {
        lounge.post(event, NOW).await;
    }
    lounge
        .store
        .actions_since
        .store(NOW - 2 * REQUEST_WINDOW_SECONDS, Ordering::SeqCst);
    let mut system = message(8, NOW, "hat den Server betreten 123456789");
    system.system = true;
    for event in [
        message(7, NOW - REQUEST_WINDOW_SECONDS - 1, "uralt 123456789"),
        system,
    ] {
        lounge.post(event, NOW).await;
    }
    assert!(turns(&lounge.brain).is_empty());
    assert!(lounge.store.states.lock().await.is_empty());
}

#[tokio::test]
async fn fehlende_bot_id_bricht_ab_ohne_brain() {
    let lounge = setup();
    lounge.port.identity_missing.store(true, Ordering::SeqCst);
    let result = lounge
        .watcher
        .handle_turn(
            &message(1, NOW, "Kannst du mich einladen?"),
            Context::Given(&[]),
            NOW,
        )
        .await;
    assert!(result.is_err());
    assert!(turns(&lounge.brain).is_empty());
}

#[test]
fn turn_text_ersetzt_erwaehnungen_und_haelt_die_grenzen_ein() {
    let mut context: Vec<LoungeMessage> = (1..=20)
        .map(|id| from(OTHER, message(id, NOW, &format!("Zeile {id}"))))
        .collect();
    context.push(from(
        OTHER,
        message(21, NOW, &"x".repeat(MAX_CHAT_LINE_CHARS + 10)),
    ));
    context.push(from(OTHER, message(22, NOW, "   ")));
    context.push(from(OTHER, message(40, NOW, "später")));
    let mut event = message(
        30,
        NOW,
        &format!(
            "<@99> <@!7> <@8> <@&5> <#6> {}",
            "y".repeat(MAX_TASK_TEXT_CHARS)
        ),
    );
    event.reply_message_id = Some(3);
    let member = member_state(None, &MemberFacts::default());
    let turn = build_turn(&event, &context, BOT, member).expect("Nachricht ist nicht leer");
    assert!(turn
        .message
        .starts_with("@Bot @Mitglied @jemand @Rolle #kanal y"));
    assert_eq!(turn.message.chars().count(), MAX_TASK_TEXT_CHARS);
    assert_eq!(turn.recent.len(), dl_brain::MAX_TURN_RECENT_LINES);
    assert_eq!(
        turn.recent.last().map(|line| line.text.chars().count()),
        Some(MAX_CHAT_LINE_CHARS)
    );
    assert!(turn.recent.iter().all(|line| line.text != "später"));
    assert_eq!(
        turn.reply_to.as_ref().map(|line| line.text.clone()),
        Some("Zeile 3".to_string())
    );
    assert!(turn.valid());
    assert!(build_turn(
        &message(31, NOW, " <@99> "),
        &[],
        BOT,
        member_state(None, &MemberFacts::default())
    )
    .is_some_and(|turn| turn.message == "@Bot"));
    assert!(build_turn(
        &message(32, NOW, "  "),
        &[],
        BOT,
        member_state(None, &MemberFacts::default())
    )
    .is_none());
}

#[test]
fn phasen_fuer_den_brain_folgen_dem_auftrag() {
    let mut state = pending(false);
    assert_eq!(
        request_phase(&state),
        InviteRequestPhase::WaitingForCommunity
    );
    state.status.claim(Some(Notice::CheckIn));
    assert_eq!(request_phase(&state), InviteRequestPhase::CheckInAsked);
    state.status.invite_now = true;
    assert_eq!(request_phase(&state), InviteRequestPhase::InProgress);
    assert_eq!(
        request_phase(&pending(true)),
        InviteRequestPhase::InProgress
    );
    let mut waiting = pending(false);
    waiting.status.wait = Some(SteamWait::Delivery);
    assert_eq!(request_phase(&waiting), InviteRequestPhase::InProgress);
    for (phase, expected) in [
        (Phase::WaitingForCode, InviteRequestPhase::WaitingForCode),
        (Phase::Dispatching, InviteRequestPhase::InProgress),
        (Phase::Attempted, InviteRequestPhase::Closed),
        (Phase::Expired, InviteRequestPhase::Closed),
    ] {
        let mut state = pending(false);
        if let Some(request) = state.request.as_mut() {
            request.phase = phase;
        }
        assert_eq!(request_phase(&state), expected);
    }
    assert_eq!(
        request_phase(&LoungeState::default()),
        InviteRequestPhase::None
    );
    let mut notified = pending(false);
    notified.status.claim(Some(Notice::FriendRequest));
    let member = member_state(Some(&notified), &MemberFacts::default());
    assert_eq!(member.last_update, Some(Notice::FriendRequest.update()));
}

#[tokio::test]
async fn rueckblick_holt_jede_nachricht_ab_der_marke_der_reihe_nach_nach() {
    let lounge = setup();
    *lounge.store.mark.lock().await = Some(2);
    let mut answered = from(OTHER, message(3, NOW - 500, "habs dir geschickt"));
    answered.reply_message_id = Some(2);
    let mut by_bot = from(BOT, message(6, NOW - 100, "<@11> Schick mir deinen Code."));
    by_bot.reply_message_id = Some(5);
    lounge.port.history.lock().await.extend([
        message(1, NOW - 900, "Kann mich wer einladen?"),
        from(10, message(2, NOW - 800, "invite bitte 111222333")),
        answered,
        message(4, NOW - 300, "hallo? 123456789"),
        from(11, message(5, NOW - 200, "brauche invite")),
        by_bot,
    ]);
    lounge.decide(vec![
        Ok(ConversationReply::Silent),
        wants(InviteTurnAction::Invite, None),
        Ok(ConversationReply::Silent),
    ]);
    lounge
        .watcher
        .catch_up(NOW, 0)
        .await
        .expect("Rückblick läuft");
    let asked: Vec<_> = turns(&lounge.brain)
        .into_iter()
        .map(|turn| (turn.message, turn.recent.len()))
        .collect();
    assert_eq!(
        asked,
        [
            ("habs dir geschickt".to_string(), 2),
            ("hallo? 123456789".to_string(), 3),
            ("brauche invite".to_string(), 4),
        ]
    );
    assert_eq!(lounge.calls().await, 1);
    assert_eq!(*lounge.store.mark.lock().await, Some(6));
    lounge
        .watcher
        .catch_up(NOW + 60, 0)
        .await
        .expect("Rückblick läuft");
    assert_eq!(turns(&lounge.brain).len(), 3);
}

#[tokio::test]
async fn rueckblick_ohne_marke_beginnt_beim_start_statt_beim_alten_verlauf() {
    let lounge = setup();
    lounge.port.history.lock().await.extend([
        message(1, NOW - 900, "Kann mich wer einladen? 123456789"),
        from(OTHER, message(5, NOW - 10, "brauche invite")),
    ]);
    lounge
        .watcher
        .catch_up(NOW, 3)
        .await
        .expect("Rückblick läuft");
    let asked: Vec<_> = turns(&lounge.brain)
        .into_iter()
        .map(|turn| turn.message)
        .collect();
    assert_eq!(asked, ["brauche invite"]);
    assert_eq!(*lounge.store.mark.lock().await, Some(5));
}

#[tokio::test]
async fn fehlgeschlagener_turn_haelt_die_marke_und_kommt_in_der_naechsten_runde() {
    let lounge = setup();
    lounge.port.history.lock().await.extend([
        message(1, NOW - 300, "Kann mich wer einladen? 123456789"),
        from(OTHER, message(2, NOW - 200, "moin")),
        message(3, NOW - 100, "hallo?"),
    ]);
    lounge.decide(vec![
        Err(BrainError::Backend("weg".into())),
        Ok(ConversationReply::Silent),
    ]);
    assert!(lounge.watcher.catch_up(NOW, 0).await.is_err());
    let first: Vec<_> = turns(&lounge.brain)
        .into_iter()
        .map(|turn| turn.message)
        .collect();
    assert_eq!(first, ["Kann mich wer einladen? 123456789", "moin"]);
    assert_eq!(*lounge.store.mark.lock().await, Some(0));
    lounge.decide(vec![
        wants(InviteTurnAction::Invite, None),
        Ok(ConversationReply::Silent),
    ]);
    lounge
        .watcher
        .catch_up(NOW + 60, 0)
        .await
        .expect("zweite Runde läuft durch");
    let second: Vec<_> = turns(&lounge.brain)
        .into_iter()
        .skip(2)
        .map(|turn| turn.message)
        .collect();
    assert_eq!(second, ["Kann mich wer einladen? 123456789", "hallo?"]);
    assert_eq!(*lounge.store.mark.lock().await, Some(3));
    assert_eq!(lounge.calls().await, 1);
}

#[tokio::test]
async fn ungelesener_verlauf_haelt_den_rueckblick_an() {
    let lounge = setup();
    lounge.port.history_fails.store(true, Ordering::SeqCst);
    assert!(lounge.watcher.catch_up(NOW, 0).await.is_err());
    assert!(turns(&lounge.brain).is_empty());
}

#[tokio::test]
async fn rueckblick_paginiert_ueber_mehr_als_hundert_nachrichten() {
    let lounge = setup();
    let history: Vec<_> = (1..=150)
        .map(|id| from(1000 + id, message(id, NOW - 1000 + id as i64, "Moin")))
        .collect();
    lounge.port.history.lock().await.extend(history);
    lounge
        .watcher
        .catch_up(NOW, 0)
        .await
        .expect("Rückblick läuft");
    assert_eq!(turns(&lounge.brain).len(), 150);
    assert!(lounge.port.history_calls.load(Ordering::SeqCst) >= 2);
    assert_eq!(*lounge.store.mark.lock().await, Some(150));
}

#[tokio::test]
async fn zeilen_von_abgemeldeten_mitgliedern_gehen_nicht_an_den_brain() {
    let lounge = setup();
    let mut joined = from(OTHER, message(3, NOW - 30, "ist dem Server beigetreten"));
    joined.system = true;
    lounge.port.history.lock().await.extend([
        from(OTHER, message(1, NOW - 60, "geheim von 8")),
        from(10, message(2, NOW - 50, "offen von 10")),
        joined,
        from(BOT, message(4, NOW - 20, "Bot-Zeile")),
    ]);
    lounge.store.hidden.lock().await.insert(OTHER);
    lounge
        .post(message(5, NOW, "Kann mich wer einladen?"), NOW)
        .await;
    let lines: Vec<_> = turns(&lounge.brain)[0]
        .recent
        .iter()
        .map(|line| line.text.clone())
        .collect();
    assert_eq!(lines, ["offen von 10", "Bot-Zeile"]);
    lounge.store.hidden_fails.store(true, Ordering::SeqCst);
    lounge.post(message(6, NOW + 5, "hallo?"), NOW + 5).await;
    let lines: Vec<_> = turns(&lounge.brain)[1]
        .recent
        .iter()
        .map(|line| line.text.clone())
        .collect();
    assert_eq!(lines, ["Bot-Zeile", "Kann mich wer einladen?"]);
}

#[tokio::test]
async fn opt_out_waehrend_der_entscheidung_schreibt_nichts_und_laedt_nicht_ein() {
    let lounge = setup();
    lounge.store.opt_out_on_write.store(true, Ordering::SeqCst);
    lounge.decide(vec![wants(InviteTurnAction::Invite, Some("Kommt sofort."))]);
    let stored = lounge
        .post(message(1, NOW, "Kannst du mich einladen? 123456789"), NOW)
        .await;
    assert_eq!(stored, None);
    assert_eq!(turns(&lounge.brain).len(), 1);
    assert!(notices(&lounge.brain).is_empty());
    assert!(lounge.store.states.lock().await.is_empty());
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn code_aus_ungelesener_eigener_zeile_zaehlt_fuer_die_einladung() {
    let lounge = setup();
    lounge.decide(vec![Err(BrainError::Backend("weg".into()))]);
    lounge.post(message(1, NOW, "123456789"), NOW).await;
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(message(2, NOW + 5, "kannst du mich einladen"), NOW + 5)
        .await;
    assert!(turns(&lounge.brain)[1].member.code_in_message);
    assert_eq!(
        lounge.invite.calls.lock().await.clone(),
        vec![(BOT, GUILD, CODE.to_string(), USER)]
    );
}

#[test]
fn gespeicherter_zustand_wird_inhaltlich_verglichen() {
    let legacy = format!(
        r#"{{"last_seen":3,"friend_code":"{CODE}","request":{{"message_id":1,"created_at":{NOW},"direct":false,"phase":"Pending"}},"hint_sent":false,"last_hint_at":null,"last_result":null,"dispatch_started_at_millis":null,"last_dispatch_finished_at_millis":5,"status":{{"channel_id":{CHECK_IN_CHANNEL_ID},"check_in_message_id":9}}}}"#
    );
    let parsed: LoungeState = serde_json::from_str(&legacy).expect("alter Zustand lesbar");
    assert!(stored_matches(Some(&legacy), Some(&parsed)));
    let mut changed = parsed.clone();
    changed.last_seen = 4;
    assert!(!stored_matches(Some(&legacy), Some(&changed)));
    assert!(stored_matches(None, None));
    assert!(!stored_matches(Some(&legacy), None));
    assert!(!stored_matches(None, Some(&parsed)));
    assert!(!stored_matches(Some("kaputt"), Some(&parsed)));
    let hinted = legacy.replace(
        r#""hint_sent":false,"last_hint_at":null"#,
        r#""hint_sent":true,"last_hint_at":1789999000"#,
    );
    assert_ne!(hinted, legacy);
    assert!(stored_matches(Some(&hinted), Some(&parsed)));
}

#[tokio::test]
async fn gelöschte_bitte_verwirft_die_antwort_still() {
    let lounge = setup();
    lounge
        .brain
        .script_notices(vec![Ok(ConversationReply::Rejected)]);
    lounge.decide(vec![wants(
        InviteTurnAction::AskFriendCode,
        Some("Schick mir deinen Freundescode."),
    )]);
    lounge
        .post(message(1, NOW, "kannst du mich einladen"), NOW)
        .await;
    lounge.poll(NOW + 3600).await;
    assert_eq!(notices(&lounge.brain).len(), 1);
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn audit_und_laufender_auftrag_blocken_ohne_zweiten_versand() {
    for (audit, expected) in [(true, AUDIT_REPLY), (false, RUNNING_REPLY)] {
        let lounge = setup();
        lounge.invite.audit.store(audit, Ordering::SeqCst);
        lounge.invite.running_task.store(!audit, Ordering::SeqCst);
        lounge.direct_request().await;
        lounge.poll(NOW + 3600).await;
        assert_eq!(lounge.invite.sends.load(Ordering::SeqCst), 0);
        assert_eq!(lounge.calls().await, 1);
        assert_eq!(
            lounge.store.state().await.last_result.as_deref(),
            Some(expected)
        );
    }
}

#[tokio::test]
async fn steam_ausfall_bleibt_still_offen_und_versucht_spaeter_erneut() {
    let lounge = setup();
    lounge.invite.fail.store(true, Ordering::SeqCst);
    lounge.direct_request().await;
    let state = lounge.store.state().await;
    assert!(state
        .last_result
        .expect("Ergebnis gespeichert")
        .contains("Fehlgeschlagen"));
    assert_eq!(
        state.request.map(|request| request.phase),
        Some(Phase::Pending)
    );
    lounge.poll(NOW + RETRY_SECONDS - 1).await;
    assert_eq!(lounge.calls().await, 1);
    lounge.poll(NOW + RETRY_SECONDS).await;
    assert_eq!(lounge.calls().await, 2);
    assert!(notices(&lounge.brain).is_empty());
    lounge.invite.fail.store(false, Ordering::SeqCst);
    lounge.poll(NOW + 2 * RETRY_SECONDS).await;
    assert_eq!(lounge.calls().await, 3);
    assert_eq!(updates(&lounge.brain), vec![sent()]);
}

#[tokio::test]
async fn fa_raus_wird_einmal_gemeldet_und_danach_die_einladung() {
    let lounge = setup();
    *lounge.invite.reply.lock().await =
        Some("📨 Freundschaftsanfrage ist raus. Sag ihm, er soll sie auf Steam annehmen.".into());
    lounge.store.progress.lock().await.open = true;
    lounge.direct_request().await;
    for at in [NOW + 61, NOW + 200, NOW + 400] {
        lounge.poll(at).await;
    }
    let friendship = InviteLoungeUpdate::Status {
        status: InviteStatus::FriendshipMissing,
    };
    assert_eq!(updates(&lounge.brain), vec![friendship]);
    *lounge.store.progress.lock().await = SteamProgress {
        outcome: Some(SteamOutcome::Sent),
        invited: true,
        open: false,
    };
    lounge.poll(NOW + 600).await;
    lounge.poll(NOW + 900).await;
    assert_eq!(lounge.calls().await, 1);
    assert_eq!(updates(&lounge.brain), vec![friendship, sent()]);
    let keys: Vec<_> = notices(&lounge.brain)
        .into_iter()
        .map(|(_, notice)| notice.idempotency_key)
        .collect();
    assert_eq!(
        keys,
        ["invite-lounge:1:friend_request", "invite-lounge:1:invite"]
    );
}

#[tokio::test]
async fn antwort_eines_helfers_ersetzt_die_nachfrage_nicht() {
    let lounge = setup();
    lounge.decide(vec![wants(InviteTurnAction::WaitForCommunity, None)]);
    lounge
        .post(message(1, NOW, "Kann mich wer einladen? 123456789"), NOW)
        .await;
    let mut answer = from(OTHER, message(2, NOW + 60, "<@7> FA ist raus"));
    answer.reply_message_id = Some(1);
    lounge.port.history.lock().await.push(answer);
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::CheckIn]);
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn von_hand_erledigte_bitte_schliesst_ohne_nachfrage() {
    let lounge = setup();
    lounge.store.put(pending(false)).await;
    lounge.store.handled.store(true, Ordering::SeqCst);
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    assert!(notices(&lounge.brain).is_empty());
    assert_eq!(lounge.calls().await, 0);
    let state = lounge.store.state().await;
    assert_eq!(
        state.request.map(|request| request.phase),
        Some(Phase::Attempted)
    );
}

#[tokio::test]
async fn abgelehnte_nachfrage_gilt_nicht_als_gestellt() {
    let lounge = setup();
    lounge.store.put(pending(false)).await;
    lounge
        .brain
        .script_notices(vec![Ok(ConversationReply::Rejected)]);
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    let state = lounge.store.state().await;
    assert!(!state.status.asked());
    assert_eq!(state.status.check_in_at, Some(NOW + CHECK_IN_SECONDS));
    assert_eq!(
        request_phase(&state),
        InviteRequestPhase::WaitingForCommunity
    );
    lounge.poll(NOW + 2 * CHECK_IN_SECONDS).await;
    assert_eq!(notices(&lounge.brain).len(), 1);
}

#[tokio::test]
async fn bitte_aus_altem_kanal_wird_ohne_versand_beendet() {
    for channel_id in [Some(1_289_721_245_281_292_291), None] {
        let lounge = setup();
        let mut state = pending(false);
        state.status.channel_id = channel_id;
        lounge.store.put(state).await;
        lounge.poll(NOW + 10).await;
        let state = lounge.store.state().await;
        assert_eq!(
            state.request.map(|request| request.phase),
            Some(Phase::Expired)
        );
        assert_eq!(
            state.last_result.as_deref(),
            Some("Bitte aus nicht mehr beobachtetem Kanal beendet")
        );
        assert!(notices(&lounge.brain).is_empty());
        assert_eq!(lounge.calls().await, 0);
    }
    let lounge = setup();
    let mut direct = pending(true);
    direct.status.channel_id = Some(1_289_721_245_281_292_291);
    lounge.store.put(direct).await;
    lounge.poll(NOW + 10).await;
    assert_eq!(lounge.calls().await, 1);
}

#[tokio::test]
async fn bereits_eingeladen_laut_steam_meldet_beim_check_in_ohne_einladung() {
    let lounge = setup();
    lounge.store.put(pending(false)).await;
    lounge.store.progress.lock().await.invited = true;
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    assert_eq!(
        updates(&lounge.brain),
        vec![InviteLoungeUpdate::Status {
            status: InviteStatus::AlreadyHasGame
        }]
    );
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn neustart_wiederholt_keinen_begonnenen_versuch() {
    let lounge = setup();
    let mut state = pending(true);
    if let Some(request) = state.request.as_mut() {
        request.phase = Phase::Attempted;
    }
    lounge.store.put(state).await;
    let restarted = watcher(
        lounge.store.clone(),
        lounge.port.clone(),
        lounge.invite.clone(),
        lounge.brain.clone(),
    );
    restarted
        .poll_due(NOW + 8000)
        .await
        .expect("Warteaufträge geprüft");
    assert_eq!(lounge.calls().await, 0);
}

fn dispatching(started_at_millis: Option<i64>) -> LoungeState {
    let mut state = pending(true);
    if let Some(request) = state.request.as_mut() {
        request.phase = Phase::Dispatching;
    }
    state.dispatch_started_at_millis = started_at_millis;
    state
}

#[tokio::test]
async fn haengender_versand_nach_neustart_prueft_steam_statt_neu_zu_senden() {
    for started in [Some(NOW * 1000 - DISPATCH_STALE_MILLIS), None] {
        let lounge = setup();
        lounge.store.put(dispatching(started)).await;
        lounge.poll(NOW).await;
        let state = lounge.store.state().await;
        assert_eq!(
            state.request.as_ref().map(|request| request.phase),
            Some(Phase::Pending)
        );
        assert_eq!(state.status.wait, Some(SteamWait::Delivery));
        lounge.poll(NOW).await;
        assert_eq!(
            lounge
                .store
                .state()
                .await
                .request
                .map(|request| request.phase),
            Some(Phase::Attempted)
        );
        assert_eq!(
            lounge.store.progress_queries.lock().await.clone(),
            vec![(NOW, NOW)]
        );
        assert_eq!(lounge.calls().await, 0);
        assert!(notices(&lounge.brain).is_empty());
    }
    let lounge = setup();
    lounge
        .store
        .put(dispatching(Some(NOW * 1000 - DISPATCH_STALE_MILLIS)))
        .await;
    *lounge.store.progress.lock().await = SteamProgress {
        outcome: Some(SteamOutcome::Sent),
        invited: true,
        open: false,
    };
    lounge.poll(NOW).await;
    lounge.poll(NOW).await;
    assert_eq!(updates(&lounge.brain), vec![sent()]);
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn laufender_versand_wird_vor_der_frist_nicht_angefasst() {
    let lounge = setup();
    let state = dispatching(Some(NOW * 1000 - DISPATCH_STALE_MILLIS + 1));
    lounge.store.put(state.clone()).await;
    lounge.poll(NOW).await;
    assert_eq!(lounge.store.state().await, state);
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn zwei_dispatcher_erhalten_nur_einen_versuch() {
    let lounge = Arc::new(setup());
    lounge.invite.hold.store(true, Ordering::SeqCst);
    lounge.store.put(pending(true)).await;
    let runner = lounge.clone();
    let first = tokio::spawn(async move { runner.watcher.dispatch_due(USER, NOW + 10).await });
    lounge.invite.started.notified().await;
    lounge
        .watcher
        .dispatch_due(USER, NOW + 10)
        .await
        .expect("zweiter Versuch");
    lounge.invite.release.notify_one();
    first
        .await
        .expect("Versandtask läuft")
        .expect("erster Versuch");
    assert_eq!(lounge.calls().await, 1);
    assert_eq!(lounge.invite.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ablauf_waehrend_load_oder_claim_verhindert_jeden_versand() {
    for point in [WaitPoint::Load, WaitPoint::Claim] {
        let mut lounge = setup();
        let clock = lounge.moving_clock();
        let mut state = pending(true);
        if let Some(request) = state.request.as_mut() {
            request.created_at = NOW - REQUEST_WINDOW_SECONDS;
        }
        lounge.store.put(state).await;
        *lounge.store.advance.lock().await = Some(ClockAdvance {
            point,
            clock,
            at: (NOW + 1) * 1000,
        });
        lounge
            .watcher
            .dispatch_due(USER, NOW)
            .await
            .expect("Abgelaufene Bitte wird abgeschlossen");
        assert_eq!(
            lounge
                .store
                .state()
                .await
                .request
                .map(|request| request.phase),
            Some(Phase::Expired)
        );
        assert_eq!(lounge.calls().await, 0);
    }
}

#[tokio::test]
async fn ablauf_waehrend_abschluss_speichert_den_versandstatus() {
    let mut lounge = setup();
    let clock = lounge.moving_clock();
    let lounge = Arc::new(lounge);
    lounge.invite.hold.store(true, Ordering::SeqCst);
    let mut state = pending(true);
    if let Some(request) = state.request.as_mut() {
        request.created_at = NOW - REQUEST_WINDOW_SECONDS;
    }
    lounge.store.put(state).await;
    let runner = lounge.clone();
    let dispatch = tokio::spawn(async move { runner.watcher.dispatch_due(USER, NOW).await });
    lounge.invite.started.notified().await;
    clock.store((NOW + 7200) * 1000, Ordering::SeqCst);
    lounge.invite.release.notify_one();
    dispatch
        .await
        .expect("Versandtask läuft")
        .expect("Versand abgeschlossen");
    assert_eq!(lounge.calls().await, 1);
    let state = lounge.store.state().await;
    assert_eq!(
        state.request.map(|request| request.phase),
        Some(Phase::Attempted)
    );
    assert!(state
        .last_result
        .expect("Versandstatus gespeichert")
        .starts_with("🎮"));
}

#[tokio::test]
async fn neue_bitte_waehrend_gehaltenem_versand_startet_keinen_zweiten() {
    let lounge = Arc::new(setup());
    lounge.invite.hold.store(true, Ordering::SeqCst);
    lounge.store.put(pending(true)).await;
    let runner = lounge.clone();
    let dispatch = tokio::spawn(async move { runner.watcher.dispatch_due(USER, NOW).await });
    lounge.invite.started.notified().await;
    lounge.decide(vec![wants(InviteTurnAction::Invite, None)]);
    lounge
        .post(
            message(2, NOW + 5, "Kannst du mich einladen? 123456789"),
            NOW + 5,
        )
        .await;
    lounge.invite.release.notify_one();
    dispatch
        .await
        .expect("Versandtask läuft")
        .expect("Versand abgeschlossen");
    lounge.poll(NOW + 600).await;
    assert_eq!(lounge.calls().await, 1);
}

#[tokio::test]
async fn statuswechsel_geht_genau_einmal_mit_allen_fakten_an_den_brain() {
    let lounge = setup();
    lounge.store.progress.lock().await.invited = true;
    lounge.store.put(pending(true)).await;
    lounge.poll(NOW + 600).await;
    lounge.poll(NOW + 900).await;
    let notices = notices(&lounge.brain);
    assert_eq!(notices.len(), 1);
    let (user, notice) = &notices[0];
    assert_eq!(*user, USER);
    assert_eq!(notice.idempotency_key, "invite-lounge:1:invite");
    assert!(notice.already_invited);
    assert_eq!(notice.answer, None);
    assert_eq!(
        notice.target,
        NoticeTarget::Reply {
            channel_id: CHECK_IN_CHANNEL_ID,
            message_id: 1
        }
    );
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn ohne_kanal_geht_die_meldung_als_dm() {
    let lounge = setup();
    lounge
        .store
        .put(LoungeState {
            outbox: vec![OutboxEntry::notice(1, None, Notice::Invite, false)],
            ..LoungeState::default()
        })
        .await;
    lounge
        .watcher
        .deliver_outbox(USER)
        .await
        .expect("Meldung zugestellt");
    let notices = notices(&lounge.brain);
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].1.target, NoticeTarget::DirectMessage);
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn alter_ausgangseintrag_wird_still_verworfen() {
    let lounge = setup();
    let retired: OutboxEntry = serde_json::from_str(
        r#"{"key":"invite-lounge:1:answer:2","request_message_id":1,"channel_id":5,"update":{"kind":"check_in_answer","outcome":"not_arrived"},"answer":"nein","already_invited":false}"#,
    )
    .expect("alter Eintrag");
    lounge
        .store
        .put(LoungeState {
            outbox: vec![retired],
            ..LoungeState::default()
        })
        .await;
    lounge
        .watcher
        .deliver_outbox(USER)
        .await
        .expect("Ausgang abgearbeitet");
    assert!(notices(&lounge.brain).is_empty());
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn fehlgeschlagene_meldung_bleibt_liegen_und_geht_genau_einmal_raus() {
    let mut lounge = setup();
    let clock = lounge.moving_clock();
    lounge
        .brain
        .script_notices(vec![Err(BrainError::Backend("Broker weg".into()))]);
    lounge.direct_request().await;
    let queued = lounge.outbox().await;
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].attempts, 1);
    lounge.poll(NOW + 30).await;
    assert_eq!(notices(&lounge.brain).len(), 1);
    clock.store((NOW + 61) * 1000, Ordering::SeqCst);
    for at in [NOW + 61, NOW + 3600] {
        lounge.poll(at).await;
    }
    let keys: Vec<_> = notices(&lounge.brain)
        .into_iter()
        .map(|(_, notice)| notice.idempotency_key)
        .collect();
    assert_eq!(keys, ["invite-lounge:1:invite", "invite-lounge:1:invite"]);
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn bereits_zugestellte_meldung_wird_nicht_doppelt_gesendet() {
    let mut lounge = setup();
    let clock = lounge.moving_clock();
    lounge
        .brain
        .script_notices(vec![Err(BrainError::Backend("Zeitüberschreitung".into()))]);
    lounge.direct_request().await;
    let mut delivered = from(BOT, message(5, NOW + 1, "<@7> Einladung ist raus."));
    delivered.reply_message_id = Some(1);
    lounge.port.history.lock().await.push(delivered);
    clock.store((NOW + 120) * 1000, Ordering::SeqCst);
    lounge.poll(NOW + 120).await;
    assert_eq!(notices(&lounge.brain).len(), 1);
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn fremde_bot_antwort_zaehlt_nicht_als_zugestellte_meldung() {
    let mut lounge = setup();
    let clock = lounge.moving_clock();
    lounge
        .brain
        .script_notices(vec![Err(BrainError::Backend("Zeitüberschreitung".into()))]);
    lounge.direct_request().await;
    let mut hint = from(
        BOT,
        message(5, NOW + 1, "Den Freundescode findest du in Steam."),
    );
    hint.reply_message_id = Some(1);
    lounge.port.history.lock().await.push(hint);
    clock.store((NOW + 120) * 1000, Ordering::SeqCst);
    lounge.poll(NOW + 120).await;
    assert_eq!(notices(&lounge.brain).len(), 2);
    assert!(lounge.outbox().await.is_empty());
}

#[tokio::test]
async fn nicht_verfuegbares_brain_verliert_keine_meldung() {
    let mut lounge = setup();
    let clock = lounge.moving_clock();
    let failures = (0..13)
        .map(|_| {
            Err(BrainError::Backend(
                "discord_invite_lounge unavailable".into(),
            ))
        })
        .collect();
    lounge.brain.script_notices(failures);
    lounge.direct_request().await;
    for hour in 1..=12 {
        let at = NOW + hour * 3600;
        clock.store(at * 1000, Ordering::SeqCst);
        lounge.poll(at).await;
    }
    assert_eq!(lounge.outbox().await.len(), 1);
    clock.store((NOW + 13 * 3600) * 1000, Ordering::SeqCst);
    lounge.poll(NOW + 13 * 3600).await;
    assert_eq!(notices(&lounge.brain).len(), 14);
    assert!(lounge.outbox().await.is_empty());
}

#[test]
fn frischefenster_hat_eine_obere_und_untere_grenze() {
    assert!(is_fresh(NOW, NOW, 10));
    assert!(is_fresh(NOW - 10, NOW, 10));
    assert!(!is_fresh(NOW - 11, NOW, 10));
    assert!(!is_fresh(NOW + 1, NOW, 10));
}

#[test]
fn freundescode_ignoriert_links() {
    assert_eq!(friend_code("mein code 123456789"), Some(CODE.into()));
    assert_eq!(
        friend_code("https://steamcommunity.com/profiles/123456789 bitte"),
        None
    );
    assert_eq!(friend_code("12345"), None);
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
    let lounge = setup();
    let watcher = watcher(
        store.clone(),
        lounge.port.clone(),
        lounge.invite.clone(),
        lounge.brain.clone(),
    );
    let mut state = pending(false);
    if let Some(request) = state.request.as_mut() {
        request.created_at = NOW - REQUEST_WINDOW_SECONDS - 1;
    }
    assert_eq!(
        store
            .compare_exchange(USER, None, &state)
            .await
            .expect("Zustand schreibbar"),
        Cas::Written
    );
    let waiting = waiting_for_code(NOW - REQUEST_WINDOW_SECONDS - 1, Some(CHECK_IN_CHANNEL_ID));
    assert_eq!(
        store
            .compare_exchange(OTHER, None, &waiting)
            .await
            .expect("Zustand schreibbar"),
        Cas::Written
    );
    let mut open = store.pending_users().await.expect("offene Bitten lesbar");
    open.sort_unstable();
    assert_eq!(open, vec![USER, OTHER]);
    watcher.poll_due(NOW).await.expect("Warteaufträge geprüft");
    for user_id in [USER, OTHER] {
        assert_eq!(
            store
                .load(user_id)
                .await
                .expect("Zustand lesbar")
                .and_then(|state| state.request)
                .map(|request| request.phase),
            Some(Phase::Expired)
        );
    }
    assert!(store
        .pending_users()
        .await
        .expect("offene Bitten lesbar")
        .is_empty());
    assert_eq!(lounge.calls().await, 0);
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
    let steam_id = steam_id64(CODE).expect("gültiger Freundescode");
    assert!(!store.handled_since(CODE, NOW).await.expect("Abfrage läuft"));
    sqlx::query(
        "INSERT INTO steam.invite_requests (steam_id64, account_id, admin_id, created_at)
         VALUES ($1, 123456789, 7, $2)",
    )
    .bind(steam_id)
    .bind(NOW + 5)
    .execute(db.pool())
    .await
    .expect("Testzeile");
    assert!(store.handled_since(CODE, NOW).await.expect("Abfrage läuft"));
    assert!(!store
        .handled_since(CODE, NOW + 10)
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
        .handled_since(CODE, NOW + 10)
        .await
        .expect("Abfrage läuft"));
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn postgres_zustand_claim_bekannter_code_und_opt_out() {
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("Testdatenbank");
    let store = PgLoungeStore {
        pool: db.pool().clone(),
    };
    let mut state = pending(false);
    assert_eq!(
        store
            .compare_exchange(USER, None, &state)
            .await
            .expect("Zustand schreibbar"),
        Cas::Written
    );
    assert_eq!(
        store
            .compare_exchange(USER, None, &state)
            .await
            .expect("Zustand schreibbar"),
        Cas::Stale
    );
    assert_eq!(
        store.pending_users().await.expect("offene Bitten lesbar"),
        vec![USER]
    );
    let previous = state.clone();
    if let Some(request) = state.request.as_mut() {
        request.phase = Phase::Attempted;
    }
    let (a, b) = tokio::join!(
        store.compare_exchange(USER, Some(&previous), &state),
        store.compare_exchange(USER, Some(&previous), &state)
    );
    assert_eq!(
        usize::from(a.expect("erster Claim").written())
            + usize::from(b.expect("zweiter Claim").written()),
        1
    );
    assert!(store
        .pending_users()
        .await
        .expect("offene Bitten lesbar")
        .is_empty());
    assert_eq!(store.load(USER).await.expect("Zustand lesbar"), Some(state));
    sqlx::query("INSERT INTO core.users (discord_id) VALUES ($1)")
        .bind(USER as i64)
        .execute(db.pool())
        .await
        .expect("Testnutzer");
    sqlx::query("INSERT INTO core.steam_links (discord_id, steam_id, primary_account, verified) VALUES ($1, '76561198083722517', true, false)")
        .bind(USER as i64)
        .execute(db.pool())
        .await
        .expect("Testverknüpfung");
    assert_eq!(store.known_code(USER).await.expect("Code lesbar"), None);
    sqlx::query("UPDATE core.steam_links SET verified = TRUE WHERE discord_id = $1")
        .bind(USER as i64)
        .execute(db.pool())
        .await
        .expect("Verknüpfung bestätigt");
    assert_eq!(
        store.known_code(USER).await.expect("Code lesbar"),
        Some(CODE.into())
    );
    assert!(!store.opted_out(USER).await);
    assert!(store.opted_out(u64::MAX).await);
    sqlx::query(
        "INSERT INTO core.user_privacy (user_id, opted_out) VALUES ($1, TRUE)
         ON CONFLICT (user_id) DO UPDATE SET opted_out = TRUE",
    )
    .bind(USER as i64)
    .execute(db.pool())
    .await
    .expect("Opt-out gesetzt");
    let current = store.load(USER).await.expect("Zustand lesbar");
    assert_eq!(
        store
            .compare_exchange(USER, current.as_ref(), &pending(false))
            .await
            .expect("Schreibversuch"),
        Cas::OptedOut
    );
    assert_eq!(
        store
            .hidden_authors(&[USER, OTHER, u64::MAX])
            .await
            .expect("Datenschutz lesbar"),
        HashSet::from([USER, u64::MAX])
    );
}

fn waiting_for_code(created_at: i64, channel_id: Option<u64>) -> LoungeState {
    LoungeState {
        last_seen: 2,
        request: Some(Request {
            message_id: 2,
            created_at,
            direct: false,
            phase: Phase::WaitingForCode,
        }),
        status: RequestStatus {
            channel_id,
            ..RequestStatus::default()
        },
        ..Default::default()
    }
}

fn phase(state: &LoungeState) -> Option<Phase> {
    state.request.as_ref().map(|request| request.phase)
}

fn has_access(text: Option<&str>) -> ScriptedReply {
    decided(InviteIntent::HasAccess, InviteTurnAction::Close, text)
}

#[tokio::test]
async fn abgelehnte_brain_nachfrage_gibt_den_claim_frei_und_der_timer_fragt_spaeter() {
    let lounge = setup();
    lounge.store.put(pending(false)).await;
    lounge
        .brain
        .script_notices(vec![Ok(ConversationReply::Rejected)]);
    lounge.decide(vec![wants(
        InviteTurnAction::AskAlreadyInvited,
        Some("Hast du schon eine Einladung bekommen?"),
    )]);
    lounge
        .post(message(2, NOW + 60, "kommt da noch was?"), NOW + 60)
        .await;
    let state = lounge.store.state().await;
    assert!(!state.status.asked());
    assert_eq!(state.status.check_in_at, None);
    assert!(state.outbox.is_empty());
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    lounge.poll(NOW + 2 * CHECK_IN_SECONDS).await;
    lounge.poll(NOW + 3 * CHECK_IN_SECONDS).await;
    assert_eq!(
        updates(&lounge.brain),
        vec![InviteLoungeUpdate::Reply, InviteLoungeUpdate::CheckIn]
    );
    assert!(lounge.store.state().await.status.asked());
    assert_eq!(lounge.calls().await, 0);
}

#[test]
fn nicht_zugestellte_eintraege_geben_ihren_claim_frei() {
    let claim = Some(Claim {
        notice: Notice::CheckIn,
        request_message_id: 1,
    });
    let given_up = |result| {
        let mut state = pending(false);
        state.status.claim(Some(Notice::CheckIn));
        state.status.check_in_at = Some(NOW);
        let mut entry =
            OutboxEntry::reply(2, CHECK_IN_CHANNEL_ID, "Schon da?".into(), false, claim);
        entry.first_attempt_at_millis = Some(0);
        state.outbox = vec![entry];
        settle_notice(&mut state, 0, result, NOTICE_MAX_AGE_MILLIS + 1);
        state
    };
    for result in [NoticeResult::Dropped, NoticeResult::Retry] {
        let state = given_up(result);
        assert!(state.outbox.is_empty());
        assert!(!state.status.asked());
        assert_eq!(state.status.check_in_at, None);
    }
    let delivered = given_up(NoticeResult::Delivered);
    assert!(delivered.status.asked());
    assert_eq!(delivered.status.check_in_at, Some(NOW));
    let mut state = pending(false);
    state.status.claim(Some(Notice::FriendRequest));
    state.outbox = vec![OutboxEntry::notice(
        1,
        Some(CHECK_IN_CHANNEL_ID),
        Notice::FriendRequest,
        false,
    )];
    settle_notice(&mut state, 0, NoticeResult::Dropped, NOW * 1000);
    assert!(state.status.notices.is_empty());
}

#[tokio::test]
async fn bestaetigung_schliesst_auch_waehrend_steam_wartet_oder_versendet() {
    let lounge = setup();
    let mut waiting = pending(true);
    waiting.status.wait = Some(SteamWait::Delivery);
    waiting.status.check_at = Some(NOW + 60);
    lounge.store.put(waiting).await;
    *lounge.store.progress.lock().await = SteamProgress {
        outcome: Some(SteamOutcome::Sent),
        invited: true,
        open: false,
    };
    lounge.decide(vec![has_access(Some("Super, viel Spaß!"))]);
    lounge
        .post(message(2, NOW + 10, "passt, ist angekommen"), NOW + 10)
        .await;
    lounge.poll(NOW + 600).await;
    let state = lounge.store.state().await;
    assert_eq!(phase(&state), Some(Phase::Attempted));
    assert_eq!(state.status.wait, None);
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::Reply]);
    assert!(lounge.store.progress_queries.lock().await.is_empty());
    assert_eq!(lounge.calls().await, 0);

    let lounge = setup();
    lounge.store.put(dispatching(Some(NOW * 1000))).await;
    lounge.decide(vec![has_access(None)]);
    lounge
        .post(message(2, NOW + 5, "hab sie schon, danke"), NOW + 5)
        .await;
    lounge
        .watcher
        .finish_dispatch(
            USER,
            1,
            Step::Finish(Some(Notice::Invite)),
            "🎮 Einladung angestoßen",
        )
        .await
        .expect("Versandabschluss läuft");
    lounge.poll(NOW + 600).await;
    let state = lounge.store.state().await;
    assert_eq!(phase(&state), Some(Phase::Attempted));
    assert_eq!(updates(&lounge.brain), vec![Notice::Confirmed.update()]);
    assert_eq!(lounge.calls().await, 0);
}

async fn decide_while_timer_runs(
    state: LoungeState,
    reply: ScriptedReply,
    handled: bool,
) -> Arc<Lounge> {
    let lounge = Arc::new(setup());
    lounge.store.put(state).await;
    lounge.store.handled.store(handled, Ordering::SeqCst);
    lounge.decide(vec![reply]);
    let (started, release) = lounge.brain.gate_first_call();
    let runner = lounge.clone();
    let turn = tokio::spawn(async move {
        runner
            .post(
                message(2, NOW + CHECK_IN_SECONDS, "kommt da noch was? 123456789"),
                NOW + CHECK_IN_SECONDS,
            )
            .await
    });
    started.notified().await;
    lounge.poll(NOW + CHECK_IN_SECONDS).await;
    release.notify_one();
    turn.await.expect("Nachricht verarbeitet");
    lounge
}

#[tokio::test]
async fn veraltete_entscheidung_wird_gegen_den_neuen_stand_verworfen() {
    let lounge = decide_while_timer_runs(
        pending(false),
        wants(
            InviteTurnAction::AskAlreadyInvited,
            Some("Hast du schon eine Einladung bekommen?"),
        ),
        false,
    )
    .await;
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::CheckIn]);
    let state = lounge.store.state().await;
    assert_eq!(state.last_seen, 2);
    assert!(state.outbox.is_empty());

    let lounge =
        decide_while_timer_runs(pending(false), wants(InviteTurnAction::Invite, None), true).await;
    let state = lounge.store.state().await;
    assert_eq!(phase(&state), Some(Phase::Attempted));
    assert_eq!(state.last_seen, 2);
    assert_eq!(lounge.calls().await, 0);
    assert!(notices(&lounge.brain).is_empty());

    let lounge = decide_while_timer_runs(pending(false), has_access(Some("Super!")), false).await;
    let state = lounge.store.state().await;
    assert_eq!(phase(&state), Some(Phase::Attempted));
    assert_eq!(
        updates(&lounge.brain),
        vec![InviteLoungeUpdate::CheckIn, InviteLoungeUpdate::Reply]
    );
    assert_eq!(lounge.calls().await, 0);
}

#[tokio::test]
async fn wartende_codebitte_laeuft_ab_auch_aus_altem_kanal() {
    for (created_at, channel_id, reason) in [
        (
            NOW - REQUEST_WINDOW_SECONDS - 1,
            Some(CHECK_IN_CHANNEL_ID),
            "Verspätete Bitte ohne Versand beendet",
        ),
        (
            NOW,
            Some(1_289_721_245_281_292_291),
            "Bitte aus nicht mehr beobachtetem Kanal beendet",
        ),
        (NOW, None, "Bitte aus nicht mehr beobachtetem Kanal beendet"),
    ] {
        let lounge = setup();
        lounge
            .store
            .put(waiting_for_code(created_at, channel_id))
            .await;
        lounge.poll(NOW).await;
        let state = lounge.store.state().await;
        assert_eq!(phase(&state), Some(Phase::Expired));
        assert_eq!(state.last_result.as_deref(), Some(reason));
        assert!(notices(&lounge.brain).is_empty());
    }
    let lounge = setup();
    let fresh = waiting_for_code(NOW, Some(CHECK_IN_CHANNEL_ID));
    lounge.store.put(fresh.clone()).await;
    lounge.poll(NOW + 60).await;
    assert_eq!(lounge.store.state().await, fresh);
}

#[tokio::test]
async fn bitte_ohne_gueltigen_code_endet_einmal_ohne_versand() {
    for code in [None, Some(STEAM_BOT_FRIEND_CODE.to_string())] {
        let lounge = setup();
        let mut state = pending(true);
        state.friend_code = code;
        lounge.store.put(state).await;
        lounge.poll(NOW + 10).await;
        let expired = lounge.store.state().await;
        assert_eq!(phase(&expired), Some(Phase::Expired));
        assert_eq!(
            expired.last_result.as_deref(),
            Some("Bitte ohne gültigen Freundescode beendet")
        );
        lounge.poll(NOW + 30).await;
        assert_eq!(lounge.store.state().await, expired);
        assert!(lounge
            .store
            .pending_users()
            .await
            .expect("lesbar")
            .is_empty());
        assert_eq!(lounge.calls().await, 0);
    }
}

#[tokio::test]
async fn ungueltiger_code_laut_steam_wird_vergessen() {
    let lounge = setup();
    *lounge.invite.reply.lock().await =
        Some("❌ Mit diesem Freundescode finde ich niemanden.".into());
    lounge.direct_request().await;
    let state = lounge.store.state().await;
    assert_eq!(state.friend_code, None);
    assert_eq!(phase(&state), Some(Phase::Attempted));
    assert_eq!(updates(&lounge.brain), vec![Notice::InvalidCode.update()]);
    lounge.decide(vec![Ok(ConversationReply::Silent)]);
    lounge
        .post(message(2, NOW + 60, "und jetzt?"), NOW + 60)
        .await;
    assert!(!turns(&lounge.brain)[1].member.has_friend_code);
}

#[tokio::test]
async fn ja_ohne_offene_bitte_antwortet_nur_mit_text() {
    let lounge = setup();
    lounge.decide(vec![has_access(Some("Schön, dann viel Spaß!"))]);
    lounge
        .post(message(1, NOW, "hab Deadlock schon, danke"), NOW)
        .await;
    assert_eq!(updates(&lounge.brain), vec![InviteLoungeUpdate::Reply]);
    assert_eq!(lounge.store.state().await.request, None);

    let lounge = setup();
    lounge.decide(vec![has_access(None)]);
    lounge
        .post(message(1, NOW, "hab Deadlock schon, danke"), NOW)
        .await;
    assert!(notices(&lounge.brain).is_empty());
    assert_eq!(
        lounge.store.state().await,
        LoungeState {
            last_seen: 1,
            ..Default::default()
        }
    );
}

#[test]
fn freundescode_prueft_bereich_und_bot_code() {
    assert_eq!(friend_code("4294967296"), None);
    assert_eq!(friend_code("4294967295"), Some("4294967295".into()));
    assert_eq!(friend_code(&STEAM_BOT_FRIEND_CODE.to_string()), None);
    assert_eq!(
        friend_code(&format!("Bot {STEAM_BOT_FRIEND_CODE}, meiner 123456789")),
        Some(CODE.into())
    );
    assert_eq!(friend_code("0000000000"), None);
    let bot_code = STEAM_BOT_FRIEND_CODE.to_string();
    let mut stored = pending(false);
    stored.friend_code = Some(bot_code.clone());
    let linked = MemberFacts {
        linked_code: Some(CODE.into()),
        ..MemberFacts::default()
    };
    assert_eq!(linked.code(Some(&stored)), Some(CODE.into()));
    assert_eq!(linked.known_code(Some(&stored)), Some(CODE.into()));
    let typed = MemberFacts {
        message_code: Some("111222333".into()),
        linked_code: Some("444555666".into()),
        invited: false,
    };
    assert_eq!(typed.code(Some(&pending(false))), Some("111222333".into()));
    assert_eq!(typed.known_code(Some(&pending(false))), Some(CODE.into()));
    assert_eq!(typed.known_code(None), Some("444555666".into()));
    assert_eq!(MemberFacts::default().code(Some(&stored)), None);
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn postgres_alte_einladung_zaehlt_nicht_fuer_die_neue_bitte() {
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("Testdatenbank");
    let store = PgLoungeStore {
        pool: db.pool().clone(),
    };
    let steam_id = steam_id64(CODE).expect("gültiger Freundescode");
    let audit = |at: i64| {
        sqlx::query(
            "INSERT INTO steam.beta_invite_audit (discord_id, steam_id64, steam_profile, invited_at)
             VALUES ($1, $2, 'profil', to_timestamp($3))",
        )
        .bind(USER as i64)
        .bind(steam_id)
        .bind(at as f64)
    };
    audit(NOW - 3600)
        .execute(db.pool())
        .await
        .expect("alte Einladung");
    let old = store
        .steam_progress(CODE, NOW, NOW)
        .await
        .expect("Steam-Stand lesbar");
    assert!(!old.invited);
    assert!(store.invited(CODE).await.expect("Einladungsstand lesbar"));
    assert_eq!(after_progress(SteamWait::Retry, old, NOW), Step::Dispatch);
    audit(NOW + 5)
        .execute(db.pool())
        .await
        .expect("neue Einladung");
    let fresh = store
        .steam_progress(CODE, NOW + 10, NOW)
        .await
        .expect("Steam-Stand lesbar");
    assert!(fresh.invited);
    assert_eq!(
        after_progress(SteamWait::Retry, fresh, NOW),
        Step::Finish(Some(Notice::Invite))
    );
}
