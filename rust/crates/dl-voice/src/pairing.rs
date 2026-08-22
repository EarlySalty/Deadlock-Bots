//! Lane-Pairing — zwei Leute, die je allein in ihrer eigenen Lane sitzen,
//! werden gefragt, ob sie zusammen zocken wollen.
//!
//! Ein Ja genügt: wer zusagt, wird in die andere Lane gezogen, so wie er auch
//! selbst hätte rüberklicken können. Die Gegenseite muss nichts bestätigen, sie
//! bekommt nur Bescheid. Sitzt der Zusagende mit anderen in einer Lane, zieht
//! seine Gruppe mit, denn gefragt wird der Owner, der auch sonst für die Lane
//! entscheidet. Ablehnen, Ignorieren oder die Lane verlassen endet folgenlos,
//! außer dass der Cooldown steht.
//!
//! Gefragt wird nur, wer seit [`MIN_ALONE`] allein in einer TempVoice-Lane
//! desselben Modus sitzt, nicht per Privacy abgemeldet ist, den Opt-out nicht
//! gesetzt hat und weder im eigenen noch im Paar-Cooldown steckt.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use dl_central_db::kv;

use crate::router::{MAX_LANE_MEMBERS, ROUTER_EMOJI_CASUAL};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};
use sqlx::PgPool;

pub const MAIN_GUILD_ID: u64 = 1289721245281292288;
/// Kategorie, in der alle Router-Lanes liegen (`TEMPVOICE_ONE_CATEGORY_ID`).
pub const LANE_CATEGORY_ID: u64 = 1289721245281292290;
/// So lange muss jemand allein sitzen, bevor der Bot ihn anspricht.
pub const MIN_ALONE: Duration = Duration::seconds(180);
/// Pro User: so lange keine neue Pairing-Frage.
pub const ASK_COOLDOWN: Duration = Duration::hours(2);
/// Pro Paar: so lange nicht erneut dieselben zwei zusammenbringen wollen.
pub const PAIR_COOLDOWN: Duration = Duration::hours(24);
pub const TICK_INTERVAL: StdDuration = StdDuration::from_secs(20);
/// Größte Gruppe, die noch einen Vorschlag bekommt (darüber ist die Lane voll
/// genug). Zusammen dürfen beide Seiten [`MAX_LANE_MEMBERS`] nicht sprengen.
pub const MAX_GROUP_SIZE: usize = 4;
/// Nach „Später vielleicht" darf der Bot so bald wieder fragen.
pub const LATER_RETRY: Duration = Duration::minutes(45);

pub const NEVER_ASK_NS: &str = "voice_pair_never";
pub const LAST_ASK_NS: &str = "voice_pair_last_ask";
pub const PAIR_ASKED_NS: &str = "voice_pair_last_pair";

pub const CUSTOM_ID_PREFIX: &str = "voice_pair:";
pub const YES_ACTION: &str = "yes";
pub const NO_ACTION: &str = "no";
pub const LATER_ACTION: &str = "later";
pub const NEVER_ACTION: &str = "never";

pub const ACCENT_GOLD: u64 = 0xC8A86B;
pub const COMPONENTS_V2_FLAG: u64 = 1 << 15;

pub const MOVED_REPLY: &str = "Ihr seid zusammen in einer Lane. Viel Spaß euch beiden.";
pub const TOO_FULL_REPLY: &str =
    "Inzwischen seid ihr zusammen zu viele für eine Lane. Beim nächsten Mal klappt es.";
pub const MOVE_PARTIAL_REPLY: &str =
    "Fast alle sind drüben, bei einem hat es nicht geklappt. Der kann einfach selbst rüberspringen.";
pub const MOVE_FAILED_REPLY: &str =
    "Das Verschieben hat gerade nicht geklappt. Spring einfach selbst rüber, die Lane steht.";
pub const GONE_REPLY: &str =
    "Knapp verpasst, die Lane ist schon leer. Ich sag dir Bescheid, sobald wieder jemand allein sitzt.";
pub const ALREADY_TOGETHER_REPLY: &str = "Ihr sitzt doch schon zusammen. Viel Spaß euch.";
pub const NOT_IN_LANE_REPLY: &str =
    "Du sitzt gerade nicht mehr in deiner Lane. Spring wieder rein, dann klappt es.";
pub const NO_REPLY: &str = "Alles gut, dann zockst du in Ruhe für dich. Viel Spaß.";
pub const LATER_REPLY: &str = "Passt, ich frag später nochmal.";
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
    /// Owner der Lane, falls die Engine ihn kennt.
    pub owner_id: Option<u64>,
}

/// Eine kleine Lane, die der Watcher schon eine Weile beobachtet. `speaker_id`
/// ist der Angesprochene: der Owner, sonst das erste Mitglied. Er entscheidet
/// für seine Lane, so wie er sie auch umbenennen oder jemanden kicken darf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneSeat {
    pub speaker_id: u64,
    pub channel_id: u64,
    pub mode: String,
    pub members: Vec<u64>,
    pub since: DateTime<Utc>,
}

impl LaneSeat {
    pub fn size(&self) -> usize {
        self.members.len().max(1)
    }
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

/// Beobachtungsstand einer Lane. Ändert sich die Besetzung, läuft die
/// Wartezeit neu: wer gerade erst Besuch bekommen hat, will nicht sofort
/// gefragt werden.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Watched {
    members: Vec<u64>,
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
    /// Sperren so weit kürzen, dass bald wieder gefragt werden darf.
    async fn soften_cooldown(&self, user_id: u64, other_user: u64) -> Result<(), String>;
    async fn member_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    /// Aktuelle Mitglieder eines Kanals (ohne Bots).
    async fn channel_members(&self, guild_id: u64, channel_id: u64) -> Vec<u64>;
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64)
        -> Result<(), String>;
    fn log_decision(&self, user_id: u64, decision: &'static str, reason: &'static str);
}

pub struct LanePairing {
    port: Arc<dyn PairingPort>,
    /// Lane-ID → seit wann diese Besetzung so sitzt.
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
            .claim_ask(first.speaker_id, second.speaker_id, now)
            .await
        {
            Err(error) => {
                tracing::warn!(%error, "Lane-Pairing: Eignungsprüfung fehlgeschlagen");
            }
            Ok(AskDecision::OptedOut) => {
                self.port
                    .log_decision(first.speaker_id, "verworfen", "privacy_opt_out");
            }
            Ok(AskDecision::NeverAsk) => {
                self.port
                    .log_decision(first.speaker_id, "verworfen", "nie_fragen");
            }
            Ok(AskDecision::UserCooldown) => {
                self.port
                    .log_decision(first.speaker_id, "verworfen", "user_cooldown");
            }
            Ok(AskDecision::PairCooldown) => {
                self.port
                    .log_decision(first.speaker_id, "verworfen", "paar_cooldown");
            }
            Ok(AskDecision::Unwanted) => {
                self.port
                    .log_decision(first.speaker_id, "verworfen", "paarung_abgelehnt");
            }
            Ok(AskDecision::Ask) => {
                self.ask(guild_id, &first, &second).await;
                self.ask(guild_id, &second, &first).await;
            }
        }
    }

    async fn ask(&self, guild_id: u64, seat: &LaneSeat, other: &LaneSeat) {
        let body = dm_body(guild_id, seat, other);
        match self.port.send_dm(seat.speaker_id, body).await {
            Ok(()) => self
                .port
                .log_decision(seat.speaker_id, "dm_zugestellt", "pairing_vorschlag"),
            Err(_) => {
                self.port
                    .log_decision(seat.speaker_id, "dm_fehlgeschlagen", "discord_fehler")
            }
        }
    }

    /// Beobachtungsstand je Lane fortschreiben und die Kandidaten liefern.
    async fn update_watched(&self, lanes: &[LaneView], now: DateTime<Utc>) -> Vec<LaneSeat> {
        let mut watched = self.watched.lock().await;
        let mut seats = Vec::new();
        let mut seen = Vec::new();
        for lane in lanes {
            if lane.members.is_empty() || lane.members.len() > MAX_GROUP_SIZE {
                continue;
            }
            seen.push(lane.channel_id);
            let mut members = lane.members.clone();
            members.sort_unstable();
            let since = match watched.get(&lane.channel_id) {
                Some(state) if state.members == members => state.since,
                _ => {
                    watched.insert(
                        lane.channel_id,
                        Watched {
                            members: members.clone(),
                            since: now,
                        },
                    );
                    now
                }
            };
            let speaker_id = lane
                .owner_id
                .filter(|owner| members.contains(owner))
                .or_else(|| members.first().copied());
            let Some(speaker_id) = speaker_id else {
                continue;
            };
            seats.push(LaneSeat {
                speaker_id,
                channel_id: lane.channel_id,
                mode: lane.mode.clone(),
                members,
                since,
            });
        }
        watched.retain(|channel_id, _| seen.contains(channel_id));
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
                    // Karte ersetzen statt antworten: sonst bleiben die Buttons
                    // stehen und jeder weitere Klick bringt eine neue Nachricht.
                    pair_abschluss_reply(NEVER_REPLY)
                }
                Err(_) => BridgeReply::ephemeral_text(NEVER_FAILED_REPLY),
            },
            PairAction::No { .. } => {
                self.port
                    .log_decision(interaction.user_id, "abgelehnt", "nein_gewaehlt");
                pair_abschluss_reply(NO_REPLY)
            }
            PairAction::Later { other_user } => {
                // Kein Nein: die Sperren werden verkürzt, damit der Bot es
                // bald wieder versuchen darf.
                if let Err(error) = self
                    .port
                    .soften_cooldown(interaction.user_id, other_user)
                    .await
                {
                    tracing::warn!(%error, user_id = interaction.user_id, "Lane-Pairing: Cooldown nicht verkürzbar");
                }
                self.port
                    .log_decision(interaction.user_id, "vertagt", "spaeter_gewaehlt");
                BridgeReply::ephemeral_text(LATER_REPLY)
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
    ) -> BridgeReply {
        match self.port.member_voice_channel(guild_id, user_id).await {
            Some(current) if current == other_lane => {
                return BridgeReply::ephemeral_text(ALREADY_TOGETHER_REPLY);
            }
            Some(current) if current == own_lane => {}
            _ => {
                self.port
                    .log_decision(user_id, "verworfen", "nicht_mehr_in_lane");
                return BridgeReply::ephemeral_text(NOT_IN_LANE_REPLY);
            }
        }

        let other_group = self.port.channel_members(guild_id, other_lane).await;
        if other_group.is_empty() {
            self.port.log_decision(user_id, "verworfen", "andere_leer");
            return BridgeReply::ephemeral_text(GONE_REPLY);
        }
        let own_group = self.port.channel_members(guild_id, own_lane).await;
        if own_group.len() + other_group.len() > MAX_LANE_MEMBERS {
            self.port.log_decision(user_id, "verworfen", "zu_voll");
            return BridgeReply::ephemeral_text(TOO_FULL_REPLY);
        }

        // Wer Ja sagt, geht rüber. Seine Lane zieht mit, denn gefragt wurde ihr
        // Owner. Die andere Seite bekommt nur Besuch und bestätigt nichts.
        let mut moved = 0usize;
        let mut failed = 0usize;
        for member in &own_group {
            match self.port.move_member(guild_id, *member, other_lane).await {
                Ok(()) => moved += 1,
                Err(error) => {
                    failed += 1;
                    tracing::warn!(%error, user_id = member, other_lane, "Lane-Pairing: Move fehlgeschlagen");
                }
            }
        }
        if moved == 0 {
            self.port
                .log_decision(user_id, "move_fehlgeschlagen", "discord_fehler");
            return BridgeReply::ephemeral_text(MOVE_FAILED_REPLY);
        }
        self.port.log_decision(user_id, "zusammengelegt", "zusage");
        let _ = self
            .port
            .send_dm(other_user, merged_dm_body(moved, other_lane))
            .await;
        if failed > 0 {
            return BridgeReply::ephemeral_text(MOVE_PARTIAL_REPLY);
        }
        BridgeReply::ephemeral_text(MOVED_REPLY)
    }
}

/// Höchstens ein Paar pro Durchlauf: die zwei am längsten wartenden Lanes im
/// selben Modus, die zusammen in eine Lane passen. Zwei Einzelne sind der
/// häufigste Fall, aber zwei plus einer zählt genauso.
pub fn pick_pair(seats: &[LaneSeat], now: DateTime<Utc>) -> Option<(LaneSeat, LaneSeat)> {
    let mut ripe: Vec<&LaneSeat> = seats
        .iter()
        .filter(|seat| now - seat.since >= MIN_ALONE)
        .filter(|seat| seat.size() <= MAX_GROUP_SIZE)
        .collect();
    ripe.sort_by_key(|seat| (seat.since, seat.channel_id));
    for (index, first) in ripe.iter().enumerate() {
        for second in ripe.iter().skip(index + 1) {
            if first.mode == second.mode
                && first.channel_id != second.channel_id
                && first.size() + second.size() <= MAX_LANE_MEMBERS
            {
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
    /// Nicht jetzt, aber ruhig bald wieder fragen.
    Later {
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
        LATER_ACTION => Some(PairAction::Later {
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

pub fn later_custom_id(other_user: u64) -> String {
    format!("{CUSTOM_ID_PREFIX}{LATER_ACTION}:{other_user}")
}

/// Ersetzt die Frage-Karte durch das Ergebnis, ohne Buttons. Danach ist ein
/// zweiter Klick auf dieselbe Karte gar nicht mehr möglich.
fn pair_abschluss_reply(text: &str) -> BridgeReply {
    BridgeReply {
        components: Some(json!([{
            "type": 17,
            "accent_color": ACCENT_GOLD,
            // Der Breiten-Streifen bleibt drin: die Ursprungs-DM haengt ihn als
            // Anhang an, ein Update ohne `attachments`-Feld laesst ihn stehen,
            // und Discord weist eine Components-V2-Nachricht mit einem nicht
            // referenzierten Anhang mit 400 zurueck.
            "components": [
                crate::router::dm_width_divider(),
                { "type": 10, "content": text }
            ],
        }])),
        update_message: true,
        message_flags: Some(COMPONENTS_V2_FLAG),
        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
        ..BridgeReply::default()
    }
}

pub fn never_custom_id() -> String {
    format!("{CUSTOM_ID_PREFIX}{NEVER_ACTION}")
}

/// Wie der Bot die andere Seite benennt: eine Person mit Namen, mehrere als
/// Gruppe. Immer grammatikalisch passend zur Anzahl.
fn other_side_label(other: &LaneSeat) -> String {
    match other.size() {
        1 => format!("sitzt <@{}> auch allein", other.speaker_id),
        2 => format!("sitzen <@{}> und noch jemand", other.speaker_id),
        size => format!("sitzen <@{}> und {} weitere", other.speaker_id, size - 1),
    }
}

/// Vorschlags-DM. Erwähnungen bleiben stumm (`allowed_mentions.parse = []`),
/// der Name wird trotzdem aufgelöst.
pub fn dm_body(guild_id: u64, seat: &LaneSeat, other: &LaneSeat) -> Value {
    let headline = if seat.size() == 1 {
        "Du sitzt gerade allein"
    } else {
        "Ihr könntet mehr sein"
    };
    let text = if seat.size() == 1 {
        format!(
            "Hey, ein paar Kanäle weiter {}. Möchtet ihr zusammen spielen?",
            other_side_label(other)
        )
    } else {
        format!(
            "Hey, ihr seid zu {} in eurer Lane, und ein paar Kanäle weiter {}. Zusammen wärt ihr zu {}. Wollt ihr euch zusammentun?",
            count_word(seat.size()),
            other_side_label(other),
            count_word(seat.size() + other.size())
        )
    };
    json!({
        "flags": COMPONENTS_V2_FLAG,
        "allowed_mentions": { "parse": [] },
        "attachments": crate::router::dm_width_attachments(),
        "components": [{
            "type": 17,
            "accent_color": ACCENT_GOLD,
            "components": [
                crate::router::dm_width_divider(),
                { "type": 10, "content": crate::router::dm_headline(ROUTER_EMOJI_CASUAL, headline) },
                { "type": 14, "divider": true, "spacing": 2 },
                { "type": 10, "content": text },
                { "type": 1, "components": [
                    { "type": 2, "style": 3, "label": "Ja", "custom_id": yes_custom_id(guild_id, seat.channel_id, other.channel_id, other.speaker_id) },
                    { "type": 2, "style": 2, "label": "Nein", "custom_id": no_custom_id(other.speaker_id) },
                    { "type": 2, "style": 2, "label": "Später vielleicht", "custom_id": later_custom_id(other.speaker_id) },
                    { "type": 2, "style": 2, "label": "Nicht mehr fragen", "custom_id": never_custom_id() }
                ]}
            ]
        }]
    })
}

/// Kleine Zahlwörter, damit die DM nicht wie eine Tabelle klingt.
fn count_word(count: usize) -> &'static str {
    match count {
        2 => "zweit",
        3 => "dritt",
        4 => "viert",
        5 => "fünft",
        6 => "sechst",
        _ => "mehreren",
    }
}

/// Ankündigung an die Lane, die Besuch bekommt.
pub fn merged_dm_body(joining: usize, lane_id: u64) -> Value {
    let who = if joining == 1 {
        "Da kommt jemand zu euch".to_string()
    } else {
        format!("Da kommen {joining} Leute zu euch")
    };
    json!({
        "allowed_mentions": { "parse": [] },
        "content": format!("{who} in <#{lane_id}>. Viel Spaß zusammen."),
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

/// „Später vielleicht": beide Sperren so umschreiben, dass sie in
/// [`LATER_RETRY`] ablaufen statt erst in Stunden.
pub(crate) async fn soften_cooldown_db(
    pool: &PgPool,
    user_id: u64,
    other_user: u64,
    now: DateTime<Utc>,
) -> Result<(), String> {
    let pair = pair_key(user_id, other_user);
    for (ns, key, window) in [
        (LAST_ASK_NS, user_id.to_string(), ASK_COOLDOWN),
        (LAST_ASK_NS, other_user.to_string(), ASK_COOLDOWN),
        (PAIR_ASKED_NS, pair, PAIR_COOLDOWN),
    ] {
        let expires_in = (now - window + LATER_RETRY).timestamp();
        sqlx::query(
            "INSERT INTO bot.kv_store (ns, k, v)
             VALUES ($1, $2, $3)
             ON CONFLICT (ns, k) DO UPDATE SET v = EXCLUDED.v",
        )
        .bind(ns)
        .bind(&key)
        .bind(expires_in.to_string())
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) async fn set_never_ask_db(pool: &PgPool, user_id: u64) -> Result<(), String> {
    kv::set(pool, NEVER_ASK_NS, &user_id.to_string(), "true")
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

    fn seat(user_id: u64, channel_id: u64, mode: &str, since: DateTime<Utc>) -> LaneSeat {
        group(&[user_id], channel_id, mode, since)
    }

    fn group(members: &[u64], channel_id: u64, mode: &str, since: DateTime<Utc>) -> LaneSeat {
        LaneSeat {
            speaker_id: members[0],
            channel_id,
            mode: mode.to_string(),
            members: members.to_vec(),
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
        assert_eq!((first.speaker_id, second.speaker_id), (1, 3));
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
        // Solo trifft Solo
        println!(
            "{}",
            serde_json::to_string(&dm_body(
                MAIN_GUILD_ID,
                &seat(1, 1523272810825252944, "casual", now),
                &seat(2, 1522769149208821881, "casual", now),
            ))
            .expect("json")
        );
        // Zweiergruppe trifft Einzelnen
        println!(
            "{}",
            serde_json::to_string(&dm_body(
                MAIN_GUILD_ID,
                &group(&[1, 3], 1523272810825252944, "ranked", now),
                &seat(2, 1522769149208821881, "ranked", now),
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
        assert_eq!(
            parse_custom_id(&later_custom_id(7)),
            Some(PairAction::Later { other_user: 7 })
        );
        assert_eq!(parse_custom_id(&never_custom_id()), Some(PairAction::Never));
        assert_eq!(parse_custom_id("voice_pair:quatsch"), None);
        assert_eq!(parse_custom_id("anderes:yes:1:2:3:4"), None);
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
        assert!(
            !text.contains("<#100>"),
            "die eigene Lane muss nicht erwähnt werden"
        );
        assert!(text.contains("<@20>"));
        assert!(
            text.contains("Möchtet ihr zusammen spielen?"),
            "der Vorschlag ist eine Frage, kein Statusbericht"
        );
        assert!(text.contains(&later_custom_id(20)));
        assert!(text.contains(&yes_custom_id(1, 100, 200, 20)));
        assert!(text.contains(&no_custom_id(20)));
        assert!(text.contains(&never_custom_id()));
        // Stumme Erwähnung: der Vorschlag darf niemanden anpingen.
        assert!(text.contains("\"parse\":[]"));
    }

    struct TestState {
        lanes: Vec<LaneView>,
        dms: Vec<(u64, Value)>,
        moves: Vec<(u64, u64)>,
        voice: HashMap<u64, u64>,
        never: Vec<u64>,
        softened: Vec<(u64, u64)>,
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
                moves: Vec::new(),
                voice: HashMap::new(),
                never: Vec::new(),
                softened: Vec::new(),
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

        async fn soften_cooldown(&self, user_id: u64, other_user: u64) -> Result<(), String> {
            self.state
                .lock()
                .expect("lock")
                .softened
                .push((user_id, other_user));
            Ok(())
        }

        async fn channel_members(&self, _guild_id: u64, channel_id: u64) -> Vec<u64> {
            let state = self.state.lock().expect("lock");
            let mut members: Vec<u64> = state
                .voice
                .iter()
                .filter(|(_, seat)| **seat == channel_id)
                .map(|(user_id, _)| *user_id)
                .collect();
            members.sort_unstable();
            members
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
            owner_id: members.first().copied(),
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
    async fn ein_ja_genuegt_und_verschiebt_sofort() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());

        let reply = pairing.handle_yes(10, 1, 100, 200, 20).await;

        assert_eq!(reply.content.as_deref(), Some(MOVED_REPLY));
        let state = port.state.lock().expect("lock");
        assert_eq!(
            state.moves,
            vec![(10, 200)],
            "der Zusagende geht rüber, ohne dass der andere bestätigt"
        );
        assert_eq!(state.dms.len(), 1, "die andere Lane erfährt vom Besuch");
    }

    #[tokio::test]
    async fn zweite_zusage_meldet_dass_man_schon_zusammen_sitzt() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        pairing.handle_yes(10, 1, 100, 200, 20).await;

        // Der andere klickt danach auch noch Ja: sie sitzen längst zusammen.
        let reply = pairing.handle_yes(20, 1, 200, 100, 10).await;
        assert_eq!(reply.content.as_deref(), Some(GONE_REPLY));
    }

    #[tokio::test]
    async fn zusage_ohne_eigene_lane_wird_abgewiesen() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(20, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let reply = pairing.handle_yes(10, 1, 100, 200, 20).await;
        assert_eq!(reply.content.as_deref(), Some(NOT_IN_LANE_REPLY));
    }

    #[tokio::test]
    async fn leere_gegenseite_loest_keinen_move_aus() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());

        let reply = pairing.handle_yes(10, 1, 100, 200, 20).await;
        assert_eq!(reply.content.as_deref(), Some(GONE_REPLY));
        assert!(port.state.lock().expect("lock").moves.is_empty());
    }

    #[tokio::test]
    async fn fehlgeschlagener_move_meldet_sich_ehrlich() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200)]),
            move_fails: true,
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let reply = pairing.handle_yes(10, 1, 100, 200, 20).await;
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
        // Der Text steckt jetzt in der ersetzten Karte, die Buttons sind weg.
        let karte = serde_json::to_string(&reply.components).expect("components");
        assert!(karte.contains(NEVER_REPLY));
        assert!(reply.update_message);
        assert!(!karte.contains(&never_custom_id()));
        // Die Frage-DM haengt den Breiten-Streifen als Anhang an. Ein Update
        // ohne `attachments`-Feld laesst ihn stehen, und eine
        // Components-V2-Nachricht mit unreferenziertem Anhang weist Discord mit
        // 400 zurueck: dann bliebe die alte Karte samt Buttons stehen.
        assert!(
            karte.contains(&format!("attachment://{}", crate::router::DM_WIDTH_IMAGE)),
            "Ersatzkarte referenziert den Anhang nicht: {karte}"
        );
        assert_eq!(port.state.lock().expect("lock").never, vec![10]);
    }

    #[test]
    fn pick_pair_legt_auch_zweier_und_einzelgruppen_zusammen() {
        let now = now();
        let seats = vec![
            group(&[1, 2], 10, "casual", now - Duration::minutes(9)),
            seat(3, 11, "casual", now - Duration::minutes(8)),
        ];
        let (first, second) = pick_pair(&seats, now).expect("Paar");
        assert_eq!((first.channel_id, second.channel_id), (10, 11));
    }

    #[test]
    fn pick_pair_sprengt_die_lane_nicht() {
        let now = now();
        let voll = vec![
            group(&[1, 2, 3, 4], 10, "casual", now - Duration::minutes(9)),
            group(&[5, 6, 7], 11, "casual", now - Duration::minutes(8)),
        ];
        assert!(
            pick_pair(&voll, now).is_none(),
            "sieben passen nicht in eine Lane"
        );

        let passt = vec![
            group(&[1, 2, 3, 4], 10, "casual", now - Duration::minutes(9)),
            group(&[5, 6], 11, "casual", now - Duration::minutes(8)),
        ];
        assert!(pick_pair(&passt, now).is_some());
    }

    #[test]
    fn grosse_gruppen_werden_nicht_mehr_gefragt() {
        let now = now();
        let seats = vec![
            group(&[1, 2, 3, 4, 5], 10, "casual", now - Duration::minutes(9)),
            seat(6, 11, "casual", now - Duration::minutes(8)),
        ];
        assert!(pick_pair(&seats, now).is_none());
    }

    #[tokio::test]
    async fn ganze_gruppe_zieht_mit_um() {
        let port = TestPort::new(TestState {
            voice: HashMap::from([(10, 100), (20, 200), (21, 200)]),
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        // Der Sprecher der Zweiergruppe sagt Ja: seine Lane zieht komplett um.
        let reply = pairing.handle_yes(20, 1, 200, 100, 10).await;

        assert_eq!(reply.content.as_deref(), Some(MOVED_REPLY));
        let state = port.state.lock().expect("lock");
        assert_eq!(
            state.moves,
            vec![(20, 100), (21, 100)],
            "die ganze Lane des Zusagenden zieht mit"
        );
    }

    #[tokio::test]
    async fn zu_volle_lanes_werden_nicht_zusammengelegt() {
        let voice = HashMap::from([
            (1, 100),
            (2, 100),
            (3, 100),
            (4, 200),
            (5, 200),
            (6, 200),
            (7, 200),
        ]);
        let port = TestPort::new(TestState {
            voice,
            ..TestState::default()
        });
        let pairing = LanePairing::new(port.clone());
        let reply = pairing.handle_yes(1, 1, 100, 200, 4).await;

        assert_eq!(reply.content.as_deref(), Some(TOO_FULL_REPLY));
        assert!(port.state.lock().expect("lock").moves.is_empty());
    }

    #[tokio::test]
    async fn spaeter_vielleicht_verkuerzt_die_sperre_statt_abzulehnen() {
        let port = TestPort::new(TestState::default());
        let pairing = LanePairing::new(port.clone());
        let reply = pairing
            .handle_interaction(BridgeInteraction {
                custom_id: later_custom_id(20),
                user_id: 10,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(LATER_REPLY));
        assert_eq!(port.state.lock().expect("lock").softened, vec![(10, 20)]);
        assert!(
            port.state.lock().expect("lock").never.is_empty(),
            "später ist kein Opt-out"
        );
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
