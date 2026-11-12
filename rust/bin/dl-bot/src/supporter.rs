pub mod links;
pub mod roles;
pub mod store;

use std::sync::Arc;
use std::time::Duration;

use serenity::model::id::{GuildId, RoleId, UserId};
use sqlx::PgPool;

use dl_core::runtime_config::{
    SUPPORTER_FUNDING_GOAL_CENTS_RANGE, SUPPORTER_MAX_REVOKES_RANGE,
    SUPPORTER_REQUEST_TIMEOUT_MS_RANGE, SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE,
};

use crate::serversync::support_publish::{SupportFunding, SUPPORT_FUNDING_GOAL_CENTS_DEFAULT};
use crate::serversync::ServerSyncService;

pub const SUPPORTER_ROLE_ID: u64 = 1_483_135_384_098_570_363;
pub const BOOSTER_ROLE_ID: u64 = 1_414_182_925_460_836_443;
const DEFAULT_MAX_REVOKES: usize = 10;
const DEFAULT_INTERVAL_SECONDS: u64 = 300;
const GRANT_REASON: &str = "Server-Unterstützer: aktive Unterstützung";
const REVOKE_REASON: &str = "Server-Unterstützer: keine aktive Unterstützung mehr";

#[derive(Clone, Debug)]
pub struct SupporterConfig {
    pub role_sync_apply: bool,
    pub max_revokes: usize,
    pub interval: Duration,
    pub request_timeout: Option<Duration>,
    pub goal_cents: i64,
    pub steam_api_base: String,
    pub steam_token: Option<String>,
    pub twitch_api_base: Option<String>,
    pub twitch_token: Option<String>,
}

impl SupporterConfig {
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let get = |key: &str| {
            lookup(key)
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        };
        let flag = |key: &str| {
            get(key).is_some_and(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
        };
        let bounded = |key: &str, range: &std::ops::RangeInclusive<u64>| {
            get(key)
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|value| range.contains(value))
        };
        Self {
            role_sync_apply: flag("DL_SUPPORTER_ROLE_SYNC_APPLY"),
            max_revokes: bounded(
                "DL_SUPPORTER_ROLE_SYNC_MAX_REVOKES",
                &SUPPORTER_MAX_REVOKES_RANGE,
            )
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(DEFAULT_MAX_REVOKES),
            interval: Duration::from_secs(
                bounded(
                    "DL_SUPPORTER_SYNC_INTERVAL_SECONDS",
                    &SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE,
                )
                .unwrap_or(DEFAULT_INTERVAL_SECONDS),
            ),
            request_timeout: bounded(
                "DL_SUPPORTER_REQUEST_TIMEOUT_MS",
                &SUPPORTER_REQUEST_TIMEOUT_MS_RANGE,
            )
            .map(Duration::from_millis),
            goal_cents: get("DL_SUPPORTER_FUNDING_GOAL_CENTS")
                .and_then(|value| value.parse::<i64>().ok())
                .filter(|value| SUPPORTER_FUNDING_GOAL_CENTS_RANGE.contains(value))
                .unwrap_or(SUPPORT_FUNDING_GOAL_CENTS_DEFAULT),
            steam_api_base: get("DL_SUPPORTER_STEAM_API_BASE")
                .or_else(|| get("STEAM_BOT_API_URL"))
                .unwrap_or_else(|| dl_bridges::steam::DEFAULT_API_URL.to_string()),
            steam_token: get("TWITCH_INTERNAL_API_TOKEN")
                .or_else(|| get("MASTER_BROKER_TOKEN"))
                .or_else(|| get("MAIN_BOT_INTERNAL_TOKEN")),
            twitch_api_base: get("DL_SUPPORTER_TWITCH_API_BASE"),
            twitch_token: get("MASTER_BROKER_TOKEN")
                .or_else(|| get("MAIN_BOT_INTERNAL_TOKEN"))
                .or_else(|| get("TWITCH_INTERNAL_API_TOKEN")),
        }
    }
}

#[async_trait::async_trait]
pub trait FundingSource: Send + Sync {
    async fn funding(&self) -> Result<SupportFunding, String>;

    fn missing_config(&self) -> Option<&'static str> {
        None
    }

    fn goal_cents(&self) -> i64 {
        SUPPORT_FUNDING_GOAL_CENTS_DEFAULT
    }
}

pub struct SupporterSync {
    pool: PgPool,
    http: Arc<serenity::http::Http>,
    guild_id: u64,
    serversync: Arc<ServerSyncService>,
    config: SupporterConfig,
}

impl SupporterSync {
    pub fn new(
        pool: PgPool,
        http: Arc<serenity::http::Http>,
        guild_id: u64,
        serversync: Arc<ServerSyncService>,
        config: SupporterConfig,
    ) -> Self {
        Self {
            pool,
            http,
            guild_id,
            serversync,
            config,
        }
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            tracing::info!(
                apply = self.config.role_sync_apply,
                max_revokes = self.config.max_revokes,
                interval_seconds = self.config.interval.as_secs(),
                "Server-Unterstützer: Abgleich gestartet"
            );
            let mut table_missing_logged = false;
            let mut ticker = tokio::time::interval(self.config.interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                match self.tick().await {
                    Ok(()) => table_missing_logged = false,
                    Err(store::StoreError::Missing) => {
                        if !table_missing_logged {
                            tracing::warn!(
                                "Server-Unterstützer: core.supporter_sources fehlt, Abgleich pausiert bis zur Migration"
                            );
                            table_missing_logged = true;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Server-Unterstützer: Abgleich übersprungen");
                    }
                }
                self.refresh_funding().await;
            }
        })
    }

    async fn tick(&self) -> Result<(), store::StoreError> {
        let members = roles::fetch_members(&self.http, self.guild_id)
            .await
            .map_err(store::StoreError::Discord)?;
        if members.is_empty() {
            return Err(store::StoreError::Discord(
                "Mitgliederliste ist leer".to_string(),
            ));
        }
        let boosters: Vec<u64> = members.iter().filter(|m| m.booster).map(|m| m.id).collect();
        let boost = store::sync_boost_rows(&self.pool, &boosters).await?;
        let holders: Vec<u64> = members
            .iter()
            .filter(|m| m.has_role)
            .map(|m| m.id)
            .collect();
        let mut import_done = store::manual_import_done(&self.pool).await?;
        if self.config.role_sync_apply && !import_done {
            let imported = store::import_manual(&self.pool, &holders).await?;
            tracing::info!(
                imported,
                "Server-Unterstützer: bisherige Rolleninhaber für 31 Tage übernommen"
            );
            import_done = true;
        }
        let mut active = store::active_supporters(&self.pool).await?;
        if !import_done {
            active.extend(holders.iter().copied());
        }
        let plan = roles::plan_roles(&members, &active, self.config.max_revokes);
        tracing::info!(
            apply = self.config.role_sync_apply,
            boosters = boosters.len(),
            boost_rows_removed = boost.removed,
            would_import = if import_done { 0 } else { holders.len() },
            active = active.len(),
            grants = plan.grants.len(),
            revokes = plan.revokes.len(),
            revokes_held = plan.revokes_held,
            grant_ids = ?plan.grants,
            revoke_ids = ?plan.revokes,
            "Server-Unterstützer: Rollenabgleich"
        );
        if self.config.role_sync_apply {
            self.apply_plan(&plan).await;
        }
        Ok(())
    }

    async fn apply_plan(&self, plan: &roles::RolePlan) {
        let guild = GuildId::new(self.guild_id);
        let role = RoleId::new(SUPPORTER_ROLE_ID);
        for user_id in &plan.grants {
            if let Err(error) = self
                .http
                .add_member_role(guild, UserId::new(*user_id), role, Some(GRANT_REASON))
                .await
            {
                tracing::warn!(user_id, %error, "Server-Unterstützer: Rolle nicht vergeben");
            }
        }
        for user_id in &plan.revokes {
            if let Err(error) = self
                .http
                .remove_member_role(guild, UserId::new(*user_id), role, Some(REVOKE_REASON))
                .await
            {
                tracing::warn!(user_id, %error, "Server-Unterstützer: Rolle nicht entzogen");
            }
        }
    }

    async fn refresh_funding(&self) {
        match self.serversync.support_funding_refresh().await {
            Ok(outcome) => tracing::debug!(?outcome, "Server-Unterstützer: Monatsziel geprüft"),
            Err(error) => {
                tracing::warn!(%error, "Server-Unterstützer: Monatsziel nicht aktualisiert")
            }
        }
    }
}

pub fn register(router: &mut dl_discord::InteractionRouter, links: Arc<dyn links::SupporterLinks>) {
    links::register(router, links);
}
