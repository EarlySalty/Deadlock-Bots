use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use dl_community::onboarding_bridge::{
    BridgeClaimResult, BridgeDmResult, MemberRole, MemberSnapshot, NativeOnboardingCompletedEvent,
    OnboardingBridge, OnboardingBridgePort, RangGuideLink, ONBOARDING_BRIDGE_DM_NS,
};
use dl_discord::{DiscordAdapter, Dispatcher};
use serde_json::{json, Map, Value};
use serenity::all::{GuildId, RoleId, UserId};
use serenity::http::HttpError;
use sqlx::PgPool;

const DM_CLAIM_CLAIMED: &str = "claimed";
const DM_CLAIM_DONE: &str = "dm_done";

pub struct OnboardingBridgeGlue {
    pub adapter: Arc<DiscordAdapter>,
    pub pool: PgPool,
}

#[async_trait]
impl OnboardingBridgePort for OnboardingBridgeGlue {
    async fn member_snapshot(&self, guild_id: u64, user_id: u64) -> Result<MemberSnapshot, String> {
        if let Some(snapshot) = self.cached_member_snapshot(guild_id, user_id) {
            return Ok(snapshot);
        }
        self.http_member_snapshot(guild_id, user_id).await
    }

    async fn claim_dm_once(&self, user_id: u64) -> Result<BridgeClaimResult, String> {
        let result = sqlx::query(
            "INSERT INTO bot.kv_store(ns, k, v)
             VALUES($1, $2, $3)
             ON CONFLICT(ns, k) DO NOTHING",
        )
        .bind(ONBOARDING_BRIDGE_DM_NS)
        .bind(user_id.to_string())
        .bind(DM_CLAIM_CLAIMED)
        .execute(&self.pool)
        .await
        .map_err(|err| err.to_string())?;

        if result.rows_affected() > 0 {
            return Ok(BridgeClaimResult::Acquired);
        }

        let current_value = sqlx::query_scalar::<_, String>(
            "SELECT v
               FROM bot.kv_store
              WHERE ns = $1 AND k = $2
              LIMIT 1",
        )
        .bind(ONBOARDING_BRIDGE_DM_NS)
        .bind(user_id.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| err.to_string())?;

        if current_value.as_deref() == Some(DM_CLAIM_DONE) {
            Ok(BridgeClaimResult::AlreadyClaimedDmDone)
        } else {
            Ok(BridgeClaimResult::AlreadyClaimedInFlight)
        }
    }

    async fn mark_dm_done(&self, user_id: u64) -> Result<(), String> {
        sqlx::query(
            "UPDATE bot.kv_store
                SET v = $3
              WHERE ns = $1 AND k = $2",
        )
        .bind(ONBOARDING_BRIDGE_DM_NS)
        .bind(user_id.to_string())
        .bind(DM_CLAIM_DONE)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|err| err.to_string())
    }

    async fn release_dm_claim(&self, user_id: u64) -> Result<(), String> {
        sqlx::query("DELETE FROM bot.kv_store WHERE ns = $1 AND k = $2")
            .bind(ONBOARDING_BRIDGE_DM_NS)
            .bind(user_id.to_string())
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    async fn rang_guide_link(&self, guild_id: u64) -> RangGuideLink {
        let channel_id = crate::serversync::rang_guide_channel_id();
        let message_id = match self.load_rang_guide_message_id().await {
            Ok(message_id) => message_id,
            Err(err) => {
                tracing::warn!(
                    %err,
                    guild_id,
                    channel_id,
                    "Onboarding-Bruecke: Rang-Guide-Message-ID konnte nicht aus KV gelesen werden"
                );
                None
            }
        };
        let url = match message_id {
            Some(message_id) => {
                format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
            }
            None => format!("https://discord.com/channels/{guild_id}/{channel_id}"),
        };

        RangGuideLink { channel_id, url }
    }

    async fn send_followup_dm(
        &self,
        user_id: u64,
        content: String,
        components: Value,
    ) -> BridgeDmResult {
        let channel = match self
            .adapter
            .http
            .create_private_channel(&json!({ "recipient_id": user_id.to_string() }))
            .await
        {
            Ok(channel) => channel,
            Err(err) if is_discord_cannot_send_messages(&err) => {
                return BridgeDmResult::CannotSend50007;
            }
            Err(err) => return BridgeDmResult::Transient(err.to_string()),
        };

        let mut body = Map::new();
        body.insert("content".into(), json!(content));
        body.insert("components".into(), components);
        match self
            .adapter
            .send_raw_public_typed(channel.id.get(), &body)
            .await
        {
            Ok(_) => BridgeDmResult::Sent,
            Err(err) if is_discord_cannot_send_messages(&err) => BridgeDmResult::CannotSend50007,
            Err(err) => BridgeDmResult::Transient(err.to_string()),
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
                Some("Onboarding-Bruecke: Rang-Verknuepfung-Followup"),
            )
            .await
            .map_err(|err| err.to_string())
    }
}

impl OnboardingBridgeGlue {
    fn cached_member_snapshot(&self, guild_id: u64, user_id: u64) -> Option<MemberSnapshot> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        let member = guild.members.get(&UserId::new(user_id))?;
        let roles = member
            .roles
            .iter()
            .filter_map(|role_id| {
                guild.roles.get(role_id).map(|role| MemberRole {
                    id: role_id.get(),
                    name: role.name.clone(),
                })
            })
            .collect();

        Some(MemberSnapshot {
            is_bot: member.user.bot,
            roles,
        })
    }

    async fn http_member_snapshot(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<MemberSnapshot, String> {
        let guild_id = GuildId::new(guild_id);
        let member = self
            .adapter
            .http
            .get_member(guild_id, UserId::new(user_id))
            .await
            .map_err(|err| err.to_string())?;
        let member_role_ids = member
            .roles
            .iter()
            .map(|role| role.get())
            .collect::<HashSet<_>>();
        let guild_roles = match self.cached_guild_roles(guild_id.get()) {
            Some(roles) => roles,
            None => self
                .adapter
                .http
                .get_guild_roles(guild_id)
                .await
                .map_err(|err| err.to_string())?
                .into_iter()
                .map(|role| MemberRole {
                    id: role.id.get(),
                    name: role.name,
                })
                .collect(),
        };
        let roles = guild_roles
            .into_iter()
            .filter(|role| member_role_ids.contains(&role.id))
            .collect();

        Ok(MemberSnapshot {
            is_bot: member.user.bot,
            roles,
        })
    }

    fn cached_guild_roles(&self, guild_id: u64) -> Option<Vec<MemberRole>> {
        let guild = self.adapter.cache().guild(GuildId::new(guild_id))?;
        Some(
            guild
                .roles
                .values()
                .map(|role| MemberRole {
                    id: role.id.get(),
                    name: role.name.clone(),
                })
                .collect(),
        )
    }

    async fn load_rang_guide_message_id(&self) -> Result<Option<u64>, String> {
        let key = crate::serversync::rang_guide_message_id_key(0);
        let raw = sqlx::query_scalar::<_, String>(
            "SELECT v
               FROM bot.kv_store
              WHERE ns = $1 AND k = $2
              LIMIT 1",
        )
        .bind(crate::serversync::serversync_kv_ns())
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| err.to_string())?;

        raw.map(|value| value.parse::<u64>().map_err(|err| err.to_string()))
            .transpose()
    }
}

pub fn spawn(
    pool: PgPool,
    adapter: Arc<DiscordAdapter>,
    dispatcher: &Dispatcher,
    main_guild_id: u64,
) -> tokio::task::JoinHandle<()> {
    let bridge = Arc::new(OnboardingBridge::new(
        Arc::new(OnboardingBridgeGlue { adapter, pool }),
        main_guild_id,
    ));
    let mut events = dispatcher.subscribe_members();

    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::MemberEvent::NativeOnboardingCompleted { guild_id, user_id }) => {
                    let bridge = bridge.clone();
                    tokio::spawn(async move {
                        bridge
                            .handle_native_onboarding_completed(NativeOnboardingCompletedEvent {
                                guild_id,
                                user_id,
                            })
                            .await;
                    });
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(
                        missed,
                        "Onboarding-Bruecke Native-Onboarding-Events verpasst"
                    );
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn is_discord_cannot_send_messages(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Http(HttpError::UnsuccessfulRequest(resp)) if resp.error.code == 50007
    )
}
