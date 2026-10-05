use super::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Mutex;

const NOW: i64 = 1_790_000_000;
const BOT: u64 = 99;
const GUILD: u64 = 42;
const USER: u64 = 7;
const AUDIT_REPLY: &str = "ℹ️ Dieser Steam-Account wurde schon eingeladen, am <t:1700000000:D>. Hier ist nichts mehr zu tun.";
const RUNNING_REPLY: &str = "⏳ Für diesen Steam-Account läuft gerade schon eine Einladung. Ich schicke sie nicht doppelt, sonst zählt sie zweimal gegen das Tageslimit.";

#[derive(Default)]
struct MemoryStore {
    states: Mutex<HashMap<u64, LoungeState>>,
    known: Mutex<HashMap<u64, String>>,
    legacy_hints: Mutex<HashMap<u64, i64>>,
    backfilled: AtomicBool,
}

#[async_trait::async_trait]
impl LoungeStore for MemoryStore {
    async fn load(&self, id: u64) -> Result<Option<LoungeState>, String> {
        Ok(self.states.lock().await.get(&id).cloned())
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
        clock_millis: || NOW * 1000,
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
        .unwrap();
    assert_eq!(
        invite.calls.lock().await.as_slice(),
        &[(BOT, GUILD, "123456789".into(), USER)]
    );
    assert_eq!(
        port.replies.lock().await.as_slice(),
        &[(1, "Die Steam-Einladung ist raus.".into())]
    );
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
    assert_eq!(
        request_kind("Mich einladen? Code 1313436779", BOT),
        Some(RequestKind::Room)
    );
}

#[test]
fn erklaerungsfragen_zu_einladungen_sind_keine_versandbitten() {
    for content in [
        "<@42> Kannst du mir erklären, warum ich mit Code 1313436779 niemanden einladen kann?",
        "<@!42> Kannst du mir erklaeren, wie ich mit Code 1313436779 jemanden einladen kann?",
        "<@42> Kannst du mir erklären, ob ich mit Code 1313436779 jemanden einladen darf?",
        "<@42> Kannst du mir erlaeutern, ob ich Code 1313436779 zum Einladen brauche?",
        "Kann mir jemand erläutern, wie eine Einladung mit Code 1313436779 funktioniert?",
        "<@42> Kannst du mich über Einladungen mit Code 1313436779 informieren?",
        "<@42> Kannst du mir sagen, warum ich mit Code 1313436779 niemanden einladen kann?",
        "<@42> Kannst du mir sagen, wie ich mit Code 1313436779 jemanden einladen kann?",
        "<@42> Kannst du mir sagen, wieso ich mit Code 1313436779 niemanden einladen kann?",
        "<@42> Kannst du mir sagen, weshalb ich mit Code 1313436779 niemanden einladen kann?",
        "<@42> Kannst du mir sagen, wann du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Kannst du mir sagen, wann du mich mit Code 1313436779 einladen kannst?",
        "<@42> Kannst du mir sagen, wo du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Kannst du mir sagen, wo du mich mit Code 1313436779 einladen kannst?",
        "<@42> Kannst du mir sagen, was ich tun muss, damit du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Kannst du mir sagen, was ich tun muss, damit du mich mit Code 1313436779 einladen kannst?",
        "<@42> Kannst du mir sagen, wer mich mit Code 1313436779 einladen kann?",
        "<@!42> Kannst du mir sagen, wer mich mit Code 1313436779 einladen kann?",
        "<@42> Kannst du mir erklären, wie ich mich einladen lassen kann? Code 1313436779",
        "<@42> Kannst du mir sagen, wie lange es dauert, bis du mich mit Code 1313436779 einladen kannst?",
        "<@42> Wie besprochen, kannst du mir erklären, wie ich mich einladen lassen kann? Code 1313436779",
        "<@42> Kannst du mir die Einladung erklären? Code 1313436779",
        "<@42> Kannst du mir den Invite erklären? Code 1313436779",
        "<@42> Kannst du mich über das Einladen informieren? Code 1313436779",
        "<@42> Kannst du mir deinen Botinvite erklären? Code 1313436779",
        "<@42> Kannst du mir eine Erklärung zum Einladen geben? Code 1313436779",
        "<@42> Gib mir bitte eine Erklärung zum Einladen. Code 1313436779",
        "<@42> Kannst du mir erklären, warum ich jemanden einladen kann und mich einladen lassen kann? Code 1313436779",
        "<@!42> Erklär mir, wie du mich einladen kannst und wie Einladungen funktionieren. Code 1313436779",
        "<@42> Sag mir, wie Einladungen funktionieren und ob du mich einladen kannst. Code 1313436779",
    ] {
        assert_eq!(
            request_kind(content, 42),
            Some(RequestKind::Information),
            "{content}"
        );
    }
}

#[test]
fn prozessfragen_zu_einladungen_sind_information() {
    for content in [
        "Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "<@42> Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "Wie kann <@42> mich mit Code 1313436779 einladen?",
        "Wie kann <@!42> mich mit Code 1313436779 einladen?",
        "<@42> Wie lange dauert es, bis du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Wie lange dauert es, bis du mich mit Code 1313436779 einladen kannst?",
        "<@42> Wie funktioniert das Einladen mit Code 1313436779?",
        "<@!42> Wie funktioniert das Einladen mit Code 1313436779?",
        "<@42> Erklär mir, wie z.B. du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Erklär mir, wie z.B. du mich mit Code 1313436779 einladen kannst?",
        "<@42> Wie klappt es, dass du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Wie klappt es, dass du mich mit Code 1313436779 einladen kannst?",
        "<@42> Erklär mir, wie z. B. du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Erklär mir, wie z. B. du mich mit Code 1313436779 einladen kannst?",
        "<@42> Erklär mir, wie i.d.R. du mich mit Code 1313436779 einladen kannst?",
        "<@!42> Erklär mir, wie i.d.R. du mich mit Code 1313436779 einladen kannst?",
        "<@42> Wie kann ich mich mit Code 1313436779 einladen lassen?",
        "<@!42> Wie könnte ich mich mit Code 1313436779 einladen lassen?",
        "Wie soll ich mich mit Code 1313436779 einladen lassen?",
        "Wie lade ich mit Code 1313436779 jemanden ein?",
        "Wie nutze ich Code 1313436779, um mich einladen zu lassen?",
        "Wie nutzen wir Code 1313436779, um uns einladen zu lassen?",
        "Wie kannst du mich mit Code 1313436779 einladen?",
        "Wie kann man mich mit Code 1313436779 einladen?",
        "Warum kannst du mich mit Code 1313436779 nicht einladen?",
        "Wieso kannst du mich mit Code 1313436779 nicht einladen?",
        "Weshalb kannst du mich mit Code 1313436779 nicht einladen?",
        "Wo kann ich mich mit Code 1313436779 einladen lassen?",
        "Wann kannst du mich mit Code 1313436779 einladen?",
        "Was muss ich tun, um mich mit Code 1313436779 einladen zu lassen?",
        "<@42> Kann ich mich mit Code 1313436779 einladen lassen?",
        "<@42> Muss ich mich mit Code 1313436779 einladen lassen?",
        "<@42> Darf ich mir mit Code 1313436779 jemanden einladen?",
        "Können wir uns mit Code 1313436779 einladen lassen?",
        "Soll man mich mit Code 1313436779 einladen?",
    ] {
        assert_eq!(
            request_kind(content, 42),
            Some(RequestKind::Information),
            "{content}"
        );
    }
    for content in [
        "Kann mich jemand mit Code 1313436779 einladen?",
        "Wer lädt mich ein? Code 1313436779",
    ] {
        assert_eq!(
            request_kind(content, 42),
            Some(RequestKind::Room),
            "{content}"
        );
    }
}

#[test]
fn direkte_bitte_mit_mejn_bleibt_eine_versandbitte() {
    for content in [
        "<@42> kannst du mich einladen mejn Code ist 1313436779",
        "<@42> kannst du mich einladen mein Code ist 1313436779",
        "<@42> Kannst du mich wie die anderen einladen? mejn Code ist 1313436779",
        "<@42> Wie wäre es, wenn du mich einladen würdest? 1313436779",
        "<@!42> Wie wäre es, wenn du mich einladen würdest? Code 1313436779",
        "<@42> Wie wäre es wenn du mich mit Code 1313436779 einladen würdest?",
        "<@!42> Wie wäre es wenn du mich mit Code 1313436779 einladen würdest?",
    ] {
        assert_eq!(
            request_kind(content, 42),
            Some(RequestKind::Direct),
            "{content}"
        );
        assert_eq!(friend_code(content).as_deref(), Some("1313436779"));
    }
}

#[tokio::test]
async fn eindeutige_bitte_bleibt_mit_zusaetzlicher_auskunft_ein_versandauftrag() {
    for mention in ["<@99>", "<@!99>"] {
        for request in [
            "Bitte lade mich ein. Danke für die Erklärung",
            "Bitte lade mich ein und sag mir, wie lange es dauert",
            "Wie besprochen, du kannst mich jetzt einladen",
            "Wie wäre es, wenn du mich einladen würdest?",
            "Wie wäre es wenn du mich einladen würdest?",
            "Danke für die Erklärung. Bitte lade mich ein",
            "Kannst du mir erklären, wie Einladungen funktionieren? Bitte lade mich ein",
            "Wie kann ich eine Einladung bekommen, und kannst du mich einladen?",
            "Erklär mir Einladungen und lade mich ein",
            "Sag mir, wie Einladungen funktionieren, und lade mich ein",
            "Wie funktioniert das Einladen. Bitte lade mich ein",
            "Erklär mir, wie z.B. du mich einladen kannst. Bitte lade mich ein",
            "Erklär mir, wie z. B. du mich einladen kannst. Bitte lade mich ein",
            "Wie kann ich mich einladen lassen! Bitte lade mich ein",
        ] {
            let content = format!("{mention} {request}. Code 1313436779");
            assert_eq!(
                request_kind(&content, BOT),
                Some(RequestKind::Direct),
                "{content}"
            );
            let (watcher, _, port, invite) = setup();
            watcher
                .handle_message(message(1, NOW, &content), NOW, Some(false))
                .await
                .expect("Die eindeutige Bitte muss sofort versendet werden");
            assert_eq!(
                invite.calls.lock().await.as_slice(),
                &[(BOT, GUILD, "1313436779".into(), USER)],
                "{content}"
            );
            assert_eq!(
                port.replies.lock().await.as_slice(),
                &[(1, "Die Steam-Einladung ist raus.".into())],
                "{content}"
            );
        }
    }
    for content in [
        "Wie kann <@99> mich mit Code 1313436779 einladen! Bitte lade mich ein",
        "Wie kann <@!99> mich mit Code 1313436779 einladen! Bitte lade mich ein",
        "<@99> Wie kann <@!> mich mit Code 1313436779 einladen?",
        "<@99> Wie kann <@!name> mich mit Code 1313436779 einladen?",
        "<@99> Wie kann <@!99name> mich mit Code 1313436779 einladen?",
        "<@99> Wie kann <@!99 > mich mit Code 1313436779 einladen?",
        "<@99> Wie kann <@!９９> mich mit Code 1313436779 einladen?",
        "<@99> Wie kann <@!99 mich mit Code 1313436779 einladen?",
    ] {
        assert_eq!(
            request_kind(content, BOT),
            Some(RequestKind::Direct),
            "{content}"
        );
    }
    assert_eq!(
        request_kind(
            "Wie kann ich mich einladen lassen? Kann mich jemand einladen? Code 1313436779",
            BOT,
        ),
        Some(RequestKind::Room),
    );
}

#[tokio::test]
async fn erklaerungsfrage_mit_code_loest_keinen_invite_aus() {
    for content in [
        "<@99> Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "Wie kann <@99> mich mit Code 1313436779 einladen?",
        "Wie kann <@!99> mich mit Code 1313436779 einladen?",
        "<@99> Wie lange dauert es, bis du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Wie lange dauert es, bis du mich mit Code 1313436779 einladen kannst?",
        "<@99> Wie funktioniert das Einladen mit Code 1313436779?",
        "<@!99> Wie funktioniert das Einladen mit Code 1313436779?",
        "<@99> Erklär mir, wie z.B. du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Erklär mir, wie z.B. du mich mit Code 1313436779 einladen kannst?",
        "<@99> Wie klappt es, dass du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Wie klappt es, dass du mich mit Code 1313436779 einladen kannst?",
        "<@99> Erklär mir, wie z. B. du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Erklär mir, wie z. B. du mich mit Code 1313436779 einladen kannst?",
        "<@99> Erklär mir, wie i.d.R. du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Erklär mir, wie i.d.R. du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir erklären, warum ich mit Code 1313436779 niemanden einladen kann?",
        "<@99> Kannst du mir sagen, wann du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Kannst du mir sagen, wann du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir sagen, wo du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Kannst du mir sagen, wo du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir sagen, was ich tun muss, damit du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Kannst du mir sagen, was ich tun muss, damit du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir sagen, wer mich mit Code 1313436779 einladen kann?",
        "<@!99> Kannst du mir sagen, wer mich mit Code 1313436779 einladen kann?",
    ] {
        let (watcher, _, port, invite) = setup();
        watcher
            .handle_message(message(1, NOW, content), NOW, Some(false))
            .await
            .expect("Die Informationsfrage muss verarbeitet werden können");
        watcher
            .poll_due(NOW + 3600)
            .await
            .expect("Die Prüfung fälliger Einladungen muss gelingen");
        assert!(invite.calls.lock().await.is_empty(), "{content}");
        assert!(port.replies.lock().await.is_empty(), "{content}");
    }
}

#[test]
fn informationsfrage_veraendert_den_lounge_zustand_nicht() {
    let mut state = LoungeState {
        last_seen: 1,
        friend_code: Some("123456789".into()),
        request: Some(Request {
            message_id: 1,
            created_at: NOW,
            direct: true,
            phase: Phase::WaitingForCode,
        }),
        ..Default::default()
    };
    let previous = state.clone();
    assert!(!update_state(
        &mut state,
        &message(
            2,
            NOW + 1,
            "Kannst du mir erklären, warum ich niemanden einladen kann?"
        ),
        Some(RequestKind::Information),
        Some("1313436779".into()),
        true,
        NOW + 1,
        (NOW + 1) * 1000,
    ));
    assert_eq!(state, previous);
}

#[tokio::test]
async fn informationsfrage_mit_code_erfuellt_offene_lounge_nachfrage_nicht() {
    for content in [
        "Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "<@99> Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Wie lange, bis du mich mit Code 1313436779 einladen kannst?",
        "Wie kann <@99> mich mit Code 1313436779 einladen?",
        "Wie kann <@!99> mich mit Code 1313436779 einladen?",
        "<@99> Wie lange dauert es, bis du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Wie lange dauert es, bis du mich mit Code 1313436779 einladen kannst?",
        "<@99> Wie funktioniert das Einladen mit Code 1313436779?",
        "<@!99> Wie funktioniert das Einladen mit Code 1313436779?",
        "<@99> Erklär mir, wie z.B. du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Erklär mir, wie z.B. du mich mit Code 1313436779 einladen kannst?",
        "<@99> Wie klappt es, dass du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Wie klappt es, dass du mich mit Code 1313436779 einladen kannst?",
        "<@99> Erklär mir, wie z. B. du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Erklär mir, wie z. B. du mich mit Code 1313436779 einladen kannst?",
        "<@99> Erklär mir, wie i.d.R. du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Erklär mir, wie i.d.R. du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir erklären, warum ich mit Code 1313436779 niemanden einladen kann?",
        "<@99> Kannst du mir sagen, warum ich mit Code 1313436779 niemanden einladen kann?",
        "<@99> Kannst du mir sagen, wann du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Kannst du mir sagen, wann du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir sagen, wo du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Kannst du mir sagen, wo du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir sagen, was ich tun muss, damit du mich mit Code 1313436779 einladen kannst?",
        "<@!99> Kannst du mir sagen, was ich tun muss, damit du mich mit Code 1313436779 einladen kannst?",
        "<@99> Kannst du mir sagen, wer mich mit Code 1313436779 einladen kann?",
        "<@!99> Kannst du mir sagen, wer mich mit Code 1313436779 einladen kann?",
        "<@99> Wie kann ich mich mit Code 1313436779 einladen lassen?",
        "<@99> Kann ich mich mit Code 1313436779 einladen lassen?",
        "<@99> Muss ich mich mit Code 1313436779 einladen lassen?",
        "<@99> Darf ich mir mit Code 1313436779 jemanden einladen?",
        "<@99> Kannst du mir erklären, wie ich mich einladen lassen kann? Code 1313436779",
        "<@99> Kannst du mir sagen, wie lange es dauert, bis du mich mit Code 1313436779 einladen kannst?",
        "<@99> Wie besprochen, kannst du mir erklären, wie ich mich einladen lassen kann? Code 1313436779",
        "<@99> Kannst du mir die Einladung erklären? Code 1313436779",
        "<@99> Kannst du mir den Invite erklären? Code 1313436779",
        "<@99> Kannst du mich über das Einladen informieren? Code 1313436779",
        "<@99> Kannst du mir erklären, warum ich jemanden einladen kann und mich einladen lassen kann? Code 1313436779",
        "<@99> Erklär mir, wie du mich einladen kannst und wie Einladungen funktionieren. Code 1313436779",
        "<@99> Sag mir, wie Einladungen funktionieren und ob du mich einladen kannst. Code 1313436779",
    ] {
        let (watcher, store, port, invite) = setup();
        watcher
            .handle_message(
                message(1, NOW, "<@99> Kannst du mich einladen?"),
                NOW,
                Some(false),
            )
            .await
            .expect("Die direkte Bitte muss eine Code-Nachfrage erzeugen");
        let previous = store.states.lock().await.get(&USER).cloned();
        watcher
            .handle_message(message(2, NOW + 1, content), NOW + 1, Some(false))
            .await
            .expect("Die Informationsfrage muss ohne Versand verarbeitet werden");
        assert!(invite.calls.lock().await.is_empty(), "{content}");
        assert_eq!(port.replies.lock().await.len(), 1, "{content}");
        assert_eq!(store.states.lock().await.get(&USER).cloned(), previous);
        watcher
            .handle_message(message(3, NOW + 2, "1313436779"), NOW + 2, Some(false))
            .await
            .expect("Die reine Zahl muss die ursprüngliche Code-Nachfrage erfüllen");
        assert_eq!(
            invite.calls.lock().await.as_slice(),
            &[(BOT, GUILD, "1313436779".into(), USER)]
        );
        assert_eq!(port.replies.lock().await.len(), 2);
    }
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
        .unwrap();
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
        .unwrap();
    watcher
        .handle_message(
            message(2, NOW + 20, "Kannste mich auch einladen?"),
            NOW + 20,
            Some(false),
        )
        .await
        .unwrap();
    assert_eq!(port.replies.lock().await.len(), 1);
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    let mut code = message(3, NOW + 40, "Mein Freundescode: 123456789");
    code.reply_message_id = Some(1);
    watcher
        .handle_message(code, NOW + 40, Some(false))
        .await
        .unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert_eq!(port.replies.lock().await.last().unwrap().0, 3);
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
        .unwrap();
    watcher.poll_due(NOW + 3599).await.unwrap();
    assert!(port.replies.lock().await.is_empty());
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    watcher.poll_due(NOW + 3600).await.unwrap();
    watcher.poll_due(NOW + 7200).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
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
        .unwrap();
    assert_eq!(port.replies.lock().await[0].1, INVITE_LOUNGE_HINT_TEXT);
    watcher
        .handle_message(message(2, NOW + 4000, "123456789"), NOW + 4000, Some(false))
        .await
        .unwrap();
    watcher.poll_due(NOW + 7599).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
    watcher.poll_due(NOW + 7600).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn raumbitte_ohne_code_verwendet_keinen_alten_code() {
    let (watcher, _, port, invite) = setup();
    watcher
        .handle_message(message(1, NOW - 100, "123456789"), NOW - 100, Some(false))
        .await
        .unwrap();
    watcher
        .handle_message(
            message(2, NOW, "Kann mich jemand einladen?"),
            NOW,
            Some(false),
        )
        .await
        .unwrap();
    watcher.poll_due(NOW + 8000).await.unwrap();
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
        .unwrap();
    let mut other = message(2, NOW + 1, "123456789");
    other.author_id = 8;
    watcher
        .handle_message(other, NOW + 1, Some(false))
        .await
        .unwrap();
    watcher.poll_due(NOW + 8000).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn audit_und_dispatch_task_blocken_und_antwort_bleibt_erhalten() {
    for (audit, expected) in [(true, AUDIT_REPLY), (false, RUNNING_REPLY)] {
        let (watcher, _, port, invite) = setup();
        invite.audit.store(audit, Ordering::SeqCst);
        invite.running_task.store(!audit, Ordering::SeqCst);
        watcher
            .handle_message(
                message(1, NOW, "Kannst du mich einladen? 123456789"),
                NOW,
                Some(false),
            )
            .await
            .unwrap();
        watcher.poll_due(NOW + 3600).await.unwrap();
        assert_eq!(invite.sends.load(Ordering::SeqCst), 0);
        assert_eq!(invite.calls.lock().await.len(), 1);
        assert_eq!(port.replies.lock().await[0].1, expected);
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
        .unwrap();
    watcher.poll_due(NOW + 3600).await.unwrap();
    watcher.poll_due(NOW + 7200).await.unwrap();
    watcher
        .handle_message(
            message(2, NOW + 7200, "Kann mich jemand einladen? 123456789"),
            NOW + 7200,
            Some(false),
        )
        .await
        .unwrap();
    assert_eq!(invite.calls.lock().await.len(), 1);
    assert!(store
        .load(USER)
        .await
        .unwrap()
        .unwrap()
        .last_result
        .unwrap()
        .contains("Fehlgeschlagen"));
    let retry = message(3, NOW + 7300, "Kannste mich auch einladen?");
    watcher
        .handle_message(retry.clone(), NOW + 7300, Some(false))
        .await
        .unwrap();
    watcher
        .handle_message(retry, NOW + 7300, Some(false))
        .await
        .unwrap();
    watcher.poll_due(NOW + 12_000).await.unwrap();
    assert_eq!(invite.calls.lock().await.len(), 2);
}

#[tokio::test]
async fn neustart_wiederholt_keinen_begonnenen_versuch() {
    let (watcher, store, port, invite) = setup();
    watcher
        .handle_message(
            message(1, NOW, "Kann mich jemand einladen? 123456789"),
            NOW,
            Some(false),
        )
        .await
        .unwrap();
    let previous = store.load(USER).await.unwrap().unwrap();
    let mut claimed = previous.clone();
    claimed.request.as_mut().unwrap().phase = Phase::Attempted;
    assert!(store
        .compare_exchange(USER, Some(&previous), &claimed)
        .await
        .unwrap());
    let restarted = InviteLoungeWatcher {
        store,
        port,
        invite: invite.clone(),
        guild_id: GUILD,
        clock_millis: || NOW * 1000,
    };
    restarted.poll_due(NOW + 8000).await.unwrap();
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
        .unwrap();
    let (first, second) = tokio::join!(
        watcher.dispatch_due(USER, NOW + 3600),
        watcher.dispatch_due(USER, NOW + 3600)
    );
    first.unwrap();
    second.unwrap();
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
            .unwrap();
    }
    let mut other = message(20, NOW, "Kannst du mich einladen? 123456789");
    other.channel_id = 100;
    watcher
        .handle_message(other, NOW, Some(false))
        .await
        .unwrap();
    let mut bot = message(21, NOW, "Kannst du mich einladen? 123456789");
    bot.is_bot = true;
    watcher.handle_message(bot, NOW, Some(false)).await.unwrap();
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
        .unwrap();
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
    watcher.handle_message(source, NOW + 2, None).await.unwrap();
    assert!(port.replies.lock().await.is_empty());
    store.legacy_hints.lock().await.insert(8, NOW);
    let mut other = message(3, NOW + 3, "Kannst du mich einladen?");
    other.author_id = 8;
    watcher
        .handle_message(other, NOW + 3, Some(false))
        .await
        .unwrap();
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
    watcher.backfill(NOW).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert!(store.backfill_done().await.unwrap());
    let fetched = port.history_calls.load(Ordering::SeqCst);
    watcher.backfill(NOW).await.unwrap();
    assert_eq!(port.history_calls.load(Ordering::SeqCst), fetched);
    watcher.poll_due(NOW + 1).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 2);
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
    watcher.backfill(NOW).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    assert_eq!(port.replies.lock().await.len(), 1);
    assert_ne!(port.replies.lock().await[0].1, INVITE_LOUNGE_HINT_TEXT);
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
    watcher.backfill(NOW).await.unwrap();
    assert!(port.history_calls.load(Ordering::SeqCst) >= 3);
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn fehlende_bot_id_erzeugt_keinen_menschlichen_admin() {
    let (watcher, store, port, invite) = setup();
    port.identity_missing.store(true, Ordering::SeqCst);
    assert!(watcher.backfill(NOW).await.is_err());
    assert!(!store.backfill_done().await.unwrap());
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
    assert!(store.compare_exchange(USER, None, &state).await.unwrap());
    assert!(!store.compare_exchange(USER, None, &state).await.unwrap());
    assert_eq!(store.pending_users().await.unwrap(), vec![USER]);
    let previous = state.clone();
    state.request.as_mut().unwrap().phase = Phase::Attempted;
    let (a, b) = tokio::join!(
        store.compare_exchange(USER, Some(&previous), &state),
        store.compare_exchange(USER, Some(&previous), &state)
    );
    assert_eq!(usize::from(a.unwrap()) + usize::from(b.unwrap()), 1);
    assert!(store.pending_users().await.unwrap().is_empty());
    assert_eq!(store.load(USER).await.unwrap(), Some(state));
    assert!(!store.backfill_done().await.unwrap());
    store.mark_backfill_done().await.unwrap();
    assert!(store.backfill_done().await.unwrap());
    kv::set(
        db.pool(),
        COOLDOWN_KV_NS,
        &cooldown_key(USER),
        &NOW.to_string(),
    )
    .await
    .unwrap();
    assert_eq!(store.last_hint_at(USER).await.unwrap(), Some(NOW));
    sqlx::query("INSERT INTO core.users (discord_id) VALUES ($1)")
        .bind(USER as i64)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO core.steam_links (discord_id, steam_id, primary_account) VALUES ($1, '76561198083722517', true)").bind(USER as i64).execute(db.pool()).await.unwrap();
    assert_eq!(
        store.known_code(USER).await.unwrap(),
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
            .unwrap();
        assert_eq!(invite.calls.lock().await.len(), 1);
        invite.release.notify_one();
    };
    let (result, ()) = tokio::join!(initial, during);
    result.unwrap();
    let state = store.load(USER).await.unwrap().unwrap();
    assert_eq!(state.last_seen, 2);
    assert_eq!(state.request.unwrap().phase, Phase::Attempted);
    assert!(state.last_result.unwrap().starts_with("🎮"));
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
        .unwrap();
    watcher
        .handle_message(
            message(2, NOW + 100, "Danke für die Einladung!"),
            NOW + 100,
            Some(false),
        )
        .await
        .unwrap();
    watcher.poll_due(NOW + 3600).await.unwrap();
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
        .unwrap();
    assert!(invite.calls.lock().await.is_empty());
    watcher
        .handle_message(
            message(3, NOW + 122, "Kannste mich auch einladen?"),
            NOW + 122,
            Some(false),
        )
        .await
        .unwrap();
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
        .unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    spawn_dispatch(&mut tasks, watcher.clone(), USER);
    invite.started.notified().await;
    let mut second = message(2, NOW + 1, "Kannst du mich einladen? 222222222");
    second.author_id = 8;
    let user_id = watcher
        .record_message(second, NOW + 1, Some(false))
        .await
        .unwrap()
        .unwrap();
    invite.hold.store(false, Ordering::SeqCst);
    spawn_dispatch(&mut tasks, watcher, user_id);
    tasks.join_next().await.unwrap().unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
    invite.release.notify_one();
    tasks.join_next().await.unwrap().unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 2);
    let calls = invite.calls.lock().await;
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].3, USER);
    assert_eq!(calls[1].3, 8);
}

#[tokio::test]
async fn geaenderter_code_im_rueckblick_startet_eine_neue_stunde() {
    let (watcher, _, port, invite) = setup();
    port.history.lock().await.extend([
        message(1, NOW - 7200, "Kann mich jemand einladen? 123456789"),
        message(2, NOW - 300, "Kann mich jemand einladen? 222222222"),
    ]);
    watcher.backfill(NOW).await.unwrap();
    assert!(invite.calls.lock().await.is_empty());
    watcher.poll_due(NOW + 3299).await.unwrap();
    assert!(invite.calls.lock().await.is_empty());
    watcher.poll_due(NOW + 3300).await.unwrap();
    assert_eq!(
        invite.calls.lock().await.as_slice(),
        &[(BOT, GUILD, "222222222".into(), USER)]
    );
    assert_eq!(port.replies.lock().await[0].0, 2);
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
        .unwrap();
    watcher
        .handle_message(message(2, NOW + 300, "123456789"), NOW + 300, Some(false))
        .await
        .unwrap();
    watcher.poll_due(NOW + 3600).await.unwrap();
    assert_eq!(invite.sends.load(Ordering::SeqCst), 1);
}
