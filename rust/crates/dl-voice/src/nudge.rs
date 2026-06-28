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
pub const NUDGE_TEST_TARGET_REQUIRED_TEXT: &str = "Bitte Ziel angeben: `!nudgesend @user`";
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
    async fn refresh_dm(
        &self,
        channel_id: u64,
        message_id: u64,
        embeds: &[Value],
        components: &Value,
    ) -> Result<bool, String>;
}

#[derive(Debug, Clone)]
struct NudgeState {
    user_id: u64,
    notified: bool,
    message_id: Option<u64>,
    channel_id: Option<u64>,
    view_version: i64,
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

    #[cfg(test)]
    async fn has_active_nudge(&self, user_id: u64) -> bool {
        self.load_nudge_state(user_id)
            .await
            .map(|state| state.notified || state.message_id.is_some())
            .unwrap_or(false)
    }

    async fn active_nudge_blocks_normal_path(&self, user_id: u64) -> bool {
        let Some(state) = self.load_nudge_state(user_id).await else {
            return false;
        };
        if self.refresh_existing_state(&state, false).await {
            return true;
        }
        self.clear_nudge_message_ref(user_id).await;
        state.notified
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
            || self.active_nudge_blocks_normal_path(user_id).await
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

    /// DM bauen + senden + persistieren (Normalpfad; dupliziert aktive States nicht).
    pub async fn send_nudge(&self, user_id: u64) -> bool {
        self.send_nudge_inner(user_id, false).await
    }

    async fn send_nudge_inner(&self, user_id: u64, force: bool) -> bool {
        if !force {
            if self.has_steam_link(user_id).await {
                return false;
            }
            if let Some(state) = self.load_nudge_state(user_id).await {
                if self.refresh_existing_state(&state, false).await {
                    return false;
                }
                self.clear_nudge_message_ref(user_id).await;
                if state.notified {
                    return false;
                }
            }
        }
        let (embed, components) = self.build_dm_payload(user_id).await;

        match self.port.send_dm(user_id, &[embed], &components).await {
            Ok((channel_id, message_id)) => {
                self.mark_notified(user_id, channel_id, message_id).await;
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
                false
            }
        }
    }

    async fn build_dm_payload(&self, user_id: u64) -> (Value, Value) {
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
        (embed, components)
    }

    async fn load_nudge_state(&self, user_id: u64) -> Option<NudgeState> {
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT user_id, notified_at, message_id, channel_id, view_version
                       FROM steam_nudge_state
                      WHERE user_id = ?1",
                    [user_id],
                    |row| {
                        let raw_user_id: i64 = row.get(0)?;
                        Ok(NudgeState {
                            user_id: u64::try_from(raw_user_id).unwrap_or(0),
                            notified: row.get::<_, Option<String>>(1)?.is_some(),
                            message_id: row
                                .get::<_, Option<i64>>(2)?
                                .and_then(|id| u64::try_from(id).ok()),
                            channel_id: row
                                .get::<_, Option<i64>>(3)?
                                .and_then(|id| u64::try_from(id).ok()),
                            view_version: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                        })
                    },
                )
                .optional()
            })
            .await
            .ok()
            .flatten()
    }

    async fn load_all_nudge_states(&self) -> Vec<NudgeState> {
        self.db
            .read(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT user_id, notified_at, message_id, channel_id, view_version
                       FROM steam_nudge_state
                      WHERE message_id IS NOT NULL",
                )?;
                let rows = stmt.query_map([], |row| {
                    let raw_user_id: i64 = row.get(0)?;
                    Ok(NudgeState {
                        user_id: u64::try_from(raw_user_id).unwrap_or(0),
                        notified: row.get::<_, Option<String>>(1)?.is_some(),
                        message_id: row
                            .get::<_, Option<i64>>(2)?
                            .and_then(|id| u64::try_from(id).ok()),
                        channel_id: row
                            .get::<_, Option<i64>>(3)?
                            .and_then(|id| u64::try_from(id).ok()),
                        view_version: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|state| state.user_id > 0)
            .collect()
    }

    async fn clear_nudge_message_ref(&self, user_id: u64) {
        let result = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE steam_nudge_state
                        SET message_id=NULL,
                            channel_id=NULL,
                            view_version=0
                      WHERE user_id=?1",
                    [user_id],
                )
                .map(|_| ())
            })
            .await;
        if let Err(err) = result {
            tracing::debug!(%err, user_id, "Nudge: Message-Ref-Clear fehlgeschlagen");
        }
    }

    async fn mark_notified(&self, user_id: u64, channel_id: u64, message_id: u64) {
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
    }

    async fn refresh_existing_state(&self, state: &NudgeState, force: bool) -> bool {
        let Some(message_id) = state.message_id else {
            return state.notified;
        };
        let Some(channel_id) = state.channel_id else {
            return state.notified;
        };
        if !force && state.view_version >= NUDGE_VIEW_VERSION {
            return true;
        }
        let (embed, components) = self.build_dm_payload(state.user_id).await;
        match self
            .port
            .refresh_dm(channel_id, message_id, &[embed], &components)
            .await
        {
            Ok(true) => {
                self.mark_notified(state.user_id, channel_id, message_id)
                    .await;
                true
            }
            Ok(false) => false,
            Err(err) => {
                tracing::debug!(%err, user_id = state.user_id, "Nudge: bestehende DM konnte nicht aktualisiert werden");
                true
            }
        }
    }

    pub async fn refresh_persistent_messages(&self) {
        for state in self.load_all_nudge_states().await {
            if !self.refresh_existing_state(&state, true).await {
                self.clear_nudge_message_ref(state.user_id).await;
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
        let default_id = std::env::var("NUDGE_TEST_DEFAULT_ID")
            .ok()
            .and_then(|raw| raw.trim().parse::<u64>().ok());
        self.nudgesend_reply_with_default(content, guild_id, author_id, default_id)
            .await
    }

    pub async fn nudgesend_reply_with_default(
        self: &Arc<Self>,
        content: &str,
        guild_id: u64,
        _author_id: u64,
        default_id: Option<u64>,
    ) -> Option<String> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_lowercase();
        if !matches!(root.as_str(), "!nudgesend" | "!t30") {
            return None;
        }
        let Some(target) = parts.find_map(parse_mention).or(default_id) else {
            return Some(NUDGE_TEST_TARGET_REQUIRED_TEXT.to_string());
        };

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
        // die DM geht direkt raus.
        if self.send_nudge_inner(target, true).await {
            Some(format!("📨 Test-DM an <@{target}> gesendet."))
        } else {
            Some(
                "⚠️ Test-DM konnte nicht gesendet werden (DMs aus? oder bereits benachrichtigt)."
                    .to_string(),
            )
        }
    }
}

pub fn spawn_restore(nudge: Arc<VoiceNudge>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        nudge.refresh_persistent_messages().await;
    })
}

/// Parst eine einzelne Discord-User-Mention (`<@123>` / `<@!123>`).
fn parse_mention(token: &str) -> Option<u64> {
    let inner = token.strip_prefix("<@")?.strip_suffix('>')?;
    inner.strip_prefix('!').unwrap_or(inner).parse::<u64>().ok()
}

/// Message-Listener für `!nudgesend`/`!t30` (Python: `@commands.hybrid_command`,
/// `administrator`). Folgt dem Muster eines eigenständigen Spawn-/Listener-Tasks:
/// eigener `MessageEvent`-Subscriber, admin-gated, Antwort über den `ChannelSender`.
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
        refreshes: StdMutex<Vec<(u64, u64)>>,
        missing_messages: StdMutex<HashSet<(u64, u64)>>,
        logs: StdMutex<Vec<String>>,
        url: Option<String>,
        roles: StdMutex<Vec<u64>>,
        fail_dm: StdMutex<bool>,
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
            if *self.fail_dm.lock().expect("lock") {
                return Err("forbidden".to_string());
            }
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
        async fn refresh_dm(
            &self,
            channel_id: u64,
            message_id: u64,
            _embeds: &[Value],
            _components: &Value,
        ) -> Result<bool, String> {
            if self
                .missing_messages
                .lock()
                .expect("lock")
                .contains(&(channel_id, message_id))
            {
                return Ok(false);
            }
            self.refreshes
                .lock()
                .expect("lock")
                .push((channel_id, message_id));
            Ok(true)
        }
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
            refreshes: StdMutex::new(Vec::new()),
            missing_messages: StdMutex::new(HashSet::new()),
            logs: StdMutex::new(Vec::new()),
            url: url.map(str::to_string),
            roles: StdMutex::new(Vec::new()),
            fail_dm: StdMutex::new(false),
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
    async fn restore_persistent_messages_refreshes_gespeicherte_dm() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        nudge
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO steam_nudge_state(user_id, notified_at, message_id, channel_id, view_version)
                     VALUES(100, CURRENT_TIMESTAMP, 901, 900, 1)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("state");

        nudge.refresh_persistent_messages().await;

        assert_eq!(
            port.refreshes.lock().expect("lock").clone(),
            vec![(900, 901)]
        );
        let version: i64 = nudge
            .db
            .read(|c| {
                c.query_row(
                    "SELECT view_version FROM steam_nudge_state WHERE user_id=100",
                    [],
                    |row| row.get(0),
                )
            })
            .await
            .expect("version");
        assert_eq!(version, NUDGE_VIEW_VERSION);
    }

    #[tokio::test]
    async fn normaler_send_refreshes_aktiven_state_und_sendet_nicht_neu() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        nudge
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO steam_nudge_state(user_id, notified_at, message_id, channel_id, view_version)
                     VALUES(100, CURRENT_TIMESTAMP, 901, 900, 1)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("state");

        assert!(!nudge.send_nudge(100).await);
        assert!(port.dms.lock().expect("lock").is_empty());
        assert_eq!(
            port.refreshes.lock().expect("lock").clone(),
            vec![(900, 901)]
        );

        let reply = nudge
            .nudgesend_reply("!nudgesend <@100>", 1, 9)
            .await
            .expect("reply");
        assert!(reply.contains("Test-DM"), "reply: {reply}");
        assert_eq!(port.dms.lock().expect("lock").clone(), vec![100]);
    }

    #[tokio::test]
    async fn handle_event_refreshes_aktiven_state_mit_alter_view_und_sendet_nicht_neu() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        nudge
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO steam_nudge_state(user_id, notified_at, message_id, channel_id, view_version)
                     VALUES(100, CURRENT_TIMESTAMP, 901, 900, 1)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("state");

        nudge
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 5,
            })
            .await;

        assert_eq!(
            port.refreshes.lock().expect("lock").clone(),
            vec![(900, 901)]
        );
        assert!(port.dms.lock().expect("lock").is_empty());
        assert!(nudge.running.lock().await.is_empty());
    }

    #[tokio::test]
    async fn handle_event_missing_aktive_message_cleart_nur_messagefelder() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        nudge
            .db
            .write(|c| {
                c.execute(
                    "INSERT INTO steam_nudge_state(user_id, notified_at, message_id, channel_id, view_version)
                     VALUES(100, CURRENT_TIMESTAMP, 901, 900, 1)",
                    [],
                )
                .map(|_| ())
            })
            .await
            .expect("state");
        port.missing_messages
            .lock()
            .expect("lock")
            .insert((900, 901));

        nudge
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 100,
                channel_id: 5,
            })
            .await;

        assert!(port.refreshes.lock().expect("lock").is_empty());
        assert!(port.dms.lock().expect("lock").is_empty());
        assert!(nudge.running.lock().await.is_empty());
        let (notified, message_id, channel_id, view_version): (
            Option<String>,
            Option<i64>,
            Option<i64>,
            i64,
        ) = nudge
            .db
            .read(|c| {
                c.query_row(
                    "SELECT notified_at, message_id, channel_id, view_version
                       FROM steam_nudge_state WHERE user_id=100",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
            })
            .await
            .expect("state row");
        assert!(notified.is_some());
        assert_eq!(message_id, None);
        assert_eq!(channel_id, None);
        assert_eq!(view_version, 0);
    }

    #[tokio::test]
    async fn dm_fehlschlag_markiert_nicht_done() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        *port.fail_dm.lock().expect("lock") = true;
        assert!(!nudge.send_nudge(100).await);
        assert_eq!(nudge.kv(DONE_NS, 100).await, None);
        assert!(!nudge.has_active_nudge(100).await);
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
    async fn nudgesend_ohne_mention_ohne_default_fordert_ziel() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        let reply = nudge.nudgesend_reply("!t30", 1, 9).await.expect("reply");
        assert_eq!(reply, NUDGE_TEST_TARGET_REQUIRED_TEXT);
        assert!(port.dms.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn nudgesend_ohne_mention_nutzt_default_id() {
        let (_dir, nudge, port) = setup(Some("https://s.test/login")).await;
        let reply = nudge
            .nudgesend_reply_with_default("!t30", 1, 9, Some(777))
            .await
            .expect("reply");
        assert!(reply.contains("Test-DM"), "reply: {reply}");
        assert_eq!(port.dms.lock().expect("lock").clone(), vec![777]);
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
