use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dl_community::concierge::Concierge;
use dl_community::onboarding_tour::{
    build_tour_custom_id, decide_onboarding_tour_actions, decide_tour_component_actions,
    decide_tour_dm_message_actions, decide_tour_retry_actions, decide_tour_send_completion_actions,
    normalize_tour_state, parse_tour_custom_id, tour_step_payload, OnboardingTourDecisionInput,
    OnboardingTourPort, ResolvedTourLink, TourAction, TourClaimResult, TourComponentAction,
    TourComponentDecisionInput, TourDmMessageDecisionInput, TourLinkTarget, TourMemberSnapshot,
    TourMessagePayload, TourMode, TourOnboardingEvent, TourSendResult, TourState, TourStatus,
    TourStepKey, ASK_BUTTON_LABEL, END_BUTTON_LABEL, FINISH_BUTTON_LABEL, NEXT_BUTTON_LABEL,
    TOUR_STEPS,
};
use dl_discord::{
    BridgeInteraction, BridgeReply, DiscordAdapter, Dispatcher, InteractionHandler,
    InteractionRouter, MessageEvent,
};
use reqwest::StatusCode;
use serde_json::{json, Map, Value};
use serenity::all::{GuildId, RoleId, UserId};
use serenity::http::HttpError;
use sqlx::PgPool;

const TOUR_OPT_IN_ROLE_ID_ENV: &str = "TOUR_OPT_IN_ROLE_ID";
const TOUR_STATE_NS: &str = "onboarding_tour";
const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

pub struct OnboardingTourGlue {
    adapter: Arc<DiscordAdapter>,
    pool: PgPool,
    concierge: Arc<Concierge>,
    http: reqwest::Client,
    discord_token: String,
    discord_api_base: String,
}

#[derive(Clone)]
pub struct OnboardingTourRuntime {
    glue: Arc<OnboardingTourGlue>,
    main_guild_id: u64,
    marker_role_id: u64,
}

impl OnboardingTourGlue {
    pub fn new(
        adapter: Arc<DiscordAdapter>,
        pool: PgPool,
        concierge: Arc<Concierge>,
        discord_token: String,
    ) -> Result<Self, String> {
        Ok(Self {
            adapter,
            pool,
            concierge,
            http: reqwest::Client::builder()
                .timeout(StdDuration::from_secs(15))
                .user_agent("DiscordBot (Deadlock-Bots dl-bot, 0.1.0)")
                .build()
                .map_err(|error| error.to_string())?,
            discord_token,
            discord_api_base: DISCORD_API_BASE.into(),
        })
    }

    async fn load_state(&self, user_id: u64) -> Result<Option<TourState>, String> {
        load_state_db(&self.pool, user_id).await
    }

    async fn load_rang_guide_message_id(&self) -> Result<Option<u64>, String> {
        let value = sqlx::query_scalar::<_, String>(
            "SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2 LIMIT 1",
        )
        .bind(crate::serversync::serversync_kv_ns())
        .bind(crate::serversync::rang_guide_message_id_key(0))
        .fetch_optional(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        value
            .map(|value| value.parse::<u64>().map_err(|error| error.to_string()))
            .transpose()
    }
}

impl OnboardingTourRuntime {
    pub fn new(glue: OnboardingTourGlue, main_guild_id: u64, marker_role_id: u64) -> Arc<Self> {
        Arc::new(Self {
            glue: Arc::new(glue),
            main_guild_id,
            marker_role_id,
        })
    }

    async fn handle_native_onboarding_completed(&self, guild_id: u64, user_id: u64) {
        let snapshot = match self.glue.load_member_snapshot(guild_id, user_id).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    user_id,
                    step = "voice",
                    action = "load_member",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        let now = Utc::now();
        let base = OnboardingTourDecisionInput {
            event: TourOnboardingEvent { guild_id, user_id },
            main_guild_id: self.main_guild_id,
            is_bot: snapshot.is_bot,
            has_tour_marker: snapshot.role_ids.contains(&self.marker_role_id),
            has_rank_marker: false,
            claim_result: None,
            send_result: None,
            now,
        };
        let initial_actions = decide_onboarding_tour_actions(&base);
        let Some(TourAction::ClaimStart(initial)) = initial_actions.first() else {
            self.log_decision(user_id, TourStepKey::Voice, "native_event", "ignored");
            return;
        };
        let claim = match self.glue.claim_or_load_state(user_id, initial).await {
            Ok(claim) => claim,
            Err(error) => {
                tracing::warn!(
                    user_id,
                    step = "voice",
                    action = "claim",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        let current = match &claim {
            TourClaimResult::Acquired(state) | TourClaimResult::Existing(state) => state.clone(),
        };
        let actions = decide_onboarding_tour_actions(&OnboardingTourDecisionInput {
            claim_result: Some(claim),
            ..base
        });
        self.execute_actions(user_id, current, actions, true).await;
    }

    async fn handle_component(&self, interaction: &BridgeInteraction) {
        let custom_id = match parse_tour_custom_id(&interaction.custom_id) {
            Ok(custom_id) => custom_id,
            Err(error) => {
                tracing::warn!(
                    user_id = interaction.user_id,
                    step = "unknown",
                    action = "component_parse",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        let state = match self.glue.load_state(interaction.user_id).await {
            Ok(Some(state)) => state,
            Ok(None) => {
                self.log_decision(
                    interaction.user_id,
                    custom_id.step,
                    "component",
                    "state_missing",
                );
                return;
            }
            Err(error) => {
                tracing::warn!(
                    user_id = interaction.user_id,
                    step = custom_id.step.as_str(),
                    action = "component_load",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        let member_still_in_guild = match self
            .glue
            .member_still_in_guild(self.main_guild_id, interaction.user_id)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    user_id = interaction.user_id,
                    step = custom_id.step.as_str(),
                    action = "component_member",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        let actions = decide_tour_component_actions(&TourComponentDecisionInput {
            state: state.clone(),
            action: custom_id.action,
            step: custom_id.step,
            member_still_in_guild,
            now: Utc::now(),
        });
        self.log_decision(
            interaction.user_id,
            custom_id.step,
            "component_decision",
            if actions.is_empty() {
                "ignored"
            } else {
                "planned"
            },
        );
        self.execute_actions(interaction.user_id, state, actions, false)
            .await;
    }

    async fn handle_message(&self, event: &MessageEvent) {
        if event.guild_id.is_some() {
            self.glue.concierge.handle_routed_message(event).await;
            return;
        }
        let mut state = match self.glue.load_state(event.author_id).await {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(
                    user_id = event.author_id,
                    step = "unknown",
                    action = "dm_state_load",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                self.glue.concierge.handle_routed_message(event).await;
                return;
            }
        };
        let now = Utc::now();
        if let Some(current) = state {
            state = match normalize_or_reload_message_state_db(
                &self.glue.pool,
                event.author_id,
                current,
                now,
            )
            .await
            {
                Ok(state) => state,
                Err(error) => {
                    tracing::warn!(
                        user_id = event.author_id,
                        step = "unknown",
                        action = "dm_normalize",
                        result = %error,
                        "Onboarding-Tour Zustandsentscheidung"
                    );
                    self.glue.concierge.handle_routed_message(event).await;
                    return;
                }
            };
        }
        match classify_message_route(
            Some(self.marker_role_id),
            event.guild_id,
            state.as_ref(),
            now,
        ) {
            TourMessageRoute::TourDm => {}
            TourMessageRoute::GuildMessage
            | TourMessageRoute::ConciergeDm
            | TourMessageRoute::Disabled => {
                self.glue.concierge.handle_routed_message(event).await;
                return;
            }
        }
        let Some(state) = state else {
            return;
        };
        let member_still_in_guild = match self
            .glue
            .member_still_in_guild(self.main_guild_id, event.author_id)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    user_id = event.author_id,
                    step = state.step.as_str(),
                    action = "dm_member",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        let actions = decide_tour_dm_message_actions(&TourDmMessageDecisionInput {
            state: state.clone(),
            channel_id: event.channel_id.to_string(),
            message_id: event.message_id.to_string(),
            content: event.content.clone(),
            member_still_in_guild,
            now: Utc::now(),
        });
        self.log_decision(
            event.author_id,
            state.step,
            "dm_decision",
            if actions.is_empty() {
                "ignored"
            } else {
                "planned"
            },
        );
        self.execute_actions(event.author_id, state, actions, false)
            .await;
    }

    async fn recover_due_retries(&self) {
        let now = Utc::now();
        let due = match self.glue.load_due_retries(now).await {
            Ok(due) => due,
            Err(error) => {
                tracing::warn!(
                    user_id = 0,
                    step = "unknown",
                    action = "load_retries",
                    result = %error,
                    "Onboarding-Tour Zustandsentscheidung"
                );
                return;
            }
        };
        for (user_id, state) in due {
            let actions = decide_tour_retry_actions(&state, now);
            self.log_decision(
                user_id,
                state.step,
                "retry_decision",
                if actions.is_empty() {
                    "ignored"
                } else {
                    "planned"
                },
            );
            self.execute_actions(user_id, state, actions, true).await;
        }
    }

    async fn execute_actions(
        &self,
        user_id: u64,
        mut current: TourState,
        actions: Vec<TourAction>,
        remove_marker_after_send: bool,
    ) {
        let mut queue = VecDeque::from(actions);
        while let Some(action) = queue.pop_front() {
            let step = current.step;
            match action {
                TourAction::PersistState { expected, next } => {
                    match self
                        .glue
                        .transition_state_locked(user_id, &expected, &next)
                        .await
                    {
                        Ok(true) => {
                            current = next;
                            self.log_decision(user_id, step, "persist", "updated");
                        }
                        Ok(false) => {
                            self.log_decision(user_id, step, "persist", "stale");
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(
                                user_id,
                                step = step.as_str(),
                                action = "persist",
                                result = %error,
                                "Onboarding-Tour Zustandsentscheidung"
                            );
                            return;
                        }
                    }
                }
                TourAction::SendStep(target_step) => {
                    let links = match self
                        .glue
                        .resolve_step_links(self.main_guild_id, target_step)
                        .await
                    {
                        Ok(links) => links,
                        Err(error) => {
                            tracing::warn!(
                                user_id,
                                step = target_step.as_str(),
                                action = "resolve_links",
                                result = %error,
                                "Onboarding-Tour Deploy-Blocker"
                            );
                            return;
                        }
                    };
                    let payload = match tour_step_payload(target_step, &links) {
                        Ok(payload) => payload,
                        Err(error) => {
                            tracing::warn!(
                                user_id,
                                step = target_step.as_str(),
                                action = "build_payload",
                                result = %error,
                                "Onboarding-Tour Zustandsentscheidung"
                            );
                            return;
                        }
                    };
                    let result = self.glue.send_tour_message(user_id, payload).await;
                    self.log_decision(user_id, target_step, "send", send_result_name(&result));
                    let follow_up = decide_tour_send_completion_actions(
                        &current,
                        &result,
                        Utc::now(),
                        remove_marker_after_send,
                    );
                    for action in follow_up.into_iter().rev() {
                        queue.push_front(action);
                    }
                }
                TourAction::AnswerQuestionViaConcierge {
                    channel_id,
                    content,
                    step,
                    resume_state,
                } => {
                    if let Err(error) = self
                        .glue
                        .answer_question_via_concierge(
                            user_id,
                            &channel_id,
                            &content,
                            step,
                            &current,
                            &resume_state,
                        )
                        .await
                    {
                        tracing::warn!(
                            user_id,
                            step = step.as_str(),
                            action = "answer_question",
                            result = %error,
                            "Onboarding-Tour Zustandsentscheidung"
                        );
                        return;
                    }
                    current = *resume_state;
                }
                TourAction::RemoveMarker => {
                    match self
                        .glue
                        .remove_marker_role(self.main_guild_id, user_id, self.marker_role_id)
                        .await
                    {
                        Ok(()) => self.log_decision(user_id, step, "remove_marker", "removed"),
                        Err(error) => tracing::warn!(
                            user_id,
                            step = step.as_str(),
                            action = "remove_marker",
                            result = %error,
                            "Onboarding-Tour Zustandsentscheidung"
                        ),
                    }
                }
                TourAction::OpenQuestionWindow(_) => {
                    self.log_decision(user_id, step, "open_question", "waiting")
                }
                TourAction::ScheduleRetry(_) => {
                    self.log_decision(user_id, step, "retry", "scheduled")
                }
                TourAction::MarkUncertain => self.log_decision(user_id, step, "send", "uncertain"),
                TourAction::Complete => self.log_decision(user_id, step, "complete", "completed"),
                TourAction::Abort(reason) => self.log_decision(
                    user_id,
                    step,
                    "abort",
                    match reason {
                        dl_community::onboarding_tour::TourAbortReason::User => "user",
                        dl_community::onboarding_tour::TourAbortReason::LeftGuild => "left_guild",
                        dl_community::onboarding_tour::TourAbortReason::DmBlocked => "dm_blocked",
                    },
                ),
                TourAction::ClaimStart(_) => {
                    self.log_decision(user_id, step, "claim", "unexpected")
                }
            }
        }
    }

    fn log_decision(&self, user_id: u64, step: TourStepKey, action: &str, result: &str) {
        tracing::info!(
            user_id,
            step = step.as_str(),
            action,
            result,
            "Onboarding-Tour Zustandsentscheidung"
        );
    }
}

struct TourComponentHandler {
    runtime: Option<Arc<OnboardingTourRuntime>>,
}

#[async_trait]
impl InteractionHandler for TourComponentHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if let Some(runtime) = &self.runtime {
            runtime.handle_component(&interaction).await;
        }
        BridgeReply {
            update_message: true,
            ..BridgeReply::default()
        }
    }
}

pub fn register(router: &mut InteractionRouter, runtime: Option<Arc<OnboardingTourRuntime>>) {
    router.on_prefix("tour:v1:", Arc::new(TourComponentHandler { runtime }));
}

pub fn spawn_if_enabled(
    runtime: Option<Arc<OnboardingTourRuntime>>,
    dispatcher: &Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    runtime.map_or_else(Vec::new, |runtime| spawn(runtime, dispatcher))
}

pub fn spawn(
    runtime: Arc<OnboardingTourRuntime>,
    dispatcher: &Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::new();
    let mut members = dispatcher.subscribe_members();
    let member_runtime = runtime.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match members.recv().await {
                Ok(dl_discord::MemberEvent::NativeOnboardingCompleted { guild_id, user_id }) => {
                    member_runtime
                        .handle_native_onboarding_completed(guild_id, user_id)
                        .await;
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Onboarding-Tour Member-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let mut messages = dispatcher.subscribe_messages();
    let message_runtime = runtime.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => message_runtime.handle_message(&event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Onboarding-Tour Message-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    handles.push(tokio::spawn(async move {
        let mut interval = tokio::time::interval(StdDuration::from_secs(60));
        loop {
            interval.tick().await;
            runtime.recover_due_retries().await;
        }
    }));
    handles
}

fn send_result_name(result: &TourSendResult) -> &'static str {
    match result {
        TourSendResult::Sent { .. } => "sent",
        TourSendResult::CannotSend50007 => "cannot_send_50007",
        TourSendResult::Transient => "transient",
        TourSendResult::Uncertain => "uncertain",
    }
}

pub fn tour_opt_in_role_id_from_lookup<F>(lookup: F) -> Option<u64>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(TOUR_OPT_IN_ROLE_ID_ENV)
        .and_then(|value| value.trim().parse().ok())
        .filter(|role_id| *role_id != 0)
}

fn channel_url(guild_id: u64, channel_id: u64) -> Result<String, String> {
    // Welle 3 muss die echte Patchnotes-ID setzen, bevor TOUR_OPT_IN_ROLE_ID
    // aktiviert wird. Ein stilles Ueberspringen wuerde den Tourvertrag brechen.
    if channel_id == 0 {
        return Err(
            "Tour-Kanal-ID ist 0; Welle 3 muss sie vor Aktivierung des Cutover-Flags setzen".into(),
        );
    }
    Ok(format!(
        "https://discord.com/channels/{guild_id}/{channel_id}"
    ))
}

fn rank_guide_url(guild_id: u64, channel_id: u64, message_id: Option<u64>) -> String {
    match message_id {
        Some(message_id) => {
            format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
        }
        None => format!("https://discord.com/channels/{guild_id}/{channel_id}"),
    }
}

fn question_response_components(step: TourStepKey) -> Value {
    let (progress, label) = if step == TourStepKey::Streamers {
        (TourComponentAction::Finish, FINISH_BUTTON_LABEL)
    } else {
        (TourComponentAction::Next, NEXT_BUTTON_LABEL)
    };
    json!([{ "type": 1, "components": [
        {
            "type": 2,
            "style": 2,
            "label": label,
            "custom_id": build_tour_custom_id(progress, step),
        },
        {
            "type": 2,
            "style": 2,
            "label": ASK_BUTTON_LABEL,
            "custom_id": build_tour_custom_id(TourComponentAction::Ask, step),
        },
        {
            "type": 2,
            "style": 2,
            "label": END_BUTTON_LABEL,
            "custom_id": build_tour_custom_id(TourComponentAction::End, step),
        }
    ]}])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TourMessageRoute {
    Disabled,
    GuildMessage,
    TourDm,
    ConciergeDm,
}

fn classify_message_route(
    role_id: Option<u64>,
    guild_id: Option<u64>,
    state: Option<&TourState>,
    now: DateTime<Utc>,
) -> TourMessageRoute {
    if role_id.is_none() {
        return TourMessageRoute::Disabled;
    }
    if guild_id.is_some() {
        return TourMessageRoute::GuildMessage;
    }
    if state.is_some_and(|state| {
        let state = normalize_tour_state(state, now);
        state.status == TourStatus::Active && state.mode == TourMode::WaitingForQuestion
    }) {
        TourMessageRoute::TourDm
    } else {
        TourMessageRoute::ConciergeDm
    }
}

async fn load_state_db(pool: &PgPool, user_id: u64) -> Result<Option<TourState>, String> {
    let value = sqlx::query_scalar::<_, String>(
        "SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2 LIMIT 1",
    )
    .bind(TOUR_STATE_NS)
    .bind(user_id.to_string())
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;
    value
        .map(|value| serde_json::from_str(&value).map_err(|error| error.to_string()))
        .transpose()
}

async fn resume_question_state_db(
    pool: &PgPool,
    user_id: u64,
    answering_state: &TourState,
    resume_state: &TourState,
) -> Result<(), String> {
    if answering_state.mode != TourMode::Answering
        || answering_state.claimed_question_message_id.is_none()
    {
        return Err("Tour-Frage hat keinen gültigen Answering-Claim".into());
    }
    if !transition_state_locked_db(pool, user_id, answering_state, resume_state).await? {
        return Err("Tour-State nach Concierge-Antwort war nicht mehr aktuell".into());
    }
    Ok(())
}

async fn normalize_or_reload_message_state_db(
    pool: &PgPool,
    user_id: u64,
    initial: TourState,
    now: DateTime<Utc>,
) -> Result<Option<TourState>, String> {
    let mut current = initial;
    for attempt in 0..=1 {
        let normalized = normalize_tour_state(&current, now);
        if normalized == current {
            return Ok(Some(current));
        }
        if transition_state_locked_db(pool, user_id, &current, &normalized).await? {
            return Ok(Some(normalized));
        }
        if attempt == 1 {
            return Ok(None);
        }
        let Some(reloaded) = load_state_db(pool, user_id).await? else {
            return Ok(None);
        };
        current = reloaded;
    }
    Ok(None)
}

async fn claim_or_load_state_db(
    pool: &PgPool,
    user_id: u64,
    initial: &TourState,
) -> Result<TourClaimResult, String> {
    let initial_json = serde_json::to_string(initial).map_err(|error| error.to_string())?;
    let inserted = sqlx::query(
        "INSERT INTO bot.kv_store(ns, k, v) VALUES($1, $2, $3)
         ON CONFLICT(ns, k) DO NOTHING",
    )
    .bind(TOUR_STATE_NS)
    .bind(user_id.to_string())
    .bind(&initial_json)
    .execute(pool)
    .await
    .map_err(|error| error.to_string())?
    .rows_affected()
        > 0;
    let state = load_state_db(pool, user_id)
        .await?
        .ok_or_else(|| "Tour-State fehlt direkt nach Claim".to_string())?;
    Ok(if inserted {
        TourClaimResult::Acquired(state)
    } else {
        TourClaimResult::Existing(state)
    })
}

async fn transition_state_locked_db(
    pool: &PgPool,
    user_id: u64,
    expected: &TourState,
    next: &TourState,
) -> Result<bool, String> {
    let next_json = serde_json::to_string(next).map_err(|error| error.to_string())?;
    let mut transaction = pool.begin().await.map_err(|error| error.to_string())?;
    let current = sqlx::query_scalar::<_, String>(
        "SELECT v FROM bot.kv_store WHERE ns = $1 AND k = $2 FOR UPDATE",
    )
    .bind(TOUR_STATE_NS)
    .bind(user_id.to_string())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| error.to_string())?;
    let current = current
        .map(|value| serde_json::from_str::<TourState>(&value).map_err(|error| error.to_string()))
        .transpose()?;
    if current.as_ref() != Some(expected) {
        transaction
            .rollback()
            .await
            .map_err(|error| error.to_string())?;
        return Ok(false);
    }
    sqlx::query("UPDATE bot.kv_store SET v = $3 WHERE ns = $1 AND k = $2")
        .bind(TOUR_STATE_NS)
        .bind(user_id.to_string())
        .bind(next_json)
        .execute(&mut *transaction)
        .await
        .map_err(|error| error.to_string())?;
    transaction
        .commit()
        .await
        .map_err(|error| error.to_string())?;
    Ok(true)
}

async fn load_due_retries_db(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Vec<(u64, TourState)>, String> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT k, v
           FROM bot.kv_store
          WHERE ns = $1
            AND v::jsonb->>'status' = 'starting'
            AND NULLIF(v::jsonb->>'next_retry_at', '')::timestamptz <= $2",
    )
    .bind(TOUR_STATE_NS)
    .bind(now)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|(user_id, value)| {
            Ok((
                user_id.parse::<u64>().map_err(|error| error.to_string())?,
                serde_json::from_str(&value).map_err(|error| error.to_string())?,
            ))
        })
        .collect()
}

async fn send_tour_message_http(
    client: &reqwest::Client,
    api_base: &str,
    token: &str,
    user_id: u64,
    payload: TourMessagePayload,
) -> TourSendResult {
    let authorization = format!("Bot {token}");
    let channel_response = match client
        .post(format!("{api_base}/users/@me/channels"))
        .header(reqwest::header::AUTHORIZATION, &authorization)
        .json(&json!({ "recipient_id": user_id.to_string() }))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return TourSendResult::Transient,
    };
    let channel_status = channel_response.status();
    let channel_body = channel_response
        .json::<Value>()
        .await
        .unwrap_or(Value::Null);
    if !channel_status.is_success() {
        return classify_discord_response(channel_status, &channel_body);
    }
    let Some(dm_channel_id) = channel_body.get("id").and_then(Value::as_str) else {
        return TourSendResult::Transient;
    };

    let mut body = Map::new();
    body.insert("content".into(), json!(payload.content));
    body.insert("components".into(), payload.components);
    body.insert("allowed_mentions".into(), json!({ "parse": [] }));
    let message_response = match client
        .post(format!("{api_base}/channels/{dm_channel_id}/messages"))
        .header(reqwest::header::AUTHORIZATION, authorization)
        .json(&body)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return TourSendResult::Uncertain,
        Err(_) => return TourSendResult::Transient,
    };
    let message_status = message_response.status();
    let message_body = message_response
        .json::<Value>()
        .await
        .unwrap_or(Value::Null);
    if !message_status.is_success() {
        return classify_discord_response(message_status, &message_body);
    }
    match message_body.get("id").and_then(Value::as_str) {
        Some(message_id) => TourSendResult::Sent {
            dm_channel_id: dm_channel_id.to_string(),
            message_id: message_id.to_string(),
        },
        None => TourSendResult::Uncertain,
    }
}

fn classify_discord_response(_status: StatusCode, body: &Value) -> TourSendResult {
    if body.get("code").and_then(Value::as_u64) == Some(50_007) {
        TourSendResult::CannotSend50007
    } else {
        TourSendResult::Transient
    }
}

#[async_trait]
impl OnboardingTourPort for OnboardingTourGlue {
    async fn load_member_snapshot(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<TourMemberSnapshot, String> {
        if let Some(guild) = self.adapter.cache().guild(GuildId::new(guild_id)) {
            if let Some(member) = guild.members.get(&UserId::new(user_id)) {
                return Ok(TourMemberSnapshot {
                    is_bot: member.user.bot,
                    role_ids: member.roles.iter().map(|role| role.get()).collect(),
                });
            }
        }
        let member = self
            .adapter
            .http
            .get_member(GuildId::new(guild_id), UserId::new(user_id))
            .await
            .map_err(|error| error.to_string())?;
        Ok(TourMemberSnapshot {
            is_bot: member.user.bot,
            role_ids: member.roles.iter().map(|role| role.get()).collect(),
        })
    }

    async fn claim_or_load_state(
        &self,
        user_id: u64,
        initial: &TourState,
    ) -> Result<TourClaimResult, String> {
        claim_or_load_state_db(&self.pool, user_id, initial).await
    }

    async fn load_due_retries(&self, now: DateTime<Utc>) -> Result<Vec<(u64, TourState)>, String> {
        load_due_retries_db(&self.pool, now).await
    }

    async fn transition_state_locked(
        &self,
        user_id: u64,
        expected: &TourState,
        next: &TourState,
    ) -> Result<bool, String> {
        transition_state_locked_db(&self.pool, user_id, expected, next).await
    }

    async fn resolve_step_links(
        &self,
        guild_id: u64,
        step: TourStepKey,
    ) -> Result<Vec<ResolvedTourLink>, String> {
        let target = TOUR_STEPS
            .iter()
            .find(|candidate| candidate.key == step)
            .ok_or_else(|| format!("unbekannter Tour-Step {step}"))?
            .link_target;
        let url = match target {
            TourLinkTarget::Channel(channel_id) => channel_url(guild_id, channel_id)?,
            TourLinkTarget::RankGuide => {
                let channel_id = crate::serversync::rang_guide_channel_id();
                rank_guide_url(
                    guild_id,
                    channel_id,
                    self.load_rang_guide_message_id().await?,
                )
            }
        };
        Ok(vec![ResolvedTourLink { step, url }])
    }

    async fn send_tour_message(&self, user_id: u64, payload: TourMessagePayload) -> TourSendResult {
        send_tour_message_http(
            &self.http,
            &self.discord_api_base,
            &self.discord_token,
            user_id,
            payload,
        )
        .await
    }

    async fn member_still_in_guild(&self, guild_id: u64, user_id: u64) -> Result<bool, String> {
        match self
            .adapter
            .http
            .get_member(GuildId::new(guild_id), UserId::new(user_id))
            .await
        {
            Ok(_) => Ok(true),
            Err(serenity::Error::Http(HttpError::UnsuccessfulRequest(response)))
                if response.status_code == StatusCode::NOT_FOUND =>
            {
                Ok(false)
            }
            Err(error) => Err(error.to_string()),
        }
    }

    async fn remove_marker_role(
        &self,
        guild_id: u64,
        user_id: u64,
        role_id: u64,
    ) -> Result<(), String> {
        self.adapter
            .http
            .remove_member_role(
                GuildId::new(guild_id),
                UserId::new(user_id),
                RoleId::new(role_id),
                Some("Onboarding-Tour: Marker nach Startversuch entfernen"),
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn answer_question_via_concierge(
        &self,
        user_id: u64,
        channel_id: &str,
        content: &str,
        step: TourStepKey,
        answering_state: &TourState,
        resume_state: &TourState,
    ) -> Result<(), String> {
        let channel_id = channel_id
            .parse::<u64>()
            .map_err(|error| error.to_string())?;
        let concierge = self.concierge.clone();
        let content = content.to_string();
        let mut task = tokio::spawn(async move {
            concierge
                .answer_tour_dm_question(
                    channel_id,
                    user_id,
                    &content,
                    step.as_str(),
                    question_response_components(step),
                )
                .await
        });
        let answer = await_join_with_timeout(&mut task, StdDuration::from_secs(5 * 60)).await;
        match &answer {
            Ok(outcome) => tracing::info!(
                user_id,
                step = step.as_str(),
                action = "answer_question",
                result = ?outcome,
                "Onboarding-Tour Zustandsentscheidung"
            ),
            Err(error) => tracing::warn!(
                user_id,
                step = step.as_str(),
                action = "answer_question",
                result = %error,
                "Onboarding-Tour Zustandsentscheidung"
            ),
        }
        resume_question_state_db(&self.pool, user_id, answering_state, resume_state).await?;
        answer.map(|_| ())
    }
}

async fn await_join_with_timeout<T>(
    task: &mut tokio::task::JoinHandle<T>,
    limit: StdDuration,
) -> Result<T, String> {
    match tokio::time::timeout(limit, &mut *task).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(format!("Concierge-Task fehlgeschlagen: {error}")),
        Err(_) => {
            task.abort();
            let _ = (&mut *task).await;
            Err("Concierge-Antwort hat das Zeitlimit überschritten".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration as StdDuration;

    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use chrono::{DateTime, Utc};
    use dl_community::onboarding_tour::{
        TourClaimResult, TourMessagePayload, TourMode, TourSendResult, TourState, TourStatus,
        TourStepKey, TOUR_STATE_SCHEMA,
    };
    use serde_json::{json, Value};

    use super::*;

    fn now() -> DateTime<Utc> {
        "2026-07-15T12:00:00Z".parse().expect("valid test time")
    }

    fn waiting_state() -> TourState {
        TourState {
            schema: TOUR_STATE_SCHEMA,
            status: TourStatus::Active,
            step: TourStepKey::Voice,
            mode: TourMode::WaitingForQuestion,
            dm_channel_id: Some("10".into()),
            last_tour_message_id: Some("20".into()),
            claimed_question_message_id: None,
            question_deadline_at: Some("2026-07-16T12:00:00Z".into()),
            answering_deadline_at: None,
            retry_count: 0,
            next_retry_at: None,
            updated_at: now().to_rfc3339(),
        }
    }

    #[derive(Clone)]
    struct MockDiscord {
        message_status: StatusCode,
        message_body: Value,
        message_delay: StdDuration,
        requests: Arc<Mutex<Vec<Value>>>,
    }

    async fn open_dm() -> (StatusCode, Json<Value>) {
        (StatusCode::OK, Json(json!({ "id": "55" })))
    }

    async fn post_message(
        State(state): State<MockDiscord>,
        Json(body): Json<Value>,
    ) -> (StatusCode, Json<Value>) {
        state.requests.lock().expect("requests").push(body);
        tokio::time::sleep(state.message_delay).await;
        (state.message_status, Json(state.message_body))
    }

    async fn mock_discord(
        message_status: StatusCode,
        message_body: Value,
        message_delay: StdDuration,
    ) -> (String, Arc<Mutex<Vec<Value>>>, tokio::task::JoinHandle<()>) {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route("/users/@me/channels", post(open_dm))
            .route("/channels/{channel_id}/messages", post(post_message))
            .with_state(MockDiscord {
                message_status,
                message_body,
                message_delay,
                requests: requests.clone(),
            });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock discord");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("mock discord server");
        });
        (base_url, requests, task)
    }

    fn payload() -> TourMessagePayload {
        TourMessagePayload {
            content: "PLATZHALTER: Testnachricht".into(),
            components: json!([]),
        }
    }

    #[tokio::test]
    async fn flag_aus_ackt_button_und_startet_keinen_event_dm_retry_oder_io_pfad() {
        for raw in [None, Some(""), Some("0"), Some("ungueltig")] {
            assert_eq!(
                tour_opt_in_role_id_from_lookup(|_| raw.map(str::to_owned)),
                None
            );
        }
        assert_eq!(
            tour_opt_in_role_id_from_lookup(|_| Some("123".into())),
            Some(123)
        );

        let mut router = InteractionRouter::new();
        register(&mut router, None);
        let handler = router
            .resolve_component("tour:v1:next:voice")
            .expect("disabled tour prefix remains registered");
        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tour:v1:next:voice".into(),
                ..BridgeInteraction::default()
            })
            .await;
        assert!(reply.update_message, "stale button must be acknowledged");

        let dispatcher = Dispatcher::new();
        assert!(spawn_if_enabled(None, &dispatcher).is_empty());
    }

    #[tokio::test]
    async fn concierge_join_panic_and_timeout_are_reported_without_hanging() {
        let mut panic_task = tokio::spawn(async { panic!("test panic") });
        assert!(
            await_join_with_timeout(&mut panic_task, StdDuration::from_secs(1))
                .await
                .expect_err("panic must become an error")
                .contains("panic")
        );

        let mut hanging_task = tokio::spawn(std::future::pending::<()>());
        assert!(
            await_join_with_timeout(&mut hanging_task, StdDuration::from_millis(1))
                .await
                .expect_err("timeout must become an error")
                .contains("Zeitlimit")
        );
        assert!(hanging_task.is_finished());
    }

    #[test]
    fn channel_url_enthaelt_guild_und_channel_und_null_ist_deploy_blocker() {
        assert_eq!(
            channel_url(1, 2).expect("valid channel"),
            "https://discord.com/channels/1/2"
        );
        assert!(channel_url(1, 0)
            .expect_err("zero channel must fail")
            .contains("Welle 3"));
    }

    #[test]
    fn rang_guide_url_nutzt_message_id_und_faellt_auf_kanal_zurueck() {
        assert_eq!(
            rank_guide_url(1, 2, Some(3)),
            "https://discord.com/channels/1/2/3"
        );
        assert_eq!(
            rank_guide_url(1, 2, None),
            "https://discord.com/channels/1/2"
        );
    }

    #[test]
    fn concierge_response_hat_nur_aktionsbuttons_des_aktuellen_steps() {
        let components = question_response_components(TourStepKey::Voice);
        let buttons = components[0]["components"]
            .as_array()
            .expect("action row buttons");
        assert_eq!(buttons.len(), 3);
        assert_eq!(buttons[0]["custom_id"], "tour:v1:next:voice");
        assert_eq!(buttons[1]["custom_id"], "tour:v1:ask:voice");
        assert_eq!(buttons[2]["custom_id"], "tour:v1:end:voice");
        assert!(buttons.iter().all(|button| button.get("url").is_none()));
    }

    #[test]
    fn dispatch_routet_tour_dm_vor_concierge_aber_nie_guild_messages() {
        let state = waiting_state();
        assert_eq!(
            classify_message_route(Some(1), Some(999), Some(&state), now()),
            TourMessageRoute::GuildMessage
        );
        assert_eq!(
            classify_message_route(Some(1), None, Some(&state), now()),
            TourMessageRoute::TourDm
        );
        assert_eq!(
            classify_message_route(Some(1), None, None, now()),
            TourMessageRoute::ConciergeDm
        );
        assert_eq!(
            classify_message_route(None, None, Some(&state), now()),
            TourMessageRoute::Disabled
        );
        let mut expired = state;
        expired.question_deadline_at = Some("2026-07-15T11:59:59Z".into());
        assert_eq!(
            classify_message_route(Some(1), None, Some(&expired), now()),
            TourMessageRoute::ConciergeDm
        );
    }

    #[tokio::test]
    async fn dm_http_erfolg_sendet_allowed_mentions_ohne_parse() {
        let (base_url, requests, server) =
            mock_discord(StatusCode::OK, json!({ "id": "66" }), StdDuration::ZERO).await;
        let result = send_tour_message_http(
            &reqwest::Client::new(),
            &base_url,
            "test-token",
            42,
            payload(),
        )
        .await;
        server.abort();

        assert_eq!(
            result,
            TourSendResult::Sent {
                dm_channel_id: "55".into(),
                message_id: "66".into(),
            }
        );
        assert_eq!(
            requests.lock().expect("requests")[0]["allowed_mentions"],
            json!({
                "parse": []
            })
        );
    }

    #[tokio::test]
    async fn dm_http_klassifiziert_50007_und_5xx() {
        for (status, body, expected) in [
            (
                StatusCode::FORBIDDEN,
                json!({ "code": 50007, "message": "cannot send" }),
                TourSendResult::CannotSend50007,
            ),
            (
                StatusCode::BAD_GATEWAY,
                json!({ "message": "bad gateway" }),
                TourSendResult::Transient,
            ),
        ] {
            let (base_url, _, server) = mock_discord(status, body, StdDuration::ZERO).await;
            let actual = send_tour_message_http(
                &reqwest::Client::new(),
                &base_url,
                "test-token",
                42,
                payload(),
            )
            .await;
            server.abort();
            assert_eq!(actual, expected);
        }
    }

    #[tokio::test]
    async fn timeout_nach_message_post_ist_uncertain() {
        let (base_url, _, server) = mock_discord(
            StatusCode::OK,
            json!({ "id": "66" }),
            StdDuration::from_millis(100),
        )
        .await;
        let client = reqwest::Client::builder()
            .timeout(StdDuration::from_millis(20))
            .build()
            .expect("client");
        let actual = send_tour_message_http(&client, &base_url, "test-token", 42, payload()).await;
        server.abort();
        assert_eq!(actual, TourSendResult::Uncertain);
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn db_cas_ist_semantisch_stale_dm_wird_neu_geroutet_und_answering_resetet(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let user_id = 42;
        let initial = TourState::starting(now());

        assert_eq!(
            claim_or_load_state_db(pool, user_id, &initial).await?,
            TourClaimResult::Acquired(initial.clone())
        );
        assert_eq!(
            claim_or_load_state_db(pool, user_id, &initial).await?,
            TourClaimResult::Existing(initial.clone())
        );

        let mut due = initial.clone();
        due.retry_count = 1;
        due.next_retry_at = Some((now() - chrono::Duration::seconds(1)).to_rfc3339());
        assert!(transition_state_locked_db(pool, user_id, &initial, &due).await?);
        assert!(!transition_state_locked_db(pool, user_id, &initial, &due).await?);
        assert_eq!(
            load_due_retries_db(pool, now()).await?,
            vec![(user_id, due.clone())]
        );

        sqlx::query("UPDATE bot.kv_store SET v = $3 WHERE ns = $1 AND k = $2")
            .bind(TOUR_STATE_NS)
            .bind(user_id.to_string())
            .bind(serde_json::to_string_pretty(&due)?)
            .execute(pool)
            .await?;
        let mut semantically_next = due.clone();
        semantically_next.updated_at = (now() + chrono::Duration::seconds(1)).to_rfc3339();
        assert!(transition_state_locked_db(pool, user_id, &due, &semantically_next).await?);

        let stale_user_id = 43;
        let expired = TourState {
            question_deadline_at: Some((now() - chrono::Duration::seconds(1)).to_rfc3339()),
            ..waiting_state()
        };
        assert_eq!(
            claim_or_load_state_db(pool, stale_user_id, &expired).await?,
            TourClaimResult::Acquired(expired.clone())
        );
        let fresh_waiting = TourState {
            updated_at: (now() + chrono::Duration::seconds(1)).to_rfc3339(),
            ..waiting_state()
        };
        assert!(transition_state_locked_db(pool, stale_user_id, &expired, &fresh_waiting).await?);
        let reloaded = normalize_or_reload_message_state_db(pool, stale_user_id, expired, now())
            .await?
            .expect("concurrent row survives reload");
        let route = classify_message_route(Some(1), None, Some(&reloaded), now());
        assert_eq!(route, TourMessageRoute::TourDm);

        let answering_user_id = 44;
        let answering = TourState {
            mode: TourMode::Answering,
            claimed_question_message_id: Some("9001".into()),
            question_deadline_at: None,
            answering_deadline_at: Some((now() + chrono::Duration::minutes(5)).to_rfc3339()),
            ..waiting_state()
        };
        assert_eq!(
            claim_or_load_state_db(pool, answering_user_id, &answering).await?,
            TourClaimResult::Acquired(answering.clone())
        );
        let resume = TourState {
            mode: TourMode::Step,
            answering_deadline_at: None,
            ..answering.clone()
        };
        resume_question_state_db(pool, answering_user_id, &answering, &resume).await?;
        let reset = load_state_db(pool, answering_user_id)
            .await?
            .expect("reset answering state");
        assert_eq!(reset.mode, TourMode::Step);
        assert_eq!(reset.claimed_question_message_id.as_deref(), Some("9001"));
        assert_eq!(reset.answering_deadline_at, None);

        let late_user_id = 45;
        assert_eq!(
            claim_or_load_state_db(pool, late_user_id, &answering).await?,
            TourClaimResult::Acquired(answering.clone())
        );
        let advanced = TourState {
            status: TourStatus::Starting,
            step: TourStepKey::CommunityQuestions,
            mode: TourMode::Step,
            answering_deadline_at: None,
            updated_at: (now() + chrono::Duration::minutes(6)).to_rfc3339(),
            ..answering.clone()
        };
        assert!(transition_state_locked_db(pool, late_user_id, &answering, &advanced).await?);
        assert!(
            resume_question_state_db(pool, late_user_id, &answering, &resume)
                .await
                .is_err()
        );
        assert_eq!(load_state_db(pool, late_user_id).await?, Some(advanced));

        let retry_user_id = 46;
        let retry_due = TourState {
            status: TourStatus::Starting,
            retry_count: 1,
            next_retry_at: Some((now() - chrono::Duration::seconds(1)).to_rfc3339()),
            ..TourState::starting(now())
        };
        assert_eq!(
            claim_or_load_state_db(pool, retry_user_id, &retry_due).await?,
            TourClaimResult::Acquired(retry_due.clone())
        );
        let retry_actions = decide_tour_retry_actions(&retry_due, now());
        let (retry_expected, retry_claim) = match retry_actions.first() {
            Some(TourAction::PersistState { expected, next }) => (expected, next),
            _ => panic!("retry claim before send"),
        };
        let (first, second) = tokio::join!(
            transition_state_locked_db(pool, retry_user_id, retry_expected, retry_claim),
            transition_state_locked_db(pool, retry_user_id, retry_expected, retry_claim),
        );
        let winners = usize::from(first?) + usize::from(second?);
        let sends = retry_actions
            .iter()
            .filter(|action| matches!(action, TourAction::SendStep(_)))
            .count()
            * winners;
        assert_eq!(winners, 1);
        assert_eq!(sends, 1);
        Ok(())
    }
}
