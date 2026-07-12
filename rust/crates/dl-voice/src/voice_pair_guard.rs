use std::sync::Arc;

use dl_discord::{Dispatcher, GatewayEvent, VoiceEvent};
use sqlx::{PgPool, Row};

use crate::{
    db::{i64_to_u64, u64_to_i64},
    glue::{merge_connect_overwrite, CONNECT_BIT},
};

pub const GUILD_ID: u64 = 1_289_721_245_281_292_288;
pub const USER_A: u64 = 887_664_726_421_671_976;
pub const USER_B: u64 = 279_971_744_964_542_464;

pub type GuardResult<T> = Result<T, VoicePairGuardError>;
pub type VoicePairOperationLock = tokio::sync::Mutex<()>;

#[derive(Debug, thiserror::Error)]
pub enum VoicePairGuardError {
    #[error(transparent)]
    VoiceDb(#[from] crate::db::VoiceDbError),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("Discord-Permission fehlgeschlagen: {0}")]
    Discord(String),
    #[error("Discord-Cache nicht verfuegbar: {0}")]
    CacheUnavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePairLock {
    pub guild_id: u64,
    pub channel_id: u64,
    pub blocked_user_id: u64,
    pub previous_allow: u64,
    pub previous_deny: u64,
    pub had_overwrite: bool,
    pub active: bool,
}

pub fn counterpart(user_id: u64) -> Option<u64> {
    match user_id {
        USER_A => Some(USER_B),
        USER_B => Some(USER_A),
        _ => None,
    }
}

#[async_trait::async_trait]
pub trait VoicePairStore: Send + Sync {
    async fn insert_lock(&self, lock: VoicePairLock) -> GuardResult<bool>;
    async fn lock(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
    ) -> GuardResult<Option<VoicePairLock>>;
    async fn list_locks(&self, guild_id: u64) -> GuardResult<Vec<VoicePairLock>>;
    async fn update_restore_connect(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
        connect: Option<bool>,
    ) -> GuardResult<()>;
    async fn set_active(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
        active: bool,
    ) -> GuardResult<()>;
    async fn delete_lock(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
    ) -> GuardResult<()>;
}

pub async fn resolve_member_connect(
    store: &dyn VoicePairStore,
    guild_id: u64,
    channel_id: u64,
    user_id: u64,
    requested: Option<bool>,
) -> GuardResult<Option<bool>> {
    let Some(lock) = store.lock(guild_id, channel_id, user_id).await? else {
        return Ok(requested);
    };
    store
        .update_restore_connect(guild_id, channel_id, user_id, requested)
        .await?;
    Ok(lock.active.then_some(false).or(requested))
}

pub async fn compose_member_connect(
    store: &dyn VoicePairStore,
    guild_id: u64,
    channel_id: u64,
    user_id: u64,
    requested: Option<bool>,
) -> GuardResult<Option<bool>> {
    Ok(
        if store
            .lock(guild_id, channel_id, user_id)
            .await?
            .is_some_and(|lock| lock.active)
        {
            Some(false)
        } else {
            requested
        },
    )
}

#[derive(Clone)]
pub struct VoicePairGuardStore {
    pool: PgPool,
}

impl VoicePairGuardStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait::async_trait]
impl VoicePairStore for VoicePairGuardStore {
    async fn insert_lock(&self, lock: VoicePairLock) -> GuardResult<bool> {
        let result = sqlx::query(
            r#"
            INSERT INTO voice.voice_pair_guard_locks (
                guild_id, channel_id, blocked_user_id,
                previous_allow, previous_deny, had_overwrite, active
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (guild_id, channel_id, blocked_user_id)
            DO UPDATE SET active = TRUE
            "#,
        )
        .bind(u64_to_i64(
            "voice_pair_guard_locks.guild_id",
            lock.guild_id,
        )?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.channel_id",
            lock.channel_id,
        )?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.blocked_user_id",
            lock.blocked_user_id,
        )?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.previous_allow",
            lock.previous_allow,
        )?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.previous_deny",
            lock.previous_deny,
        )?)
        .bind(lock.had_overwrite)
        .bind(lock.active)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn lock(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
    ) -> GuardResult<Option<VoicePairLock>> {
        let row = sqlx::query(
            r#"
            SELECT guild_id, channel_id, blocked_user_id,
                   previous_allow, previous_deny, had_overwrite, active
              FROM voice.voice_pair_guard_locks
             WHERE guild_id = $1 AND channel_id = $2 AND blocked_user_id = $3
            "#,
        )
        .bind(u64_to_i64("voice_pair_guard_locks.guild_id", guild_id)?)
        .bind(u64_to_i64("voice_pair_guard_locks.channel_id", channel_id)?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.blocked_user_id",
            blocked_user_id,
        )?)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(VoicePairLock {
                guild_id: i64_to_u64("voice_pair_guard_locks.guild_id", row.try_get(0)?)?,
                channel_id: i64_to_u64("voice_pair_guard_locks.channel_id", row.try_get(1)?)?,
                blocked_user_id: i64_to_u64(
                    "voice_pair_guard_locks.blocked_user_id",
                    row.try_get(2)?,
                )?,
                previous_allow: i64_to_u64(
                    "voice_pair_guard_locks.previous_allow",
                    row.try_get(3)?,
                )?,
                previous_deny: i64_to_u64("voice_pair_guard_locks.previous_deny", row.try_get(4)?)?,
                had_overwrite: row.try_get(5)?,
                active: row.try_get(6)?,
            })
        })
        .transpose()
    }

    async fn list_locks(&self, guild_id: u64) -> GuardResult<Vec<VoicePairLock>> {
        let rows = sqlx::query(
            r#"
            SELECT guild_id, channel_id, blocked_user_id,
                   previous_allow, previous_deny, had_overwrite, active
              FROM voice.voice_pair_guard_locks
             WHERE guild_id = $1
            "#,
        )
        .bind(u64_to_i64("voice_pair_guard_locks.guild_id", guild_id)?)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(VoicePairLock {
                    guild_id: i64_to_u64("voice_pair_guard_locks.guild_id", row.try_get(0)?)?,
                    channel_id: i64_to_u64("voice_pair_guard_locks.channel_id", row.try_get(1)?)?,
                    blocked_user_id: i64_to_u64(
                        "voice_pair_guard_locks.blocked_user_id",
                        row.try_get(2)?,
                    )?,
                    previous_allow: i64_to_u64(
                        "voice_pair_guard_locks.previous_allow",
                        row.try_get(3)?,
                    )?,
                    previous_deny: i64_to_u64(
                        "voice_pair_guard_locks.previous_deny",
                        row.try_get(4)?,
                    )?,
                    had_overwrite: row.try_get(5)?,
                    active: row.try_get(6)?,
                })
            })
            .collect()
    }

    async fn update_restore_connect(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
        connect: Option<bool>,
    ) -> GuardResult<()> {
        let Some(lock) = self.lock(guild_id, channel_id, blocked_user_id).await? else {
            return Ok(());
        };
        let merged = merge_connect_overwrite(lock.previous_allow, lock.previous_deny, connect);
        let (allow, deny) = merged.unwrap_or_default();
        sqlx::query(
            r#"
            UPDATE voice.voice_pair_guard_locks
               SET previous_allow = $4, previous_deny = $5, had_overwrite = $6
             WHERE guild_id = $1 AND channel_id = $2 AND blocked_user_id = $3
            "#,
        )
        .bind(u64_to_i64("voice_pair_guard_locks.guild_id", guild_id)?)
        .bind(u64_to_i64("voice_pair_guard_locks.channel_id", channel_id)?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.blocked_user_id",
            blocked_user_id,
        )?)
        .bind(u64_to_i64("voice_pair_guard_locks.previous_allow", allow)?)
        .bind(u64_to_i64("voice_pair_guard_locks.previous_deny", deny)?)
        .bind(merged.is_some())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete_lock(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
    ) -> GuardResult<()> {
        sqlx::query(
            r#"
            DELETE FROM voice.voice_pair_guard_locks
             WHERE guild_id = $1 AND channel_id = $2 AND blocked_user_id = $3
            "#,
        )
        .bind(u64_to_i64("voice_pair_guard_locks.guild_id", guild_id)?)
        .bind(u64_to_i64("voice_pair_guard_locks.channel_id", channel_id)?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.blocked_user_id",
            blocked_user_id,
        )?)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn set_active(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
        active: bool,
    ) -> GuardResult<()> {
        sqlx::query(
            r#"
            UPDATE voice.voice_pair_guard_locks
               SET active = $4
             WHERE guild_id = $1 AND channel_id = $2 AND blocked_user_id = $3
            "#,
        )
        .bind(u64_to_i64("voice_pair_guard_locks.guild_id", guild_id)?)
        .bind(u64_to_i64("voice_pair_guard_locks.channel_id", channel_id)?)
        .bind(u64_to_i64(
            "voice_pair_guard_locks.blocked_user_id",
            blocked_user_id,
        )?)
        .bind(active)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait::async_trait]
pub trait VoicePairPort: Send + Sync {
    async fn member_overwrite_raw(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
    ) -> GuardResult<Option<(u64, u64)>>;
    async fn set_member_overwrite_raw(
        &self,
        guild_id: u64,
        channel_id: u64,
        user_id: u64,
        overwrite: Option<(u64, u64)>,
    ) -> GuardResult<()>;
    async fn voice_channel(&self, guild_id: u64, user_id: u64) -> GuardResult<Option<u64>>;
}

pub struct VoicePairGuard {
    store: Arc<dyn VoicePairStore>,
    port: Arc<dyn VoicePairPort>,
    operations: Arc<VoicePairOperationLock>,
}

impl VoicePairGuard {
    pub fn new(
        store: Arc<dyn VoicePairStore>,
        port: Arc<dyn VoicePairPort>,
        operations: Arc<VoicePairOperationLock>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            port,
            operations,
        })
    }

    async fn apply_lock(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
    ) -> GuardResult<()> {
        let _operation = self.operations.lock().await;
        let existing = self
            .store
            .lock(guild_id, channel_id, blocked_user_id)
            .await?;
        if existing.is_some() {
            self.store
                .set_active(guild_id, channel_id, blocked_user_id, true)
                .await?;
        }
        let previous = self
            .port
            .member_overwrite_raw(guild_id, channel_id, blocked_user_id)
            .await?;
        let (previous_allow, previous_deny) = previous.unwrap_or_default();
        if existing.is_none() {
            self.store
                .insert_lock(VoicePairLock {
                    guild_id,
                    channel_id,
                    blocked_user_id,
                    previous_allow,
                    previous_deny,
                    had_overwrite: previous.is_some(),
                    active: true,
                })
                .await?;
        }
        let overwrite = merge_connect_overwrite(previous_allow, previous_deny, Some(false));
        self.port
            .set_member_overwrite_raw(guild_id, channel_id, blocked_user_id, overwrite)
            .await
    }

    pub async fn restore(
        &self,
        guild_id: u64,
        channel_id: u64,
        blocked_user_id: u64,
    ) -> GuardResult<()> {
        let _operation = self.operations.lock().await;
        let Some(lock) = self
            .store
            .lock(guild_id, channel_id, blocked_user_id)
            .await?
        else {
            return Ok(());
        };
        self.store
            .set_active(guild_id, channel_id, blocked_user_id, false)
            .await?;
        let current = self
            .port
            .member_overwrite_raw(guild_id, channel_id, blocked_user_id)
            .await?
            .unwrap_or_default();
        let previous_connect = if lock.previous_deny & CONNECT_BIT != 0 {
            Some(false)
        } else if lock.previous_allow & CONNECT_BIT != 0 {
            Some(true)
        } else {
            None
        };
        let restore = merge_connect_overwrite(current.0, current.1, previous_connect);
        self.port
            .set_member_overwrite_raw(guild_id, channel_id, blocked_user_id, restore)
            .await?;
        self.store
            .delete_lock(guild_id, channel_id, blocked_user_id)
            .await?;
        Ok(())
    }

    async fn apply_for_user(&self, guild_id: u64, channel_id: u64, user_id: u64) {
        let Some(blocked_user_id) = counterpart(user_id).filter(|_| guild_id == GUILD_ID) else {
            return;
        };
        if let Err(err) = self.apply_lock(guild_id, channel_id, blocked_user_id).await {
            tracing::warn!(%err, guild_id, channel_id, user_id, blocked_user_id, "Voice-Pair-Guard: Sperre fehlgeschlagen");
        }
    }

    async fn restore_for_user(&self, guild_id: u64, channel_id: u64, user_id: u64) {
        let Some(blocked_user_id) = counterpart(user_id).filter(|_| guild_id == GUILD_ID) else {
            return;
        };
        if let Err(err) = self.restore(guild_id, channel_id, blocked_user_id).await {
            tracing::warn!(%err, guild_id, channel_id, user_id, blocked_user_id, "Voice-Pair-Guard: Restore fehlgeschlagen");
        }
    }

    pub async fn handle_event(&self, event: VoiceEvent) {
        match event {
            VoiceEvent::Join {
                guild_id,
                user_id,
                channel_id,
            } => self.apply_for_user(guild_id, channel_id, user_id).await,
            VoiceEvent::Leave {
                guild_id,
                user_id,
                channel_id,
            } => self.restore_for_user(guild_id, channel_id, user_id).await,
            VoiceEvent::Move {
                guild_id,
                user_id,
                from_channel_id,
                to_channel_id,
            } => {
                self.apply_for_user(guild_id, to_channel_id, user_id).await;
                self.restore_for_user(guild_id, from_channel_id, user_id)
                    .await;
            }
            VoiceEvent::Update { .. } => {}
        }
    }

    pub async fn reconcile(&self, guild_id: u64) {
        if guild_id != GUILD_ID {
            return;
        }
        let mut channels = Vec::with_capacity(2);
        for user_id in [USER_A, USER_B] {
            let channel_id = match self.port.voice_channel(guild_id, user_id).await {
                Ok(channel_id) => channel_id,
                Err(err) => {
                    tracing::warn!(%err, guild_id, user_id, "Voice-Pair-Guard: Cache-Reconciliation fehlgeschlagen");
                    return;
                }
            };
            if let Some(channel_id) = channel_id {
                self.apply_for_user(guild_id, channel_id, user_id).await;
            }
            channels.push((user_id, channel_id));
        }
        let locks = match self.store.list_locks(guild_id).await {
            Ok(locks) => locks,
            Err(err) => {
                tracing::warn!(%err, guild_id, "Voice-Pair-Guard: Lock-Reconciliation fehlgeschlagen");
                return;
            }
        };
        for lock in locks {
            let counterpart_present = match counterpart(lock.blocked_user_id) {
                Some(user_id) => channels.contains(&(user_id, Some(lock.channel_id))),
                None => false,
            };
            if counterpart_present {
                continue;
            }
            if let Err(err) = self
                .restore(guild_id, lock.channel_id, lock.blocked_user_id)
                .await
            {
                tracing::warn!(%err, guild_id, channel_id = lock.channel_id, blocked_user_id = lock.blocked_user_id, "Voice-Pair-Guard: stale Lock-Restore fehlgeschlagen");
            }
        }
    }
}

pub fn spawn(guard: Arc<VoicePairGuard>, dispatcher: &Dispatcher) -> tokio::task::JoinHandle<()> {
    let mut voice_events = dispatcher.subscribe_voice();
    let mut gateway_events = dispatcher.subscribe_gateway();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                event = voice_events.recv() => match event {
                    Ok(event) => guard.handle_event(event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "Voice-Pair-Guard: Voice-Events verpasst");
                        guard.reconcile(GUILD_ID).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                event = gateway_events.recv() => match event {
                    Ok(GatewayEvent::CacheReady { guild_ids }) if guild_ids.contains(&GUILD_ID) => {
                        guard.reconcile(GUILD_ID).await;
                    }
                    Ok(GatewayEvent::Ready { .. } | GatewayEvent::CacheReady { .. }) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                        tracing::warn!(missed, "Voice-Pair-Guard: Gateway-Events verpasst");
                        guard.reconcile(GUILD_ID).await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::{
            atomic::{AtomicBool, AtomicUsize, Ordering},
            Arc,
        },
    };

    use dl_discord::VoiceEvent;
    use tokio::sync::{Mutex, Notify};

    use super::*;

    type LockKey = (u64, u64, u64);
    type OverwriteBits = (u64, u64);

    #[derive(Default)]
    struct MemoryVoicePairStore {
        locks: Mutex<HashMap<LockKey, VoicePairLock>>,
        fail_set_active: AtomicBool,
        fail_delete: AtomicBool,
        insert_calls: AtomicUsize,
        delete_calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl VoicePairStore for MemoryVoicePairStore {
        async fn insert_lock(&self, lock: VoicePairLock) -> GuardResult<bool> {
            self.insert_calls.fetch_add(1, Ordering::SeqCst);
            let mut locks = self.locks.lock().await;
            let key = (lock.guild_id, lock.channel_id, lock.blocked_user_id);
            if let Some(existing) = locks.get_mut(&key) {
                existing.active = true;
                return Ok(false);
            }
            locks.insert(key, lock);
            Ok(true)
        }

        async fn lock(
            &self,
            guild_id: u64,
            channel_id: u64,
            blocked_user_id: u64,
        ) -> GuardResult<Option<VoicePairLock>> {
            Ok(self
                .locks
                .lock()
                .await
                .get(&(guild_id, channel_id, blocked_user_id))
                .cloned())
        }

        async fn list_locks(&self, guild_id: u64) -> GuardResult<Vec<VoicePairLock>> {
            Ok(self
                .locks
                .lock()
                .await
                .values()
                .filter(|lock| lock.guild_id == guild_id)
                .cloned()
                .collect())
        }

        async fn update_restore_connect(
            &self,
            guild_id: u64,
            channel_id: u64,
            blocked_user_id: u64,
            connect: Option<bool>,
        ) -> GuardResult<()> {
            let mut locks = self.locks.lock().await;
            let Some(lock) = locks.get_mut(&(guild_id, channel_id, blocked_user_id)) else {
                return Ok(());
            };
            let merged = crate::glue::merge_connect_overwrite(
                lock.previous_allow,
                lock.previous_deny,
                connect,
            );
            let (allow, deny) = merged.unwrap_or_default();
            lock.previous_allow = allow;
            lock.previous_deny = deny;
            lock.had_overwrite = merged.is_some();
            Ok(())
        }

        async fn delete_lock(
            &self,
            guild_id: u64,
            channel_id: u64,
            blocked_user_id: u64,
        ) -> GuardResult<()> {
            self.delete_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail_delete.swap(false, Ordering::SeqCst) {
                return Err(VoicePairGuardError::Discord(
                    "Test-DB-Delete fehlgeschlagen".to_string(),
                ));
            }
            self.locks
                .lock()
                .await
                .remove(&(guild_id, channel_id, blocked_user_id));
            Ok(())
        }

        async fn set_active(
            &self,
            guild_id: u64,
            channel_id: u64,
            blocked_user_id: u64,
            active: bool,
        ) -> GuardResult<()> {
            if self.fail_set_active.swap(false, Ordering::SeqCst) {
                return Err(VoicePairGuardError::Discord(
                    "Test-DB-Status fehlgeschlagen".to_string(),
                ));
            }
            if let Some(lock) =
                self.locks
                    .lock()
                    .await
                    .get_mut(&(guild_id, channel_id, blocked_user_id))
            {
                lock.active = active;
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockVoicePairPort {
        overwrites: Mutex<HashMap<LockKey, OverwriteBits>>,
        voice_channels: Mutex<HashMap<(u64, u64), u64>>,
        channels: Mutex<HashSet<(u64, u64)>>,
        writes: Mutex<Vec<u64>>,
        fail_write: AtomicBool,
        cache_unavailable: AtomicBool,
    }

    impl MockVoicePairPort {
        async fn connect_state(&self, channel_id: u64, user_id: u64) -> Option<bool> {
            let (allow, deny) = self.member_overwrite(channel_id, user_id).await?;
            let bit = 1 << 20;
            if deny & bit != 0 {
                Some(false)
            } else if allow & bit != 0 {
                Some(true)
            } else {
                None
            }
        }

        async fn member_overwrite(&self, channel_id: u64, user_id: u64) -> Option<(u64, u64)> {
            self.overwrites
                .lock()
                .await
                .get(&(GUILD_ID, channel_id, user_id))
                .copied()
        }

        async fn channels_written(&self) -> Vec<u64> {
            self.writes.lock().await.clone()
        }

        async fn set_voice_channel(&self, guild_id: u64, user_id: u64, channel_id: Option<u64>) {
            let mut voice_channels = self.voice_channels.lock().await;
            if let Some(channel_id) = channel_id {
                voice_channels.insert((guild_id, user_id), channel_id);
            } else {
                voice_channels.remove(&(guild_id, user_id));
            }
        }

        async fn member_move_calls(&self) -> usize {
            0
        }
    }

    #[async_trait::async_trait]
    impl VoicePairPort for MockVoicePairPort {
        async fn member_overwrite_raw(
            &self,
            guild_id: u64,
            channel_id: u64,
            user_id: u64,
        ) -> GuardResult<Option<(u64, u64)>> {
            if !self.channels.lock().await.contains(&(guild_id, channel_id)) {
                return Err(VoicePairGuardError::CacheUnavailable(
                    "Test-Channel fehlt".to_string(),
                ));
            }
            Ok(self
                .overwrites
                .lock()
                .await
                .get(&(guild_id, channel_id, user_id))
                .copied())
        }

        async fn set_member_overwrite_raw(
            &self,
            guild_id: u64,
            channel_id: u64,
            user_id: u64,
            overwrite: Option<(u64, u64)>,
        ) -> GuardResult<()> {
            if self.fail_write.swap(false, Ordering::SeqCst) {
                return Err(VoicePairGuardError::Discord(
                    "Test-Discord-Write fehlgeschlagen".to_string(),
                ));
            }
            let mut overwrites = self.overwrites.lock().await;
            let key = (guild_id, channel_id, user_id);
            if let Some(overwrite) = overwrite {
                overwrites.insert(key, overwrite);
            } else {
                overwrites.remove(&key);
            }
            self.writes.lock().await.push(channel_id);
            Ok(())
        }

        async fn voice_channel(&self, guild_id: u64, user_id: u64) -> GuardResult<Option<u64>> {
            if self.cache_unavailable.load(Ordering::SeqCst) {
                return Err(VoicePairGuardError::CacheUnavailable(
                    "Test-Guild fehlt".to_string(),
                ));
            }
            Ok(self
                .voice_channels
                .lock()
                .await
                .get(&(guild_id, user_id))
                .copied())
        }
    }

    async fn setup_guard_with_previous(
        previous: Option<(u64, u64)>,
    ) -> (
        Arc<VoicePairGuard>,
        Arc<MockVoicePairPort>,
        Arc<MemoryVoicePairStore>,
    ) {
        let port = Arc::new(MockVoicePairPort::default());
        port.channels
            .lock()
            .await
            .extend([(GUILD_ID, 42), (GUILD_ID, 43)]);
        if let Some(previous) = previous {
            port.overwrites
                .lock()
                .await
                .insert((GUILD_ID, 42, USER_B), previous);
        }
        let store = Arc::new(MemoryVoicePairStore::default());
        let guard = VoicePairGuard::new(
            store.clone(),
            port.clone(),
            Arc::new(VoicePairOperationLock::new(())),
        );
        (guard, port, store)
    }

    async fn setup_guard() -> (
        Arc<VoicePairGuard>,
        Arc<MockVoicePairPort>,
        Arc<MemoryVoicePairStore>,
    ) {
        setup_guard_with_previous(None).await
    }

    #[tokio::test]
    async fn gemeinsamer_mutex_serialisiert_guard_und_tempvoice_schreibpfade() {
        let port = Arc::new(MockVoicePairPort::default());
        port.channels.lock().await.insert((GUILD_ID, 42));
        let store = Arc::new(MemoryVoicePairStore::default());
        let operations = Arc::new(Mutex::new(()));
        let tempvoice_started = Arc::new(Notify::new());
        let release_tempvoice = Arc::new(Notify::new());
        let tempvoice_task = {
            let operations = operations.clone();
            let tempvoice_started = tempvoice_started.clone();
            let release_tempvoice = release_tempvoice.clone();
            tokio::spawn(async move {
                let _operation = operations.lock().await;
                tempvoice_started.notify_one();
                release_tempvoice.notified().await;
            })
        };
        tempvoice_started.notified().await;

        let guard = VoicePairGuard::new(store.clone(), port, operations);
        let guard_task = tokio::spawn(async move {
            guard
                .handle_event(VoiceEvent::Join {
                    guild_id: GUILD_ID,
                    user_id: USER_A,
                    channel_id: 42,
                })
                .await;
        });
        tokio::task::yield_now().await;

        assert!(store.list_locks(GUILD_ID).await.expect("locks").is_empty());
        release_tempvoice.notify_one();
        tempvoice_task.await.expect("tempvoice task");
        guard_task.await.expect("guard task");
        assert_eq!(store.list_locks(GUILD_ID).await.expect("locks").len(), 1);
    }

    async fn setup_guard_with_lock(
        channel_id: u64,
        blocked_user_id: u64,
        previous: Option<(u64, u64)>,
    ) -> (
        Arc<VoicePairGuard>,
        Arc<MockVoicePairPort>,
        Arc<MemoryVoicePairStore>,
    ) {
        let (guard, port, store) = setup_guard().await;
        let (previous_allow, previous_deny) = previous.unwrap_or_default();
        store
            .insert_lock(VoicePairLock {
                guild_id: GUILD_ID,
                channel_id,
                blocked_user_id,
                previous_allow,
                previous_deny,
                had_overwrite: previous.is_some(),
                active: true,
            })
            .await
            .expect("lock");
        port.overwrites
            .lock()
            .await
            .insert((GUILD_ID, channel_id, blocked_user_id), (0, 1 << 20));
        (guard, port, store)
    }

    #[tokio::test]
    async fn join_sperrt_den_gegenpart_auf_jedem_voice_channel() {
        let (guard, port, _) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;
        assert_eq!(port.connect_state(42, USER_B).await, Some(false));
    }

    #[tokio::test]
    async fn beide_nutzer_haben_identische_guard_rechte() {
        let (guard, port, _) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_B,
                channel_id: 43,
            })
            .await;
        assert_eq!(port.connect_state(43, USER_A).await, Some(false));
    }

    #[tokio::test]
    async fn move_sperrt_zuerst_neu_und_restauriert_dann_alt() {
        let (guard, port, _) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;
        port.writes.lock().await.clear();

        guard
            .handle_event(VoiceEvent::Move {
                guild_id: GUILD_ID,
                user_id: USER_A,
                from_channel_id: 42,
                to_channel_id: 43,
            })
            .await;

        assert_eq!(port.channels_written().await, vec![43, 42]);
    }

    #[tokio::test]
    async fn restore_aendert_nur_connect_und_erhaelt_aktuelle_fremde_bits() {
        let current_allow = 1 << 22;
        let current_deny = (1 << 23) | (1 << 20);
        for (previous, expected) in [
            (None, Some((current_allow, 1 << 23))),
            (
                Some(((1 << 10) | (1 << 20), 0)),
                Some((current_allow | (1 << 20), 1 << 23)),
            ),
            (
                Some((0, (1 << 11) | (1 << 20))),
                Some((current_allow, current_deny)),
            ),
        ] {
            let (guard, port, _) = setup_guard_with_previous(previous).await;
            guard
                .handle_event(VoiceEvent::Join {
                    guild_id: GUILD_ID,
                    user_id: USER_A,
                    channel_id: 42,
                })
                .await;
            port.overwrites
                .lock()
                .await
                .insert((GUILD_ID, 42, USER_B), (current_allow, current_deny));
            guard
                .handle_event(VoiceEvent::Leave {
                    guild_id: GUILD_ID,
                    user_id: USER_A,
                    channel_id: 42,
                })
                .await;
            assert_eq!(port.member_overwrite(42, USER_B).await, expected);
        }
    }

    #[tokio::test]
    async fn delete_fehler_hinterlaesst_restauriertes_discord_und_db_lock() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        store.fail_delete.store(true, Ordering::SeqCst);

        assert!(guard.restore(GUILD_ID, 42, USER_B).await.is_err());

        assert_eq!(port.member_overwrite(42, USER_B).await, None);
        assert_eq!(port.channels_written().await, vec![42]);
        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| !lock.active));
    }

    #[tokio::test]
    async fn discord_restore_fehler_verzichtet_auf_best_effort_reinsert() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        port.fail_write.store(true, Ordering::SeqCst);

        assert!(guard.restore(GUILD_ID, 42, USER_B).await.is_err());

        assert_eq!(store.delete_calls.load(Ordering::SeqCst), 0);
        assert_eq!(store.insert_calls.load(Ordering::SeqCst), 1);
        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| !lock.active));
    }

    #[tokio::test]
    async fn status_fehler_aendert_discord_nicht() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        store.fail_set_active.store(true, Ordering::SeqCst);

        assert!(guard.restore(GUILD_ID, 42, USER_B).await.is_err());

        assert_eq!(
            port.member_overwrite(42, USER_B).await,
            Some((0, CONNECT_BIT))
        );
        assert!(port.channels_written().await.is_empty());
        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| lock.active));
    }

    #[tokio::test]
    async fn inaktiver_cleanup_lock_erzwingt_keinen_deny_und_merkt_owner_wunsch() {
        let (_, _, store) = setup_guard_with_lock(42, USER_B, None).await;
        store
            .set_active(GUILD_ID, 42, USER_B, false)
            .await
            .expect("inactive");

        assert_eq!(
            resolve_member_connect(store.as_ref(), GUILD_ID, 42, USER_B, Some(true))
                .await
                .expect("resolve"),
            Some(true)
        );
        let lock = store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .expect("lock");
        assert!(!lock.active);
        assert_eq!(lock.previous_allow & CONNECT_BIT, CONNECT_BIT);
        assert_eq!(lock.previous_deny & CONNECT_BIT, 0);
    }

    #[tokio::test]
    async fn apply_lock_aktiviert_inaktiven_lock_erneut() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        store
            .set_active(GUILD_ID, 42, USER_B, false)
            .await
            .expect("inactive");

        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| lock.active));
        assert_eq!(port.connect_state(42, USER_B).await, Some(false));
    }

    #[tokio::test]
    async fn apply_lock_persistiert_reaktivierung_vor_cache_read() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        store
            .set_active(GUILD_ID, 42, USER_B, false)
            .await
            .expect("inactive");
        port.channels.lock().await.remove(&(GUILD_ID, 42));

        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| lock.active));
    }

    #[tokio::test]
    async fn neuer_lock_bleibt_nach_discord_fehler_aktiv() {
        let (guard, port, store) = setup_guard().await;
        port.fail_write.store(true, Ordering::SeqCst);

        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| lock.active));
    }

    #[tokio::test]
    async fn unbeteiligte_guilds_und_nutzer_bleiben_unveraendert() {
        let (guard, port, _) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: 1,
                user_id: 2,
                channel_id: 3,
            })
            .await;
        assert!(port.channels_written().await.is_empty());
    }

    #[tokio::test]
    async fn aktiver_guard_haelt_deny_trotz_owner_unban() {
        let (guard, port, store) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        let effective = resolve_member_connect(store.as_ref(), GUILD_ID, 42, USER_B, None)
            .await
            .expect("resolve");
        let current = port.member_overwrite(42, USER_B).await.unwrap_or_default();
        port.set_member_overwrite_raw(
            GUILD_ID,
            42,
            USER_B,
            crate::glue::merge_connect_overwrite(current.0, current.1, effective),
        )
        .await
        .expect("overwrite");

        assert_eq!(port.connect_state(42, USER_B).await, Some(false));
    }

    #[tokio::test]
    async fn owner_unban_aktualisiert_waehrend_lock_nur_den_restore_zustand() {
        let (guard, _, store) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        assert_eq!(
            resolve_member_connect(store.as_ref(), GUILD_ID, 42, USER_B, None)
                .await
                .expect("resolve"),
            Some(false)
        );
        let restore = store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .expect("lock");
        assert!(!restore.had_overwrite);
        assert_eq!((restore.previous_allow, restore.previous_deny), (0, 0));
    }

    #[tokio::test]
    async fn stale_batch_nach_owner_unban_aendert_restore_wunsch_nicht() {
        let (guard, _, store) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        resolve_member_connect(store.as_ref(), GUILD_ID, 42, USER_B, None)
            .await
            .expect("owner unban");
        compose_member_connect(store.as_ref(), GUILD_ID, 42, USER_B, Some(false))
            .await
            .expect("stale batch");

        let lock = store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .expect("lock");
        assert!(!lock.had_overwrite);
        assert_eq!((lock.previous_allow, lock.previous_deny), (0, 0));
    }

    #[tokio::test]
    async fn normaler_owner_ban_bleibt_nach_guard_leave_erhalten() {
        let (guard, port, store) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;

        let effective = resolve_member_connect(store.as_ref(), GUILD_ID, 42, USER_B, Some(false))
            .await
            .expect("resolve");
        let current = port.member_overwrite(42, USER_B).await.unwrap_or_default();
        port.set_member_overwrite_raw(
            GUILD_ID,
            42,
            USER_B,
            crate::glue::merge_connect_overwrite(current.0, current.1, effective),
        )
        .await
        .expect("overwrite");
        guard.restore(GUILD_ID, 42, USER_B).await.expect("restore");

        assert_eq!(port.connect_state(42, USER_B).await, Some(false));
    }

    #[tokio::test]
    async fn reconcile_setzt_fehlenden_lock_fuer_bereits_verbundenen_nutzer() {
        let (guard, port, _) = setup_guard().await;
        port.set_voice_channel(GUILD_ID, USER_A, Some(42)).await;

        guard.reconcile(GUILD_ID).await;

        assert_eq!(port.connect_state(42, USER_B).await, Some(false));
    }

    #[tokio::test]
    async fn reconcile_reaktiviert_inaktiven_lock_ohne_restore_luecke() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        store
            .set_active(GUILD_ID, 42, USER_B, false)
            .await
            .expect("inactive");
        port.set_voice_channel(GUILD_ID, USER_A, Some(42)).await;
        port.writes.lock().await.clear();

        guard.reconcile(GUILD_ID).await;

        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| lock.active));
        assert_eq!(port.connect_state(42, USER_B).await, Some(false));
        assert_eq!(port.channels_written().await, vec![42]);
    }

    #[tokio::test]
    async fn reconcile_restauriert_stale_locks_und_behaelt_geloeschte_kanaele_inaktiv() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        store
            .insert_lock(VoicePairLock {
                guild_id: GUILD_ID,
                channel_id: 99,
                blocked_user_id: USER_A,
                previous_allow: 0,
                previous_deny: 0,
                had_overwrite: false,
                active: true,
            })
            .await
            .expect("deleted channel lock");
        port.set_voice_channel(GUILD_ID, USER_A, None).await;

        guard.reconcile(GUILD_ID).await;

        assert_eq!(port.member_overwrite(42, USER_B).await, None);
        let locks = store.list_locks(GUILD_ID).await.expect("locks");
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].channel_id, 99);
        assert!(!locks[0].active);
    }

    #[tokio::test]
    async fn reconcile_entfernt_bei_cache_unverfuegbarkeit_keinen_lock() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        port.cache_unavailable.store(true, Ordering::SeqCst);

        guard.reconcile(GUILD_ID).await;

        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some());
    }

    #[tokio::test]
    async fn channel_cache_miss_laesst_inaktiven_lock_fuer_reconcile_stehen() {
        let (guard, port, store) = setup_guard_with_lock(42, USER_B, None).await;
        port.channels.lock().await.remove(&(GUILD_ID, 42));

        assert!(guard.restore(GUILD_ID, 42, USER_B).await.is_err());

        assert!(store
            .lock(GUILD_ID, 42, USER_B)
            .await
            .expect("store")
            .is_some_and(|lock| !lock.active));
        assert_eq!(
            port.member_overwrite(42, USER_B).await,
            Some((0, CONNECT_BIT))
        );
    }

    #[tokio::test]
    async fn guard_ruft_niemals_disconnect_oder_move_auf() {
        let (guard, port, _) = setup_guard().await;
        guard
            .handle_event(VoiceEvent::Join {
                guild_id: GUILD_ID,
                user_id: USER_A,
                channel_id: 42,
            })
            .await;
        assert_eq!(port.member_move_calls().await, 0);
    }
}
