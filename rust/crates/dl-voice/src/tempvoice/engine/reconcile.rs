//! Periodischer Abgleich, unabhängig von verlorenen Voice-Ereignissen.

use super::*;
use dl_discord::voice_cache::GuildVoiceSnapshot;
use std::time::Duration;
use tokio::time::{timeout, Instant};

pub const RECONCILE_INTERVAL_SECONDS: u64 = 60;
pub(super) const CLEANUP_TIMEOUT: Duration = Duration::from_secs(10);

impl TempVoiceEngine {
    pub async fn set_voice_tracker(&self, tracker: &Arc<crate::tracker::VoiceTracker>) {
        *self.voice_tracker.write().await = Some(Arc::downgrade(tracker));
    }

    fn cleanup_excluded(&self, channel_id: u64) -> bool {
        self.config.fixed_lane_ids.contains(&channel_id)
            || self.config.staging_channels.contains(&channel_id)
    }

    async fn cleanup_snapshot(&self) -> Option<GuildVoiceSnapshot> {
        match timeout(
            CLEANUP_TIMEOUT,
            self.port.guild_voice_snapshot(self.config.guild_id_hint),
        )
        .await
        {
            Ok(snapshot) => snapshot,
            Err(_) => {
                tracing::warn!("TempVoice: Reconcile-Cache-Abfrage hat Zeitlimit erreicht");
                None
            }
        }
    }

    /// History vor dem Lane-Delete korrigieren, damit dessen Zeitpunkt keinen
    /// erfundenen Session-Abschluss auslöst. Fehler lassen die Lane zum Retry stehen.
    async fn reconcile_history(&self, snapshot: &GuildVoiceSnapshot) -> bool {
        match timeout(
            CLEANUP_TIMEOUT,
            dl_activity::journey::reconcile_voice_open_sessions(&self.store.pool, snapshot),
        )
        .await
        {
            Ok(Ok(closed)) => {
                if closed > 0 {
                    tracing::info!(closed, "TempVoice: offene Voice-Metadaten korrigiert");
                }
            }
            Ok(Err(err)) => {
                tracing::warn!(%err, "TempVoice: Voice-History-Abgleich fehlgeschlagen");
                return false;
            }
            Err(_) => {
                tracing::warn!("TempVoice: Voice-History-Abgleich hat Zeitlimit erreicht");
                return false;
            }
        }
        let tracker = self
            .voice_tracker
            .read()
            .await
            .as_ref()
            .and_then(Weak::upgrade);
        if let Some(tracker) = tracker {
            match timeout(CLEANUP_TIMEOUT, tracker.reconcile_snapshot(snapshot)).await {
                Ok(Ok(_)) => {}
                Ok(Err(err)) => {
                    tracing::warn!(%err, "TempVoice: Session-Recorder-Abgleich fehlgeschlagen");
                    return false;
                }
                Err(_) => {
                    tracing::warn!("TempVoice: Session-Recorder-Abgleich hat Zeitlimit erreicht");
                    return false;
                }
            }
        }
        true
    }

    /// Startup-Vertrag: Cache-Misses überspringen und bekannte leere Kanäle
    /// ohne die zusätzliche Reconcile-Frist entfernen.
    pub async fn purge_empty_lanes(&self) {
        let Some(snapshot) = self.cleanup_snapshot().await else {
            return;
        };
        if !self.reconcile_history(&snapshot).await {
            return;
        }
        let mut lanes: HashSet<u64> = self.state.lock().await.lanes.keys().copied().collect();
        lanes.extend(
            snapshot
                .channels
                .iter()
                .filter_map(|(&channel, &category)| {
                    category
                        .filter(|category| self.config.tempvoice_categories.contains(category))
                        .map(|_| channel)
                }),
        );
        let mut purged = 0;
        for channel_id in lanes {
            if self.cleanup_excluded(channel_id)
                || !snapshot.channels.contains_key(&channel_id)
                || !snapshot.channel_is_empty(channel_id)
            {
                continue;
            }
            if self
                .cleanup_lane(channel_id, "TempVoice: Lane leer (Startup-Purge)")
                .await
            {
                purged += 1;
            }
        }
        if purged > 0 {
            tracing::info!(purged, "TempVoice: leere Lanes beim Start geräumt");
        }
    }

    pub async fn reconcile_lanes(self: &Arc<Self>) {
        self.reconcile_lanes_at(Instant::now()).await;
    }

    pub(super) async fn reconcile_lanes_at(self: &Arc<Self>, now: Instant) {
        let Some(snapshot) = self.cleanup_snapshot().await else {
            // Unbekannt ist kein Beleg für eine fortlaufend leere Lane.
            self.state.lock().await.empty_since.clear();
            tracing::info!("TempVoice: Reconcile wartet auf vollständigen Gateway-Cache");
            return;
        };
        let (candidates, ghosts, checked) = {
            let mut state = self.state.lock().await;
            let mut lanes: HashSet<u64> = state.lanes.keys().copied().collect();
            lanes.extend(state.cleanup_retry.iter().copied());
            lanes.extend(
                snapshot
                    .channels
                    .iter()
                    .filter_map(|(&channel, &category)| {
                        category
                            .filter(|category| self.config.tempvoice_categories.contains(category))
                            .map(|_| channel)
                    }),
            );
            lanes.retain(|channel| !self.cleanup_excluded(*channel));
            state
                .empty_since
                .retain(|channel, _| lanes.contains(channel));
            let mut ghosts = 0;
            let mut candidates = Vec::new();
            for &channel_id in &lanes {
                let members = state.join_time.entry(channel_id).or_default();
                let before = members.len();
                members.retain(|user, joined| {
                    snapshot.members.get(user) == Some(&channel_id)
                        || *joined > snapshot.observed_at.naive_utc()
                });
                ghosts += before - members.len();
                for (&user, &channel) in &snapshot.members {
                    if channel == channel_id {
                        members
                            .entry(user)
                            .or_insert(snapshot.observed_at.naive_utc());
                    }
                }
                let book_empty = members.is_empty();
                if !book_empty || !snapshot.channel_is_empty(channel_id) {
                    state.empty_since.remove(&channel_id);
                    continue;
                }
                let since = *state.empty_since.entry(channel_id).or_insert(now);
                if !snapshot.channels.contains_key(&channel_id)
                    || now.saturating_duration_since(since)
                        >= Duration::from_secs(self.config.empty_lane_grace_seconds)
                {
                    candidates.push(channel_id);
                }
            }
            (candidates, ghosts, lanes.len())
        };
        if ghosts > 0 {
            tracing::info!(
                ghosts,
                "TempVoice: verwaiste Voice-Mitgliedschaften abgeglichen"
            );
        }
        if !self.reconcile_history(&snapshot).await {
            return;
        }
        // Discord wartet intern auf Rate-Limits. Getrennte, begrenzte Requests
        // verhindern, dass ein hängender Delete die übrigen Lanes blockiert.
        let mut deletes = tokio::task::JoinSet::new();
        for channel_id in candidates {
            let engine = self.clone();
            deletes.spawn(async move {
                engine
                    .cleanup_lane(channel_id, "TempVoice: Lane leer (Reconcile)")
                    .await
            });
        }
        let mut removed = 0;
        while let Some(result) = deletes.join_next().await {
            match result {
                Ok(true) => removed += 1,
                Ok(false) => {}
                Err(err) => tracing::warn!(%err, "TempVoice: Reconcile-Delete-Task fehlgeschlagen"),
            }
        }
        tracing::info!(
            checked,
            removed,
            ghosts,
            "TempVoice: Reconcile abgeschlossen"
        );
    }

    pub(super) async fn reconcile_gateway(self: &Arc<Self>, event: GatewayEvent) {
        match event {
            GatewayEvent::Ready { .. } => {
                self.state.lock().await.empty_since.clear();
                self.reconcile_lanes().await;
            }
            // RESUMED und GUILD_CREATE veröffentlichen ebenfalls CacheReady.
            GatewayEvent::CacheReady { guild_ids } => {
                if guild_ids.contains(&self.config.guild_id_hint) {
                    self.reconcile_lanes().await;
                }
            }
        }
    }

    /// Zustand und DB-Eintrag bleiben bei einem fehlgeschlagenen Delete erhalten.
    pub async fn cleanup_lane(&self, channel_id: u64, reason: &str) -> bool {
        if self.cleanup_excluded(channel_id) {
            return false;
        }
        let Some(snapshot) = self.cleanup_snapshot().await else {
            return false;
        };
        // Erneute Prüfung unmittelbar vor REST, nicht nur bei Kandidatenauswahl.
        if !snapshot.channel_is_empty(channel_id)
            || self
                .state
                .lock()
                .await
                .join_time
                .get(&channel_id)
                .is_some_and(|members| !members.is_empty())
        {
            self.state.lock().await.empty_since.remove(&channel_id);
            return false;
        }
        self.state.lock().await.cleanup_retry.insert(channel_id);
        match timeout(
            CLEANUP_TIMEOUT,
            self.port.delete_channel(channel_id, reason),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::warn!(%err, channel_id, "TempVoice: Channel-Delete fehlgeschlagen, nächster Zyklus versucht erneut");
                return false;
            }
            Err(_) => {
                tracing::warn!(channel_id, "TempVoice: Channel-Delete hat Zeitlimit erreicht, nächster Zyklus versucht erneut");
                return false;
            }
        }
        match timeout(CLEANUP_TIMEOUT, self.store.delete_lane(channel_id)).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                tracing::warn!(%err, channel_id, "TempVoice: Lane-DB-Delete fehlgeschlagen");
                return false;
            }
            Err(_) => {
                tracing::warn!(
                    channel_id,
                    "TempVoice: Lane-DB-Delete hat Zeitlimit erreicht"
                );
                return false;
            }
        }
        let lfg = self.lfg.read().await.as_ref().and_then(Weak::upgrade);
        if let Some(lfg) = lfg {
            if timeout(CLEANUP_TIMEOUT, lfg.on_lane_deleted(channel_id))
                .await
                .is_err()
            {
                tracing::warn!(channel_id, "TempVoice: LFG-Cleanup hat Zeitlimit erreicht");
                return false;
            }
        }
        let mut state = self.state.lock().await;
        state.lanes.remove(&channel_id);
        state.join_time.remove(&channel_id);
        state.empty_since.remove(&channel_id);
        state.cleanup_retry.remove(&channel_id);
        state.tag_blocked.remove(&channel_id);
        state.minrank_blocked.remove(&channel_id);
        tracing::info!(channel_id, "TempVoice: Lane gelöscht und Zustand entfernt");
        true
    }
}
