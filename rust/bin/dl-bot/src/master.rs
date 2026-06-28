use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use dl_discord::{ChannelSender, DiscordAdapter, Dispatcher, InteractionRouter, MessageEvent};
use serde_json::{json, Value};
use tokio::sync::mpsc;

pub const DEFAULT_GUILD_ID: u64 = 1_289_721_245_281_292_288;
const DEFAULT_PID_FILE: &str = "deadlock-bot-rust.pid";

pub const FEATURE_MODULES: &[&str] = &[
    "broker",
    "changelog",
    "steam_bridge",
    "twitch_live_bridge",
    "streamer_matcher",
    "streamer_intent",
    "steam_link_nudge",
    "tempvoice",
    "activity_analyzer",
    "lane_router",
    "voice_feedback",
    "tags",
    "security_guard",
    "onboarding",
    "privacy",
    "coaching_requests",
    "faq_chat",
    "feedback_hub",
    "clips",
    "leave_survey",
    "retention",
    "ai_moderator",
    "voice_tracker",
    "rank_voice_manager",
    "website_invites",
    "coaching_sync",
    "text_stats",
    "build_publisher",
    "dm_assistant",
    "lfg",
    "adaptive_lanes",
    "voice_status",
];

const INVALID_SCOPE: &str = "❌ Ungültiger scope. Erlaubt: `both`, `global`, `guild`.";
const INVALID_MODE: &str = "❌ Ungültiger mode. Erlaubt: `force`, `auto`.";
const RESTART_EXIT_CODE: u8 = 75;

pub fn restart_exit_code() -> std::process::ExitCode {
    std::process::ExitCode::from(RESTART_EXIT_CODE)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSyncScope {
    Both,
    Global,
    Guild,
}

impl CommandSyncScope {
    fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "both" => Some(Self::Both),
            "global" => Some(Self::Global),
            "guild" => Some(Self::Guild),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Global => "global",
            Self::Guild => "guild",
        }
    }

    fn includes_global(self) -> bool {
        matches!(self, Self::Both | Self::Global)
    }

    fn includes_guild(self) -> bool {
        matches!(self, Self::Both | Self::Guild)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandSyncStartupConfig {
    pub enabled: bool,
    pub scope: CommandSyncScope,
    pub guild_id: Option<u64>,
}

impl CommandSyncStartupConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let enabled = lookup("DL_BOT_COMMAND_SYNC")
            .or_else(|| lookup("COMMAND_SYNC_ON_START"))
            .map(|raw| env_bool(&raw, true))
            .unwrap_or(true);
        let scope = lookup("DL_BOT_COMMAND_START_SCOPE")
            .or_else(|| lookup("DL_BOT_COMMAND_SCOPE"))
            .or_else(|| lookup("COMMAND_SYNC_START_SCOPE"))
            .and_then(|raw| CommandSyncScope::parse(&raw))
            .unwrap_or(CommandSyncScope::Guild);
        let guild_id = lookup("DL_BOT_COMMAND_GUILD_ID")
            .or_else(|| lookup("GUILD_ID"))
            .or_else(|| lookup("OUR_GUILD_ID"))
            .or_else(|| lookup("MAIN_GUILD_ID"))
            .and_then(|raw| parse_u64(&raw))
            .or(Some(DEFAULT_GUILD_ID));

        Self {
            enabled,
            scope,
            guild_id,
        }
    }
}

fn env_bool(raw: &str, default: bool) -> bool {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => true,
        "0" | "false" | "no" | "n" | "off" => false,
        _ => default,
    }
}

fn parse_u64(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok().filter(|value| *value > 0)
}

pub fn owner_id_from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<u64> {
    lookup("OWNER_ID").and_then(|raw| parse_u64(&raw))
}

pub fn is_owner(author_id: u64, owner_id: Option<u64>) -> bool {
    owner_id
        .filter(|owner_id| *owner_id > 0)
        .is_some_and(|owner_id| owner_id == author_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MasterCommand {
    Status,
    Restart,
    SyncCommands {
        scope: CommandSyncScope,
        force: bool,
    },
    Invalid(&'static str),
}

impl MasterCommand {
    pub fn parse(content: &str) -> Option<Self> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_ascii_lowercase();
        if root != "!master" && root != "!m" {
            return None;
        }
        let subcommand = parts.next()?.to_ascii_lowercase();
        match subcommand.as_str() {
            "status" | "s" => Some(Self::Status),
            "restart" | "reboot" => Some(Self::Restart),
            "sync_commands" | "synccommands" | "sync" => {
                let scope = match parts.next() {
                    Some(raw) => match CommandSyncScope::parse(raw) {
                        Some(scope) => scope,
                        None => return Some(Self::Invalid(INVALID_SCOPE)),
                    },
                    None => CommandSyncScope::Both,
                };
                let force = match parts.next() {
                    Some(raw) => match raw.trim().to_ascii_lowercase().as_str() {
                        "force" => true,
                        "auto" => false,
                        _ => return Some(Self::Invalid(INVALID_MODE)),
                    },
                    None => true,
                };
                Some(Self::SyncCommands { scope, force })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MasterStatusSnapshot {
    pub startup_text: String,
    pub guild_count: usize,
    pub user_count: usize,
    pub command_count: usize,
    pub active_modules: Vec<String>,
}

pub fn startup_text_now() -> String {
    chrono::Local::now().format("%d.%m.%Y %H:%M:%S").to_string()
}

pub fn status_embed(snapshot: &MasterStatusSnapshot) -> Value {
    let mut fields = vec![json!({
        "name": "🔧 System",
        "value": format!(
            "Guilds: {}\nUsers: {}\nCommands: {}",
            snapshot.guild_count, snapshot.user_count, snapshot.command_count
        ),
        "inline": true
    })];

    if !snapshot.active_modules.is_empty() {
        let value = snapshot
            .active_modules
            .iter()
            .map(|module| format!("✅ {module}"))
            .collect::<Vec<_>>()
            .join("\n");
        fields.push(json!({
            "name": format!("📦 Loaded Cogs ({})", snapshot.active_modules.len()),
            "value": value,
            "inline": true
        }));
    }

    json!({
        "title": "📊 Master Bot Status",
        "description": format!("Bot läuft seit: {}", snapshot.startup_text),
        "color": 0x00FF00,
        "fields": fields
    })
}

pub trait StatusPort: Send + Sync {
    fn snapshot(&self) -> MasterStatusSnapshot;
}

pub struct DiscordStatusPort {
    adapter: Arc<DiscordAdapter>,
    router: Arc<InteractionRouter>,
    startup_text: String,
}

impl DiscordStatusPort {
    pub fn new(
        adapter: Arc<DiscordAdapter>,
        router: Arc<InteractionRouter>,
        startup_text: String,
    ) -> Self {
        Self {
            adapter,
            router,
            startup_text,
        }
    }
}

impl StatusPort for DiscordStatusPort {
    fn snapshot(&self) -> MasterStatusSnapshot {
        let mut users = HashSet::new();
        let guilds = self.adapter.cache().guilds();
        for guild_id in &guilds {
            if let Some(guild) = self.adapter.cache().guild(*guild_id) {
                users.extend(guild.members.keys().map(|user_id| user_id.get()));
            }
        }
        MasterStatusSnapshot {
            startup_text: self.startup_text.clone(),
            guild_count: guilds.len(),
            user_count: users.len(),
            command_count: self.router.command_definitions().len(),
            active_modules: FEATURE_MODULES
                .iter()
                .map(|module| (*module).to_string())
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncSummary {
    pub status: String,
    pub scope: CommandSyncScope,
    pub global_count: usize,
    pub guild_counts: BTreeMap<String, usize>,
    pub errors: BTreeMap<String, String>,
}

impl SyncSummary {
    fn new(scope: CommandSyncScope) -> Self {
        Self {
            status: "synced".to_string(),
            scope,
            global_count: 0,
            guild_counts: BTreeMap::new(),
            errors: BTreeMap::new(),
        }
    }

    fn finalize(mut self, successes: usize) -> Self {
        self.status = if self.errors.is_empty() {
            "synced".to_string()
        } else if successes > 0 {
            "partial".to_string()
        } else {
            "error".to_string()
        };
        self
    }
}

#[async_trait::async_trait]
pub trait CommandSyncPort: Send + Sync {
    async fn sync_commands(&self, scope: CommandSyncScope) -> SyncSummary;
}

pub struct DiscordCommandSync {
    adapter: Arc<DiscordAdapter>,
    router: Arc<InteractionRouter>,
    guild_id: Option<u64>,
}

impl DiscordCommandSync {
    pub fn new(
        adapter: Arc<DiscordAdapter>,
        router: Arc<InteractionRouter>,
        guild_id: Option<u64>,
    ) -> Self {
        Self {
            adapter,
            router,
            guild_id,
        }
    }

    pub async fn sync_scope(&self, scope: CommandSyncScope) -> SyncSummary {
        let mut summary = SyncSummary::new(scope);
        let mut successes = 0usize;

        if scope.includes_global() {
            match dl_discord::dispatch::sync_commands(&self.adapter.http, &self.router, None).await
            {
                Ok(count) => {
                    summary.global_count = count;
                    successes += 1;
                }
                Err(err) => {
                    summary.errors.insert("global".to_string(), err.to_string());
                }
            }
        }

        if scope.includes_guild() {
            if let Some(guild_id) = self.guild_id {
                match dl_discord::dispatch::sync_commands(
                    &self.adapter.http,
                    &self.router,
                    Some(guild_id),
                )
                .await
                {
                    Ok(count) => {
                        summary.guild_counts.insert(guild_id.to_string(), count);
                        successes += 1;
                    }
                    Err(err) => {
                        summary
                            .errors
                            .insert(format!("guild:{guild_id}"), err.to_string());
                    }
                }
            } else {
                summary.errors.insert(
                    "guild".to_string(),
                    "Keine Guild-ID konfiguriert (DL_BOT_COMMAND_GUILD_ID) — Guild-Sync übersprungen.".to_string(),
                );
            }
        }

        summary.finalize(successes)
    }
}

#[async_trait::async_trait]
impl CommandSyncPort for DiscordCommandSync {
    async fn sync_commands(&self, scope: CommandSyncScope) -> SyncSummary {
        self.sync_scope(scope).await
    }
}

pub fn sync_start_embed(scope: CommandSyncScope, force: bool) -> Value {
    let mode = if force { "force" } else { "auto" };
    json!({
        "title": "🔄 App-Command Sync",
        "description": format!(
            "Starte Sync (`scope={}`, `mode={}`)...",
            scope.as_str(),
            mode
        ),
        "color": 0x00AAFF
    })
}

pub fn sync_result_embed(summary: &SyncSummary) -> Value {
    let color = match summary.status.as_str() {
        "synced" | "skipped" => 0x00FF00,
        "partial" => 0xFFAA00,
        _ => 0xFF0000,
    };
    let mut details = format!(
        "Status: `{}`\nScope: `{}`\nGlobal synced: `{}`\nGuild syncs: `{}`",
        summary.status,
        summary.scope.as_str(),
        summary.global_count,
        summary.guild_counts.len()
    );
    if !summary.errors.is_empty() {
        let error_preview = summary
            .errors
            .iter()
            .take(8)
            .map(|(key, value)| format!("• {key}: {value}"))
            .collect::<Vec<_>>()
            .join("\n");
        details.push_str("\nFehler:\n");
        details.push_str(&error_preview);
    }
    json!({
        "title": "🔄 App-Command Sync",
        "description": details,
        "color": color
    })
}

pub fn restart_embed() -> Value {
    json!({
        "title": "🔁 Bot-Neustart",
        "description": "Restart angefordert. Der Bot trennt gleich die Verbindung und startet neu.",
        "color": 0x00FF00
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MasterAction {
    Restart,
}

pub fn spawn_control(
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
    sync: Arc<dyn CommandSyncPort>,
    status: Arc<dyn StatusPort>,
    owner_id: Option<u64>,
    action_tx: mpsc::UnboundedSender<MasterAction>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            let event = match messages.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let Some(command) = MasterCommand::parse(event.content.trim()) else {
                continue;
            };
            if !is_owner(event.author_id, owner_id) {
                continue;
            }
            handle_command(
                command,
                &event,
                sender.as_ref(),
                sync.as_ref(),
                status.as_ref(),
                &action_tx,
            )
            .await;
        }
    })
}

async fn handle_command(
    command: MasterCommand,
    event: &MessageEvent,
    sender: &dyn ChannelSender,
    sync: &dyn CommandSyncPort,
    status: &dyn StatusPort,
    action_tx: &mpsc::UnboundedSender<MasterAction>,
) {
    match command {
        MasterCommand::Status => {
            let embed = status_embed(&status.snapshot());
            let _ = sender
                .send_to_channel(event.channel_id, None, &[embed])
                .await;
        }
        MasterCommand::Restart => {
            let _ = sender
                .send_to_channel(event.channel_id, None, &[restart_embed()])
                .await;
            let _ = action_tx.send(MasterAction::Restart);
        }
        MasterCommand::SyncCommands { scope, force } => {
            let _ = sender
                .send_to_channel(event.channel_id, None, &[sync_start_embed(scope, force)])
                .await;
            let summary = sync.sync_commands(scope).await;
            let _ = sender
                .send_to_channel(event.channel_id, None, &[sync_result_embed(&summary)])
                .await;
        }
        MasterCommand::Invalid(message) => {
            let _ = sender
                .send_to_channel(event.channel_id, Some(message), &[])
                .await;
        }
    }
}

#[derive(Debug)]
pub enum PidLockError {
    AlreadyRunning {
        pid: u32,
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl fmt::Display for PidLockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyRunning { pid, path } => write!(
                f,
                "Rust Bot läuft bereits als PID {pid}. Zweite Instanz wird NICHT gestartet (verhindert Token-Race-Conditions). Beende PID {pid} zuerst oder lösche {} manuell.",
                path.display()
            ),
            Self::Io { path, source } => {
                write!(f, "PID-Lock {} konnte nicht verwendet werden: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for PidLockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::AlreadyRunning { .. } => None,
        }
    }
}

pub struct PidLock {
    path: PathBuf,
    pid: u32,
}

impl PidLock {
    pub fn acquire_default() -> Result<Self, PidLockError> {
        let path = std::env::var("DL_BOT_PID_FILE")
            .ok()
            .map(|raw| raw.trim().to_string())
            .filter(|raw| !raw.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(DEFAULT_PID_FILE));
        Self::acquire(path)
    }

    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, PidLockError> {
        let path = path.as_ref().to_path_buf();
        let pid = std::process::id();
        loop {
            match create_pid_file(&path, pid) {
                Ok(()) => return Ok(Self { path, pid }),
                Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Some(existing_pid) = read_pid(&path) {
                        if pid_exists(existing_pid) {
                            return Err(PidLockError::AlreadyRunning {
                                pid: existing_pid,
                                path,
                            });
                        }
                    }
                    match fs::remove_file(&path) {
                        Ok(()) => continue,
                        Err(remove_err) if remove_err.kind() == std::io::ErrorKind::NotFound => {
                            continue;
                        }
                        Err(source) => return Err(PidLockError::Io { path, source }),
                    }
                }
                Err(source) => return Err(PidLockError::Io { path, source }),
            }
        }
    }
}

impl Drop for PidLock {
    fn drop(&mut self) {
        if read_pid(&self.path) == Some(self.pid) {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn create_pid_file(path: &Path, pid: u32) -> Result<(), std::io::Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    writeln!(file, "{pid}")?;
    Ok(())
}

fn read_pid(path: &Path) -> Option<u32> {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| raw.trim().parse::<u32>().ok())
        .filter(|pid| *pid > 0)
}

fn pid_exists(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn lookup_from<'a>(map: &'a HashMap<&'a str, &'a str>) -> impl Fn(&str) -> Option<String> + 'a {
        move |key| map.get(key).map(|value| (*value).to_string())
    }

    #[test]
    fn startup_sync_defaults_to_on_and_guild_scope() {
        let cfg = CommandSyncStartupConfig::from_lookup(|_| None);
        assert!(cfg.enabled);
        assert_eq!(cfg.scope, CommandSyncScope::Guild);
        assert_eq!(cfg.guild_id, Some(1_289_721_245_281_292_288));
    }

    #[test]
    fn startup_sync_keeps_existing_env_overrides() {
        let map = HashMap::from([
            ("DL_BOT_COMMAND_SYNC", "0"),
            ("DL_BOT_COMMAND_GUILD_ID", "42"),
        ]);
        let cfg = CommandSyncStartupConfig::from_lookup(lookup_from(&map));
        assert!(!cfg.enabled);
        assert_eq!(cfg.guild_id, Some(42));
    }

    #[test]
    fn master_prefix_is_owner_only_and_accepts_aliases() {
        assert!(is_owner(42, Some(42)));
        assert!(!is_owner(7, Some(42)));
        assert!(!is_owner(42, None));

        assert_eq!(
            MasterCommand::parse("!master status"),
            Some(MasterCommand::Status)
        );
        assert_eq!(MasterCommand::parse("!m s"), Some(MasterCommand::Status));
        assert_eq!(
            MasterCommand::parse("!master restart"),
            Some(MasterCommand::Restart)
        );
        assert_eq!(
            MasterCommand::parse("!master sync_commands guild auto"),
            Some(MasterCommand::SyncCommands {
                scope: CommandSyncScope::Guild,
                force: false
            })
        );
    }

    #[test]
    fn status_embed_uses_python_surface_text() {
        let embed = status_embed(&MasterStatusSnapshot {
            startup_text: "27.06.2026 12:34:56".to_string(),
            guild_count: 2,
            user_count: 5,
            command_count: 17,
            active_modules: vec!["faq".to_string(), "voice".to_string()],
        });

        assert_eq!(embed["title"], "📊 Master Bot Status");
        assert_eq!(embed["description"], "Bot läuft seit: 27.06.2026 12:34:56");
        assert_eq!(embed["fields"][0]["name"], "🔧 System");
        assert_eq!(embed["fields"][1]["name"], "📦 Loaded Cogs (2)");
    }

    #[test]
    fn restart_uses_non_success_exit_code_for_systemd_restart() {
        assert_ne!(restart_exit_code(), std::process::ExitCode::SUCCESS);
    }

    #[test]
    fn pid_lock_rejects_second_live_instance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("dl-bot-rust.pid");
        let first = PidLock::acquire(&path).expect("first lock");
        let second = PidLock::acquire(&path);
        assert!(matches!(second, Err(PidLockError::AlreadyRunning { .. })));
        drop(first);
        assert!(!path.exists());
    }
}
