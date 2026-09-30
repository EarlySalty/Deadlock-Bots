use super::*;
use dl_activity::qualified_invite_voice;
use dl_voice::tracker::VoiceActivityObserver;

impl TwitchInvites {
    pub async fn reset_voice_clock(&self) -> Result<(), sqlx::Error> {
        let guild_id = i64::try_from(self.guild_id)
            .map_err(|_| sqlx::Error::Protocol("Ungültige Guild-ID".into()))?;
        self.voice_needs_reset.store(true, Ordering::Release);
        qualified_invite_voice::reset_live_clocks(&self.pool, guild_id).await?;
        self.voice_sequences
            .lock()
            .map_err(|_| sqlx::Error::Protocol("Voice-Sequenzsperre beschädigt".into()))?
            .clear();
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
        if let Some(event) = event {
            let guild = match event {
                dl_discord::VoiceEvent::Join { guild_id, .. }
                | dl_discord::VoiceEvent::Update { guild_id, .. }
                | dl_discord::VoiceEvent::Move { guild_id, .. }
                | dl_discord::VoiceEvent::Leave { guild_id, .. } => *guild_id,
            };
            if guild != self.guild_id {
                return Ok(());
            }
        }
        let Some(snapshot) = self.adapter.voice_cache_snapshot(self.guild_id) else {
            self.reset_voice_clock()
                .await
                .map_err(|error| error.to_string())?;
            return Err("Voice-Cache ist nicht vollständig bestätigt".into());
        };
        if self.voice_generation.load(Ordering::Acquire) != snapshot.generation {
            self.reset_voice_clock()
                .await
                .map_err(|error| error.to_string())?;
            self.voice_generation
                .store(snapshot.generation, Ordering::Release);
        }
        let excluded = self.excluded_channels()?;
        let guild = i64::try_from(self.guild_id).map_err(|_| "Ungültige Guild-ID")?;
        let previous = self
            .voice_sequences
            .lock()
            .map_err(|_| "Voice-Sequenzsperre beschädigt")?
            .clone();
        write_observed_snapshot(&self.pool, guild, &snapshot, &previous, &excluded).await?;
        // Consume transitions only after every write succeeded. Failure makes the
        // next observation reset both persisted clocks and this sequence state.
        *self
            .voice_sequences
            .lock()
            .map_err(|_| "Voice-Sequenzsperre beschädigt")? = snapshot
            .observations
            .iter()
            .map(|(user, value)| (*user, value.sequence))
            .collect();
        Ok(())
    }
}

async fn write_observed_snapshot(
    pool: &PgPool,
    guild: i64,
    snapshot: &dl_discord::voice_cache::GuildVoiceSnapshot,
    previous: &HashMap<u64, u64>,
    excluded: &[i64],
) -> Result<(), String> {
    for (user, observation) in &snapshot.observations {
        let old_sequence = previous.get(user).copied();
        if old_sequence == Some(observation.sequence) {
            continue;
        }
        if old_sequence.and_then(|value| value.checked_add(1)) == Some(observation.sequence) {
            let members = observation
                .channel
                .map(|channel| HashMap::from([(*user, channel)]))
                .unwrap_or_default();
            qualified_invite_voice::record_snapshot(
                pool,
                guild,
                &members,
                excluded,
                observation.changed_at,
                Some(*user),
                true,
            )
            .await
            .map_err(|error| error.to_string())?;
        } else {
            sqlx::query(
                "UPDATE activity.twitch_invite_members SET voice_channel_id=NULL,
                    voice_started_at=NULL, voice_observed_at=NULL WHERE guild_id=$1 AND user_id=$2",
            )
            .bind(guild)
            .bind(i64::try_from(*user).map_err(|_| "Ungültige User-ID")?)
            .execute(pool)
            .await
            .map_err(|error| error.to_string())?;
        }
    }
    qualified_invite_voice::record_snapshot(
        pool,
        guild,
        &snapshot.members,
        excluded,
        snapshot.observed_at,
        None,
        false,
    )
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::voice_cache::{GuildVoiceSnapshot, VoiceObservation};
    mod peer_database {
        include!("../../../test-support/peer_database.rs");
    }

    #[tokio::test]
    async fn delayed_gateway_transition_does_not_turn_fourteen_minutes_into_fifteen() {
        let db = peer_database::database().await;
        let start = Utc::now() - chrono::Duration::minutes(16);
        sqlx::query(
            "INSERT INTO activity.twitch_invite_members
            (guild_id,user_id,first_joined_at,current_joined_at,prior_member,
             voice_channel_id,voice_started_at,voice_observed_at)
            VALUES (1,100,$1,$1,FALSE,10,$1,$2)",
        )
        .bind(start)
        .bind(start + chrono::Duration::minutes(13))
        .execute(db.pool())
        .await
        .expect("live clock");
        let mut snapshot = GuildVoiceSnapshot {
            guild_id: 1,
            generation: 1,
            observed_at: start + chrono::Duration::minutes(16),
            channels: HashMap::from([(10, None), (20, None)]),
            members: HashMap::from([(100, 20)]),
            observations: HashMap::from([(
                100,
                VoiceObservation {
                    sequence: 2,
                    changed_at: start + chrono::Duration::minutes(14),
                    from_channel: Some(10),
                    channel: Some(20),
                    muted_since: None,
                },
            )]),
        };
        write_observed_snapshot(db.pool(), 1, &snapshot, &HashMap::from([(100, 1)]), &[])
            .await
            .expect("delayed move");
        let row: (Option<DateTime<Utc>>,Option<i64>,Option<DateTime<Utc>>) = sqlx::query_as(
            "SELECT voice_qualified_at,voice_channel_id,voice_started_at FROM activity.twitch_invite_members WHERE user_id=100")
            .fetch_one(db.pool()).await.expect("clock after move");
        assert_eq!(
            row,
            (None, Some(20), Some(start + chrono::Duration::minutes(14)))
        );
        // Leave+rejoin was already visible before the old subscriber event drained.
        snapshot
            .observations
            .get_mut(&100)
            .expect("observation")
            .sequence = 4;
        snapshot.observed_at = start + chrono::Duration::minutes(17);
        write_observed_snapshot(db.pool(), 1, &snapshot, &HashMap::from([(100, 2)]), &[])
            .await
            .expect("missed transitions");
        let started: DateTime<Utc> = sqlx::query_scalar(
            "SELECT voice_started_at FROM activity.twitch_invite_members WHERE user_id=100",
        )
        .fetch_one(db.pool())
        .await
        .expect("reset start");
        assert_eq!(started, snapshot.observed_at);
        for minute in 18..=32 {
            snapshot.observed_at = start + chrono::Duration::minutes(minute);
            write_observed_snapshot(db.pool(), 1, &snapshot, &HashMap::from([(100, 4)]), &[])
                .await
                .expect("fresh continuous observation");
        }
        let qualified: DateTime<Utc> = sqlx::query_scalar(
            "SELECT voice_qualified_at FROM activity.twitch_invite_members WHERE user_id=100",
        )
        .fetch_one(db.pool())
        .await
        .expect("new fifteen minutes");
        assert_eq!(qualified, start + chrono::Duration::minutes(32));
    }
}
