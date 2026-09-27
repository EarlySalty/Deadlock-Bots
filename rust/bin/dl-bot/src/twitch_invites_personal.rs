use super::{store, InviteDestination, PersonalInvite, TwitchInvitesConfig};
use chrono::{DateTime, Utc};
use dl_broker::{port::InviteInfo, DiscordPort};
use dl_discord::DiscordAdapter;
use serenity::all::GuildId;
use sqlx::PgPool;

#[async_trait::async_trait]
pub(super) trait InviteIssuer: Send + Sync {
    async fn count(&self, guild_id: u64) -> Result<usize, String>;
    async fn create(&self, channel_id: u64, reason: &str) -> Result<InviteInfo, String>;
    async fn revoke(&self, code: &str);
}

#[async_trait::async_trait]
impl InviteIssuer for DiscordAdapter {
    async fn count(&self, guild_id: u64) -> Result<usize, String> {
        self.http
            .get_guild_invites(GuildId::new(guild_id))
            .await
            .map(|invites| invites.len())
            .map_err(|_| "Discord-Invitebestand nicht verfügbar".into())
    }

    async fn create(&self, channel_id: u64, reason: &str) -> Result<InviteInfo, String> {
        self.create_invite(channel_id, reason)
            .await
            .map_err(|_| "Discord-Einladung nicht verfügbar".into())
    }

    async fn revoke(&self, code: &str) {
        if self
            .http
            .delete_invite(
                code,
                Some("Twitch-Zuordnung konnte nicht gespeichert werden"),
            )
            .await
            .is_err()
        {
            tracing::warn!("Nicht zugeordnete Discord-Einladung konnte nicht entfernt werden");
        }
    }
}

pub(super) async fn resolve(
    pool: &PgPool,
    config: &TwitchInvitesConfig,
    destination: &InviteDestination,
    inviter_id: &str,
    issuer: &dyn InviteIssuer,
) -> Result<PersonalInvite, String> {
    let fallback = || PersonalInvite {
        invite_url: destination.invite_url.clone(),
        personal: false,
    };
    if inviter_id == destination.streamer_twitch_user_id {
        return Ok(fallback());
    }
    let guild_id = i64::try_from(destination.guild_id).map_err(|_| "Ungültige Guild-ID")?;
    let channel_id = i64::try_from(destination.channel_id).map_err(|_| "Ungültige Kanal-ID")?;
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    store::lock_changes(&mut tx, guild_id)
        .await
        .map_err(|error| error.to_string())?;
    let unchanged: Option<i32> = sqlx::query_scalar(
        "SELECT 1 FROM bot.twitch_streamer_invites
         WHERE twitch_user_id = $1 AND guild_id = $2 AND channel_id = $3
             AND streamer_login = $4 AND invite_url = $5 FOR SHARE",
    )
    .bind(&destination.streamer_twitch_user_id)
    .bind(guild_id)
    .bind(channel_id)
    .bind(&destination.streamer_login)
    .bind(&destination.invite_url)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    if unchanged.is_none() {
        return Err("Streamer-Zuordnung wurde geändert".into());
    }
    let existing: Option<(String, Option<DateTime<Utc>>, i64, i64)> = sqlx::query_as(
        "SELECT invite_url, revoked_at, guild_id, channel_id FROM bot.twitch_personal_invites
         WHERE streamer_twitch_user_id = $1 AND inviter_twitch_user_id = $2",
    )
    .bind(&destination.streamer_twitch_user_id)
    .bind(inviter_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    if let Some((url, revoked, stored_guild, stored_channel)) = existing {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(
            if revoked.is_none() && stored_guild == guild_id && stored_channel == channel_id {
                PersonalInvite {
                    invite_url: url,
                    personal: true,
                }
            } else {
                fallback()
            },
        );
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM bot.twitch_personal_invites WHERE streamer_twitch_user_id = $1",
    )
    .bind(&destination.streamer_twitch_user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    if count >= i64::from(config.personal_links_per_channel) {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(fallback());
    }
    let count = match issuer.count(destination.guild_id).await {
        Ok(count) => count,
        Err(_) => {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(fallback());
        }
    };
    if count >= 1000_usize.saturating_sub(usize::from(config.guild_invite_reserve)) {
        tx.commit().await.map_err(|error| error.to_string())?;
        return Ok(fallback());
    }
    let reason = format!(
        "twitch-personal:{}:{inviter_id}",
        destination.streamer_twitch_user_id
    );
    let invite = match issuer.create(destination.channel_id, &reason).await {
        Ok(invite) if invite.guild_id == destination.guild_id && !invite.code.is_empty() => invite,
        Ok(invite) => {
            issuer.revoke(&invite.code).await;
            return Err("Discord-Einladung gehört nicht zur erwarteten Guild".into());
        }
        Err(_) => {
            tx.commit().await.map_err(|error| error.to_string())?;
            return Ok(fallback());
        }
    };
    let saved = async {
        sqlx::query(
            "INSERT INTO bot.twitch_personal_invites
             (streamer_twitch_user_id, inviter_twitch_user_id, streamer_login, guild_id, channel_id, invite_code, invite_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        ).bind(&destination.streamer_twitch_user_id).bind(inviter_id).bind(&destination.streamer_login)
            .bind(guild_id).bind(channel_id).bind(&invite.code).bind(&invite.invite_url)
            .execute(&mut *tx).await?;
        sqlx::query(
            "INSERT INTO bot.discord_invite_codes (guild_id, invite_code)
             VALUES ($1, $2) ON CONFLICT (guild_id, invite_code) DO UPDATE SET last_seen_at = clock_timestamp()",
        ).bind(guild_id).bind(&invite.code).execute(&mut *tx).await?;
        Ok::<_, sqlx::Error>(())
    }.await;
    if let Err(error) = saved {
        let _ = tx.rollback().await;
        issuer.revoke(&invite.code).await;
        return Err(error.to_string());
    }
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(PersonalInvite {
        invite_url: invite.invite_url,
        personal: true,
    })
}

#[cfg(test)]
#[path = "twitch_invites_personal_tests.rs"]
mod tests;
