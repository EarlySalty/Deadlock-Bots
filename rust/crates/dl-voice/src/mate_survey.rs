//! Mitspieler-Umfrage — nach einer gemeinsamen Session fragt der Bot per DM:
//! „Würdest du wieder mit XY spielen?"
//!
//! Zweck ist das Matching. `activity.user_co_players` weiß nur, wer wie lange
//! mit wem im Voice saß; ob es Spaß gemacht hat, weiß nur der Mensch. Die
//! Antworten landen in `activity.voice_mate_ratings` und sind die Grundlage
//! dafür, wen der Bot künftig zusammenbringt und wen bewusst nicht.
//!
//! Zurückhaltung ist eingebaut: gefragt wird erst ab [`MIN_SECONDS`] gemeinsamer
//! Zeit, höchstens einmal pro [`ASK_COOLDOWN`] je Person, pro Paarung nur alle
//! [`PAIR_COOLDOWN`], nie bei Privacy-Opt-out und nie wieder nach dem
//! Nicht-mehr-fragen-Knopf. Die Antwort sieht nur das Team, nie der Bewertete.

use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use dl_central_db::kv;
use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};
use sqlx::PgPool;

/// So lange muss die gemeinsame Session gedauert haben.
pub const MIN_SECONDS: i64 = 20 * 60;
/// Pro Person höchstens eine Umfrage in diesem Fenster.
pub const ASK_COOLDOWN: Duration = Duration::hours(48);
/// Dieselbe Paarung nicht öfter als einmal im Monat.
pub const PAIR_COOLDOWN: Duration = Duration::days(30);

pub const NEVER_ASK_NS: &str = "voice_mate_survey_never";
pub const LAST_ASK_NS: &str = "voice_mate_survey_last_ask";
pub const PAIR_ASKED_NS: &str = "voice_mate_survey_last_pair";

pub const CUSTOM_ID_PREFIX: &str = "voice_mate:";
pub const COMMENT_MODAL_ID: &str = "voice_mate_comment";
pub const COMMENT_FIELD: &str = "comment";

pub const ACCENT_GOLD: u64 = 0xC8A86B;
/// Server-Emojis im Brand-Look, dieselben wie im Router-Panel.
pub const EMOJI_RANKED: (&str, &str) = crate::router::ROUTER_EMOJI_RANKED;
pub const EMOJI_CASUAL: (&str, &str) = crate::router::ROUTER_EMOJI_CASUAL;
pub const COMPONENTS_V2_FLAG: u64 = 1 << 15;

pub const RATING_AGAIN: &str = "again";
pub const RATING_OK: &str = "ok";
pub const RATING_RATHER_NOT: &str = "rather_not";

pub const THANKS_AGAIN: &str =
    "Danke dir. Ich merk mir das und bring euch künftig lieber wieder zusammen.";
pub const THANKS_OK: &str = "Danke dir, ist notiert.";
pub const THANKS_RATHER_NOT: &str =
    "Danke für die Ehrlichkeit. Ich schlag euch nicht mehr als Paarung vor.";
pub const THANKS_COMMENT: &str =
    "Angekommen, danke dir. Das liest nur das Team, dein Mitspieler sieht davon nichts.";
pub const NEVER_REPLY: &str = "Erledigt, ich frag dich nicht mehr nach deinen Mitspielern.";
pub const SAVE_FAILED_REPLY: &str =
    "Das konnte ich gerade nicht speichern. Probier es gleich nochmal.";
pub const INVALID_REPLY: &str = "Diese Umfrage ist abgelaufen.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskDecision {
    Ask,
    TooShort,
    NoMate,
    OptedOut,
    NeverAsk,
    UserCooldown,
    PairCooldown,
}

/// Eine gespeicherte Bewertung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MateRating {
    pub rater_id: u64,
    pub mate_id: u64,
    pub rating: String,
    pub comment: Option<String>,
    pub channel_id: u64,
    pub session_seconds: i64,
}

#[async_trait::async_trait]
pub trait MateSurveyPort: Send + Sync {
    /// Privacy, Opt-out und beide Cooldowns prüfen und die Frage buchen.
    async fn claim_ask(
        &self,
        rater_id: u64,
        mate_id: u64,
        now: DateTime<Utc>,
    ) -> Result<AskDecision, String>;
    async fn send_dm(&self, user_id: u64, body: Value) -> Result<(), String>;
    async fn set_never_ask(&self, user_id: u64) -> Result<(), String>;
    async fn save_rating(&self, rating: &MateRating) -> Result<(), String>;
    async fn attach_comment(
        &self,
        rater_id: u64,
        mate_id: u64,
        comment: &str,
    ) -> Result<(), String>;
    fn log_decision(&self, user_id: u64, decision: &'static str, reason: &'static str);
}

pub struct MateSurvey {
    port: Arc<dyn MateSurveyPort>,
}

impl MateSurvey {
    pub fn new(port: Arc<dyn MateSurveyPort>) -> Arc<Self> {
        Arc::new(Self { port })
    }

    /// Hook aus dem Session-Tracker. Fragt höchstens nach einem Mitspieler.
    pub async fn on_session_end(
        self: &Arc<Self>,
        user_id: u64,
        channel_id: u64,
        co_player_ids: Vec<u64>,
        seconds: i64,
        now: DateTime<Utc>,
    ) {
        if seconds < MIN_SECONDS {
            return;
        }
        let Some(mate_id) = pick_mate(user_id, &co_player_ids) else {
            return;
        };
        match self.port.claim_ask(user_id, mate_id, now).await {
            Err(error) => {
                tracing::warn!(%error, user_id, "Mitspieler-Umfrage: Prüfung fehlgeschlagen")
            }
            Ok(AskDecision::Ask) => {
                let body = dm_body(mate_id, channel_id, seconds);
                match self.port.send_dm(user_id, body).await {
                    Ok(()) => {
                        self.port
                            .log_decision(user_id, "dm_zugestellt", "umfrage_nach_session")
                    }
                    Err(_) => {
                        self.port
                            .log_decision(user_id, "dm_fehlgeschlagen", "discord_fehler")
                    }
                }
            }
            Ok(AskDecision::OptedOut) => {
                self.port
                    .log_decision(user_id, "verworfen", "privacy_opt_out")
            }
            Ok(AskDecision::NeverAsk) => self.port.log_decision(user_id, "verworfen", "nie_fragen"),
            Ok(AskDecision::UserCooldown) => {
                self.port
                    .log_decision(user_id, "verworfen", "user_cooldown")
            }
            Ok(AskDecision::PairCooldown) => {
                self.port
                    .log_decision(user_id, "verworfen", "paar_cooldown")
            }
            Ok(AskDecision::TooShort) | Ok(AskDecision::NoMate) => {}
        }
    }

    pub async fn handle_interaction(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.custom_id == COMMENT_MODAL_ID {
            return self.handle_comment(&interaction).await;
        }
        let Some(action) = parse_custom_id(&interaction.custom_id) else {
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        };
        match action {
            SurveyAction::Never => match self.port.set_never_ask(interaction.user_id).await {
                Ok(()) => {
                    self.port
                        .log_decision(interaction.user_id, "abgelehnt", "nie_fragen_gewaehlt");
                    BridgeReply::ephemeral_text(NEVER_REPLY)
                }
                Err(_) => BridgeReply::ephemeral_text(SAVE_FAILED_REPLY),
            },
            SurveyAction::Comment { mate_id } => BridgeReply {
                modal: Some(comment_modal(mate_id)),
                ..BridgeReply::default()
            },
            SurveyAction::Rate {
                mate_id,
                rating,
                channel_id,
                seconds,
            } => {
                let record = MateRating {
                    rater_id: interaction.user_id,
                    mate_id,
                    rating: rating.to_string(),
                    comment: None,
                    channel_id,
                    session_seconds: seconds,
                };
                match self.port.save_rating(&record).await {
                    Ok(()) => {
                        self.port.log_decision(
                            interaction.user_id,
                            "bewertet",
                            rating_reason(rating),
                        );
                        BridgeReply::ephemeral_text(rating_thanks(rating))
                    }
                    Err(error) => {
                        tracing::warn!(%error, user_id = interaction.user_id, "Mitspieler-Umfrage: Bewertung nicht speicherbar");
                        BridgeReply::ephemeral_text(SAVE_FAILED_REPLY)
                    }
                }
            }
        }
    }

    async fn handle_comment(&self, interaction: &BridgeInteraction) -> BridgeReply {
        let mate_id = interaction
            .options
            .get("mate_id")
            .and_then(Value::as_str)
            .and_then(|value| value.parse::<u64>().ok());
        let comment = interaction
            .options
            .get(COMMENT_FIELD)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        if comment.is_empty() {
            return BridgeReply::ephemeral_text(THANKS_COMMENT);
        }
        let Some(mate_id) = mate_id else {
            return BridgeReply::ephemeral_text(INVALID_REPLY);
        };
        match self
            .port
            .attach_comment(interaction.user_id, mate_id, comment)
            .await
        {
            Ok(()) => {
                self.port
                    .log_decision(interaction.user_id, "kommentiert", "freitext");
                BridgeReply::ephemeral_text(THANKS_COMMENT)
            }
            Err(error) => {
                tracing::warn!(%error, user_id = interaction.user_id, "Mitspieler-Umfrage: Kommentar nicht speicherbar");
                BridgeReply::ephemeral_text(SAVE_FAILED_REPLY)
            }
        }
    }
}

/// Der Mitspieler, nach dem gefragt wird: deterministisch der erste, damit ein
/// wiederholter Session-Abschluss nicht plötzlich jemand anderen erwischt.
pub fn pick_mate(user_id: u64, co_player_ids: &[u64]) -> Option<u64> {
    let mut candidates: Vec<u64> = co_player_ids
        .iter()
        .copied()
        .filter(|candidate| *candidate != user_id)
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    candidates.first().copied()
}

fn rating_thanks(rating: &str) -> &'static str {
    match rating {
        RATING_AGAIN => THANKS_AGAIN,
        RATING_RATHER_NOT => THANKS_RATHER_NOT,
        _ => THANKS_OK,
    }
}

fn rating_reason(rating: &str) -> &'static str {
    match rating {
        RATING_AGAIN => "gerne_wieder",
        RATING_RATHER_NOT => "lieber_nicht",
        _ => "war_okay",
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurveyAction {
    Rate {
        mate_id: u64,
        rating: &'static str,
        channel_id: u64,
        seconds: i64,
    },
    Comment {
        mate_id: u64,
    },
    Never,
}

/// `voice_mate:rate:<rating>:<mate>:<channel>:<seconds>`,
/// `voice_mate:comment:<mate>`, `voice_mate:never`.
pub fn parse_custom_id(custom_id: &str) -> Option<SurveyAction> {
    let rest = custom_id.strip_prefix(CUSTOM_ID_PREFIX)?;
    let mut parts = rest.split(':');
    match parts.next()? {
        "never" => Some(SurveyAction::Never),
        "comment" => Some(SurveyAction::Comment {
            mate_id: parts.next()?.parse().ok()?,
        }),
        "rate" => {
            let rating = match parts.next()? {
                RATING_AGAIN => RATING_AGAIN,
                RATING_OK => RATING_OK,
                RATING_RATHER_NOT => RATING_RATHER_NOT,
                _ => return None,
            };
            Some(SurveyAction::Rate {
                rating,
                mate_id: parts.next()?.parse().ok()?,
                channel_id: parts.next()?.parse().ok()?,
                seconds: parts.next()?.parse().ok()?,
            })
        }
        _ => None,
    }
}

pub fn rate_custom_id(rating: &str, mate_id: u64, channel_id: u64, seconds: i64) -> String {
    format!("{CUSTOM_ID_PREFIX}rate:{rating}:{mate_id}:{channel_id}:{seconds}")
}

pub fn comment_custom_id(mate_id: u64) -> String {
    format!("{CUSTOM_ID_PREFIX}comment:{mate_id}")
}

pub fn never_custom_id() -> String {
    format!("{CUSTOM_ID_PREFIX}never")
}

pub fn comment_modal(mate_id: u64) -> ModalSpec {
    ModalSpec {
        custom_id: COMMENT_MODAL_ID.to_string(),
        title: "Kommentar zur Runde".to_string(),
        fields: vec![
            ModalField {
                custom_id: COMMENT_FIELD.to_string(),
                label: "Was möchtest du uns mitgeben?".to_string(),
                placeholder: "Alles, was für dich zählt. Liest nur das Team.".to_string(),
                value: None,
                required: false,
                min_length: 0,
                max_length: 1000,
                paragraph: true,
            },
            // Discord gibt im Modal-Submit keine Button-custom_id zurück, also
            // reist der Bezug als vorbelegtes Feld mit.
            ModalField {
                custom_id: "mate_id".to_string(),
                label: "Bezug (bitte stehen lassen)".to_string(),
                placeholder: String::new(),
                value: Some(mate_id.to_string()),
                required: true,
                min_length: 1,
                max_length: 32,
                paragraph: false,
            },
        ],
    }
}

/// Server-Emoji als Discord-Tag (`<:name:id>`).
fn emoji_tag((name, id): (&str, &str)) -> String {
    format!("<:{name}:{id}>")
}

fn minutes_label(seconds: i64) -> String {
    let minutes = (seconds / 60).max(1);
    if minutes >= 90 {
        let hours = minutes / 60;
        let rest = minutes % 60;
        if rest == 0 {
            return format!("{hours} Stunden");
        }
        return format!("{hours} Stunden und {rest} Minuten");
    }
    format!("{minutes} Minuten")
}

/// Die Umfrage-DM. Erwähnungen bleiben stumm, gepingt wird niemand.
pub fn dm_body(mate_id: u64, channel_id: u64, seconds: i64) -> Value {
    json!({
        "flags": COMPONENTS_V2_FLAG,
        "allowed_mentions": { "parse": [] },
        "components": [{
            "type": 17,
            "accent_color": ACCENT_GOLD,
            "components": [
                { "type": 10, "content": format!("## {} Kurz gefragt", emoji_tag(EMOJI_RANKED)) },
                { "type": 14, "divider": true, "spacing": 2 },
                { "type": 10, "content": format!(
                    "Ihr wart gerade {} zusammen in <#{channel_id}> unterwegs.\n\n**Würdest du wieder mit <@{mate_id}> spielen?**\nSagst du Ja, setze ich euch öfter zusammen in eine Lane.\n\n🔒 **Deine Antwort bleibt privat.** <@{mate_id}> erfährt nie, was du hier klickst.",
                    minutes_label(seconds)
                ) },
                { "type": 1, "components": [
                    { "type": 2, "style": 3, "label": "Gerne wieder", "custom_id": rate_custom_id(RATING_AGAIN, mate_id, channel_id, seconds), "emoji": { "name": EMOJI_CASUAL.0, "id": EMOJI_CASUAL.1 } },
                    { "type": 2, "style": 2, "label": "War okay", "custom_id": rate_custom_id(RATING_OK, mate_id, channel_id, seconds) },
                    { "type": 2, "style": 4, "label": "Lieber nicht", "custom_id": rate_custom_id(RATING_RATHER_NOT, mate_id, channel_id, seconds) }
                ]},
                { "type": 14, "divider": false, "spacing": 1 },
                { "type": 1, "components": [
                    { "type": 2, "style": 2, "label": "Kommentar dazu", "custom_id": comment_custom_id(mate_id) },
                    { "type": 2, "style": 2, "label": "Nicht mehr fragen", "custom_id": never_custom_id() }
                ]}
            ]
        }]
    })
}

/// Privacy, Opt-out und beide Cooldowns in einer Transaktion; bei `Ask` sind
/// die Marker gesetzt.
pub(crate) async fn claim_ask_db(
    pool: &PgPool,
    rater_id: u64,
    mate_id: u64,
    now: DateTime<Utc>,
) -> Result<AskDecision, String> {
    let rater_i64 = i64::try_from(rater_id).map_err(|error| error.to_string())?;
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    if dl_central_db::lock_user_privacy_and_is_opted_out(&mut tx, rater_i64)
        .await
        .map_err(|error| error.to_string())?
    {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(AskDecision::OptedOut);
    }
    let key = rater_id.to_string();
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
    let pair = pair_key(rater_id, mate_id);
    if let Some(last) = read_timestamp(&mut tx, PAIR_ASKED_NS, &pair).await? {
        if now - last < PAIR_COOLDOWN {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(AskDecision::PairCooldown);
        }
    }
    for (ns, key) in [(LAST_ASK_NS, key.clone()), (PAIR_ASKED_NS, pair)] {
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

/// Richtungsabhängig: „A hat über B geurteilt" ist eine andere Aussage als
/// umgekehrt, aber gefragt wird pro Paarung nur einmal im Fenster.
pub fn pair_key(one: u64, other: u64) -> String {
    let (low, high) = if one <= other {
        (one, other)
    } else {
        (other, one)
    };
    format!("{low}:{high}")
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
        .map_err(|error| format!("ungültiger Umfrage-Zeitstempel: {error}"))?;
    Ok(DateTime::from_timestamp(timestamp, 0))
}

pub(crate) async fn set_never_ask_db(pool: &PgPool, user_id: u64) -> Result<(), String> {
    kv::set(pool, NEVER_ASK_NS, &user_id.to_string(), "true")
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn save_rating_db(pool: &PgPool, rating: &MateRating) -> Result<(), String> {
    let rater_id = i64::try_from(rating.rater_id).map_err(|error| error.to_string())?;
    let mate_id = i64::try_from(rating.mate_id).map_err(|error| error.to_string())?;
    let channel_id = i64::try_from(rating.channel_id).map_err(|error| error.to_string())?;
    sqlx::query(
        "INSERT INTO activity.voice_mate_ratings
            (rater_user_id, mate_user_id, rating, comment, channel_id, session_seconds)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(rater_id)
    .bind(mate_id)
    .bind(&rating.rating)
    .bind(rating.comment.as_deref())
    .bind(channel_id)
    .bind(rating.session_seconds)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Kommentar an die jüngste Bewertung derselben Paarung hängen. Gibt es noch
/// keine (der User klickt direkt auf Kommentar), entsteht eine neutrale Zeile.
pub(crate) async fn attach_comment_db(
    pool: &PgPool,
    rater_id: u64,
    mate_id: u64,
    comment: &str,
) -> Result<(), String> {
    let rater = i64::try_from(rater_id).map_err(|error| error.to_string())?;
    let mate = i64::try_from(mate_id).map_err(|error| error.to_string())?;
    let updated = sqlx::query(
        "UPDATE activity.voice_mate_ratings
            SET comment = $3
          WHERE id = (
              SELECT id
                FROM activity.voice_mate_ratings
               WHERE rater_user_id = $1
                 AND mate_user_id = $2
               ORDER BY created_at DESC
               LIMIT 1
          )",
    )
    .bind(rater)
    .bind(mate)
    .bind(comment)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?;
    if updated.rows_affected() > 0 {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO activity.voice_mate_ratings
            (rater_user_id, mate_user_id, rating, comment)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(rater)
    .bind(mate)
    .bind(RATING_OK)
    .bind(comment)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Hat einer der beiden den anderen zuletzt abgelehnt? Der Pairing-Vorschlag
/// fragt das, bevor er zwei Leute zusammenbringen will.
pub async fn pair_is_unwanted(pool: &PgPool, one: u64, other: u64) -> Result<bool, String> {
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
    .bind(RATING_RATHER_NOT)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    Ok(unwanted.unwrap_or(false))
}

struct SurveyHandler {
    survey: Arc<MateSurvey>,
}

#[async_trait::async_trait]
impl InteractionHandler for SurveyHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        self.survey.handle_interaction(interaction).await
    }
}

pub fn register(router: &mut InteractionRouter, survey: Arc<MateSurvey>) {
    let handler = Arc::new(SurveyHandler { survey });
    router.on_prefix(CUSTOM_ID_PREFIX, handler.clone());
    router.on_custom_id(COMMENT_MODAL_ID, handler);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex as StdMutex;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp")
    }

    struct TestState {
        decision: AskDecision,
        dms: Vec<(u64, Value)>,
        ratings: Vec<MateRating>,
        comments: Vec<(u64, u64, String)>,
        never: Vec<u64>,
        save_fails: bool,
    }

    impl Default for TestState {
        fn default() -> Self {
            Self {
                decision: AskDecision::Ask,
                dms: Vec::new(),
                ratings: Vec::new(),
                comments: Vec::new(),
                never: Vec::new(),
                save_fails: false,
            }
        }
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

    #[async_trait::async_trait]
    impl MateSurveyPort for TestPort {
        async fn claim_ask(
            &self,
            _rater_id: u64,
            _mate_id: u64,
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

        async fn save_rating(&self, rating: &MateRating) -> Result<(), String> {
            let mut state = self.state.lock().expect("lock");
            if state.save_fails {
                return Err("db kaputt".to_string());
            }
            state.ratings.push(rating.clone());
            Ok(())
        }

        async fn attach_comment(
            &self,
            rater_id: u64,
            mate_id: u64,
            comment: &str,
        ) -> Result<(), String> {
            self.state.lock().expect("lock").comments.push((
                rater_id,
                mate_id,
                comment.to_string(),
            ));
            Ok(())
        }

        fn log_decision(&self, _user_id: u64, _decision: &'static str, _reason: &'static str) {}
    }

    #[test]
    fn pick_mate_ist_deterministisch_und_ohne_selbstbezug() {
        assert_eq!(pick_mate(1, &[3, 2, 2, 1]), Some(2));
        assert_eq!(pick_mate(1, &[1]), None);
        assert_eq!(pick_mate(1, &[]), None);
    }

    #[test]
    fn custom_ids_sind_round_trip_fest() {
        assert_eq!(
            parse_custom_id(&rate_custom_id(RATING_AGAIN, 7, 99, 1800)),
            Some(SurveyAction::Rate {
                mate_id: 7,
                rating: RATING_AGAIN,
                channel_id: 99,
                seconds: 1800
            })
        );
        assert_eq!(
            parse_custom_id(&comment_custom_id(7)),
            Some(SurveyAction::Comment { mate_id: 7 })
        );
        assert_eq!(
            parse_custom_id(&never_custom_id()),
            Some(SurveyAction::Never)
        );
        assert_eq!(parse_custom_id("voice_mate:rate:quatsch:7:9:1"), None);
        assert_eq!(parse_custom_id("fremd:never"), None);
    }

    #[test]
    #[ignore = "Hilfsausgabe: druckt die echte Umfrage-DM zum Gegenlesen"]
    fn dump_survey_dm() {
        println!(
            "{}",
            serde_json::to_string(&dm_body(2, 1523272810825252944, 2700)).expect("json")
        );
    }

    #[test]
    fn minuten_label_bleibt_lesbar() {
        assert_eq!(minutes_label(1200), "20 Minuten");
        assert_eq!(minutes_label(30), "1 Minuten");
        assert_eq!(minutes_label(7200), "2 Stunden");
        assert_eq!(minutes_label(5700), "1 Stunden und 35 Minuten");
    }

    #[test]
    fn dm_body_fragt_nach_dem_mitspieler_und_pingt_nicht() {
        let text = serde_json::to_string(&dm_body(7, 99, 1800)).expect("json");
        assert!(text.contains("<@7>"));
        assert!(text.contains("<#99>"));
        assert!(text.contains("30 Minuten"));
        assert!(text.contains("Würdest du wieder mit"));
        assert!(
            text.contains("Deine Antwort bleibt privat"),
            "die Vertraulichkeit steht in einer eigenen, betonten Zeile"
        );
        assert!(text.contains("dl_ranked"), "Header nutzt die Server-Emojis");
        assert!(text.contains(&rate_custom_id(RATING_AGAIN, 7, 99, 1800)));
        assert!(text.contains(&comment_custom_id(7)));
        assert!(text.contains(&never_custom_id()));
        assert!(text.contains("\"parse\":[]"));
    }

    #[tokio::test]
    async fn kurze_sessions_loesen_keine_umfrage_aus() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        survey
            .on_session_end(1, 99, vec![2], MIN_SECONDS - 1, now())
            .await;
        assert!(port.state.lock().expect("lock").dms.is_empty());
    }

    #[tokio::test]
    async fn session_ohne_mitspieler_loest_keine_umfrage_aus() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        survey
            .on_session_end(1, 99, vec![], MIN_SECONDS * 2, now())
            .await;
        assert!(port.state.lock().expect("lock").dms.is_empty());
    }

    #[tokio::test]
    async fn lange_session_fragt_genau_einmal() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        survey
            .on_session_end(1, 99, vec![2, 3], MIN_SECONDS, now())
            .await;
        let dms = port.state.lock().expect("lock").dms.clone();
        assert_eq!(dms.len(), 1);
        assert_eq!(dms[0].0, 1);
        assert!(serde_json::to_string(&dms[0].1)
            .expect("json")
            .contains("<@2>"));
    }

    #[tokio::test]
    async fn cooldown_unterdrueckt_die_umfrage() {
        let port = TestPort::new(TestState {
            decision: AskDecision::PairCooldown,
            ..TestState::default()
        });
        let survey = MateSurvey::new(port.clone());
        survey
            .on_session_end(1, 99, vec![2], MIN_SECONDS * 3, now())
            .await;
        assert!(port.state.lock().expect("lock").dms.is_empty());
    }

    #[tokio::test]
    async fn bewertung_wird_gespeichert_und_bedankt() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        let reply = survey
            .handle_interaction(BridgeInteraction {
                custom_id: rate_custom_id(RATING_AGAIN, 2, 99, 1800),
                user_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(THANKS_AGAIN));
        let ratings = port.state.lock().expect("lock").ratings.clone();
        assert_eq!(ratings.len(), 1);
        assert_eq!(ratings[0].rater_id, 1);
        assert_eq!(ratings[0].mate_id, 2);
        assert_eq!(ratings[0].rating, RATING_AGAIN);
        assert_eq!(ratings[0].session_seconds, 1800);
    }

    #[tokio::test]
    async fn ablehnung_bekommt_eigene_antwort() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        let reply = survey
            .handle_interaction(BridgeInteraction {
                custom_id: rate_custom_id(RATING_RATHER_NOT, 2, 99, 1800),
                user_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(THANKS_RATHER_NOT));
    }

    #[tokio::test]
    async fn speicherfehler_wird_nicht_als_erfolg_verkauft() {
        let port = TestPort::new(TestState {
            save_fails: true,
            ..TestState::default()
        });
        let survey = MateSurvey::new(port.clone());
        let reply = survey
            .handle_interaction(BridgeInteraction {
                custom_id: rate_custom_id(RATING_OK, 2, 99, 1800),
                user_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(SAVE_FAILED_REPLY));
    }

    #[tokio::test]
    async fn kommentar_button_oeffnet_das_modal() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        let reply = survey
            .handle_interaction(BridgeInteraction {
                custom_id: comment_custom_id(2),
                user_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        let modal = reply.modal.expect("Modal");
        assert_eq!(modal.custom_id, COMMENT_MODAL_ID);
        assert_eq!(modal.fields[1].value.as_deref(), Some("2"));
        assert!(
            modal.fields[0].paragraph,
            "Freitext braucht ein großes Feld"
        );
    }

    #[tokio::test]
    async fn kommentar_landet_an_der_bewertung() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        let reply = survey
            .handle_interaction(BridgeInteraction {
                custom_id: COMMENT_MODAL_ID.to_string(),
                user_id: 1,
                options: HashMap::from([
                    (COMMENT_FIELD.to_string(), json!("war lustig")),
                    ("mate_id".to_string(), json!("2")),
                ]),
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(THANKS_COMMENT));
        assert_eq!(
            port.state.lock().expect("lock").comments,
            vec![(1, 2, "war lustig".to_string())]
        );
    }

    #[tokio::test]
    async fn nie_mehr_fragen_wird_gespeichert() {
        let port = TestPort::new(TestState::default());
        let survey = MateSurvey::new(port.clone());
        let reply = survey
            .handle_interaction(BridgeInteraction {
                custom_id: never_custom_id(),
                user_id: 1,
                ..BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(NEVER_REPLY));
        assert_eq!(port.state.lock().expect("lock").never, vec![1]);
    }
}
