//! Reiner Zustandskern fuer die optionale Mini-Server-Tour.
//!
//! Discord, Persistenz, Membership und Concierge bleiben hinter
//! [`OnboardingTourPort`]. Alle Uebergaenge werden vorher durch die drei
//! `decide_*`-Funktionen bestimmt.

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{de::Error as _, Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

pub const TOUR_STATE_SCHEMA: u8 = 1;
pub const QUESTION_WINDOW_HOURS: i64 = 24;
pub const ANSWERING_LEASE_MINUTES: i64 = 5;
pub const MAX_RETRY_COUNT: u32 = 3;
pub const RETRY_DELAY_SECONDS: i64 = 60;

// TODO(Welle 3): Nach Live-Aufloesung des Kanals durch die belegte Snowflake ersetzen.
pub const PATCHNOTES_CHANNEL_ID: u64 = 0;

pub const VOICE_CONTENT: &str = "**Sprachkanäle, das Herz vom Server** 🎧\n\nDu musst niemanden kennen, um zu joinen. Spring einfach in eine offene Lane, die Leute freuen sich über Mitspieler. Und wenn du lieber deine eigene Lane willst, klick unten auf den Button, wähl deinen Modus, und der Bot baut dir sofort einen eigenen Kanal.";
pub const COMMUNITY_QUESTIONS_CONTENT: &str = "**Fragen? Immer her damit** 💬\n\nEgal ob Frage zum Spiel oder zum Server, stell sie einfach in frag-die-community. Hier beißt niemand, auch die simpelste Frage ist willkommen, und meistens antwortet ziemlich schnell jemand.";
pub const RANK_LINK_CONTENT: &str = "**Dein echter Rang, automatisch** 🔗\n\nVerknüpf einmal kurz deinen Steam-Account, dann holt sich der Bot deinen Rang aus deinen echten Matches und hält ihn von selbst aktuell. Dauert zwei Minuten, die Anleitung findest du hinter dem Button.";
pub const PATCHNOTES_CONTENT: &str = "**Immer auf dem Laufenden** 📰\n\nDeadlock ändert sich ständig. Jedes Update landet bei uns auf Deutsch übersetzt im Patchnotes-Kanal, sobald es rauskommt. Ein kurzer Blick nach jedem Patch, und du weißt sofort, was sich geändert hat.";
pub const SUPPORT_CONTENT: &str = "**Wenn mal was hakt** 🎟️\n\nEin Problem mit dem Server, einem Bot oder einem anderen Mitglied? Mach einfach ein Ticket auf. Das liest nur das Team, und wir kümmern uns.";
pub const STREAMERS_CONTENT: &str = "**Streamer aus der Community** 🎥\n\nEin paar Leute von hier streamen regelmäßig Deadlock. Wenn dir mal langweilig ist, schau im Streamer-Kanal vorbei, da siehst du, wer gerade live ist.";
pub const FINAL_CONTENT: &str = "Das war die Tour, schön, dass du da bist! Wenn später noch Fragen aufkommen, schreib mir einfach eine DM oder frag in der Community. Und jetzt viel Spaß, man sieht sich im Voice! 👋";
pub const NEXT_BUTTON_LABEL: &str = "Weiter";
pub const ASK_BUTTON_LABEL: &str = "Ich hab noch eine Frage";
pub const END_BUTTON_LABEL: &str = "Tour beenden";
pub const FINISH_BUTTON_LABEL: &str = "Alles klar, danke!";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourStatus {
    Starting,
    Active,
    Completed,
    AbortedByUser,
    AbortedLeftGuild,
    AbortedDmBlocked,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourMode {
    Step,
    WaitingForQuestion,
    Answering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourStepKey {
    Voice,
    CommunityQuestions,
    RankLink,
    Patchnotes,
    Support,
    Streamers,
}

impl TourStepKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Voice => "voice",
            Self::CommunityQuestions => "community_questions",
            Self::RankLink => "rank_link",
            Self::Patchnotes => "patchnotes",
            Self::Support => "support",
            Self::Streamers => "streamers",
        }
    }

    pub const fn next(self) -> Option<Self> {
        match self {
            Self::Voice => Some(Self::CommunityQuestions),
            Self::CommunityQuestions => Some(Self::RankLink),
            Self::RankLink => Some(Self::Patchnotes),
            Self::Patchnotes => Some(Self::Support),
            Self::Support => Some(Self::Streamers),
            Self::Streamers => None,
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "voice" => Some(Self::Voice),
            "community_questions" => Some(Self::CommunityQuestions),
            "rank_link" => Some(Self::RankLink),
            "patchnotes" => Some(Self::Patchnotes),
            "support" => Some(Self::Support),
            "streamers" => Some(Self::Streamers),
            _ => None,
        }
    }
}

impl std::fmt::Display for TourStepKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TourState {
    pub schema: u8,
    pub status: TourStatus,
    pub step: TourStepKey,
    pub mode: TourMode,
    pub dm_channel_id: Option<String>,
    pub last_tour_message_id: Option<String>,
    pub claimed_question_message_id: Option<String>,
    pub question_deadline_at: Option<String>,
    pub answering_deadline_at: Option<String>,
    pub retry_count: u32,
    pub next_retry_at: Option<String>,
    pub updated_at: String,
}

#[derive(Deserialize)]
struct TourStateWire {
    schema: u8,
    status: TourStatus,
    step: TourStepKey,
    mode: TourMode,
    dm_channel_id: Option<String>,
    last_tour_message_id: Option<String>,
    claimed_question_message_id: Option<String>,
    question_deadline_at: Option<String>,
    #[serde(default)]
    answering_deadline_at: Option<String>,
    retry_count: u32,
    next_retry_at: Option<String>,
    updated_at: String,
}

impl<'de> Deserialize<'de> for TourState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let state = TourStateWire::deserialize(deserializer)?;
        if state.schema != TOUR_STATE_SCHEMA {
            return Err(D::Error::custom(format!(
                "unbekannte Tour-State-Schema-Version {}",
                state.schema
            )));
        }
        Ok(Self {
            schema: state.schema,
            status: state.status,
            step: state.step,
            mode: state.mode,
            dm_channel_id: state.dm_channel_id,
            last_tour_message_id: state.last_tour_message_id,
            claimed_question_message_id: state.claimed_question_message_id,
            question_deadline_at: state.question_deadline_at,
            answering_deadline_at: state.answering_deadline_at,
            retry_count: state.retry_count,
            next_retry_at: state.next_retry_at,
            updated_at: state.updated_at,
        })
    }
}

impl TourState {
    pub fn starting(now: DateTime<Utc>) -> Self {
        Self {
            schema: TOUR_STATE_SCHEMA,
            status: TourStatus::Starting,
            step: TourStepKey::Voice,
            mode: TourMode::Step,
            dm_channel_id: None,
            last_tour_message_id: None,
            claimed_question_message_id: None,
            question_deadline_at: None,
            answering_deadline_at: None,
            retry_count: 0,
            next_retry_at: None,
            updated_at: now.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourLinkTarget {
    Channel(u64),
    RankGuide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TourStep {
    pub key: TourStepKey,
    pub content: &'static str,
    pub link_target: TourLinkTarget,
    pub link_label: &'static str,
}

pub const TOUR_STEPS: [TourStep; 6] = [
    TourStep {
        key: TourStepKey::Voice,
        content: VOICE_CONTENT,
        link_target: TourLinkTarget::Channel(1_513_468_476_365_209_670),
        link_label: "Lane erstellen",
    },
    TourStep {
        key: TourStepKey::CommunityQuestions,
        content: COMMUNITY_QUESTIONS_CONTENT,
        link_target: TourLinkTarget::Channel(1_426_220_702_054_355_077),
        link_label: "Zum Kanal",
    },
    TourStep {
        key: TourStepKey::RankLink,
        content: RANK_LINK_CONTENT,
        link_target: TourLinkTarget::RankGuide,
        link_label: "Zur Anleitung",
    },
    TourStep {
        key: TourStepKey::Patchnotes,
        content: PATCHNOTES_CONTENT,
        link_target: TourLinkTarget::Channel(PATCHNOTES_CHANNEL_ID),
        link_label: "Zum Kanal",
    },
    TourStep {
        key: TourStepKey::Support,
        content: SUPPORT_CONTENT,
        link_target: TourLinkTarget::Channel(1_459_628_609_705_738_539),
        link_label: "Ticket eröffnen",
    },
    TourStep {
        key: TourStepKey::Streamers,
        content: STREAMERS_CONTENT,
        link_target: TourLinkTarget::Channel(1_304_169_815_505_637_458),
        link_label: "Zum Kanal",
    },
];

fn tour_step(step: TourStepKey) -> &'static TourStep {
    &TOUR_STEPS[match step {
        TourStepKey::Voice => 0,
        TourStepKey::CommunityQuestions => 1,
        TourStepKey::RankLink => 2,
        TourStepKey::Patchnotes => 3,
        TourStepKey::Support => 4,
        TourStepKey::Streamers => 5,
    }]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourComponentAction {
    Next,
    Ask,
    End,
    Finish,
}

impl TourComponentAction {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Next => "next",
            Self::Ask => "ask",
            Self::End => "end",
            Self::Finish => "finish",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TourCustomId {
    pub action: TourComponentAction,
    pub step: TourStepKey,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TourCustomIdError {
    #[error("malformed tour custom_id")]
    Malformed,
    #[error("foreign custom_id prefix")]
    ForeignPrefix,
    #[error("unsupported tour custom_id version")]
    UnsupportedVersion,
    #[error("unknown tour action")]
    UnknownAction,
    #[error("unknown tour step")]
    UnknownStep,
}

pub fn build_tour_custom_id(action: TourComponentAction, step: TourStepKey) -> String {
    format!("tour:v1:{}:{}", action.as_str(), step.as_str())
}

pub fn parse_tour_custom_id(value: &str) -> Result<TourCustomId, TourCustomIdError> {
    let mut parts = value.split(':');
    let (Some(prefix), Some(version), Some(action), Some(step), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(TourCustomIdError::Malformed);
    };
    if prefix != "tour" {
        return Err(TourCustomIdError::ForeignPrefix);
    }
    if version != "v1" {
        return Err(TourCustomIdError::UnsupportedVersion);
    }
    let action = match action {
        "next" => TourComponentAction::Next,
        "ask" => TourComponentAction::Ask,
        "end" => TourComponentAction::End,
        "finish" => TourComponentAction::Finish,
        _ => return Err(TourCustomIdError::UnknownAction),
    };
    let step = TourStepKey::parse(step).ok_or(TourCustomIdError::UnknownStep)?;
    Ok(TourCustomId { action, step })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTourLink {
    pub step: TourStepKey,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TourMessagePayload {
    pub content: String,
    pub components: Value,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TourPayloadError {
    #[error("no resolved link for tour step {0}")]
    MissingStepLink(TourStepKey),
}

pub fn tour_step_payload(
    step_key: TourStepKey,
    links: &[ResolvedTourLink],
) -> Result<TourMessagePayload, TourPayloadError> {
    let step = tour_step(step_key);
    let mut buttons = links
        .iter()
        .filter(|link| link.step == step_key)
        .map(|link| {
            json!({
                "type": 2,
                "style": 5,
                "label": step.link_label,
                "url": link.url,
            })
        })
        .collect::<Vec<_>>();
    if buttons.is_empty() {
        return Err(TourPayloadError::MissingStepLink(step_key));
    }

    let (progress_action, progress_label) = if step_key == TourStepKey::Streamers {
        (TourComponentAction::Finish, FINISH_BUTTON_LABEL)
    } else {
        (TourComponentAction::Next, NEXT_BUTTON_LABEL)
    };
    for (action, label) in [
        (progress_action, progress_label),
        (TourComponentAction::Ask, ASK_BUTTON_LABEL),
        (TourComponentAction::End, END_BUTTON_LABEL),
    ] {
        buttons.push(json!({
            "type": 2,
            "style": 2,
            "label": label,
            "custom_id": build_tour_custom_id(action, step_key),
        }));
    }

    let content = if step_key == TourStepKey::Streamers {
        format!("{}\n\n{FINAL_CONTENT}", step.content)
    } else {
        step.content.to_string()
    };
    Ok(TourMessagePayload {
        content,
        components: json!(buttons
            .chunks(5)
            .take(5)
            .map(|buttons| json!({ "type": 1, "components": buttons }))
            .collect::<Vec<_>>()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TourOnboardingEvent {
    pub guild_id: u64,
    pub user_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TourClaimResult {
    Acquired(TourState),
    Existing(TourState),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TourSendResult {
    Sent {
        dm_channel_id: String,
        message_id: String,
    },
    CannotSend50007,
    Transient,
    Uncertain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TourAbortReason {
    User,
    LeftGuild,
    DmBlocked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TourAction {
    ClaimStart(TourState),
    SendStep(TourStepKey),
    PersistState {
        expected: TourState,
        next: TourState,
    },
    OpenQuestionWindow(DateTime<Utc>),
    AnswerQuestionViaConcierge {
        channel_id: String,
        content: String,
        step: TourStepKey,
        resume_state: Box<TourState>,
    },
    ScheduleRetry(DateTime<Utc>),
    MarkUncertain,
    RemoveMarker,
    Complete,
    Abort(TourAbortReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnboardingTourDecisionInput {
    pub event: TourOnboardingEvent,
    pub main_guild_id: u64,
    pub is_bot: bool,
    pub has_tour_marker: bool,
    pub has_rank_marker: bool,
    pub claim_result: Option<TourClaimResult>,
    pub send_result: Option<TourSendResult>,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TourComponentDecisionInput {
    pub state: TourState,
    pub action: TourComponentAction,
    pub step: TourStepKey,
    pub member_still_in_guild: bool,
    pub now: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TourDmMessageDecisionInput {
    pub state: TourState,
    pub channel_id: String,
    pub message_id: String,
    pub content: String,
    pub member_still_in_guild: bool,
    pub now: DateTime<Utc>,
}

fn persist_state(expected: &TourState, next: TourState) -> TourAction {
    TourAction::PersistState {
        expected: expected.clone(),
        next,
    }
}

pub fn normalize_tour_state(state: &TourState, now: DateTime<Utc>) -> TourState {
    let mut normalized = state.clone();
    match state.mode {
        TourMode::Step => {
            normalized.answering_deadline_at = None;
        }
        TourMode::WaitingForQuestion => {
            let deadline_is_valid = state
                .question_deadline_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc) > now)
                .unwrap_or(false);
            if !deadline_is_valid {
                normalized.mode = TourMode::Step;
                normalized.question_deadline_at = None;
                normalized.updated_at = now.to_rfc3339();
            }
        }
        TourMode::Answering => {
            let deadline_is_valid = state
                .answering_deadline_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc) > now)
                .unwrap_or(false);
            if !deadline_is_valid {
                normalized.mode = TourMode::Step;
                normalized.question_deadline_at = None;
                normalized.answering_deadline_at = None;
                normalized.updated_at = now.to_rfc3339();
            }
        }
    }
    normalized
}

fn normalized_noop(expected: &TourState, normalized: TourState) -> Vec<TourAction> {
    if *expected == normalized {
        Vec::new()
    } else {
        vec![persist_state(expected, normalized)]
    }
}

pub fn decide_onboarding_tour_actions(input: &OnboardingTourDecisionInput) -> Vec<TourAction> {
    if input.event.guild_id != input.main_guild_id || input.is_bot || !input.has_tour_marker {
        return Vec::new();
    }

    match input.claim_result.as_ref() {
        None => vec![TourAction::ClaimStart(TourState::starting(input.now))],
        Some(TourClaimResult::Existing(state)) => {
            let normalized = normalize_tour_state(state, input.now);
            let mut actions = normalized_noop(state, normalized.clone());
            match normalized.status {
                TourStatus::Starting => actions,
                TourStatus::Active
                | TourStatus::Completed
                | TourStatus::AbortedByUser
                | TourStatus::AbortedLeftGuild
                | TourStatus::AbortedDmBlocked
                | TourStatus::Uncertain => {
                    actions.push(TourAction::RemoveMarker);
                    actions
                }
            }
        }
        Some(TourClaimResult::Acquired(state)) => {
            let normalized = normalize_tour_state(state, input.now);
            match normalized.status {
                TourStatus::Starting => match input.send_result.as_ref() {
                    None => vec![TourAction::SendStep(normalized.step)],
                    Some(result) => {
                        decide_tour_send_completion_actions(state, result, input.now, true)
                    }
                },
                TourStatus::Active
                | TourStatus::Completed
                | TourStatus::AbortedByUser
                | TourStatus::AbortedLeftGuild
                | TourStatus::AbortedDmBlocked
                | TourStatus::Uncertain => normalized_noop(state, normalized),
            }
        }
    }
}

pub fn decide_tour_component_actions(input: &TourComponentDecisionInput) -> Vec<TourAction> {
    let expected = &input.state;
    let mut state = normalize_tour_state(expected, input.now);
    let actionable_status = match state.status {
        TourStatus::Active | TourStatus::Uncertain => true,
        TourStatus::Starting
        | TourStatus::Completed
        | TourStatus::AbortedByUser
        | TourStatus::AbortedLeftGuild
        | TourStatus::AbortedDmBlocked => false,
    };
    let actionable_mode = match state.mode {
        TourMode::Step | TourMode::WaitingForQuestion => true,
        TourMode::Answering => false,
    };
    if state.step != input.step || !actionable_status || !actionable_mode {
        return normalized_noop(expected, state);
    }

    state.status = TourStatus::Active;
    if !input.member_still_in_guild {
        state.status = TourStatus::AbortedLeftGuild;
        state.updated_at = input.now.to_rfc3339();
        return vec![
            persist_state(expected, state),
            TourAction::Abort(TourAbortReason::LeftGuild),
        ];
    }

    match input.action {
        TourComponentAction::Next => {
            let Some(next_step) = input.step.next() else {
                return Vec::new();
            };
            let pending = pending_send_state(&state, next_step, input.now);
            vec![
                persist_state(expected, pending),
                TourAction::SendStep(next_step),
            ]
        }
        TourComponentAction::Ask => {
            if state.mode == TourMode::WaitingForQuestion {
                return Vec::new();
            }
            let deadline = input.now + Duration::hours(QUESTION_WINDOW_HOURS);
            state.mode = TourMode::WaitingForQuestion;
            state.question_deadline_at = Some(deadline.to_rfc3339());
            state.answering_deadline_at = None;
            state.updated_at = input.now.to_rfc3339();
            vec![
                persist_state(expected, state),
                TourAction::OpenQuestionWindow(deadline),
            ]
        }
        TourComponentAction::End => {
            state.status = TourStatus::AbortedByUser;
            state.question_deadline_at = None;
            state.answering_deadline_at = None;
            state.updated_at = input.now.to_rfc3339();
            vec![
                persist_state(expected, state),
                TourAction::Abort(TourAbortReason::User),
            ]
        }
        TourComponentAction::Finish => {
            if input.step != TourStepKey::Streamers {
                return Vec::new();
            }
            state.status = TourStatus::Completed;
            state.question_deadline_at = None;
            state.answering_deadline_at = None;
            state.updated_at = input.now.to_rfc3339();
            vec![persist_state(expected, state), TourAction::Complete]
        }
    }
}

pub fn decide_tour_dm_message_actions(input: &TourDmMessageDecisionInput) -> Vec<TourAction> {
    let expected = &input.state;
    let state = normalize_tour_state(expected, input.now);
    let active = match state.status {
        TourStatus::Active => true,
        TourStatus::Starting
        | TourStatus::Completed
        | TourStatus::AbortedByUser
        | TourStatus::AbortedLeftGuild
        | TourStatus::AbortedDmBlocked
        | TourStatus::Uncertain => false,
    };
    let waiting = match state.mode {
        TourMode::WaitingForQuestion => true,
        TourMode::Step | TourMode::Answering => false,
    };
    let already_claimed = state
        .claimed_question_message_id
        .as_deref()
        .is_some_and(|watermark| {
            match (input.message_id.parse::<u64>(), watermark.parse::<u64>()) {
                (Ok(message_id), Ok(watermark)) => message_id <= watermark,
                _ => input.message_id == watermark,
            }
        });
    if !active || !waiting {
        return normalized_noop(expected, state);
    }

    if !input.member_still_in_guild {
        let mut aborted = state;
        aborted.status = TourStatus::AbortedLeftGuild;
        aborted.mode = TourMode::Step;
        aborted.question_deadline_at = None;
        aborted.answering_deadline_at = None;
        aborted.updated_at = input.now.to_rfc3339();
        return vec![
            persist_state(expected, aborted),
            TourAction::Abort(TourAbortReason::LeftGuild),
        ];
    }

    if state
        .dm_channel_id
        .as_ref()
        .is_some_and(|channel_id| channel_id != &input.channel_id)
        || already_claimed
    {
        return Vec::new();
    }

    let mut answering = state;
    answering.mode = TourMode::Answering;
    answering.claimed_question_message_id = Some(input.message_id.clone());
    answering.question_deadline_at = None;
    answering.answering_deadline_at =
        Some((input.now + Duration::minutes(ANSWERING_LEASE_MINUTES)).to_rfc3339());
    answering.updated_at = input.now.to_rfc3339();

    let mut resume = answering.clone();
    resume.mode = TourMode::Step;
    resume.answering_deadline_at = None;
    vec![
        persist_state(expected, answering),
        TourAction::AnswerQuestionViaConcierge {
            channel_id: input.channel_id.clone(),
            content: input.content.clone(),
            step: input.state.step,
            resume_state: Box::new(resume),
        },
    ]
}

fn pending_send_state(
    state: &TourState,
    target_step: TourStepKey,
    now: DateTime<Utc>,
) -> TourState {
    TourState {
        status: TourStatus::Starting,
        step: target_step,
        mode: TourMode::Step,
        question_deadline_at: None,
        answering_deadline_at: None,
        next_retry_at: None,
        updated_at: now.to_rfc3339(),
        ..state.clone()
    }
}

pub fn decide_tour_send_completion_actions(
    pending_state: &TourState,
    result: &TourSendResult,
    now: DateTime<Utc>,
    remove_marker: bool,
) -> Vec<TourAction> {
    match pending_state.status {
        TourStatus::Starting => actions_after_send(pending_state, result, now, remove_marker),
        TourStatus::Active
        | TourStatus::Completed
        | TourStatus::AbortedByUser
        | TourStatus::AbortedLeftGuild
        | TourStatus::AbortedDmBlocked
        | TourStatus::Uncertain => Vec::new(),
    }
}

pub fn decide_tour_retry_actions(state: &TourState, now: DateTime<Utc>) -> Vec<TourAction> {
    let normalized = normalize_tour_state(state, now);
    match normalized.status {
        TourStatus::Starting => {
            let due = normalized
                .next_retry_at
                .as_deref()
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc) <= now)
                .unwrap_or(false);
            if due {
                let mut claimed = normalized.clone();
                claimed.next_retry_at =
                    Some((now + Duration::seconds(RETRY_DELAY_SECONDS)).to_rfc3339());
                claimed.updated_at = now.to_rfc3339();
                vec![
                    persist_state(state, claimed),
                    TourAction::SendStep(normalized.step),
                ]
            } else {
                normalized_noop(state, normalized)
            }
        }
        TourStatus::Active
        | TourStatus::Completed
        | TourStatus::AbortedByUser
        | TourStatus::AbortedLeftGuild
        | TourStatus::AbortedDmBlocked
        | TourStatus::Uncertain => normalized_noop(state, normalized),
    }
}

fn actions_after_send(
    state: &TourState,
    result: &TourSendResult,
    now: DateTime<Utc>,
    remove_marker: bool,
) -> Vec<TourAction> {
    let target_step = state.step;
    match result {
        TourSendResult::Sent {
            dm_channel_id,
            message_id,
        } => {
            let mut next = state.clone();
            next.status = TourStatus::Active;
            next.step = target_step;
            next.mode = TourMode::Step;
            next.dm_channel_id = Some(dm_channel_id.clone());
            next.last_tour_message_id = Some(message_id.clone());
            next.question_deadline_at = None;
            next.answering_deadline_at = None;
            next.retry_count = 0;
            next.next_retry_at = None;
            next.updated_at = now.to_rfc3339();
            let mut actions = vec![persist_state(state, next)];
            if remove_marker {
                actions.push(TourAction::RemoveMarker);
            }
            actions
        }
        TourSendResult::CannotSend50007 => {
            let mut aborted = state.clone();
            aborted.status = TourStatus::AbortedDmBlocked;
            aborted.step = target_step;
            aborted.question_deadline_at = None;
            aborted.answering_deadline_at = None;
            aborted.next_retry_at = None;
            aborted.updated_at = now.to_rfc3339();
            let mut actions = vec![
                persist_state(state, aborted),
                TourAction::Abort(TourAbortReason::DmBlocked),
            ];
            if remove_marker {
                actions.push(TourAction::RemoveMarker);
            }
            actions
        }
        TourSendResult::Transient if state.retry_count < MAX_RETRY_COUNT => {
            let retry_at = now + Duration::seconds(RETRY_DELAY_SECONDS);
            let mut retry = pending_send_state(state, target_step, now);
            retry.retry_count += 1;
            retry.next_retry_at = Some(retry_at.to_rfc3339());
            vec![
                persist_state(state, retry),
                TourAction::ScheduleRetry(retry_at),
            ]
        }
        TourSendResult::Transient | TourSendResult::Uncertain => {
            let mut uncertain = state.clone();
            uncertain.status = TourStatus::Uncertain;
            uncertain.step = target_step;
            uncertain.mode = TourMode::Step;
            uncertain.question_deadline_at = None;
            uncertain.answering_deadline_at = None;
            uncertain.next_retry_at = None;
            uncertain.updated_at = now.to_rfc3339();
            vec![persist_state(state, uncertain), TourAction::MarkUncertain]
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TourMemberSnapshot {
    pub is_bot: bool,
    pub role_ids: Vec<u64>,
}

/// Seiteneffekt-Port fuer die in Welle 2 folgende Runtime-Verdrahtung.
///
/// Implementierungen muessen Claim und Transition atomar ausfuehren. Insbesondere
/// sperrt `transition_state_locked` die User-Row und schreibt nur bei passendem
/// `expected`-State.
#[async_trait]
pub trait OnboardingTourPort: Send + Sync {
    async fn load_member_snapshot(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<TourMemberSnapshot, String>;

    async fn claim_or_load_state(
        &self,
        user_id: u64,
        initial: &TourState,
    ) -> Result<TourClaimResult, String>;

    async fn load_due_retries(&self, now: DateTime<Utc>) -> Result<Vec<(u64, TourState)>, String>;

    async fn transition_state_locked(
        &self,
        user_id: u64,
        expected: &TourState,
        next: &TourState,
    ) -> Result<bool, String>;

    async fn resolve_step_links(
        &self,
        guild_id: u64,
        step: TourStepKey,
    ) -> Result<Vec<ResolvedTourLink>, String>;

    async fn send_tour_message(&self, user_id: u64, payload: TourMessagePayload) -> TourSendResult;

    async fn member_still_in_guild(&self, guild_id: u64, user_id: u64) -> Result<bool, String>;

    async fn remove_marker_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), String>;

    async fn answer_question_via_concierge(
        &self,
        user_id: u64,
        channel_id: &str,
        content: &str,
        step: TourStepKey,
        answering_state: &TourState,
        resume_state: &TourState,
    ) -> Result<(), String>;
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use serde_json::json;

    use super::*;

    const MAIN_GUILD_ID: u64 = 1;

    fn now() -> DateTime<Utc> {
        "2026-07-15T12:00:00Z".parse().expect("valid test time")
    }

    fn state(status: TourStatus, step: TourStepKey, mode: TourMode) -> TourState {
        TourState {
            schema: TOUR_STATE_SCHEMA,
            status,
            step,
            mode,
            dm_channel_id: Some("123".into()),
            last_tour_message_id: Some("456".into()),
            claimed_question_message_id: None,
            question_deadline_at: None,
            answering_deadline_at: None,
            retry_count: 0,
            next_retry_at: None,
            updated_at: now().to_rfc3339(),
        }
    }

    fn start_input() -> OnboardingTourDecisionInput {
        OnboardingTourDecisionInput {
            event: TourOnboardingEvent {
                guild_id: MAIN_GUILD_ID,
                user_id: 2,
            },
            main_guild_id: MAIN_GUILD_ID,
            is_bot: false,
            has_tour_marker: true,
            has_rank_marker: false,
            claim_result: None,
            send_result: None,
            now: now(),
        }
    }

    fn component_input(
        state: TourState,
        action: TourComponentAction,
        step: TourStepKey,
    ) -> TourComponentDecisionInput {
        TourComponentDecisionInput {
            state,
            action,
            step,
            member_still_in_guild: true,
            now: now(),
        }
    }

    fn persisted(actions: &[TourAction]) -> &TourState {
        actions
            .iter()
            .find_map(|action| match action {
                TourAction::PersistState { next, .. } => Some(next),
                _ => None,
            })
            .expect("expected persisted state")
    }

    #[test]
    fn tour_state_json_roundtrip_v1_and_unknown_schema_fail_closed() {
        let original = TourState {
            question_deadline_at: Some(
                (now() + Duration::hours(QUESTION_WINDOW_HOURS)).to_rfc3339(),
            ),
            ..state(
                TourStatus::Active,
                TourStepKey::Voice,
                TourMode::WaitingForQuestion,
            )
        };

        let encoded = serde_json::to_string(&original).expect("serialize state");
        assert_eq!(
            serde_json::from_str::<TourState>(&encoded).expect("deserialize v1"),
            original
        );

        let mut old_schema = serde_json::to_value(&original).expect("state value");
        old_schema
            .as_object_mut()
            .expect("state object")
            .remove("answering_deadline_at");
        assert_eq!(
            serde_json::from_value::<TourState>(old_schema)
                .expect("schema 1 without answering lease")
                .answering_deadline_at,
            None
        );

        let mut unsupported = serde_json::to_value(&original).expect("state value");
        unsupported["schema"] = json!(2);
        assert!(serde_json::from_value::<TourState>(unsupported).is_err());
    }

    #[test]
    fn tour_steps_have_the_fixed_order_and_targets() {
        assert_eq!(
            TOUR_STEPS.map(|step| step.key),
            [
                TourStepKey::Voice,
                TourStepKey::CommunityQuestions,
                TourStepKey::RankLink,
                TourStepKey::Patchnotes,
                TourStepKey::Support,
                TourStepKey::Streamers,
            ]
        );
        assert_eq!(
            TOUR_STEPS.map(|step| step.link_target),
            [
                TourLinkTarget::Channel(1_513_468_476_365_209_670),
                TourLinkTarget::Channel(1_426_220_702_054_355_077),
                TourLinkTarget::RankGuide,
                TourLinkTarget::Channel(PATCHNOTES_CHANNEL_ID),
                TourLinkTarget::Channel(1_459_628_609_705_738_539),
                TourLinkTarget::Channel(1_304_169_815_505_637_458),
            ]
        );
        assert_eq!(PATCHNOTES_CHANNEL_ID, 0);
    }

    #[test]
    fn custom_id_parser_accepts_only_the_versioned_schema() {
        for (raw, action, step) in [
            (
                "tour:v1:next:voice",
                TourComponentAction::Next,
                TourStepKey::Voice,
            ),
            (
                "tour:v1:ask:community_questions",
                TourComponentAction::Ask,
                TourStepKey::CommunityQuestions,
            ),
            (
                "tour:v1:end:rank_link",
                TourComponentAction::End,
                TourStepKey::RankLink,
            ),
            (
                "tour:v1:finish:streamers",
                TourComponentAction::Finish,
                TourStepKey::Streamers,
            ),
        ] {
            let parsed = parse_tour_custom_id(raw).expect("valid custom id");
            assert_eq!(parsed.action, action);
            assert_eq!(parsed.step, step);
            assert_eq!(build_tour_custom_id(action, step), raw);
        }

        for raw in [
            "other:v1:next:voice",
            "tour:v2:next:voice",
            "tour:v1:skip:voice",
            "tour:v1:next:unknown",
            "tour:v1:next",
            "tour:v1:next:voice:extra",
            "tour::next:voice",
            "",
        ] {
            assert!(parse_tour_custom_id(raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn payload_builder_makes_one_step_message_with_only_its_links() {
        for (index, step) in TOUR_STEPS.iter().enumerate() {
            let own_url = format!("https://discord.com/channels/1/{index}");
            let foreign_step = if step.key == TourStepKey::Voice {
                TourStepKey::Support
            } else {
                TourStepKey::Voice
            };
            let links = [
                ResolvedTourLink {
                    step: step.key,
                    url: own_url.clone(),
                },
                ResolvedTourLink {
                    step: foreign_step,
                    url: "https://example.invalid/foreign".into(),
                },
            ];
            let payload = tour_step_payload(step.key, &links).expect("step payload");
            let rendered = payload.components.to_string();

            assert!(payload.content.starts_with(step.content));
            assert!(rendered.contains(step.link_label));
            assert!(rendered.contains(&own_url));
            assert!(!rendered.contains("example.invalid/foreign"));
            assert!(rendered.contains(&build_tour_custom_id(TourComponentAction::Ask, step.key)));
            assert!(rendered.contains(&build_tour_custom_id(TourComponentAction::End, step.key)));

            let next_action = if index + 1 == TOUR_STEPS.len() {
                TourComponentAction::Finish
            } else {
                TourComponentAction::Next
            };
            assert!(rendered.contains(&build_tour_custom_id(next_action, step.key)));
            assert_eq!(rendered.contains(":next:"), index + 1 != TOUR_STEPS.len());
        }
    }

    #[test]
    fn payload_builder_rejects_a_step_without_its_own_link() {
        assert_eq!(
            tour_step_payload(TourStepKey::Support, &[]),
            Err(TourPayloadError::MissingStepLink(TourStepKey::Support))
        );
    }

    #[test]
    fn onboarding_filters_wrong_guild_bots_and_missing_tour_marker() {
        let mut wrong_guild = start_input();
        wrong_guild.event.guild_id += 1;
        let mut bot = start_input();
        bot.is_bot = true;
        let mut no_marker = start_input();
        no_marker.has_tour_marker = false;

        for input in [wrong_guild, bot, no_marker] {
            assert!(decide_onboarding_tour_actions(&input).is_empty());
        }
    }

    #[test]
    fn rank_marker_without_tour_marker_is_not_a_tour_start() {
        let mut input = start_input();
        input.has_tour_marker = false;
        input.has_rank_marker = true;
        assert!(decide_onboarding_tour_actions(&input).is_empty());
    }

    #[test]
    fn onboarding_claim_table_handles_new_starting_active_and_terminal_states() {
        let initial = TourState::starting(now());
        assert_eq!(
            decide_onboarding_tour_actions(&start_input()),
            vec![TourAction::ClaimStart(initial.clone())]
        );

        let mut acquired = start_input();
        acquired.claim_result = Some(TourClaimResult::Acquired(initial.clone()));
        assert_eq!(
            decide_onboarding_tour_actions(&acquired),
            vec![TourAction::SendStep(TourStepKey::Voice)]
        );

        let mut existing_starting = start_input();
        existing_starting.claim_result = Some(TourClaimResult::Existing(initial));
        assert!(decide_onboarding_tour_actions(&existing_starting).is_empty());

        for status in [
            TourStatus::Active,
            TourStatus::Completed,
            TourStatus::AbortedByUser,
            TourStatus::AbortedLeftGuild,
            TourStatus::AbortedDmBlocked,
            TourStatus::Uncertain,
        ] {
            let mut input = start_input();
            input.claim_result = Some(TourClaimResult::Existing(state(
                status,
                TourStepKey::Voice,
                TourMode::Step,
            )));
            assert_eq!(
                decide_onboarding_tour_actions(&input),
                vec![TourAction::RemoveMarker]
            );
        }
    }

    #[test]
    fn successful_first_send_activates_voice_and_removes_marker() {
        let mut input = start_input();
        input.claim_result = Some(TourClaimResult::Acquired(TourState::starting(now())));
        input.send_result = Some(TourSendResult::Sent {
            dm_channel_id: "10".into(),
            message_id: "11".into(),
        });

        let actions = decide_onboarding_tour_actions(&input);
        assert_eq!(persisted(&actions).status, TourStatus::Active);
        assert_eq!(persisted(&actions).step, TourStepKey::Voice);
        assert_eq!(persisted(&actions).dm_channel_id.as_deref(), Some("10"));
        assert!(actions.contains(&TourAction::RemoveMarker));
    }

    #[test]
    fn send_50007_aborts_and_removes_marker() {
        let mut input = start_input();
        input.claim_result = Some(TourClaimResult::Acquired(TourState::starting(now())));
        input.send_result = Some(TourSendResult::CannotSend50007);

        let actions = decide_onboarding_tour_actions(&input);
        assert_eq!(persisted(&actions).status, TourStatus::AbortedDmBlocked);
        assert!(actions.contains(&TourAction::Abort(TourAbortReason::DmBlocked)));
        assert!(actions.contains(&TourAction::RemoveMarker));
    }

    #[test]
    fn transient_send_schedules_a_persistent_retry() {
        let mut input = start_input();
        input.claim_result = Some(TourClaimResult::Acquired(TourState::starting(now())));
        input.send_result = Some(TourSendResult::Transient);

        let actions = decide_onboarding_tour_actions(&input);
        let retry_at = now() + Duration::seconds(RETRY_DELAY_SECONDS);
        assert_eq!(persisted(&actions).retry_count, 1);
        assert_eq!(
            persisted(&actions).next_retry_at.as_deref(),
            Some(retry_at.to_rfc3339().as_str())
        );
        assert!(actions.contains(&TourAction::ScheduleRetry(retry_at)));
    }

    #[test]
    fn retry_limit_and_unclear_send_mark_the_tour_uncertain() {
        for send_result in [TourSendResult::Transient, TourSendResult::Uncertain] {
            let mut starting = TourState::starting(now());
            starting.retry_count = if send_result == TourSendResult::Transient {
                MAX_RETRY_COUNT
            } else {
                0
            };
            let mut input = start_input();
            input.claim_result = Some(TourClaimResult::Acquired(starting));
            input.send_result = Some(send_result);

            let actions = decide_onboarding_tour_actions(&input);
            assert_eq!(persisted(&actions).status, TourStatus::Uncertain);
            assert!(actions.contains(&TourAction::MarkUncertain));
            assert!(!actions
                .iter()
                .any(|action| matches!(action, TourAction::ScheduleRetry(_))));
        }
    }

    #[test]
    fn component_next_walks_steps_zero_to_five_and_finish_completes() {
        let mut current = state(TourStatus::Active, TourStepKey::Voice, TourMode::Step);

        for expected in [
            TourStepKey::CommunityQuestions,
            TourStepKey::RankLink,
            TourStepKey::Patchnotes,
            TourStepKey::Support,
            TourStepKey::Streamers,
        ] {
            let before = current.step;
            let input = component_input(current, TourComponentAction::Next, before);
            let pending_actions = decide_tour_component_actions(&input);
            assert_eq!(persisted(&pending_actions).status, TourStatus::Starting);
            assert_eq!(persisted(&pending_actions).step, expected);
            assert!(pending_actions.contains(&TourAction::SendStep(expected)));
            current = persisted(&decide_tour_send_completion_actions(
                persisted(&pending_actions),
                &TourSendResult::Sent {
                    dm_channel_id: "123".into(),
                    message_id: format!("message-{expected}"),
                },
                now(),
                false,
            ))
            .clone();
            assert_eq!(current.step, expected);
        }

        let finish = component_input(current, TourComponentAction::Finish, TourStepKey::Streamers);
        let actions = decide_tour_component_actions(&finish);
        assert_eq!(persisted(&actions).status, TourStatus::Completed);
        assert!(actions.contains(&TourAction::Complete));
    }

    #[test]
    fn component_end_aborts_by_user() {
        let input = component_input(
            state(TourStatus::Active, TourStepKey::RankLink, TourMode::Step),
            TourComponentAction::End,
            TourStepKey::RankLink,
        );
        let actions = decide_tour_component_actions(&input);
        assert_eq!(persisted(&actions).status, TourStatus::AbortedByUser);
        assert!(actions.contains(&TourAction::Abort(TourAbortReason::User)));
    }

    #[test]
    fn component_aborts_if_membership_was_lost() {
        let mut input = component_input(
            state(TourStatus::Active, TourStepKey::Voice, TourMode::Step),
            TourComponentAction::Next,
            TourStepKey::Voice,
        );
        input.member_still_in_guild = false;
        let actions = decide_tour_component_actions(&input);
        assert_eq!(persisted(&actions).status, TourStatus::AbortedLeftGuild);
        assert!(actions.contains(&TourAction::Abort(TourAbortReason::LeftGuild)));
    }

    #[test]
    fn stale_or_duplicate_component_is_a_noop() {
        let advanced = state(
            TourStatus::Active,
            TourStepKey::CommunityQuestions,
            TourMode::Step,
        );
        let stale = component_input(advanced, TourComponentAction::Next, TourStepKey::Voice);
        assert!(decide_tour_component_actions(&stale).is_empty());

        let waiting = TourState {
            question_deadline_at: Some(
                (now() + Duration::hours(QUESTION_WINDOW_HOURS)).to_rfc3339(),
            ),
            ..state(
                TourStatus::Active,
                TourStepKey::Voice,
                TourMode::WaitingForQuestion,
            )
        };
        let duplicate_ask = component_input(waiting, TourComponentAction::Ask, TourStepKey::Voice);
        assert!(decide_tour_component_actions(&duplicate_ask).is_empty());
    }

    #[test]
    fn ask_waiting_question_claim_answer_resumes_the_same_step() {
        let ask = component_input(
            state(TourStatus::Active, TourStepKey::RankLink, TourMode::Step),
            TourComponentAction::Ask,
            TourStepKey::RankLink,
        );
        let ask_actions = decide_tour_component_actions(&ask);
        let waiting = persisted(&ask_actions).clone();
        assert_eq!(waiting.mode, TourMode::WaitingForQuestion);
        assert_eq!(
            waiting.question_deadline_at,
            Some((now() + Duration::hours(QUESTION_WINDOW_HOURS)).to_rfc3339())
        );
        assert!(ask_actions
            .iter()
            .any(|action| matches!(action, TourAction::OpenQuestionWindow(_))));

        let dm_actions = decide_tour_dm_message_actions(&TourDmMessageDecisionInput {
            state: waiting,
            channel_id: "123".into(),
            message_id: "1001".into(),
            content: "PLATZHALTER: Frageinhalt".into(),
            member_still_in_guild: true,
            now: now(),
        });
        assert_eq!(persisted(&dm_actions).mode, TourMode::Answering);
        assert_eq!(
            persisted(&dm_actions).answering_deadline_at,
            Some((now() + Duration::minutes(ANSWERING_LEASE_MINUTES)).to_rfc3339())
        );
        let resume = dm_actions
            .iter()
            .find_map(|action| match action {
                TourAction::AnswerQuestionViaConcierge { resume_state, .. } => {
                    Some(resume_state.as_ref())
                }
                _ => None,
            })
            .expect("concierge action");
        assert_eq!(resume.status, TourStatus::Active);
        assert_eq!(resume.mode, TourMode::Step);
        assert_eq!(resume.step, TourStepKey::RankLink);
        assert_eq!(resume.claimed_question_message_id.as_deref(), Some("1001"));
        assert_eq!(resume.answering_deadline_at, None);
    }

    #[test]
    fn expired_answering_lease_preserves_watermark_and_unblocks_buttons() {
        let answering = TourState {
            claimed_question_message_id: Some("1001".into()),
            answering_deadline_at: Some((now() - Duration::seconds(1)).to_rfc3339()),
            ..state(
                TourStatus::Active,
                TourStepKey::Support,
                TourMode::Answering,
            )
        };

        let actions = decide_tour_component_actions(&component_input(
            answering.clone(),
            TourComponentAction::Next,
            TourStepKey::Support,
        ));

        assert!(matches!(
            actions.first(),
            Some(TourAction::PersistState { expected, next })
                if expected == &answering
                    && next.status == TourStatus::Starting
                    && next.step == TourStepKey::Streamers
                    && next.mode == TourMode::Step
                    && next.claimed_question_message_id.as_deref() == Some("1001")
                    && next.answering_deadline_at.is_none()
        ));
        assert!(actions.contains(&TourAction::SendStep(TourStepKey::Streamers)));
    }

    #[test]
    fn multiple_questions_stay_on_the_same_step() {
        let first_resume = state(TourStatus::Active, TourStepKey::Support, TourMode::Step);
        let ask_again = component_input(
            TourState {
                claimed_question_message_id: Some("question-1".into()),
                ..first_resume
            },
            TourComponentAction::Ask,
            TourStepKey::Support,
        );
        let waiting = persisted(&decide_tour_component_actions(&ask_again)).clone();
        let actions = decide_tour_dm_message_actions(&TourDmMessageDecisionInput {
            state: waiting,
            channel_id: "123".into(),
            message_id: "1002".into(),
            content: "PLATZHALTER: zweite Frage".into(),
            member_still_in_guild: true,
            now: now(),
        });
        let resume = actions
            .iter()
            .find_map(|action| match action {
                TourAction::AnswerQuestionViaConcierge { resume_state, .. } => {
                    Some(resume_state.as_ref())
                }
                _ => None,
            })
            .expect("second concierge action");
        assert_eq!(resume.step, TourStepKey::Support);
        assert_eq!(resume.claimed_question_message_id.as_deref(), Some("1002"));
    }

    #[test]
    fn expired_question_window_does_not_route_to_the_tour() {
        let waiting = TourState {
            question_deadline_at: Some(now().to_rfc3339()),
            ..state(
                TourStatus::Active,
                TourStepKey::Voice,
                TourMode::WaitingForQuestion,
            )
        };
        let actions = decide_tour_dm_message_actions(&TourDmMessageDecisionInput {
            state: waiting,
            channel_id: "123".into(),
            message_id: "1003".into(),
            content: "PLATZHALTER: verspätete Frage".into(),
            member_still_in_guild: true,
            now: now(),
        });
        assert_eq!(actions.len(), 1);
        assert_eq!(persisted(&actions).mode, TourMode::Step);
        assert!(!actions
            .iter()
            .any(|action| matches!(action, TourAction::AnswerQuestionViaConcierge { .. })));
    }

    #[test]
    fn duplicate_message_id_and_wrong_dm_channel_are_noops() {
        let waiting = TourState {
            claimed_question_message_id: Some("question-1".into()),
            question_deadline_at: Some(
                (now() + Duration::hours(QUESTION_WINDOW_HOURS)).to_rfc3339(),
            ),
            ..state(
                TourStatus::Active,
                TourStepKey::Voice,
                TourMode::WaitingForQuestion,
            )
        };
        for (channel_id, message_id) in [("123", "question-1"), ("999", "question-2")] {
            assert!(decide_tour_dm_message_actions(&TourDmMessageDecisionInput {
                state: waiting.clone(),
                channel_id: channel_id.into(),
                message_id: message_id.into(),
                content: "PLATZHALTER: Frageinhalt".into(),
                member_still_in_guild: true,
                now: now(),
            })
            .is_empty());
        }
    }

    #[test]
    fn persisted_pending_send_completes_with_the_pending_cas_state() {
        fn apply_cas(row: &mut TourState, actions: &[TourAction]) {
            let (expected, next) = actions
                .iter()
                .find_map(|action| match action {
                    TourAction::PersistState { expected, next } => Some((expected, next)),
                    _ => None,
                })
                .expect("persist action");
            assert_eq!(row, expected, "CAS expected the persisted row");
            *row = next.clone();
        }

        let mut row = state(TourStatus::Active, TourStepKey::Voice, TourMode::Step);
        let start_send = decide_tour_component_actions(&component_input(
            row.clone(),
            TourComponentAction::Next,
            TourStepKey::Voice,
        ));
        apply_cas(&mut row, &start_send);
        assert_eq!(row.status, TourStatus::Starting);
        assert_eq!(row.step, TourStepKey::CommunityQuestions);

        let finish_send = decide_tour_send_completion_actions(
            &row,
            &TourSendResult::Sent {
                dm_channel_id: "123".into(),
                message_id: "789".into(),
            },
            now(),
            false,
        );
        apply_cas(&mut row, &finish_send);
        assert_eq!(row.status, TourStatus::Active);
        assert_eq!(row.step, TourStepKey::CommunityQuestions);
    }

    fn waiting_state(step: TourStepKey) -> TourState {
        TourState {
            question_deadline_at: Some(
                (now() + Duration::hours(QUESTION_WINDOW_HOURS)).to_rfc3339(),
            ),
            ..state(TourStatus::Active, step, TourMode::WaitingForQuestion)
        }
    }

    fn dm_input(state: TourState, message_id: &str, content: &str) -> TourDmMessageDecisionInput {
        TourDmMessageDecisionInput {
            state,
            channel_id: "123".into(),
            message_id: message_id.into(),
            content: content.into(),
            member_still_in_guild: true,
            now: now(),
        }
    }

    #[test]
    fn concierge_action_carries_the_question_content() {
        let actions = decide_tour_dm_message_actions(&dm_input(
            waiting_state(TourStepKey::Support),
            "1001",
            "PLATZHALTER: exakter Fragetext",
        ));
        assert!(actions.iter().any(|action| matches!(
            action,
            TourAction::AnswerQuestionViaConcierge { content, .. }
                if content == "PLATZHALTER: exakter Fragetext"
        )));
    }

    #[test]
    fn persisted_due_retry_is_recovered_after_restart() {
        let persisted_rows = [TourState {
            status: TourStatus::Starting,
            step: TourStepKey::Patchnotes,
            retry_count: 1,
            next_retry_at: Some((now() - Duration::seconds(1)).to_rfc3339()),
            ..state(
                TourStatus::Starting,
                TourStepKey::Patchnotes,
                TourMode::Step,
            )
        }];
        let recovered = persisted_rows
            .iter()
            .flat_map(|state| decide_tour_retry_actions(state, now()))
            .collect::<Vec<_>>();
        assert!(matches!(
            recovered.as_slice(),
            [
                TourAction::PersistState { expected, next },
                TourAction::SendStep(TourStepKey::Patchnotes)
            ] if expected.next_retry_at == persisted_rows[0].next_retry_at
                && next.next_retry_at == Some((now() + Duration::seconds(RETRY_DELAY_SECONDS)).to_rfc3339())
        ));
    }

    #[test]
    fn two_retry_workers_claim_exactly_one_send() {
        let mut row = TourState {
            status: TourStatus::Starting,
            step: TourStepKey::Patchnotes,
            retry_count: 1,
            next_retry_at: Some((now() - Duration::seconds(1)).to_rfc3339()),
            ..state(
                TourStatus::Starting,
                TourStepKey::Patchnotes,
                TourMode::Step,
            )
        };
        let workers = [
            decide_tour_retry_actions(&row, now()),
            decide_tour_retry_actions(&row, now()),
        ];
        let mut sends = 0;

        for actions in workers {
            let Some(TourAction::PersistState { expected, next }) = actions.first() else {
                panic!("retry claim must be persisted before send");
            };
            if &row == expected {
                row = next.clone();
                sends += actions
                    .iter()
                    .filter(|action| matches!(action, TourAction::SendStep(_)))
                    .count();
            }
        }

        assert_eq!(sends, 1);
    }

    #[test]
    fn dm_question_aborts_for_departed_member_without_claiming() {
        let mut input = dm_input(
            waiting_state(TourStepKey::CommunityQuestions),
            "1001",
            "PLATZHALTER: Frage",
        );
        input.member_still_in_guild = false;
        let actions = decide_tour_dm_message_actions(&input);
        assert_eq!(persisted(&actions).status, TourStatus::AbortedLeftGuild);
        assert!(actions.contains(&TourAction::Abort(TourAbortReason::LeftGuild)));
        assert!(!actions
            .iter()
            .any(|action| matches!(action, TourAction::AnswerQuestionViaConcierge { .. })));
        assert_eq!(
            persisted(&actions).claimed_question_message_id,
            input.state.claimed_question_message_id
        );
    }

    #[test]
    fn ask_after_expired_question_window_opens_a_fresh_window() {
        let expired = TourState {
            question_deadline_at: Some((now() - Duration::seconds(1)).to_rfc3339()),
            ..state(
                TourStatus::Active,
                TourStepKey::Voice,
                TourMode::WaitingForQuestion,
            )
        };
        let actions = decide_tour_component_actions(&component_input(
            expired.clone(),
            TourComponentAction::Ask,
            TourStepKey::Voice,
        ));
        assert_eq!(persisted(&actions).mode, TourMode::WaitingForQuestion);
        assert_eq!(
            persisted(&actions).question_deadline_at,
            Some((now() + Duration::hours(QUESTION_WINDOW_HOURS)).to_rfc3339())
        );
        assert!(matches!(
            actions.first(),
            Some(TourAction::PersistState { expected, .. }) if expected == &expired
        ));
    }

    #[test]
    fn uncertain_send_from_waiting_state_resets_mode_to_step() {
        let pending = TourState {
            status: TourStatus::Starting,
            step: TourStepKey::RankLink,
            ..waiting_state(TourStepKey::RankLink)
        };
        let actions =
            decide_tour_send_completion_actions(&pending, &TourSendResult::Uncertain, now(), false);
        assert_eq!(persisted(&actions).status, TourStatus::Uncertain);
        assert_eq!(persisted(&actions).mode, TourMode::Step);
    }

    #[test]
    fn question_snowflake_watermark_rejects_older_replay() {
        let first = decide_tour_dm_message_actions(&dm_input(
            waiting_state(TourStepKey::Support),
            "100",
            "PLATZHALTER: Frage eins",
        ));
        let first_resume = first
            .iter()
            .find_map(|action| match action {
                TourAction::AnswerQuestionViaConcierge { resume_state, .. } => {
                    Some(resume_state.as_ref().clone())
                }
                _ => None,
            })
            .expect("first resume");
        let second_waiting = persisted(&decide_tour_component_actions(&component_input(
            first_resume,
            TourComponentAction::Ask,
            TourStepKey::Support,
        )))
        .clone();
        let second = decide_tour_dm_message_actions(&dm_input(
            second_waiting,
            "200",
            "PLATZHALTER: Frage zwei",
        ));
        let second_resume = second
            .iter()
            .find_map(|action| match action {
                TourAction::AnswerQuestionViaConcierge { resume_state, .. } => {
                    Some(resume_state.as_ref().clone())
                }
                _ => None,
            })
            .expect("second resume");
        let replay_waiting = persisted(&decide_tour_component_actions(&component_input(
            second_resume,
            TourComponentAction::Ask,
            TourStepKey::Support,
        )))
        .clone();
        assert!(decide_tour_dm_message_actions(&dm_input(
            replay_waiting,
            "100",
            "PLATZHALTER: Replay Frage eins",
        ))
        .is_empty());
    }

    #[test]
    fn payload_with_three_links_chunks_buttons_into_discord_rows() {
        let links = ["one", "two", "three"].map(|suffix| ResolvedTourLink {
            step: TourStepKey::Voice,
            url: format!("https://example.invalid/{suffix}"),
        });
        let payload = tour_step_payload(TourStepKey::Voice, &links).expect("payload");
        let rows = payload.components.as_array().expect("component rows");
        assert_eq!(rows.len(), 2);
        assert!(rows.len() <= 5);
        assert!(rows.iter().all(|row| row["components"]
            .as_array()
            .is_some_and(|buttons| buttons.len() <= 5)));
    }

    #[test]
    fn decision_boundaries_cover_every_status_and_mode() {
        for (status, actionable) in [
            (TourStatus::Starting, false),
            (TourStatus::Active, true),
            (TourStatus::Completed, false),
            (TourStatus::AbortedByUser, false),
            (TourStatus::AbortedLeftGuild, false),
            (TourStatus::AbortedDmBlocked, false),
            (TourStatus::Uncertain, true),
        ] {
            let actions = decide_tour_component_actions(&component_input(
                state(status, TourStepKey::Voice, TourMode::Step),
                TourComponentAction::Next,
                TourStepKey::Voice,
            ));
            assert_eq!(!actions.is_empty(), actionable, "status {status:?}");
        }
        for (mode, actionable) in [
            (TourMode::Step, true),
            (TourMode::WaitingForQuestion, true),
            (TourMode::Answering, false),
        ] {
            let mut current = state(TourStatus::Active, TourStepKey::Voice, mode);
            if mode == TourMode::WaitingForQuestion {
                current.question_deadline_at = Some((now() + Duration::hours(1)).to_rfc3339());
            } else if mode == TourMode::Answering {
                current.answering_deadline_at = Some((now() + Duration::minutes(1)).to_rfc3339());
            }
            let actions = decide_tour_component_actions(&component_input(
                current,
                TourComponentAction::Next,
                TourStepKey::Voice,
            ));
            assert_eq!(!actions.is_empty(), actionable, "mode {mode:?}");
        }
    }
}
