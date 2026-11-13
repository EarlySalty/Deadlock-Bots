pub mod links;
pub mod roles;
pub mod store;

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use serenity::model::id::{GuildId, RoleId, UserId};
use sqlx::PgPool;

use dl_core::internal_tokens::{first_token, STEAM_BOT_RECEIVER, TWITCH_DASHBOARD_RECEIVER};
use dl_core::runtime_config::{
    CommunityOptions, SUPPORTER_CUTOVER_GRACE_DAYS, SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS,
    SUPPORTER_DEFAULT_MAX_REVOKES, SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS,
    SUPPORTER_DEFAULT_TIERS_EUR,
};

use crate::serversync::support_publish::{SupportFunding, SupportPanelOptions};
use crate::serversync::{FundingRefresh, ServerSyncService};

pub const SUPPORTER_ROLE_ID: u64 = 1_483_135_384_098_570_363;
pub const BOOSTER_ROLE_ID: u64 = 1_414_182_925_460_836_443;
const GRANT_REASON: &str = "Server-Unterstützer: aktive Unterstützung";
const REVOKE_REASON: &str = "Server-Unterstützer: keine aktive Unterstützung mehr";

#[derive(Clone)]
pub struct SupporterConfig {
    pub role_grant_apply: bool,
    pub role_sync_apply: bool,
    pub max_revokes: u64,
    pub interval: Duration,
    pub goal_cents: i64,
    pub steam_api_base: String,
    pub steam_token: Option<String>,
    pub twitch_api_base: Option<String>,
    pub twitch_token: Option<String>,
    pub tiers_eur: Vec<u64>,
}

impl fmt::Debug for SupporterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SupporterConfig")
            .field("role_grant_apply", &self.role_grant_apply)
            .field("role_sync_apply", &self.role_sync_apply)
            .field("max_revokes", &self.max_revokes)
            .field("interval", &self.interval)
            .field("goal_cents", &self.goal_cents)
            .field("steam_api_base", &self.steam_api_base)
            .field("steam_token_set", &self.steam_token.is_some())
            .field("twitch_api_base", &self.twitch_api_base)
            .field("twitch_token_set", &self.twitch_token.is_some())
            .field("tiers_eur", &self.tiers_eur)
            .finish()
    }
}

impl SupporterConfig {
    pub fn new(
        community: &CommunityOptions,
        steam_api_base: &str,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            role_grant_apply: community.supporter_role_grant_apply.unwrap_or(false),
            role_sync_apply: community.supporter_role_sync_apply.unwrap_or(false),
            max_revokes: community
                .supporter_role_sync_max_revokes
                .unwrap_or(SUPPORTER_DEFAULT_MAX_REVOKES),
            interval: Duration::from_secs(
                community
                    .supporter_sync_interval_seconds
                    .unwrap_or(SUPPORTER_DEFAULT_SYNC_INTERVAL_SECONDS),
            ),
            goal_cents: community
                .supporter_funding_goal_cents
                .unwrap_or(SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS),
            steam_api_base: steam_api_base.to_string(),
            steam_token: first_token(&lookup, STEAM_BOT_RECEIVER),
            twitch_api_base: community.supporter_twitch_api_base.clone(),
            twitch_token: first_token(&lookup, TWITCH_DASHBOARD_RECEIVER),
            tiers_eur: community
                .supporter_tiers_eur
                .clone()
                .unwrap_or_else(|| SUPPORTER_DEFAULT_TIERS_EUR.to_vec()),
        }
    }

    pub fn panel_options(&self) -> SupportPanelOptions {
        SupportPanelOptions {
            tiers_eur: self.tiers_eur.clone(),
            twitch: self.twitch_api_base.is_some() && self.twitch_token.is_some(),
            role_grant: self.role_grant_apply,
        }
    }

    pub fn log_effective(&self) {
        tracing::info!(
            goal_cents = self.goal_cents,
            tiers_eur = ?self.tiers_eur,
            twitch = self.panel_options().twitch,
            role_grant_apply = self.role_grant_apply,
            "Server-Unterstützer: Panel-Werte"
        );
    }
}

#[async_trait::async_trait]
pub trait FundingSource: Send + Sync {
    async fn funding(&self) -> Result<SupportFunding, String>;

    fn missing_config(&self) -> Option<&'static str>;

    fn goal_cents(&self) -> i64;

    fn panel_options(&self) -> SupportPanelOptions;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RefreshState {
    Fine,
    Skipped(&'static str),
    Failed(String),
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
                grant_apply = self.config.role_grant_apply,
                revoke_apply = self.config.role_sync_apply,
                max_revokes = self.config.max_revokes,
                interval_seconds = self.config.interval.as_secs(),
                "Server-Unterstützer: Abgleich gestartet"
            );
            tokio::join!(self.role_loop(), self.funding_loop());
        })
    }

    fn ticker(&self) -> tokio::time::Interval {
        let mut ticker = tokio::time::interval(self.config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker
    }

    async fn role_loop(&self) {
        let mut table_missing_logged = false;
        let mut ticker = self.ticker();
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
        }
    }

    async fn funding_loop(&self) {
        let mut refresh_state = RefreshState::Fine;
        let mut ticker = self.ticker();
        loop {
            ticker.tick().await;
            refresh_state = self.refresh_funding(refresh_state).await;
        }
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
                grace_days = SUPPORTER_CUTOVER_GRACE_DAYS,
                "Server-Unterstützer: bisherige Rolleninhaber übernommen"
            );
            import_done = true;
        }
        let mut active = store::active_supporters(&self.pool).await?;
        if !import_done {
            active.extend(holders.iter().copied());
        }
        let plan = roles::plan_roles(&members, &active, self.config.max_revokes);
        tracing::info!(
            grant_apply = self.config.role_grant_apply,
            revoke_apply = self.config.role_sync_apply,
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
        if self.config.role_grant_apply {
            self.apply_grants(&plan.grants).await;
        }
        if self.config.role_sync_apply {
            self.apply_revokes(&plan.revokes).await;
        }
        Ok(())
    }

    async fn apply_grants(&self, grants: &[u64]) {
        let guild = GuildId::new(self.guild_id);
        let role = RoleId::new(SUPPORTER_ROLE_ID);
        for user_id in grants {
            if let Err(error) = self
                .http
                .add_member_role(guild, UserId::new(*user_id), role, Some(GRANT_REASON))
                .await
            {
                tracing::warn!(user_id, %error, "Server-Unterstützer: Rolle nicht vergeben");
            }
        }
    }

    async fn apply_revokes(&self, revokes: &[u64]) {
        let guild = GuildId::new(self.guild_id);
        let role = RoleId::new(SUPPORTER_ROLE_ID);
        for user_id in revokes {
            if let Err(error) = self
                .http
                .remove_member_role(guild, UserId::new(*user_id), role, Some(REVOKE_REASON))
                .await
            {
                tracing::warn!(user_id, %error, "Server-Unterstützer: Rolle nicht entzogen");
            }
        }
    }

    async fn refresh_funding(&self, previous: RefreshState) -> RefreshState {
        let outcome = self.serversync.support_funding_refresh().await;
        let next = match &outcome {
            Ok(FundingRefresh::Skipped(reason)) => RefreshState::Skipped(reason),
            Ok(_) => RefreshState::Fine,
            Err(error) => RefreshState::Failed(error.to_string()),
        };
        let changed = next != previous;
        match outcome {
            Ok(FundingRefresh::Skipped(reason)) if changed => {
                tracing::warn!(reason, "Server-Unterstützer: Monatsziel nicht aktualisiert")
            }
            Err(error) if changed => {
                tracing::warn!(%error, "Server-Unterstützer: Monatsziel nicht abrufbar")
            }
            Ok(outcome) => {
                if changed {
                    tracing::info!(?outcome, "Server-Unterstützer: Monatsziel wieder aktuell");
                }
                tracing::debug!(?outcome, "Server-Unterstützer: Monatsziel geprüft");
            }
            Err(error) => {
                tracing::debug!(%error, "Server-Unterstützer: Monatsziel weiter nicht abrufbar")
            }
        }
        next
    }
}

pub fn register(
    router: &mut dl_discord::InteractionRouter,
    links: Arc<dyn links::SupporterLinks>,
    config: &SupporterConfig,
) {
    links::register(router, links, config);
}
