use super::*;
use dl_activity::qualified_invite_voice;
use dl_voice::tracker::VoiceActivityObserver;

impl TwitchInvites {
    pub async fn reset_voice_clock(&self) -> Result<(), sqlx::Error> {
        let guild_id = i64::try_from(self.guild_id)
            .map_err(|_| sqlx::Error::Protocol("Ungültige Guild-ID".into()))?;
        self.voice_needs_reset.store(true, Ordering::Release);
        qualified_invite_voice::reset_live_clocks(&self.pool, guild_id).await?;
        self.voice_needs_reset.store(false, Ordering::Release);
        Ok(())
    }

    pub(super) async fn voice_observation(
        &self,
        event: Option<&dl_discord::VoiceEvent>,
    ) -> Result<(), String> {
        let _guard = self.voice_observation_lock.lock().await;
        if self.voice_needs_reset.load(Ordering::Acquire) {
            self.reset_voice_clock()
                .await
                .map_err(|error| error.to_string())?;
        }
        let result = self.voice_observation_inner(event).await;
        if result.is_err() {
            self.voice_needs_reset.store(true, Ordering::Release);
        }
        result
    }

    async fn voice_observation_inner(
        &self,
        event: Option<&dl_discord::VoiceEvent>,
    ) -> Result<(), String> {
        let (event_user, explicit_transition) = match event {
            Some(dl_discord::VoiceEvent::Join {
                guild_id, user_id, ..
            })
            | Some(dl_discord::VoiceEvent::Update {
                guild_id, user_id, ..
            }) => {
                if *guild_id != self.guild_id {
                    return Ok(());
                }
                (Some(*user_id), false)
            }
            Some(dl_discord::VoiceEvent::Move {
                guild_id, user_id, ..
            })
            | Some(dl_discord::VoiceEvent::Leave {
                guild_id, user_id, ..
            }) => {
                if *guild_id != self.guild_id {
                    return Ok(());
                }
                (Some(*user_id), true)
            }
            None => (None, false),
        };
        let Some(mut snapshot) = self.adapter.voice_cache_snapshot(self.guild_id) else {
            self.reset_voice_clock()
                .await
                .map_err(|error| error.to_string())?;
            return Err("Voice-Cache ist nicht vollständig bestätigt".into());
        };
        match event {
            Some(dl_discord::VoiceEvent::Join {
                user_id,
                channel_id,
                ..
            }) => {
                sqlx::query(
                    "UPDATE activity.twitch_invite_members SET voice_channel_id = NULL,
                     voice_started_at = NULL, voice_observed_at = NULL WHERE guild_id = $1 AND user_id = $2",
                ).bind(i64::try_from(self.guild_id).map_err(|_| "Ungültige Guild-ID")?)
                    .bind(i64::try_from(*user_id).map_err(|_| "Ungültige User-ID")?)
                    .execute(&self.pool).await.map_err(|error| error.to_string())?;
                snapshot.members.insert(*user_id, *channel_id);
            }
            Some(dl_discord::VoiceEvent::Update {
                user_id,
                channel_id,
                ..
            }) => {
                snapshot.members.insert(*user_id, *channel_id);
            }
            Some(dl_discord::VoiceEvent::Move {
                user_id,
                to_channel_id,
                ..
            }) => {
                snapshot.members.insert(*user_id, *to_channel_id);
            }
            Some(dl_discord::VoiceEvent::Leave { user_id, .. }) => {
                snapshot.members.remove(user_id);
            }
            None => {}
        }
        let excluded = self.excluded_channels()?;
        qualified_invite_voice::record_snapshot(
            &self.pool,
            i64::try_from(self.guild_id).map_err(|_| "Ungültige Guild-ID")?,
            &snapshot.members,
            &excluded,
            snapshot.observed_at,
            event_user,
            explicit_transition,
        )
        .await
        .map_err(|error| error.to_string())
    }
}

#[async_trait::async_trait]
impl VoiceActivityObserver for TwitchInvites {
    async fn invalidate(&self) {
        let _guard = self.voice_observation_lock.lock().await;
        if let Err(error) = self.reset_voice_clock().await {
            tracing::warn!(%error, "Nicht bestätigte Voice-Uhren konnten nicht zurückgesetzt werden");
        }
    }

    async fn observe_event(&self, event: &dl_discord::VoiceEvent) {
        if let Err(error) = self.voice_observation(Some(event)).await {
            tracing::debug!(%error, "Voice-Nachweis für Einladungen bleibt unbestätigt");
        }
    }

    async fn observe_tick(&self) {
        if let Err(error) = self.voice_observation(None).await {
            tracing::debug!(%error, "Voice-Nachweis für Einladungen bleibt unbestätigt");
        }
    }
}
