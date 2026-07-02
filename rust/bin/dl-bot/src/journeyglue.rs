use std::sync::Arc;

use chrono::Utc;
use dl_community::tags::{TagEvent, TagService};
use serde_json::json;
use sqlx::PgPool;

const WEICHE_ROLE_CHOICES: &[(u64, &str)] = &[
    (
        dl_community::ai_onboarding::ROLE_STREAMER_ONBOARD_ID,
        "streamer_onboarding",
    ),
    (
        dl_community::ai_onboarding::ROLE_STREAMER_PARTNER_ID,
        "streamer_partner",
    ),
    (dl_community::ai_onboarding::ROLE_LFG_PING_ID, "lfg_ping"),
    (
        dl_community::ai_onboarding::ROLE_CUSTOM_GAMES_PING_ID,
        "custom_games_ping",
    ),
    (
        dl_community::ai_onboarding::ROLE_PATCHNOTES_PING_ID,
        "patchnotes_ping",
    ),
    (dl_community::ai_onboarding::ROLE_RANKED_ID, "ranked"),
    (dl_community::ai_onboarding::ROLE_CASUAL_ID, "casual"),
];

fn weiche_choice_for_role(role_id: u64) -> Option<&'static str> {
    WEICHE_ROLE_CHOICES
        .iter()
        .find_map(|(candidate, choice)| (*candidate == role_id).then_some(*choice))
}

async fn record_role_delta(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    role_id: u64,
    change: &'static str,
) {
    if change == "gained" && role_id == dl_community::onboarding::ONBOARD_COMPLETE_ROLE_ID {
        if let Err(err) = dl_activity::journey::record_native_onboarding_completed_event(
            pool,
            user_id,
            guild_id,
            "role_event",
            Utc::now(),
            json!({
                "completed_role_id": role_id,
                "change": change,
            }),
        )
        .await
        {
            tracing::warn!(%err, guild_id, user_id, role_id, "Journey native_onboarding_completed fehlgeschlagen");
        }
    }

    if let Some(choice) = weiche_choice_for_role(role_id) {
        if let Err(err) = dl_activity::journey::record_weiche_changed_event(
            pool,
            user_id,
            guild_id,
            choice,
            change,
            "role_event",
            Utc::now(),
        )
        .await
        {
            tracing::warn!(%err, guild_id, user_id, role_id, "Journey weiche_changed aus RoleEvent fehlgeschlagen");
        }
    }
}

pub fn spawn_role_events(
    pool: PgPool,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_roles();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::RoleEvent::Gained {
                    guild_id,
                    user_id,
                    role_ids,
                }) => {
                    for role_id in role_ids {
                        record_role_delta(&pool, guild_id, user_id, role_id, "gained").await;
                    }
                }
                Ok(dl_discord::RoleEvent::Removed {
                    guild_id,
                    user_id,
                    role_ids,
                }) => {
                    for role_id in role_ids {
                        record_role_delta(&pool, guild_id, user_id, role_id, "removed").await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Journey RoleEvents verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn spawn_tag_events(
    pool: PgPool,
    tags: Arc<TagService>,
    guild_id: u64,
) -> tokio::task::JoinHandle<()> {
    let mut events = tags.subscribe();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(TagEvent::UserTagChanged {
                    user_id, key, new, ..
                }) if key == "age" || key == "tone" => {
                    let choice = new
                        .map(|value| format!("{key}:{value}"))
                        .unwrap_or_else(|| format!("{key}:cleared"));
                    if let Err(err) = dl_activity::journey::record_weiche_changed_event(
                        &pool,
                        user_id,
                        guild_id,
                        &choice,
                        "tag_changed",
                        "tag_service",
                        Utc::now(),
                    )
                    .await
                    {
                        tracing::warn!(%err, guild_id, user_id, key, "Journey weiche_changed aus TagEvent fehlgeschlagen");
                    }
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Journey TagEvents verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}
