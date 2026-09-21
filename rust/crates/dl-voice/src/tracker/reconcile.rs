//! Konservativer Session-Abschluss nach verlorenen Gateway-Ereignissen.

use super::*;
use dl_discord::voice_cache::GuildVoiceSnapshot;

impl VoiceTracker {
    /// Der Session-Log bleibt beim bisherigen Writer. Kein Feedback und keine
    /// Umfrage für rückwirkend korrigierte Geister-Sessions.
    pub async fn reconcile_snapshot(&self, snapshot: &GuildVoiceSnapshot) -> VoiceDbResult<usize> {
        let observed = snapshot.observed_at.min(Utc::now()).naive_utc();
        let mut state = self.state.lock().await;
        let keys: Vec<_> = state
            .sessions
            .iter()
            .filter(|(_, session)| {
                session.guild_id == snapshot.guild_id && session.last_update <= observed
            })
            .map(|(key, _)| *key)
            .collect();
        let mut closed = 0;
        for key in keys {
            let Some(session) = state.sessions.get_mut(&key) else {
                continue;
            };
            if snapshot.members.get(&session.user_id) == Some(&session.channel_id) {
                session.last_update = observed;
                continue;
            }
            let end_time = session.last_update.min(observed);
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
