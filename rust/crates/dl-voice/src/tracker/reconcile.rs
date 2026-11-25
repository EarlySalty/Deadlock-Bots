//! Konservativer Session-Abschluss nach verlorenen Gateway-Ereignissen.

use super::*;
use dl_discord::voice_cache::GuildVoiceSnapshot;

impl VoiceTracker {
    /// Der Session-Log bleibt beim bisherigen Writer. Kein Feedback und keine
    /// Umfrage für rückwirkend korrigierte Geister-Sessions.
    pub async fn reconcile_snapshot(&self, snapshot: &GuildVoiceSnapshot) -> VoiceDbResult<usize> {
        let observed = snapshot.observed_at.min(Utc::now()).naive_utc();
        let mut state = self.state.lock().await;
        if state
            .observed
            .get(&snapshot.guild_id)
            .is_some_and(|latest| *latest > observed)
        {
            return Ok(0);
        }
        if state
            .generations
            .get(&snapshot.guild_id)
            .is_some_and(|generation| *generation > snapshot.generation)
        {
            return Ok(0);
        }
        state
            .generations
            .insert(snapshot.guild_id, snapshot.generation);
        // This is an ordering watermark, not a persistence acknowledgement.
        // A partial close must still exclude older continuations; equal-time
        // retries remain allowed by the strict comparison above.
        state.observed.insert(snapshot.guild_id, observed);
        let keys: Vec<_> = state
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.guild_id == snapshot.guild_id
                    && (session.generation != snapshot.generation
                        || session.last_update <= observed)
            })
            .map(|(key, _)| *key)
            .collect();
        let mut closed = 0;
        for key in keys {
            let Some(session) = state.sessions.get_mut(&key) else {
                continue;
            };
            let observation = snapshot.observations.get(&session.user_id);
            let sequence = observation.map_or(0, |value| value.sequence);
            if session.generation == snapshot.generation
                && session.sequence == sequence
                && snapshot.members.get(&session.user_id) == Some(&session.channel_id)
            {
                // Mute/grace activity is decided with current role state by
                // update_channel, not from a delayed event's processing time.
                if observation.and_then(|value| value.muted_since).is_none() {
                    session.last_update = observed;
                }
                continue;
            }
            let end_time = observation
                .filter(|value| {
                    session.generation == snapshot.generation
                        && session.sequence.checked_add(1) == Some(value.sequence)
                        && value.from_channel == Some(session.channel_id)
                        && value.channel == snapshot.members.get(&session.user_id).copied()
                })
                .map_or(session.last_update, |value| value.changed_at.naive_utc())
                .min(observed);
            let seconds = (end_time - session.start_time).num_seconds().max(0);
            if seconds > 0 {
                let points = calculate_points(seconds, session.peak_users.max(1));
                // Der Runtime-Lock verhindert einen zweiten parallelen Leave-
                // Abschluss. Bei Persistenzfehler bleibt die Session zum Retry.
                self.persist_finalized_session(session.clone(), end_time, seconds, points)
                    .await?;
            }
            state.sessions.remove(&key);
            state.grace.remove(&key);
            closed += 1;
        }
        if closed > 0 {
            tracing::info!(
                closed,
                guild_id = snapshot.guild_id,
                "TempVoice: verwaiste Sessions am letzten belegten Zeitpunkt geschlossen"
            );
        }
        Ok(closed)
    }

    pub(super) async fn reconcile_keepalive(&self) {
        let guilds: BTreeSet<_> = self
            .state
            .lock()
            .await
            .sessions
            .values()
            .map(|session| session.guild_id)
            .collect();
        for guild_id in guilds {
            let result = tokio::time::timeout(Duration::from_secs(10), async {
                let Some(snapshot) = self.snapshot.guild_voice_snapshot(guild_id).await else {
                    return Ok(0);
                };
                self.reconcile_snapshot(&snapshot).await
            })
            .await;
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => {
                    tracing::warn!(%err, guild_id, "TempVoice: Session-Keepalive-Abgleich fehlgeschlagen")
                }
                Err(_) => tracing::warn!(
                    guild_id,
                    "TempVoice: Session-Keepalive-Abgleich hat Zeitlimit erreicht"
                ),
            }
        }
    }
}
