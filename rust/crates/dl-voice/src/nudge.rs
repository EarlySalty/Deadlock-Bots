//! Steam-Link-Voice-Nudge — Port von `cogs/steam_link_voice_nudge.py`.
//!
//! Erinnert User ohne Steam-Verknüpfung per DM, nachdem sie an ihrem
//! ZWEITEN Voice-Tag 30 Minuten am Stück im Voice waren. Einmalig pro User
//! (kv `voice_nudge_done`), Erst-Sichtung in kv `voice_nudge_first_seen`,
//! DM-Referenz in `steam_nudge_state` (Restore des Close-Buttons über die
//! persistente custom_id `nudge_close`).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use dl_db::Db;
use dl_discord::{
    BridgeInteraction, BridgeReply, ChannelSender, Dispatcher, InteractionHandler,
    InteractionRouter, VoiceEvent,
};
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

pub const MIN_VOICE_MINUTES: u64 = 30;
pub const POLL_INTERVAL: Duration = Duration::from_secs(15);
pub const LOG_CHANNEL_ID: u64 = 1374364800817303632;
pub const NUDGE_VIEW_VERSION: i64 = 2;
pub const FIRST_SEEN_NS: &str = "voice_nudge_first_seen";
pub const DONE_NS: &str = "voice_nudge_done";
pub const CLOSE_CUSTOM_ID: &str = "nudge_close";
/// English-Only-Rolle ist ausgenommen (wie _EXEMPT_DEFAULT).
pub const EXEMPT_ROLE_IDS: [u64; 1] = [1309741866098491479];

const DM_DESCRIPTION: &str = "**Was bringt das?**\n\
• Rang wird korrekt erkannt und auf dem Server zugeordnet\n\
• Live-Status in den Voice Lanes funktioniert\n\
• Spielersuche funktioniert richtig für dich\n\n\
**So funktioniert's:**\n\
1. Du meldest dich kurz bei Steam an (OpenID - kein Passwort nötig)\n\
2. Du gibst deinen Steam-Freundescode ein\n\
3. Wir schicken dir eine Freundschaftsanfrage - einfach annehmen\n\
4. Fertig! Du bist verifiziert\n\n\
**Was wir NICHT machen:**\n\
❌ Keine Passwörter oder Zugangsdaten\n\
❌ Keine Steam-Freundschaftsliste auslesen\n\
❌ Keine Spielstände oder Profile einsehen\n\
❌ Keine Daten an Dritte weitergeben\n\
❌ Keine Werbung oder Tracking\n\n\
**Was wir speichern:**\n\
✓ Discord-ID (damit wir dich zuordnen können)\n\
✓ SteamID64 (technische ID von Steam)\n\
✓ Rang-Daten (nur zur Server-Zuordnung)\n\n\
**Open Source:**\n\
<https://github.com/NaniDerEchte2/Deadlock-Bots>";

/// Discord-Seite des Nudges (Tests mocken sie).
#[async_trait::async_trait]
pub trait NudgePort: Send + Sync {
    /// Ist der User aktuell in irgendeinem Voice-Kanal?
    async fn is_in_voice(&self, guild_id: u64, user_id: u64) -> bool;
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
    /// DM senden → (dm_channel_id, message_id).
    async fn send_dm(
        &self,
        user_id: u64,
        embeds: &[Value],
        components: &Value,
    ) -> Result<(u64, u64), String>;
    async fn send_log(&self, text: String);
    /// Frische Steam-Login-URL (Einmal-Link) vom Rust-Steam-Bot.
    async fn fetch_steam_link_url(&self, user_id: u64) -> Option<String>;
    async fn delete_message(&self, channel_id: u64, message_id: u64);
}

pub struct VoiceNudge {
    db: Db,
    port: Arc<dyn NudgePort>,
    running: tokio::sync::Mutex<HashSet<u64>>,
}

impl VoiceNudge {
    pub fn new(db: Db, port: Arc<dyn NudgePort>) -> Arc<Self> {
        Arc::new(Self {
            db,
            port,
            running: tokio::sync::Mutex::new(HashSet::new()),
        })
    }

    fn today() -> String {
        chrono::Utc::now().date_naive().to_string()
    }

    async fn kv(&self, ns: &'static str, user_id: u64) -> Option<String> {
        self.db.kv_get(ns, user_id.to_string()).await.ok().flatten()
    }

    async fn is_opted_out(&self, user_id: u64) -> bool {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT opted_out FROM user_privacy WHERE user_id = ?1",
                    [user_id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    async fn has_steam_link(&self, user_id: u64) -> bool {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT 1 FROM steam_links WHERE user_id = ?1 LIMIT 1",
                    [user_id],
                    |_| Ok(()),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .is_some()
    }

    async fn has_active_nudge(&self, user_id: u64) -> bool {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT message_id FROM steam_nudge_state WHERE user_id = ?1",
                    [user_id],
                    |row| row.get::<_, Option<i64>>(0),
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
            .flatten()
            .is_some()
    }

    pub async fn handle_event(self: &Arc<Self>, event: VoiceEvent) {
        // Nur ECHTE Joins (None → Channel), wie das Original
        let VoiceEvent::Join {
            guild_id, user_id, ..
        } = event
        else {
            return;
        };
        if self.is_opted_out(user_id).await {
            return;
        }
        let roles = self.port.member_role_ids(guild_id, user_id).await;
        if roles.iter().any(|r| EXEMPT_ROLE_IDS.contains(r)) {
            return;
        }
        if self.has_steam_link(user_id).await
            || self.kv(DONE_NS, user_id).await.is_some()
            || self.has_active_nudge(user_id).await
        {
            return;
        }

        // Tag 1: nur Erst-Sichtung merken, Nudge frühestens am nächsten Tag
        let today = Self::today();
        match self.kv(FIRST_SEEN_NS, user_id).await {
            None => {
                let _ = self
                    .db
                    .kv_set(FIRST_SEEN_NS, user_id.to_string(), today)
                    .await;
                return;
            }
            Some(first_seen) if first_seen == today => return,
            Some(_) => {}
        }

        // 30-Minuten-Voice-Watch (ein Task pro User)
        {
            let mut running = self.running.lock().await;
            if !running.insert(user_id) {
                return;
            }
        }
        let nudge = self.clone();
        tokio::spawn(async move {
            nudge.wait_and_notify(guild_id, user_id).await;
            nudge.running.lock().await.remove(&user_id);
        });
    }

    async fn wait_and_notify(self: &Arc<Self>, guild_id: u64, user_id: u64) {
        let mut seen = Duration::ZERO;
        let target = Duration::from_secs(MIN_VOICE_MINUTES * 60);
        while seen < target {
            tokio::time::sleep(POLL_INTERVAL).await;
            if !self.port.is_in_voice(guild_id, user_id).await {
                return; // früher gegangen
            }
            seen += POLL_INTERVAL;
        }
        if self.has_steam_link(user_id).await {
            return;
        }
        self.send_nudge(user_id).await;
    }

    /// DM bauen + senden + persistieren (auch für !nudgesend nutzbar).
    pub async fn send_nudge(&self, user_id: u64) -> bool {
        let steam_url = self.port.fetch_steam_link_url(user_id).await;
        let mut description = DM_DESCRIPTION.to_string();
        if steam_url.is_none() {
            description.push_str(
                "\n\n_Heads-up:_ Der Link-Dienst ist gerade nicht verfügbar. Nutze vorerst **/account_verknüpfen**.",
            );
        }
        let embed = json!({
            "title": "Steam-Verknüpfung empfohlen 🔗",
            "description": description,
            "color": 0x5865F2,
            "footer": { "text": "Kurzbefehle: /account_verknüpfen · /steam unlink · /steam setprimary" },
        });
        let steam_button = match &steam_url {
            Some(url) => {
                json!({ "type": 2, "style": 5, "label": "Mit Steam anmelden", "emoji": {"name": "🎮"}, "url": url })
            }
            None => {
                json!({ "type": 2, "style": 2, "label": "Mit Steam anmelden", "emoji": {"name": "🎮"}, "disabled": true, "custom_id": "nudge_steam_disabled" })
            }
        };
        let components = json!([
            { "type": 1, "components": [steam_button] },
            { "type": 1, "components": [{
                "type": 2, "style": 2, "label": "Schließen", "emoji": {"name": "❌"},
                "custom_id": CLOSE_CUSTOM_ID,
            }]},
        ]);

        match self.port.send_dm(user_id, &[embed], &components).await {
            Ok((channel_id, message_id)) => {
                let result = self
                    .db
                    .write(move |conn| {
                        conn.execute(
                            "INSERT INTO steam_nudge_state(user_id, notified_at, message_id, channel_id, view_version)
                             VALUES(?1, CURRENT_TIMESTAMP, ?2, ?3, ?4)
                             ON CONFLICT(user_id) DO UPDATE SET
                               notified_at = excluded.notified_at,
                               message_id = excluded.message_id,
                               channel_id = excluded.channel_id,
                               view_version = excluded.view_version",
                            rusqlite::params![user_id, message_id, channel_id, NUDGE_VIEW_VERSION],
                        )
                        .map(|_| ())
                    })
                    .await;
                if let Err(err) = result {
                    tracing::warn!(%err, user_id, "Nudge: State-Persist fehlgeschlagen");
                }
                let _ = self
                    .db
                    .kv_set(DONE_NS, user_id.to_string(), "sent".to_string())
                    .await;
                self.port
                    .send_log(format!("📨 Steam-Nudge gesendet an <@{user_id}>"))
                    .await;
                true
            }
            Err(err) => {
                tracing::info!(%err, user_id, "Nudge: DM fehlgeschlagen (DMs zu?)");
                let _ = self
                    .db
                    .kv_set(DONE_NS, user_id.to_string(), "dm_failed".to_string())
                    .await;
                false
            }
        }
    }
}

/// Close-Button der Nudge-DM (persistent: custom_id `nudge_close`).
struct CloseHandler {
    nudge: Arc<VoiceNudge>,
}

#[async_trait::async_trait]
impl InteractionHandler for CloseHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if let Some(message_id) = interaction.message_id {
            self.nudge
                .port
                .delete_message(interaction.channel_id, message_id)
                .await;
            let user_id = interaction.user_id;
            let _ = self
                .nudge
                .db
                .write(move |conn| {
                    conn.execute(
                        "UPDATE steam_nudge_state SET message_id=NULL, channel_id=NULL, view_version=0 WHERE user_id=?1",
                        [user_id],
                    )
                    .map(|_| ())
                })
                .await;
        }
        BridgeReply {
            content: Some("Geschlossen.".to_string()),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }
}

pub fn register(router: &mut InteractionRouter, nudge: Arc<VoiceNudge>) {
    router.on_custom_id(CLOSE_CUSTOM_ID, Arc::new(CloseHandler { nudge }));
}

impl VoiceNudge {
    /// `!nudgesend [@user]` / `!t30` — Admin-Test der Steam-Nudge-DM (Port von
    /// `nudgesend`/`_resolve_test_target`). Ziel = erwähnter User, sonst der
    /// Aufrufer. Opt-out und ausgenommene Rolle werden — wie im Original —
    /// vor dem Versand respektiert. Antwort als Text in denselben Kanal.
    ///
    /// `None`, wenn es kein `!nudgesend`/`!t30`-Befehl ist. Der Aufrufer muss
    /// (am Listener) Admin sein.
    pub async fn nudgesend_reply(
        self: &Arc<Self>,
        content: &str,
        guild_id: u64,
        author_id: u64,
    ) -> Option<String> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_lowercase();
        if !matches!(root.as_str(), "!nudgesend" | "!t30") {
            return None;
        }
        // Ziel: erste Mention im Rest, sonst der Aufrufer selbst.
        let target = parts.find_map(parse_mention).unwrap_or(author_id);

        if self.is_opted_out(target).await {
            return Some(
                "⚠️ Nutzer hat ein Opt-out aktiviert; keine Nudge-DM gesendet.".to_string(),
            );
        }
        let roles = self.port.member_role_ids(guild_id, target).await;
        if roles.iter().any(|r| EXEMPT_ROLE_IDS.contains(r)) {
            return Some("ℹ️ Test abgebrochen: Ziel hat eine ausgenommene Rolle.".to_string());
        }

        // force=True im Original: Steam-Link- und State-Check werden übersprungen,
        // die DM geht direkt raus. `send_nudge` bildet genau das ab.
        if self.send_nudge(target).await {
            Some(format!("📨 Test-DM an <@{target}> gesendet."))
        } else {
            Some(
                "⚠️ Test-DM konnte nicht gesendet werden (DMs aus? oder bereits benachrichtigt)."
                    .to_string(),
            )
        }
    }
}

/// Parst eine einzelne Discord-User-Mention (`<@123>` / `<@!123>`).
fn parse_mention(token: &str) -> Option<u64> {
    let inner = token.strip_prefix("<@")?.strip_suffix('>')?;
    inner.strip_prefix('!').unwrap_or(inner).parse::<u64>().ok()
}

/// Message-Listener für `!nudgesend`/`!t30` (Python: `@commands.hybrid_command`,
/// `administrator`). Folgt dem `balance_cmd::spawn`-Muster: eigener
/// `MessageEvent`-Subscriber, admin-gated, Antwort über den `ChannelSender`.
/// Nicht-Admins werden still ignoriert (kein „fehlende Berechtigung“-Reply).
pub fn spawn_command(
    nudge: Arc<VoiceNudge>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    if !event.author_is_admin {
                        continue;
                    }
                    let content = event.content.trim();
                    let Some(reply) = nudge
                        .nudgesend_reply(content, guild_id, event.author_id)
                        .await
                    else {
                        continue;
                    };
                    let _ = sender
                        .send_to_channel(event.channel_id, Some(&reply), &[])
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn spawn(nudge: Arc<VoiceNudge>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_voice();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => nudge.handle_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    struct MockPort {
        in_voice: StdMutex<bool>,
        dms: StdMutex<Vec<u64>>,
        logs: StdMutex<Vec<String>>,
        url: Option<String>,
        roles: StdMutex<Vec<u64>>,
    }

    #[async_trait::async_trait]
    impl NudgePort for MockPort {
        async fn is_in_voice(&self, _g: u64, _u: u64) -> bool {
            *self.in_voice.lock().expect("lock")
        }
        async fn member_role_ids(&self, _g: u64, _u: u64) -> Vec<u64> {
            self.roles.lock().expect("lock").clone()
        }
        async fn send_dm(
            &self,
            user_id: u64,
            _embeds: &[Value],
            _components: &Value,
        ) -> Result<(u64, u64), String> {
            self.dms.lock().expect("lock").push(user_id);
            Ok((900, 901))
        }
        async fn send_log(&self, text: String) {
            self.logs.lock().expect("lock").push(text);
        }
        async fn fetch_steam_link_url(&self, _u: u64) -> Option<String> {
            self.url.clone()
        }
        async fn delete_message(&self, _c: u64, _m: u64) {}
    }

    const DDLS: [&str; 4] = [
        "CREATE TABLE kv_store(ns TEXT NOT NULL, k TEXT NOT NULL, v TEXT NOT NULL, PRIMARY KEY(ns, k))",
        "CREATE TABLE user_privacy(user_id INTEGER PRIMARY KEY, opted_out INTEGER NOT NULL DEFAULT 0)",
        "CREATE TABLE steam_links(user_id INTEGER NOT NULL, steam_id TEXT NOT NULL, name TEXT, verified INTEGER DEFAULT 0, primary_account INTEGER DEFAULT 0, PRIMARY KEY (user_id, steam_id))",
        "CREATE TABLE steam_nudge_state(user_id INTEGER PRIMARY KEY, notified_at DATETIME, first_seen DATETIME DEFAULT CURRENT_TIMESTAMP, message_id INTEGER, channel_id INTEGER, view_version INTEGER DEFAULT 0)",
    ];

    async fn setup(url: Option<&str>) -> (tempfile::TempDir, Arc<VoiceNudge>, Arc<MockPort>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        for ddl in DDLS {
            db.write(move |c| c.execute(ddl, []).map(|_| ()))
                .await
                .expect("ddl");
        }
        let port = Arc::new(MockPort {
            in_voice: StdMutex::new(true),
            dms: StdMutex::new(Vec::new()),
            logs: StdMutex::new(Vec::new()),
            url: url.map(str::to_string),
            roles: StdMutex::new(Vec::new()),
        });
        (dir, VoiceNudge::new(db, port.clone()), port)
    }

    #[tokio::test]
    async fn erster_tag_merkt_nur_vor() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        nudge
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 5,
            })
            .await;
        // Erst-Sichtung gespeichert, kein Task, keine DM
        assert_eq!(
            nudge.kv(FIRST_SEEN_NS, 100).await,
            Some(VoiceNudge::today())
        );
        assert!(port.dms.lock().expect("lock").is_empty());
        assert!(nudge.running.lock().await.is_empty());
    }

    #[tokio::test]
    async fn zweiter_tag_started_watch() {
        let (_dir, nudge, _port) = setup(Some("https://s.test/login")).await;
        // Erst-Sichtung war gestern
        nudge
            .db
            .kv_set(FIRST_SEEN_NS, "100", "2020-01-01".to_string())
            .await
            .expect("kv");
        nudge
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 5,
            })
            .await;
        assert!(nudge.running.lock().await.contains(&100));
    }

    #[tokio::test]
    async fn send_nudge_persistiert_und_markiert_done() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        assert!(nudge.send_nudge(100).await);
        assert_eq!(port.dms.lock().expect("lock").clone(), vec![100]);
        assert_eq!(nudge.kv(DONE_NS, 100).await.as_deref(), Some("sent"));
        assert!(nudge.has_active_nudge(100).await);
        assert!(!port.logs.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn bereits_verlinkt_oder_done_wird_uebersprungen() {
        let (_dir, nudge, port) = setup(None).await;
        nudge
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO steam_links(user_id, steam_id) VALUES(100, 'x')",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("link");
        nudge
            .db
            .kv_set(FIRST_SEEN_NS, "100", "2020-01-01".to_string())
            .await
            .expect("kv");
        nudge
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 5,
            })
            .await;
        assert!(nudge.running.lock().await.is_empty());
        assert!(port.dms.lock().expect("lock").is_empty());
    }

    #[test]
    fn mention_parsen() {
        assert_eq!(parse_mention("<@123>"), Some(123));
        assert_eq!(parse_mention("<@!456>"), Some(456));
        assert_eq!(parse_mention("abc"), None);
    }

    #[tokio::test]
    async fn nudgesend_an_mention_sendet() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        let reply = nudge
            .nudgesend_reply("!nudgesend <@200>", 1, 9)
            .await
            .expect("reply");
        assert!(reply.contains("Test-DM"), "reply: {reply}");
        // DM ging an die Mention (200), nicht an den Aufrufer (9).
        assert_eq!(port.dms.lock().expect("lock").clone(), vec![200]);
    }

    #[tokio::test]
    async fn nudgesend_ohne_mention_nimmt_aufrufer() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        // Alias !t30 ohne Mention → Aufrufer (9).
        let reply = nudge.nudgesend_reply("!t30", 1, 9).await.expect("reply");
        assert!(reply.contains("Test-DM"), "reply: {reply}");
        assert_eq!(port.dms.lock().expect("lock").clone(), vec![9]);
    }

    #[tokio::test]
    async fn nudgesend_exempt_rolle_bricht_ab() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        *port.roles.lock().expect("lock") = vec![EXEMPT_ROLE_IDS[0]];
        let reply = nudge
            .nudgesend_reply("!nudgesend <@200>", 1, 9)
            .await
            .expect("reply");
        assert!(reply.contains("ausgenommene Rolle"), "reply: {reply}");
        assert!(port.dms.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn nudgesend_opt_out_bricht_ab() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        nudge
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO user_privacy(user_id, opted_out) VALUES(200, 1)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("opt-out");
        let reply = nudge
            .nudgesend_reply("!nudgesend <@200>", 1, 9)
            .await
            .expect("reply");
        assert!(reply.contains("Opt-out"), "reply: {reply}");
        assert!(port.dms.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn kein_nudgesend_befehl_ist_none() {
        let (_dir, nudge, _port) = setup(None).await;
        assert!(nudge.nudgesend_reply("hallo welt", 1, 9).await.is_none());
    }
}
