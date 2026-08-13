//! Lane-Pairing — zwei Leute, die je allein in ihrer eigenen Lane sitzen,
//! werden gefragt, ob sie zusammen zocken wollen.
//!
//! Doppel-Opt-in: der Bot verschiebt niemanden, bevor beide zugesagt haben.
//! Wer zuerst Ja klickt, hinterlässt eine Zusage (kv, mit Ablauf); klickt der
//! andere innerhalb des Fensters auch Ja, zieht der zweite Zusager in die Lane
//! des ersten um. Alles andere (Ablehnung, Ablauf, Lane verlassen) endet
//! folgenlos, außer dass der Cooldown steht.
//!
//! Gefragt wird nur, wer seit [`MIN_ALONE`] allein in einer TempVoice-Lane
//! desselben Modus sitzt, nicht per Privacy abgemeldet ist, den Opt-out nicht
//! gesetzt hat und weder im eigenen noch im Paar-Cooldown steckt.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use dl_central_db::kv;
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgPool;

pub const MAIN_GUILD_ID: u64 = 1289721245281292288;
/// Kategorie, in der alle Router-Lanes liegen (`TEMPVOICE_ONE_CATEGORY_ID`).
pub const LANE_CATEGORY_ID: u64 = 1289721245281292290;
/// So lange muss jemand allein sitzen, bevor der Bot ihn anspricht.
pub const MIN_ALONE: Duration = Duration::seconds(180);
/// So lange gilt die Zusage des Ersten, bis sie verfällt.
pub const ACCEPT_WINDOW: Duration = Duration::minutes(10);
/// Pro User: so lange keine neue Pairing-Frage.
pub const ASK_COOLDOWN: Duration = Duration::hours(2);
/// Pro Paar: so lange nicht erneut dieselben zwei zusammenbringen wollen.
pub const PAIR_COOLDOWN: Duration = Duration::hours(24);
pub const TICK_INTERVAL: StdDuration = StdDuration::from_secs(20);

pub const NEVER_ASK_NS: &str = "voice_pair_never";
pub const LAST_ASK_NS: &str = "voice_pair_last_ask";
pub const PAIR_ASKED_NS: &str = "voice_pair_last_pair";
pub const ACCEPT_NS: &str = "voice_pair_accept";

pub const CUSTOM_ID_PREFIX: &str = "voice_pair:";
pub const YES_ACTION: &str = "yes";
pub const NO_ACTION: &str = "no";
pub const NEVER_ACTION: &str = "never";

pub const ACCENT_GOLD: u64 = 0xC8A86B;
pub const COMPONENTS_V2_FLAG: u64 = 1 << 15;

pub const WAITING_REPLY: &str =
    "Alles klar, ich frag den anderen. Wenn er auch will, zieh ich euch zusammen.";
pub const MOVED_REPLY: &str = "Passt, ihr seid jetzt zusammen in einer Lane. Viel Spaß.";
pub const MOVE_FAILED_REPLY: &str =
    "Das Verschieben hat gerade nicht geklappt. Spring einfach selbst rüber, die Lane steht.";
pub const GONE_REPLY: &str =
    "Zu spät, der andere sitzt da nicht mehr. Ich meld mich, wenn wieder jemand allein ist.";
pub const NOT_IN_LANE_REPLY: &str =
    "Du sitzt gerade nicht mehr in deiner Lane. Geh wieder rein, dann klappt es.";
pub const NO_REPLY: &str = "Alles gut, dann nicht. Ich lass dich in Ruhe zocken.";
pub const NEVER_REPLY: &str =
    "Erledigt, ich frag dich nicht mehr wegen Mitspielern. Über die Mitspieler-Suche findest du trotzdem jederzeit Leute.";
pub const NEVER_FAILED_REPLY: &str =
    "Das konnte ich gerade nicht speichern. Probier es gleich nochmal.";
pub const INVALID_REPLY: &str = "Diese Anfrage ist abgelaufen.";

/// Eine Lane, wie der Watcher sie sieht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneView {
    pub channel_id: u64,
    /// Router-Modus der Lane (`casual`, `ranked`, `street_brawl`).
    pub mode: String,
    pub members: Vec<u64>,
}

/// Ein Einzelsitzer, den der Watcher schon eine Weile beobachtet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoloSeat {
    pub user_id: u64,
    pub channel_id: u64,
    pub mode: String,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskDecision {
    Ask,
    OptedOut,
    NeverAsk,
    UserCooldown,
    PairCooldown,
    /// Einer der beiden hat den anderen in der Mitspieler-Umfrage abgelehnt.
    Unwanted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAccept {
    pub user_id: u64,
    pub channel_id: u64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Watched {
    channel_id: u64,
    since: DateTime<Utc>,
}

#[async_trait::async_trait]
pub trait PairingPort: Send + Sync {
    /// Alle TempVoice-Lanes der Lane-Kategorie mit Modus und Mitgliedern.
    async fn lane_views(&self, guild_id: u64) -> Vec<LaneView>;
    /// Privacy, Opt-out und beide Cooldowns prüfen und die Frage buchen.
    async fn claim_ask(
        &self,
        first: u64,
        second: u64,
        now: DateTime<Utc>,
    ) -> Result<AskDecision, String>;
    async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String>;
    async fn set_never_ask(&self, user_id: u64) -> Result<(), String>;
    async fn load_accept(&self, pair_key: &str) -> Result<Option<PendingAccept>, String>;
    async fn save_accept(&self, pair_key: &str, accept: &PendingAccept) -> Result<(), String>;
    async fn clear_accept(&self, pair_key: &str) -> Result<(), String>;
    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64)
        -> Result<(), String>;
    fn log_decision(&self, user_id: u64, decision: &'static str, reason: &'static str);
}

pub struct LanePairing {
    port: Arc<dyn PairingPort>,
    watched: tokio::sync::Mutex<HashMap<u64, Watched>>,
}

impl LanePairing {
    pub fn new(port: Arc<dyn PairingPort>) -> Arc<Self> {
        Arc::new(Self {
            port,
            watched: tokio::sync::Mutex::new(HashMap::new()),
        })
    }

    /// Ein Durchlauf: Einzelsitzer fortschreiben, dann höchstens ein Paar fragen.
    pub async fn tick_at(&self, guild_id: u64, now: DateTime<Utc>) {
        let lanes = self.port.lane_views(guild_id).await;
        let seats = self.update_watched(&lanes, now).await;
        let Some((first, second)) = pick_pair(&seats, now) else {
            return;
        };
        match self
            .port
            .claim_ask(first.user_id, second.user_id, now)
            .await
        {
            Err(error) => {
                tracing::warn!(%error, "Lane-Pairing: Eignungsprüfung fehlgeschlagen");
            }
            Ok(AskDecision::OptedOut) => {
                self.port
                    .log_decision(first.user_id, "verworfen", "privacy_opt_out");
            }
            Ok(AskDecision::NeverAsk) => {
                self.port
                    .log_decision(first.user_id, "verworfen", "nie_fragen");
            }
            Ok(AskDecision::UserCooldown) => {
                self.port
                    .log_decision(first.user_id, "verworfen", "user_cooldown");
            }
            Ok(AskDecision::PairCooldown) => {
                self.port
                    .log_decision(first.user_id, "verworfen", "paar_cooldown");
            }
            Ok(AskDecision::Unwanted) => {
                self.port
                    .log_decision(first.user_id, "verworfen", "paarung_abgelehnt");
            }
            Ok(AskDecision::Ask) => {
                self.ask(guild_id, &first, &second).await;
                self.ask(guild_id, &second, &first).await;
            }
        }
    }

    async fn ask(&self, guild_id: u64, seat: &SoloSeat, other: &SoloSeat) {
        let body = dm_body(guild_id, seat, other);
        match self.port.send_dm(seat.user_id, body).await {
            Ok(()) => self
                .port
                .log_decision(seat.user_id, "dm_zugestellt", "pairing_vorschlag"),
            Err(_) => self
                .port
                .log_decision(seat.user_id, "dm_fehlgeschlagen", "discord_fehler"),
        }
    }

    /// Einzelsitzer-Zeitstempel fortschreiben und die reifen Sitze liefern.
    async fn update_watched(&self, lanes: &[LaneView], now: DateTime<Utc>) -> Vec<SoloSeat> {
        let mut watched = self.watched.lock().await;
        let mut seats = Vec::new();
        let mut still_alone = Vec::new();
        for lane in lanes {
            let [user_id] = lane.members.as_slice() else {
                continue;
            };
            still_alone.push(*user_id);
            let since = match watched.get(user_id) {
                Some(state) if state.channel_id == lane.channel_id => state.since,
                _ => {
                    watched.insert(
                        *user_id,
                        Watched {
                            channel_id: lane.channel_id,
                            since: now,
                        },
                    );
                    now
                }
            };
            seats.push(SoloSeat {
                user_id: *user_id,
                channel_id: lane.channel_id,
                mode: lane.mode.clone(),
                since,
            });
        }
        watched.retain(|user_id, _| still_alone.contains(user_id));
        seats
    }

    pub async fn handle_interaction(&self, interaction: BridgeInteraction) -> BridgeReply {
        let Some(action) = parse_custom_id(&interaction.custom_id) else {
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        };
        match action {
            PairAction::Never => match self.port.set_never_ask(interaction.user_id).await {
                Ok(()) => {
                    self.port
                        .log_decision(interaction.user_id, "abgelehnt", "nie_fragen_gewaehlt");
                    BridgeReply::ephemeral_text(NEVER_REPLY)
                }
                Err(_) => BridgeReply::ephemeral_text(NEVER_FAILED_REPLY),
            },
            PairAction::No { .. } => {
                self.port
                    .log_decision(interaction.user_id, "abgelehnt", "nicht_jetzt_gewaehlt");
                BridgeReply::ephemeral_text(NO_REPLY)
            }
            PairAction::Yes {
                guild_id,
                own_lane,
                other_lane,
                other_user,
            } => {
                self.handle_yes(
                    interaction.user_id,
                    guild_id,
                    own_lane,
                    other_lane,
                    other_user,
                    Utc::now(),
                )
                .await
            }
        }
    }

    async fn handle_yes(
        &self,
        user_id: u64,
        guild_id: u64,
        own_lane: u64,
        other_lane: u64,
        other_user: u64,
        now: DateTime<Utc>,
    ) -> BridgeReply {
        if self.port.member_voice_channel(guild_id, user_id).await != Some(own_lane) {
            self.port
                .log_decision(user_id, "verworfen", "nicht_mehr_in_lane");
            return BridgeReply::ephemeral_text(NOT_IN_LANE_REPLY);
        }
        let key = pair_key(own_lane, other_lane);
        let stored = self.port.load_accept(&key).await.unwrap_or(None);
        let partner_ready = stored
            .as_ref()
            .filter(|accept| accept.user_id == other_user)
            .filter(|accept| accept_is_fresh(accept, now))
            .is_some();
        if !partner_ready {
            let accept = PendingAccept {
                user_id,
                channel_id: own_lane,
                created_at: now.timestamp(),
            };
            if let Err(error) = self.port.save_accept(&key, &accept).await {
                tracing::warn!(%error, user_id, "Lane-Pairing: Zusage nicht speicherbar");
            }
            self.port.log_decision(user_id, "zugesagt", "wartet_auf_2");
            return BridgeReply::ephemeral_text(WAITING_REPLY);
        }
        if self.port.member_voice_channel(guild_id, other_user).await != Some(other_lane) {
            let _ = self.port.clear_accept(&key).await;
            self.port.log_decision(user_id, "verworfen", "partner_weg");
            return BridgeReply::ephemeral_text(GONE_REPLY);
        }
        let _ = self.port.clear_accept(&key).await;
        match self.port.move_member(guild_id, user_id, other_lane).await {
            Ok(()) => {
                self.port
                    .log_decision(user_id, "zusammengelegt", "beide_zugesagt");
                let _ = self
                    .port
                    .send_dm(other_user, merged_dm_body(user_id, other_lane))
                    .await;
                BridgeReply::ephemeral_text(MOVED_REPLY)
            }
            Err(error) => {
                tracing::warn!(%error, user_id, other_lane, "Lane-Pairing: Move fehlgeschlagen");
                self.port
                    .log_decision(user_id, "move_fehlgeschlagen", "discord_fehler");
                BridgeReply::ephemeral_text(MOVE_FAILED_REPLY)
            }
        }
    }
}

/// Höchstens ein Paar pro Durchlauf: die zwei am längsten Wartenden im selben
/// Modus. Beide müssen [`MIN_ALONE`] voll haben.
pub fn pick_pair(seats: &[SoloSeat], now: DateTime<Utc>) -> Option<(SoloSeat, SoloSeat)> {
    let mut ripe: Vec<&SoloSeat> = seats
        .iter()
        .filter(|seat| now - seat.since >= MIN_ALONE)
        .collect();
    ripe.sort_by_key(|seat| (seat.since, seat.user_id));
    for (index, first) in ripe.iter().enumerate() {
        for second in ripe.iter().skip(index + 1) {
            if first.mode == second.mode && first.channel_id != second.channel_id {
                return Some(((*first).clone(), (*second).clone()));
            }
        }
    }
    None
}

/// Paar-Schlüssel, unabhängig davon, wer zuerst klickt.
pub fn pair_key(one: u64, other: u64) -> String {
    let (low, high) = if one <= other {
        (one, other)
    } else {
        (other, one)
    };
    format!("{low}:{high}")
}

pub fn accept_is_fresh(accept: &PendingAccept, now: DateTime<Utc>) -> bool {
    DateTime::from_timestamp(accept.created_at, 0)
        .is_some_and(|created_at| now - created_at < ACCEPT_WINDOW)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairAction {
    Yes {
        guild_id: u64,
        own_lane: u64,
        other_lane: u64,
        other_user: u64,
    },
    No {
        other_user: u64,
    },
    Never,
}

/// `voice_pair:yes:<guild>:<own_lane>:<other_lane>:<other_user>`,
/// `voice_pair:no:<other_user>`, `voice_pair:never`.
pub fn parse_custom_id(custom_id: &str) -> Option<PairAction> {
    let rest = custom_id.strip_prefix(CUSTOM_ID_PREFIX)?;
    let mut parts = rest.split(':');
    match parts.next()? {
        NEVER_ACTION => Some(PairAction::Never),
        NO_ACTION => Some(PairAction::No {
            other_user: parts.next()?.parse().ok()?,
        }),
        YES_ACTION => Some(PairAction::Yes {
            guild_id: parts.next()?.parse().ok()?,
            own_lane: parts.next()?.parse().ok()?,
            other_lane: parts.next()?.parse().ok()?,
            other_user: parts.next()?.parse().ok()?,
        }),
        _ => None,
    }
}

pub fn yes_custom_id(guild_id: u64, own_lane: u64, other_lane: u64, other_user: u64) -> String {
    format!("{CUSTOM_ID_PREFIX}{YES_ACTION}:{guild_id}:{own_lane}:{other_lane}:{other_user}")
}

pub fn no_custom_id(other_user: u64) -> String {
    format!("{CUSTOM_ID_PREFIX}{NO_ACTION}:{other_user}")
}

pub fn never_custom_id() -> String {
    format!("{CUSTOM_ID_PREFIX}{NEVER_ACTION}")
}

fn mode_label(mode: &str) -> &'static str {
    match mode {
        "ranked" => "Ranked",
        "street_brawl" => "Street Brawl",
        _ => "Casual",
    }
}

/// Vorschlags-DM. Erwähnungen bleiben stumm (`allowed_mentions.parse = []`),
/// der Name wird trotzdem aufgelöst.
pub fn dm_body(guild_id: u64, seat: &SoloSeat, other: &SoloSeat) -> Value {
    let label = mode_label(&seat.mode);
    json!({
        "flags": COMPONENTS_V2_FLAG,
        "allowed_mentions": { "parse": [] },
        "components": [{
            "type": 17,
            "accent_color": ACCENT_GOLD,
            "components": [
                { "type": 10, "content": "## 🎧 Da sitzt noch jemand allein\n-# Zwei einzelne Lanes, gleicher Modus. Zusammen macht es mehr Spaß." },
                { "type": 14, "divider": true, "spacing": 2 },
                { "type": 10, "content": format!(
                    "<@{}> sitzt gerade allein in <#{}>, du in <#{}>, beide auf **{label}**. Soll ich euch zusammenlegen?\n\nIch verschiebe niemanden von allein: erst wenn ihr beide zusagt, geht es los. Sagt der andere ab oder ist er weg, passiert einfach nichts.",
                    other.user_id, other.channel_id, seat.channel_id
                ) },
                { "type": 1, "components": [
                    { "type": 2, "style": 3, "label": "Ja, zusammen zocken", "custom_id": yes_custom_id(guild_id, seat.channel_id, other.channel_id, other.user_id) },
                    { "type": 2, "style": 2, "label": "Jetzt nicht", "custom_id": no_custom_id(other.user_id) },
                    { "type": 2, "style": 2, "label": "Nicht mehr fragen", "custom_id": never_custom_id() }
                ]}
            ]
        }]
    })
}

/// Bestätigung an den, der zuerst zugesagt hat.
pub fn merged_dm_body(joining_user: u64, lane_id: u64) -> Value {
    json!({
        "allowed_mentions": { "parse": [] },
        "content": format!("<@{joining_user}> kommt zu dir in <#{lane_id}>. Viel Spaß euch."),
    })
}

/// Privacy, Opt-out und beide Cooldowns in einer Transaktion; bei `Ask` sind
/// die Marker für beide User und das Paar direkt gesetzt.
pub(crate) async fn claim_ask_db(
    pool: &PgPool,
    first: u64,
    second: u64,
    now: DateTime<Utc>,
) -> Result<AskDecision, String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    for user_id in [first, second] {
        let user_id_i64 = i64::try_from(user_id).map_err(|error| error.to_string())?;
        if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, user_id_i64)
            .await
            .map_err(|error| error.to_string())?
        {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(AskDecision::OptedOut);
        }
        let key = user_id.to_string();
        let never: Option<String> =
            sqlx::query_scalar("SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2")
                .bind(NEVER_ASK_NS)
                .bind(&key)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|error| error.to_string())?;
        if never.is_some() {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(AskDecision::NeverAsk);
        }
        if let Some(last) = read_timestamp(&mut tx, LAST_ASK_NS, &key).await? {
            if now - last < ASK_COOLDOWN {
                tx.commit().await.map_err(|error| error.to_string())?;
                return Ok(AskDecision::UserCooldown);
            }
        }
    }
    let pair = pair_key(first, second);
    if let Some(last) = read_timestamp(&mut tx, PAIR_ASKED_NS, &pair).await? {
        if now - last < PAIR_COOLDOWN {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(AskDecision::PairCooldown);
        }
    }
    // Wer in der Mitspieler-Umfrage "lieber nicht" geklickt hat, bekommt genau
    // diese Paarung nie wieder vorgeschlagen.
    if pair_is_unwanted_tx(&mut tx, first, second).await? {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(AskDecision::Unwanted);
    }
    for (ns, key) in [
        (LAST_ASK_NS, first.to_string()),
        (LAST_ASK_NS, second.to_string()),
        (PAIR_ASKED_NS, pair),
    ] {
        sqlx::query(
            "INSERT INTO bot.kv_store (ns, k, v)
             VALUES ($1, $2, $3)
             ON CONFLICT (ns, k) DO UPDATE SET v = EXCLUDED.v",
        )
        .bind(ns)
        .bind(&key)
        .bind(now.timestamp().to_string())
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(AskDecision::Ask)
}

/// Letztes Urteil der Paarung aus der Mitspieler-Umfrage, in derselben
/// Transaktion wie die Cooldown-Prüfung.
async fn pair_is_unwanted_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    one: u64,
    other: u64,
) -> Result<bool, String> {
    let one = i64::try_from(one).map_err(|error| error.to_string())?;
    let other = i64::try_from(other).map_err(|error| error.to_string())?;
    let unwanted: Option<bool> = sqlx::query_scalar(
        "SELECT rating = $3
           FROM activity.voice_mate_ratings
          WHERE (rater_user_id = $1 AND mate_user_id = $2)
             OR (rater_user_id = $2 AND mate_user_id = $1)
          ORDER BY created_at DESC
          LIMIT 1",
    )
    .bind(one)
    .bind(other)
    .bind(crate::mate_survey::RATING_RATHER_NOT)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| error.to_string())?;
    Ok(unwanted.unwrap_or(false))
}

async fn read_timestamp(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ns: &str,
    key: &str,
) -> Result<Option<DateTime<Utc>>, String> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2 FOR UPDATE")
            .bind(ns)
            .bind(key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| error.to_string())?;
    let Some(value) = value else {
        return Ok(None);
    };
    let timestamp = value
        .parse::<i64>()
        .map_err(|error| format!("ungültiger Pairing-Zeitstempel: {error}"))?;
    Ok(DateTime::from_timestamp(timestamp, 0))
}

pub(crate) async fn set_never_ask_db(pool: &PgPool, user_id: u64) -> Result<(), String> {
    kv::set(pool, NEVER_ASK_NS, &user_id.to_string(), "true")
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn load_accept_db(
    pool: &PgPool,
    pair_key: &str,
) -> Result<Option<PendingAccept>, String> {
    let value = kv::get(pool, ACCEPT_NS, pair_key)
        .await
        .map_err(|error| error.to_string())?;
    value
        .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()
}

pub(crate) async fn save_accept_db(
    pool: &PgPool,
    pair_key: &str,
    accept: &PendingAccept,
) -> Result<(), String> {
    let value = serde_json::to_string(accept).map_err(|error| error.to_string())?;
    kv::set(pool, ACCEPT_NS, pair_key, &value)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn clear_accept_db(pool: &PgPool, pair_key: &str) -> Result<(), String> {
    kv::delete(pool, ACCEPT_NS, pair_key)
        .await
        .map_err(|error| error.to_string())
}

struct PairingHandler {
    pairing: Arc<LanePairing>,
}

#[async_trait::async_trait]
impl InteractionHandler for PairingHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        self.pairing.handle_interaction(interaction).await
    }
}

pub fn register(router: &mut InteractionRouter, pairing: Arc<LanePairing>) {
    router.on_prefix(CUSTOM_ID_PREFIX, Arc::new(PairingHandler { pairing }));
}

pub fn spawn(pairing: Arc<LanePairing>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(
            takt_sekunden = TICK_INTERVAL.as_secs(),
            allein_ab_sekunden = MIN_ALONE.num_seconds(),
            "Lane-Pairing aktiv"
        );
        let mut ticker = tokio::time::interval(TICK_INTERVAL);
        loop {
            ticker.tick().await;
            pairing.tick_at(MAIN_GUILD_ID, Utc::now()).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn seat(user_id: u64, channel_id: u64, mode: &str, since: DateTime<Utc>) -> SoloSeat {
        SoloSeat {
            user_id,
            channel_id,
            mode: mode.to_string(),
            since,
        }
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp")
    }

    #[test]
    fn pick_pair_nimmt_die_zwei_laengsten_wartenden_im_selben_modus() {
        let now = now();
        let seats = vec![
            seat(1, 10, "casual", now - Duration::minutes(9)),
            seat(2, 11, "ranked", now - Duration::minutes(8)),
            seat(3, 12, "casual", now - Duration::minutes(7)),
        ];
        let (first, second) = pick_pair(&seats, now).expect("Paar");
        assert_eq!((first.user_id, second.user_id), (1, 3));
    }

    #[test]
    fn pick_pair_ignoriert_zu_frische_und_fremde_modi() {
        let now = now();
        let frisch = vec![
            seat(1, 10, "casual", now - Duration::seconds(10)),
            seat(2, 11, "casual", now - Duration::minutes(9)),
        ];
        assert!(pick_pair(&frisch, now).is_none());

        let gemischt = vec![
            seat(1, 10, "casual", now - Duration::minutes(9)),
            seat(2, 11, "ranked", now - Duration::minutes(9)),
        ];
        assert!(pick_pair(&gemischt, now).is_none());
    }

    #[test]
    fn pick_pair_paart_niemanden_mit_sich_selbst() {
        let now = now();
        let seats = vec![seat(1, 10, "casual", now - Duration::minutes(9))];
        assert!(pick_pair(&seats, now).is_none());
    }

    #[test]
    #[ignore = "Hilfsausgabe: druckt den echten Pairing-Vorschlag zum Gegenlesen"]
    fn dump_pairing_dm() {
        let now = now();
        println!(
            "{}",
            serde_json::to_string(&dm_body(
                MAIN_GUILD_ID,
                &seat(1, 1523272810825252944, "casual", now),
                &seat(2, 1513468587195633674, "casual", now),
            ))
            .expect("json")
        );
    }

    #[test]
    fn pair_key_ist_richtungsunabhaengig() {
        assert_eq!(pair_key(10, 20), pair_key(20, 10));
        assert_eq!(pair_key(10, 20), "10:20");
    }

    #[test]
    fn custom_ids_sind_round_trip_fest() {
        assert_eq!(
            parse_custom_id(&yes_custom_id(1, 2, 3, 4)),
            Some(PairAction::Yes {
                guild_id: 1,
                own_lane: 2,
                other_lane: 3,
                other_user: 4
            })
        );
        assert_eq!(
            parse_custom_id(&no_custom_id(7)),
            Some(PairAction::No { other_user: 7 })
        );
        assert_eq!(parse_custom_id(&never_custom_id()), Some(PairAction::Never));
        assert_eq!(parse_custom_id("voice_pair:quatsch"), None);
        assert_eq!(parse_custom_id("anderes:yes:1:2:3:4"), None);
    }

    #[test]
    fn accept_verfaellt_nach_dem_fenster() {
        let now = now();
        let frisch = PendingAccept {
            user_id: 1,
            channel_id: 10,
            created_at: (now - Duration::minutes(5)).timestamp(),
        };
        let alt = PendingAccept {
            user_id: 1,
            channel_id: 10,
            created_at: (now - Duration::minutes(30)).timestamp(),
        };
        assert!(accept_is_fresh(&frisch, now));
        assert!(!accept_is_fresh(&alt, now));
    }

    #[test]
    fn dm_body_traegt_beide_lanes_und_die_drei_buttons() {
        let now = now();
        let text = serde_json::to_string(&dm_body(
            1,
            &seat(10, 100, "casual", now),
            &seat(20, 200, "casual", now),
        ))
        .expect("json");
        assert!(text.contains("<#100>"));
        assert!(text.contains("<#200>"));
        assert!(text.contains("<@20>"));
        assert!(text.contains("Casual"));
        assert!(text.contains(&yes_custom_id(1, 100, 200, 20)));
        assert!(text.contains(&no_custom_id(20)));
        assert!(text.contains(&never_custom_id()));
        // Stumme Erwähnung: der Vorschlag darf niemanden anpingen.
        assert!(text.contains("\"parse\":[]"));
    }

    struct TestState {
        lanes: Vec<LaneView>,
        dms: Vec<(u64, Value)>,
        accepts: HashMap<String, PendingAccept>,
        moves: Vec<(u64, u64)>,
        voice: HashMap<u64, u64>,
        never: Vec<u64>,
        decision: AskDecision,
        move_fails: bool,
    }

    struct TestPort {
        state: StdMutex<TestState>,
    }

    impl TestPort {
        fn new(state: TestState) -> Arc<Self> {
            Arc::new(Self {
                state: StdMutex::new(state),
            })
        }
    }

    impl Default for TestState {
        fn default() -> Self {
            Self {
                lanes: Vec::new(),
                dms: Vec::new(),
                accepts: HashMap::new(),
                moves: Vec::new(),
                voice: HashMap::new(),
                never: Vec::new(),
                decision: AskDecision::Ask,
                move_fails: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl PairingPort for TestPort {
        async fn lane_views(&self, _guild_id: u64) -> Vec<LaneView> {
            self.state.lock().expect("lock").lanes.clone()
        }

        async fn claim_ask(
            &self,
            _first: u64,
            _second: u64,
            _now: DateTime<Utc>,
        ) -> Result<AskDecision, String> {
            Ok(self.state.lock().expect("lock").decision)
        }

        async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String> {
            self.state.lock().expect("lock").dms.push((user_id, body));
            Ok(())
        }

        async fn set_never_ask(&self, user_id: u64) -> Result<(), String> {
            self.state.lock().expect("lock").never.push(user_id);
            Ok(())
        }

        async fn load_accept(&self, pair_key: &str) -> Result<Option<PendingAccept>, String> {
            Ok(self
                .state
                .lock()
                .expect("lock")
                .accepts
                .get(pair_key)
                .cloned())
        }

        async fn save_accept(&self, pair_key: &str, accept: &PendingAccept) -> Result<(), String> {
            self.state
                .lock()
                .expect("lock")
                .accepts
                .insert(pair_key.to_string(), accept.clone());
            Ok(())
        }

        async fn clear_accept(&self, pair_key: &str) -> Result<(), String> {
            self.state.lock().expect("lock").accepts.remove(pair_key);
            Ok(())
        }

        async fn member_voice_channel(&self, _guild_id: u64, user_id: u64) -> Option<u64> {
            self.state
                .lock()
                .expect("lock")
                .voice
                .get(&user_id)
                .copied()
        }

        async fn move_member(
            &self,
            _guild_id: u64,
            user_id: u64,
            channel_id: u64,
        ) -> Result<(), String> {
            let mut state = self.state.lock().expect("lock");
            if state.move_fails {
                return Err("move kaputt".to_string());
            }
            state.moves.push((user_id, channel_id));
            state.voice.insert(user_id, channel_id);
            Ok(())
        }

        fn log_decision(&self, _user_id: u64, _decision: &'static str, _reason: &'static str) {}
    }

    fn lane(channel_id: u64, mode: &str, members: &[u64]) -> LaneView {
        LaneView {
            channel_id,
            mode: mode.to_string(),
            members: members.to_vec(),
        }
    }

    #[tokio::test]
    async fn tick_fragt_erst_nach_der_wartezeit_und_dann_beide() {
        let port = TestPort::new(TestState {
            lanes: vec![lane(100, "casual", &[10]), lane(200, "casual", &[20])],
            decision: AskDecision::Ask,
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let start = now();

        pairing.tick_at(1, start).await;
        assert!(
            port.state.lock().expect("lock").dms.is_empty(),
            "vor der Wartezeit fragt der Bot niemanden"
        );

        pairing.tick_at(1, start + Duration::minutes(5)).await;
        let dms = port.state.lock().expect("lock").dms.clone();
        assert_eq!(dms.len(), 2, "beide bekommen die Frage");
        assert_eq!(dms[0].0, 10);
        assert_eq!(dms[1].0, 20);
    }

    #[tokio::test]
    async fn tick_setzt_die_wartezeit_zurueck_wenn_die_lane_wechselt() {
        let port = TestPort::new(TestState {
            lanes: vec![lane(100, "casual", &[10]), lane(200, "casual", &[20])],
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let start = now();
        pairing.tick_at(1, start).await;

        port.state.lock().expect("lock").lanes = vec![lane(300, "casual", &[10])];
        pairing.tick_at(1, start + Duration::minutes(4)).await;
        port.state.lock().expect("lock").lanes =
            vec![lane(300, "casual", &[10]), lane(200, "casual", &[20])];
        pairing.tick_at(1, start + Duration::minutes(5)).await;

        assert!(
            port.state.lock().expect("lock").dms.is_empty(),
            "nach dem Lane-Wechsel läuft die Wartezeit neu"
        );
    }

    #[tokio::test]
    async fn erste_zusage_wartet_zweite_verschiebt() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());

        let first = pairing.handle_yes(10, 1, 100, 200, 20, now()).await;
        assert_eq!(first.content.as_deref(), Some(WAITING_REPLY));
        assert!(port.state.lock().expect("lock").moves.is_empty());

        let second = pairing.handle_yes(20, 1, 200, 100, 10, now()).await;
        assert_eq!(second.content.as_deref(), Some(MOVED_REPLY));
        let state = port.state.lock().expect("lock");
        assert_eq!(state.moves, vec![(20, 100)], "der Zweite zieht zum Ersten");
        assert!(state.accepts.is_empty(), "die Zusage wird verbraucht");
        assert_eq!(
            state.dms.len(),
            1,
            "der Erste erfährt, dass jemand rüberkommt"
        );
    }

    #[tokio::test]
    async fn zusage_verfaellt_und_wird_nicht_zum_move() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let start = now();

        pairing.handle_yes(10, 1, 100, 200, 20, start).await;
        let late = pairing
            .handle_yes(20, 1, 200, 100, 10, start + Duration::minutes(30))
            .await;

        assert_eq!(late.content.as_deref(), Some(WAITING_REPLY));
        assert!(port.state.lock().expect("lock").moves.is_empty());
    }

    #[tokio::test]
    async fn zusage_ohne_eigene_lane_wird_abgewiesen() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let reply = pairing.handle_yes(10, 1, 100, 200, 20, now()).await;
        assert_eq!(reply.content.as_deref(), Some(NOT_IN_LANE_REPLY));
    }

    #[tokio::test]
    async fn partner_weg_loest_keinen_move_aus() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        pairing.handle_yes(10, 1, 100, 200, 20, now()).await;
        port.state.lock().expect("lock").voice.remove(&10);

        let reply = pairing.handle_yes(20, 1, 200, 100, 10, now()).await;
        assert_eq!(reply.content.as_deref(), Some(GONE_REPLY));
        let state = port.state.lock().expect("lock");
        assert!(state.moves.is_empty());
        assert!(state.accepts.is_empty(), "die tote Zusage wird geräumt");
    }

    #[tokio::test]
    async fn fehlgeschlagener_move_meldet_sich_ehrlich() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            move_fails: true,
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        pairing.handle_yes(10, 1, 100, 200, 20, now()).await;
        let reply = pairing.handle_yes(20, 1, 200, 100, 10, now()).await;
        assert_eq!(reply.content.as_deref(), Some(MOVE_FAILED_REPLY));
    }

    #[tokio::test]
    async fn nie_mehr_fragen_wird_gespeichert() {
        let port = TestPort::new(TestState::default());
        let pairing = LanePairing::new(port.clone());
        let reply = pairing
            .handle_interaction(BridgeInteraction {
                custom_id: never_custom_id(),
                user_id: 10,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(NEVER_REPLY));
        assert_eq!(port.state.lock().expect("lock").never, vec![10]);
    }

    #[tokio::test]
    async fn cooldown_verhindert_die_frage() {
        let port = TestPort::new(TestState {
            lanes: vec![lane(100, "casual", &[10]), lane(200, "casual", &[20])],
            decision: AskDecision::UserCooldown,
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let start = now();
        pairing.tick_at(1, start).await;
        pairing.tick_at(1, start + Duration::minutes(5)).await;
        assert!(port.state.lock().expect("lock").dms.is_empty());
    }
}
