use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dl_community::tags::{TagEvent, TagService};
use serde_json::json;
use serenity::all::{GuildId, UserId};
use sqlx::PgPool;

pub const NATIVE_ONBOARDING_DEDUPE_NS: &str = "native_onboarding:completed";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOnboardingRole {
    pub role_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeOnboardingChoice {
    InviteGuest,
    Freshling,
    Player,
}

impl NativeOnboardingChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::InviteGuest => "invite_guest",
            Self::Freshling => "freshling",
            Self::Player => "player_lfg",
        }
    }
}

#[async_trait]
pub trait NativeOnboardingLookup: Send + Sync {
    async fn member_roles_and_guild_roles(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<(Vec<u64>, Vec<NativeOnboardingRole>), String>;
}

struct SerenityNativeOnboardingLookup {
    adapter: Arc<dl_discord::DiscordAdapter>,
}

#[async_trait]
impl NativeOnboardingLookup for SerenityNativeOnboardingLookup {
    async fn member_roles_and_guild_roles(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<(Vec<u64>, Vec<NativeOnboardingRole>), String> {
        let guild = GuildId::new(guild_id);
        let member = self
            .adapter
            .http
            .get_member(guild, UserId::new(user_id))
            .await
            .map_err(|err| err.to_string())?;
        let mut member_role_ids = member
            .roles
            .iter()
            .map(|role| role.get())
            .collect::<Vec<_>>();
        member_role_ids.sort_unstable();
        let roles = self
            .adapter
            .http
            .get_guild_roles(guild)
            .await
            .map_err(|err| err.to_string())?
            .into_iter()
            .map(|role| NativeOnboardingRole {
                role_id: role.id.get(),
                name: role.name,
            })
            .collect();
        Ok((member_role_ids, roles))
    }
}

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

pub fn classify_native_onboarding_choice(
    member_role_ids: &[u64],
    roles: &[NativeOnboardingRole],
) -> NativeOnboardingChoice {
    if has_named_role(member_role_ids, roles, "Invite-Gast") {
        NativeOnboardingChoice::InviteGuest
    } else if has_named_role(member_role_ids, roles, "Frischling") {
        NativeOnboardingChoice::Freshling
    } else {
        NativeOnboardingChoice::Player
    }
}

fn has_named_role(member_role_ids: &[u64], roles: &[NativeOnboardingRole], expected: &str) -> bool {
    let expected = normalized_role_name(expected);
    roles.iter().any(|role| {
        member_role_ids.contains(&role.role_id) && normalized_role_name(&role.name) == expected
    })
}

fn normalized_role_name(name: &str) -> String {
    name.trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .trim()
        .to_lowercase()
}

pub async fn claim_native_onboarding_once(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
) -> Result<bool, sqlx::Error> {
    let key = format!("{guild_id}:{user_id}");
    let value = json!({
        "guild_id": guild_id,
        "user_id": user_id,
        "claimed_at": Utc::now().to_rfc3339(),
    })
    .to_string();
    sqlx::query(
        "INSERT INTO bot.kv_store(ns, k, v)
         VALUES($1, $2, $3)
         ON CONFLICT(ns, k) DO NOTHING",
    )
    .bind(NATIVE_ONBOARDING_DEDUPE_NS)
    .bind(key)
    .bind(value)
    .execute(pool)
    .await
    .map(|result| result.rows_affected() > 0)
}

async fn handle_native_onboarding_completed(
    pool: PgPool,
    lookup: Arc<dyn NativeOnboardingLookup>,
    guild_id: u64,
    user_id: u64,
    reread_delay: Duration,
) {
    tokio::time::sleep(reread_delay).await;
    let (member_role_ids, roles) = match lookup
        .member_roles_and_guild_roles(guild_id, user_id)
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(%err, guild_id, user_id, "Native-Onboarding Rollen-Nachlese fehlgeschlagen");
            return;
        }
    };
    let choice = classify_native_onboarding_choice(&member_role_ids, &roles);
    match claim_native_onboarding_once(&pool, guild_id, user_id).await {
        Ok(true) => {}
        Ok(false) => return,
        Err(err) => {
            tracing::warn!(%err, guild_id, user_id, "Native-Onboarding Dedupe fehlgeschlagen");
            return;
        }
    }

    let matched_role_id = match choice {
        NativeOnboardingChoice::InviteGuest => {
            role_id_by_name(&member_role_ids, &roles, "Invite-Gast")
        }
        NativeOnboardingChoice::Freshling => {
            role_id_by_name(&member_role_ids, &roles, "Frischling")
        }
        NativeOnboardingChoice::Player => None,
    };
    if let Err(err) = dl_activity::journey::record_native_onboarding_completed_event(
        &pool,
        user_id,
        guild_id,
        "member_flags_completed_onboarding",
        Utc::now(),
        json!({
            "weiche_choice": choice.as_str(),
            "matched_role_id": matched_role_id,
            "member_role_count": member_role_ids.len(),
        }),
    )
    .await
    {
        tracing::warn!(%err, guild_id, user_id, "Journey native_onboarding_completed aus MemberFlags fehlgeschlagen");
    }
}

fn role_id_by_name(
    member_role_ids: &[u64],
    roles: &[NativeOnboardingRole],
    expected: &str,
) -> Option<u64> {
    let expected = normalized_role_name(expected);
    roles
        .iter()
        .find(|role| {
            member_role_ids.contains(&role.role_id) && normalized_role_name(&role.name) == expected
        })
        .map(|role| role.role_id)
}

pub fn spawn_native_onboarding_completed(
    pool: PgPool,
    adapter: Arc<dl_discord::DiscordAdapter>,
    dispatcher: &dl_discord::Dispatcher,
    guild_id_filter: u64,
) -> tokio::task::JoinHandle<()> {
    spawn_native_onboarding_completed_with_lookup(
        pool,
        Arc::new(SerenityNativeOnboardingLookup { adapter }),
        dispatcher,
        guild_id_filter,
        Duration::from_secs(7),
    )
}

pub fn spawn_native_onboarding_completed_with_lookup(
    pool: PgPool,
    lookup: Arc<dyn NativeOnboardingLookup>,
    dispatcher: &dl_discord::Dispatcher,
    guild_id_filter: u64,
    reread_delay: Duration,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::MemberEvent::NativeOnboardingCompleted { guild_id, user_id })
                    if guild_id == guild_id_filter =>
                {
                    let pool = pool.clone();
                    let lookup = lookup.clone();
                    tokio::spawn(handle_native_onboarding_completed(
                        pool,
                        lookup,
                        guild_id,
                        user_id,
                        reread_delay,
                    ));
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Journey Native-Onboarding-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_onboarding_weiche_klassifikation_priorisiert_invite_vor_frischling() {
        let roles = vec![
            NativeOnboardingRole {
                role_id: 10,
                name: "Frischling".to_string(),
            },
            NativeOnboardingRole {
                role_id: 11,
                name: "Invite-Gast".to_string(),
            },
        ];

        assert_eq!(
            classify_native_onboarding_choice(&[10, 11], &roles),
            NativeOnboardingChoice::InviteGuest
        );
        assert_eq!(
            classify_native_onboarding_choice(&[10], &roles),
            NativeOnboardingChoice::Freshling
        );
        assert_eq!(
            classify_native_onboarding_choice(&[], &roles),
            NativeOnboardingChoice::Player
        );
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn native_onboarding_dedupe_marker_ist_persistent_und_einmalig(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        assert!(claim_native_onboarding_once(pool, 1, 42).await?);
        assert!(!claim_native_onboarding_once(pool, 1, 42).await?);
        assert!(claim_native_onboarding_once(pool, 2, 42).await?);

        Ok(())
    }
}
