#[path = "twitch_invites_personal.rs"]
mod personal;
#[path = "twitch_invites_voice.rs"]
mod voice;

use std::{
    collections::VecDeque,
    sync::{atomic::Ordering, Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use dl_activity::qualified_invites::{self as store, Cursor, MemberProof};
use dl_broker::twitch_invites::{
    InviteDestination, PersonalInvite, QualifiedQuery, TwitchInvitePort,
};
use dl_core::bot_config::{BotConfig, TwitchInvitesConfig};
use dl_discord::DiscordAdapter;
use serenity::all::{GuildId, UserId};
use sqlx::PgPool;

pub struct TwitchInvites {
    pool: PgPool,
    adapter: Arc<DiscordAdapter>,
    guild_id: u64,
    config: TwitchInvitesConfig,
    last_warning: Mutex<VecDeque<Instant>>,
    voice_needs_reset: std::sync::atomic::AtomicBool,
    voice_observation_lock: tokio::sync::Mutex<()>,
}

fn warning_allowed(warnings: &mut VecDeque<Instant>, now: Instant) -> bool {
    let window = Duration::from_secs(7 * 24 * 60 * 60);
    while warnings
        .front()
        .is_some_and(|warning| now.saturating_duration_since(*warning) >= window)
    {
        warnings.pop_front();
    }
    if warnings.len() >= 2
        || warnings.back().is_some_and(|warning| {
            now.saturating_duration_since(*warning) < Duration::from_secs(24 * 60 * 60)
        })
    {
        return false;
    }
    warnings.push_back(now);
    true
}

impl TwitchInvites {
    pub fn new(pool: PgPool, adapter: Arc<DiscordAdapter>, config: &BotConfig) -> Arc<Self> {
        Arc::new(Self {
            pool,
            adapter,
            guild_id: config
                .discord
                .guild_id
                .as_deref()
                .and_then(|id| id.parse().ok())
                .unwrap_or(0),
            config: config.twitch_invites.clone(),
            last_warning: Mutex::new(VecDeque::new()),
            voice_needs_reset: std::sync::atomic::AtomicBool::new(true),
            voice_observation_lock: tokio::sync::Mutex::new(()),
        })
    }

    fn excluded_channels(&self) -> Result<Vec<i64>, String> {
        if !self.adapter.gateway_ready.load(Ordering::Acquire) {
            return Err("Discord-Gateway noch nicht bereit".into());
        }
        let guild = self
            .adapter
            .cache()
            .guild(GuildId::new(self.guild_id))
            .ok_or("Discord-Guild nicht im Cache")?;
        let mut ids = self.config.excluded_voice_channel_ids.clone();
        let staging = dl_voice::tempvoice::TempVoiceConfig::production();
        if staging.guild_id_hint == self.guild_id {
            ids.extend(staging.staging_channels);
        }
        if let Some(afk) = &guild.afk_metadata {
            ids.push(afk.afk_channel_id.get());
        }
        let mut ids: Vec<i64> = ids
            .into_iter()
            .map(i64::try_from)
            .collect::<Result<_, _>>()
            .map_err(|_| "Kanal-ID außerhalb von BIGINT")?;
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    async fn member_proof(&self, user_id: i64) -> Result<Option<MemberProof>, String> {
        let user_id = u64::try_from(user_id).map_err(|_| "Ungültige Mitglieds-ID")?;
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            self.adapter
                .http
                .get_member(GuildId::new(self.guild_id), UserId::new(user_id)),
        )
        .await
        .map_err(|_| "Discord-Mitgliedsprüfung überschritt das Zeitlimit")?;
        match result {
            Ok(member) => {
                let joined_at = member
                    .joined_at
                    .and_then(|value| DateTime::parse_from_rfc3339(&value.to_string()).ok())
                    .map(|value| value.with_timezone(&Utc));
                if joined_at.is_none() && !member.user.bot {
                    return Err("Discord-Beitrittszeit fehlt".into());
                }
                Ok(Some(MemberProof {
                    joined_at,
                    is_bot: member.user.bot,
                    checked_at: Utc::now(),
                }))
            }
            Err(serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(
                response,
            ))) if response.error.code == 10007 => Ok(None),
            Err(_) => Err("Discord-Mitgliedsprüfung nicht verfügbar".into()),
        }
    }

    async fn evaluate_cycle(&self) -> Result<(), String> {
        let guild_id = i64::try_from(self.guild_id).map_err(|_| "Ungültige Guild-ID")?;
        let excluded = self.excluded_channels()?;
        self.voice_observation(None).await?;
        store::reconcile_attribution(&self.pool, guild_id)
            .await
            .map_err(|error| error.to_string())?;
        let pending = store::pending(&self.pool, guild_id)
            .await
            .map_err(|error| error.to_string())?;
        let mut unavailable = false;
        for invite in pending {
            let now = Utc::now();
            let retention = invite.joined_at + chrono::Duration::days(14);
            let early_leave = invite.left_at.is_some_and(|left| left <= retention);
            if invite.eligible && !early_leave && now < retention {
                continue;
            }
            let proof = if invite.eligible && !early_leave {
                match self.member_proof(invite.user_id).await {
                    Ok(proof) => proof,
                    Err(_) => {
                        unavailable = true;
                        continue;
                    }
                }
            } else {
                None
            };
            store::evaluate_one(&self.pool, &invite, proof.as_ref(), &excluded, Utc::now())
                .await
                .map_err(|error| error.to_string())?;
        }
        if unavailable {
            Err("Einzelne Mitgliedsprüfungen fehlgeschlagen; Einladungen bleiben ausstehend".into())
        } else {
            Ok(())
        }
    }

    fn should_warn(&self) -> bool {
        let Ok(mut warnings) = self.last_warning.lock() else {
            return false;
        };
        warning_allowed(&mut warnings, Instant::now())
    }

    async fn record_qualification_status(&self, succeeded: bool) -> Result<(), String> {
        let interval = i32::try_from(self.config.evaluation_interval_seconds)
            .map_err(|_| "Ungültiges Qualifikationsintervall")?;
        sqlx::query("SELECT bot.record_twitch_invite_qualification($1, $2)")
            .bind(succeeded)
            .bind(interval)
            .execute(&self.pool)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    async fn run_qualification_loop(&self) {
        let mut tick =
            tokio::time::interval(Duration::from_secs(self.config.evaluation_interval_seconds));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let error = match self.evaluate_cycle().await {
                Ok(()) => self.record_qualification_status(true).await.err(),
                Err(cycle_error) => {
                    let marker_error = self.record_qualification_status(false).await.err();
                    Some(match marker_error {
                        Some(marker_error) => format!("{cycle_error}; Qualifikationsstatus konnte nicht gespeichert werden: {marker_error}"),
                        None => cycle_error,
                    })
                }
            };
            if let Some(error) = error {
                if self.should_warn() {
                    tracing::warn!(%error, "Einladungsqualifikation wird erneut geprüft");
                }
            }
        }
    }

    pub fn spawn(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut worker = tokio::task::JoinSet::new();
            let interval = Duration::from_secs(self.config.evaluation_interval_seconds);
            loop {
                let task = self.clone();
                worker.spawn(async move { task.run_qualification_loop().await });
                let exit = match worker.join_next().await {
                    Some(Ok(())) => "returned",
                    Some(Err(error)) if error.is_cancelled() => "cancelled",
                    Some(Err(_)) => "panicked",
                    None => "missing",
                };
                let marker_error = self.record_qualification_status(false).await.err();
                if self.should_warn() {
                    if let Some(error) = marker_error {
                        tracing::warn!(task_exit = exit, %error, "Einladungsqualifikation wird neu gestartet");
                    } else {
                        tracing::warn!(
                            task_exit = exit,
                            "Einladungsqualifikation wird neu gestartet"
                        );
                    }
                }
                tokio::time::sleep(interval).await;
            }
        })
    }
}

#[async_trait::async_trait]
impl TwitchInvitePort for TwitchInvites {
    fn guild_id(&self) -> u64 {
        self.guild_id
    }

    async fn destination(&self, streamer_id: &str) -> Result<Option<InviteDestination>, String> {
        let guild_id = i64::try_from(self.guild_id).map_err(|_| "Ungültige Guild-ID")?;
        let rows: Vec<(String, i64, Option<i64>, Option<String>)> = sqlx::query_as(
            "SELECT streamer_login, guild_id, channel_id, invite_url
             FROM bot.twitch_streamer_invites WHERE twitch_user_id = $1 AND guild_id = $2",
        )
        .bind(streamer_id)
        .bind(guild_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|error| error.to_string())?;
        if rows.len() != 1 {
            return Ok(None);
        }
        let (login, guild_id, channel_id, invite_url) = &rows[0];
        let Some(channel_id) = channel_id
            .and_then(|id| u64::try_from(id).ok())
            .filter(|id| *id > 0)
        else {
            return Ok(None);
        };
        let Some(invite_url) = invite_url
            .as_ref()
            .filter(|url| url.starts_with("https://discord.gg/"))
        else {
            return Ok(None);
        };
        Ok(Some(InviteDestination {
            guild_id: u64::try_from(*guild_id).map_err(|_| "Ungültige Guild-ID")?,
            channel_id,
            streamer_login: login.clone(),
            streamer_twitch_user_id: streamer_id.into(),
            invite_url: invite_url.clone(),
        }))
    }

    async fn personal_invite(
        &self,
        destination: &InviteDestination,
        inviter_id: &str,
    ) -> Result<PersonalInvite, String> {
        personal::resolve(
            &self.pool,
            &self.config,
            destination,
            inviter_id,
            self.adapter.as_ref(),
        )
        .await
    }

    async fn qualified_invites(&self, query: &QualifiedQuery) -> Result<serde_json::Value, String> {
        let guild_id = i64::try_from(self.guild_id).map_err(|_| "Ungültige Guild-ID")?;
        let after = query.after.map(|(updated_at, join_id)| Cursor {
            updated_at,
            join_id,
        });
        let page = store::list_page(
            &self.pool,
            guild_id,
            query.since,
            query.until,
            after.as_ref(),
            query.limit,
        )
        .await
        .map_err(|error| error.to_string())?;
        serde_json::to_value(page).map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod warning_tests {
    use super::warning_allowed;
    use std::{
        collections::VecDeque,
        time::{Duration, Instant},
    };

    #[test]
    fn warning_debounce_caps_repeated_messages_at_two_per_week() {
        let start = Instant::now();
        let mut warnings = VecDeque::new();

        assert!(warning_allowed(&mut warnings, start));
        assert!(!warning_allowed(&mut warnings, start));
        assert!(!warning_allowed(
            &mut warnings,
            start + Duration::from_secs(24 * 60 * 60 - 1)
        ));
        assert!(warning_allowed(
            &mut warnings,
            start + Duration::from_secs(24 * 60 * 60)
        ));
        assert!(!warning_allowed(
            &mut warnings,
            start + Duration::from_secs(2 * 24 * 60 * 60)
        ));
        assert!(warning_allowed(
            &mut warnings,
            start + Duration::from_secs(7 * 24 * 60 * 60)
        ));
        assert!(!warning_allowed(
            &mut warnings,
            start + Duration::from_secs(7 * 24 * 60 * 60 + 1)
        ));
    }
}
