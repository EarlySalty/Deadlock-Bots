use std::sync::Arc;
use std::time::{Duration, Instant};

use dl_activity::lfg::{lane_activity_label, LaneActivity, LaneInfo, LaneLabel};
use dl_central_db::kv;
use dl_discord::Dispatcher;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use tokio::sync::Mutex;

use crate::router::{
    NON_LANE_CHANNEL_IDS, ROUTER_ACCENT_GOLD, ROUTER_COMPONENTS_V2_FLAG, ROUTER_EMOJI_BRAWL,
    ROUTER_EMOJI_CASUAL, ROUTER_EMOJI_RANKED, ROUTER_VC_ID,
};

pub const LANE_PANEL_KV_NS: &str = "lane_live_panel";
pub const LANE_PANEL_TICK: Duration = Duration::from_secs(15);
pub const LANE_PANEL_MIN_EDIT_GAP: Duration = Duration::from_secs(30);
pub const LANE_PANEL_HEARTBEAT: Duration = Duration::from_secs(10 * 60);
pub const LANE_PANEL_REPOST_AFTER_MESSAGES: usize = 5;
pub const LANE_PANEL_REPOST_QUIET: Duration = Duration::from_secs(90);
pub const LANE_PANEL_FORCE_REPOST_AFTER_MESSAGES: usize = 20;
pub const LANE_PANEL_MAX_ROWS: usize = 12;
const UNLIMITED_LANE_LIMIT: usize = 99;
const EMOJI_NEW_PLAYER: (&str, &str) = ("dl_rang_einsteiger", "1522801035922636900");

#[async_trait::async_trait]
pub trait LanePanelPort: Send + Sync {
    async fn lanes(&self, guild_id: u64) -> Vec<LaneInfo>;
    async fn post(&self, channel_id: u64, body: &Map<String, Value>) -> Result<u64, String>;
    async fn edit(
        &self,
        channel_id: u64,
        message_id: u64,
        body: &Map<String, Value>,
    ) -> Result<(), String>;
    async fn delete(&self, channel_id: u64, message_id: u64) -> Result<(), String>;
    async fn messages_after(&self, channel_id: u64, message_id: u64) -> Result<usize, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelAction {
    Post,
    Repost,
    Edit,
    Keep,
}

#[derive(Debug, Default)]
struct PanelState {
    loaded: bool,
    message_id: Option<u64>,
    last_key: Option<String>,
    last_edit: Option<Instant>,
    messages_since: usize,
    last_user_message: Option<Instant>,
}

impl PanelState {
    fn decide(&self, key: &str, now: Instant) -> PanelAction {
        if self.message_id.is_none() {
            return PanelAction::Post;
        }
        let quiet = self
            .last_user_message
            .is_none_or(|at| now.duration_since(at) >= LANE_PANEL_REPOST_QUIET);
        if self.messages_since >= LANE_PANEL_FORCE_REPOST_AFTER_MESSAGES
            || (self.messages_since >= LANE_PANEL_REPOST_AFTER_MESSAGES && quiet)
        {
            return PanelAction::Repost;
        }
        let since_edit = self.last_edit.map(|at| now.duration_since(at));
        if since_edit.is_some_and(|gap| gap < LANE_PANEL_MIN_EDIT_GAP) {
            return PanelAction::Keep;
        }
        let changed = self.last_key.as_deref() != Some(key);
        let heartbeat_due = since_edit.is_none_or(|gap| gap >= LANE_PANEL_HEARTBEAT);
        if changed || heartbeat_due {
            PanelAction::Edit
        } else {
            PanelAction::Keep
        }
    }
}

fn is_full(lane: &LaneInfo) -> bool {
    lane.member_count >= lane.user_limit
}

fn row_tier(lane: &LaneInfo) -> u8 {
    if is_full(lane) {
        return 3;
    }
    match lane.activity {
        Some(LaneActivity::Lobby) => 0,
        None => 1,
        Some(LaneActivity::Match { .. }) => 2,
    }
}

pub fn panel_rows(lanes: &[LaneInfo]) -> Vec<&LaneInfo> {
    let mut rows: Vec<&LaneInfo> = lanes
        .iter()
        .filter(|lane| lane.member_count > 0 && !NON_LANE_CHANNEL_IDS.contains(&lane.channel_id))
        .collect();
    rows.sort_by_key(|lane| (row_tier(lane), std::cmp::Reverse(lane.member_count)));
    rows
}

fn label_emoji(label: &LaneLabel) -> String {
    let (name, id) = match label {
        LaneLabel::Casual => ROUTER_EMOJI_CASUAL,
        LaneLabel::Ranked => ROUTER_EMOJI_RANKED,
        LaneLabel::StreetBrawl => ROUTER_EMOJI_BRAWL,
        LaneLabel::NewPlayer => EMOJI_NEW_PLAYER,
    };
    format!("<:{name}:{id}>")
}

fn player_count_text(lane: &LaneInfo) -> String {
    if lane.user_limit >= UNLIMITED_LANE_LIMIT {
        format!("{} im Voice", lane.member_count)
    } else {
        format!("**{}/{}**", lane.member_count, lane.user_limit)
    }
}

fn known_rank_label(lane: &LaneInfo) -> Option<&str> {
    let label = lane.avg_rank_label.as_str();
    (lane.avg_rank_value > 0.0 && label != "Leer" && label != "Unbekannt").then_some(label)
}

pub fn panel_row_text(lane: &LaneInfo) -> String {
    let mut parts = vec![
        format!("{} <#{}>", label_emoji(&lane.label), lane.channel_id),
        player_count_text(lane),
    ];
    if let Some(rank) = known_rank_label(lane) {
        parts.push(format!("Ø {rank}"));
    }
    if let Some(activity) = lane.activity {
        parts.push(lane_activity_label(activity));
    }
    parts.join(" · ")
}

pub fn panel_rows_text(rows: &[&LaneInfo]) -> String {
    if rows.is_empty() {
        return format!(
            "Gerade ist keine Lane besetzt. Tritt <#{ROUTER_VC_ID}> bei, dann bekommst du deine eigene Lane und andere finden dich hier."
        );
    }
    let mut lines: Vec<String> = rows
        .iter()
        .take(LANE_PANEL_MAX_ROWS)
        .map(|lane| panel_row_text(lane))
        .collect();
    if rows.len() > LANE_PANEL_MAX_ROWS {
        lines.push(format!(
            "-# und {} weitere",
            rows.len() - LANE_PANEL_MAX_ROWS
        ));
    }
    lines.join("\n")
}

pub fn panel_body(rows_text: &str, has_rows: bool, stand_unix: i64) -> Map<String, Value> {
    let mut blocks = vec![
        json!({ "type": 10, "content": format!("## Aktive Lanes\n-# Stand <t:{stand_unix}:R>") }),
        json!({ "type": 14, "divider": true, "spacing": 1 }),
        json!({ "type": 10, "content": rows_text }),
    ];
    if has_rows {
        blocks.push(json!({ "type": 14, "divider": false, "spacing": 1 }));
        blocks.push(json!({
            "type": 10,
            "content": format!("-# Lobby- und Matchstatus kommt aus Steam, sobald jemand in der Lane sein Konto verknüpft hat. Eigene Lane: <#{ROUTER_VC_ID}>")
        }));
    }
    let mut body = Map::new();
    body.insert("flags".into(), json!(ROUTER_COMPONENTS_V2_FLAG));
    body.insert("allowed_mentions".into(), json!({ "parse": [] }));
    body.insert(
        "components".into(),
        json!([{ "type": 17, "accent_color": ROUTER_ACCENT_GOLD, "components": blocks }]),
    );
    body
}

pub struct LanePanel {
    pool: PgPool,
    port: Arc<dyn LanePanelPort>,
    guild_id: u64,
    channel_id: u64,
    state: Mutex<PanelState>,
}

impl LanePanel {
    pub fn new(
        pool: PgPool,
        port: Arc<dyn LanePanelPort>,
        guild_id: u64,
        channel_id: u64,
    ) -> Arc<Self> {
        Arc::new(Self {
            pool,
            port,
            guild_id,
            channel_id,
            state: Mutex::new(PanelState::default()),
        })
    }

    fn kv_key(&self) -> String {
        format!("message_id:{}:{}", self.guild_id, self.channel_id)
    }

    async fn load_state(&self, state: &mut PanelState) {
        let stored = match kv::get(&self.pool, LANE_PANEL_KV_NS, &self.kv_key()).await {
            Ok(value) => value.and_then(|raw| raw.parse::<u64>().ok()),
            Err(err) => {
                tracing::warn!(%err, "LanePanel: Message-ID konnte nicht geladen werden");
                return;
            }
        };
        state.loaded = true;
        state.message_id = stored;
        if let Some(message_id) = stored {
            match self.port.messages_after(self.channel_id, message_id).await {
                Ok(count) => state.messages_since = count,
                Err(err) => tracing::warn!(%err, "LanePanel: Nachrichten seit Panel unbekannt"),
            }
        }
    }

    async fn store_message_id(&self, message_id: u64) {
        if let Err(err) = kv::set(
            &self.pool,
            LANE_PANEL_KV_NS,
            &self.kv_key(),
            &message_id.to_string(),
        )
        .await
        {
            tracing::warn!(%err, "LanePanel: Message-ID konnte nicht gespeichert werden");
        }
    }

    pub async fn note_user_message(&self, channel_id: u64) {
        if channel_id != self.channel_id {
            return;
        }
        let mut state = self.state.lock().await;
        state.messages_since += 1;
        state.last_user_message = Some(Instant::now());
    }

    pub async fn tick(&self) {
        let mut state = self.state.lock().await;
        if !state.loaded {
            self.load_state(&mut state).await;
            if !state.loaded {
                return;
            }
        }
        let lanes = self.port.lanes(self.guild_id).await;
        let rows = panel_rows(&lanes);
        let key = panel_rows_text(&rows);
        let now = Instant::now();
        let body = panel_body(&key, !rows.is_empty(), chrono::Utc::now().timestamp());
        match state.decide(&key, now) {
            PanelAction::Keep => {}
            PanelAction::Post | PanelAction::Repost => {
                self.repost(&mut state, &body, key, now).await;
            }
            PanelAction::Edit => {
                let Some(message_id) = state.message_id else {
                    return;
                };
                match self.port.edit(self.channel_id, message_id, &body).await {
                    Ok(()) => {
                        state.last_key = Some(key);
                        state.last_edit = Some(now);
                    }
                    Err(err) => {
                        tracing::warn!(%err, message_id, "LanePanel: Edit fehlgeschlagen, poste neu");
                        self.repost(&mut state, &body, key, now).await;
                    }
                }
            }
        }
    }

    async fn repost(
        &self,
        state: &mut PanelState,
        body: &Map<String, Value>,
        key: String,
        now: Instant,
    ) {
        let message_id = match self.port.post(self.channel_id, body).await {
            Ok(id) => id,
            Err(err) => {
                tracing::warn!(%err, "LanePanel: Panel konnte nicht gepostet werden");
                return;
            }
        };
        self.store_message_id(message_id).await;
        if let Some(old) = state.message_id.replace(message_id) {
            if let Err(err) = self.port.delete(self.channel_id, old).await {
                tracing::info!(%err, old, "LanePanel: altes Panel nicht gelöscht");
            }
        }
        state.last_key = Some(key);
        state.last_edit = Some(now);
        state.messages_since = 0;
        state.last_user_message = None;
    }
}

pub fn spawn(panel: Arc<LanePanel>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    let listener = panel.clone();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => listener.note_user_message(event.channel_id).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    tokio::spawn(async move {
        loop {
            panel.tick().await;
            tokio::time::sleep(LANE_PANEL_TICK).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane(id: u64, members: usize, limit: usize, activity: Option<LaneActivity>) -> LaneInfo {
        LaneInfo {
            channel_id: id,
            label: LaneLabel::Casual,
            member_count: members,
            user_limit: limit,
            avg_rank_value: 0.0,
            co_players_present: 0,
            name: format!("Lane {id}"),
            avg_rank_label: "Leer".to_string(),
            category_id: 0,
            position: id as i64,
            is_staging: false,
            co_player_names: Vec::new(),
            member_ids: Vec::new(),
            activity,
        }
    }

    #[test]
    fn rows_order_joinable_lobby_first_full_last() {
        let lanes = vec![
            lane(1, 6, 6, Some(LaneActivity::Lobby)),
            lane(2, 3, 6, Some(LaneActivity::Match { minute: 14 })),
            lane(3, 2, 6, None),
            lane(4, 4, 6, Some(LaneActivity::Lobby)),
            lane(5, 0, 6, None),
            lane(ROUTER_VC_ID, 1, 99, None),
        ];
        let order: Vec<u64> = panel_rows(&lanes).iter().map(|l| l.channel_id).collect();
        assert_eq!(order, vec![4, 3, 2, 1]);
    }

    #[test]
    fn row_without_presence_has_count_only() {
        let text = panel_row_text(&lane(3, 2, 6, None));
        assert!(text.ends_with("**2/6**"), "{text}");
        let text = panel_row_text(&lane(2, 3, 6, Some(LaneActivity::Match { minute: 14 })));
        assert!(text.ends_with("**3/6** · Im Match · Min 14"), "{text}");
    }

    #[test]
    fn repost_needs_enough_messages_and_quiet() {
        let now = Instant::now();
        let mut state = PanelState {
            loaded: true,
            message_id: Some(1),
            last_key: Some("k".into()),
            last_edit: Some(now),
            messages_since: LANE_PANEL_REPOST_AFTER_MESSAGES,
            last_user_message: Some(now),
        };
        assert_eq!(state.decide("k", now), PanelAction::Keep);
        state.last_user_message = now.checked_sub(LANE_PANEL_REPOST_QUIET);
        assert_eq!(state.decide("k", now), PanelAction::Repost);
        state.messages_since = 0;
        assert_eq!(state.decide("x", now), PanelAction::Keep);
        state.last_edit = now.checked_sub(LANE_PANEL_MIN_EDIT_GAP);
        assert_eq!(state.decide("x", now), PanelAction::Edit);
        assert_eq!(state.decide("k", now), PanelAction::Keep);
    }
}
