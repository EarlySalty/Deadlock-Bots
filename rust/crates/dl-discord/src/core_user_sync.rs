use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use dl_central_db::CentralDbError;
use serenity::all::{Interaction, User};
use sqlx::PgPool;

const DEFAULT_COOLDOWN: Duration = Duration::from_secs(10 * 60);
const PRUNE_AFTER_ENTRIES: usize = 8192;

type CoreUserUpsertFuture =
    Pin<Box<dyn Future<Output = Result<(), CentralDbError>> + Send + 'static>>;

trait CoreUserUpsert: Send + Sync {
    fn upsert(&self, pool: PgPool, profile: CoreUserProfile) -> CoreUserUpsertFuture;
}

struct CentralDbCoreUserUpsert;

impl CoreUserUpsert for CentralDbCoreUserUpsert {
    fn upsert(&self, pool: PgPool, profile: CoreUserProfile) -> CoreUserUpsertFuture {
        Box::pin(async move {
            dl_central_db::upsert_user(
                &pool,
                profile.discord_id,
                profile.username.as_deref(),
                profile.global_name.as_deref(),
                profile.avatar.as_deref(),
            )
            .await
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CoreUserProfile {
    pub discord_id: i64,
    pub username: Option<String>,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum CoreUserEventKind {
    MessageCreate,
    InteractionCreate,
    GuildMemberAdd,
    GuildMemberUpdate,
}

impl CoreUserEventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MessageCreate => "message_create",
            Self::InteractionCreate => "interaction_create",
            Self::GuildMemberAdd => "guild_member_add",
            Self::GuildMemberUpdate => "guild_member_update",
        }
    }
}

pub(crate) struct CoreUserSync {
    pool: PgPool,
    upsert: Arc<dyn CoreUserUpsert>,
    cooldown: Duration,
    last_write: Mutex<HashMap<i64, Instant>>,
}

impl CoreUserSync {
    pub fn new(pool: PgPool) -> Self {
        Self::with_cooldown(pool, DEFAULT_COOLDOWN)
    }

    fn with_cooldown(pool: PgPool, cooldown: Duration) -> Self {
        Self::with_upsert(pool, cooldown, Arc::new(CentralDbCoreUserUpsert))
    }

    fn with_upsert(pool: PgPool, cooldown: Duration, upsert: Arc<dyn CoreUserUpsert>) -> Self {
        Self {
            pool,
            upsert,
            cooldown,
            last_write: Mutex::new(HashMap::new()),
        }
    }

    pub fn record(self: &Arc<Self>, kind: CoreUserEventKind, profile: CoreUserProfile) -> bool {
        let reserved_at = Instant::now();
        if profile.discord_id == 0 || !self.reserve(profile.discord_id, reserved_at) {
            return false;
        }

        let sync = Arc::clone(self);
        tokio::spawn(async move {
            let discord_id = profile.discord_id;
            let source = kind.as_str();
            match sync.upsert.upsert(sync.pool.clone(), profile).await {
                Ok(()) => tracing::debug!(
                    discord_id,
                    source,
                    "core.users aus Gateway-Event aktualisiert"
                ),
                Err(err) => {
                    sync.clear_reservation(discord_id, reserved_at);
                    tracing::warn!(
                        discord_id,
                        source,
                        %err,
                        "core.users-Upsert aus Gateway-Event fehlgeschlagen"
                    );
                }
            }
        });

        true
    }

    fn reserve(&self, discord_id: i64, now: Instant) -> bool {
        let mut guard = self
            .last_write
            .lock()
            .expect("core user cooldown mutex poisoned");
        if guard
            .get(&discord_id)
            .is_some_and(|last| now.duration_since(*last) < self.cooldown)
        {
            return false;
        }

        if guard.len() >= PRUNE_AFTER_ENTRIES {
            let cooldown = self.cooldown;
            guard.retain(|_, last| now.duration_since(*last) < cooldown);
        }

        guard.insert(discord_id, now);
        true
    }

    fn clear_reservation(&self, discord_id: i64, reserved_at: Instant) {
        let mut guard = self
            .last_write
            .lock()
            .expect("core user cooldown mutex poisoned");
        if guard.get(&discord_id) == Some(&reserved_at) {
            guard.remove(&discord_id);
        }
    }
}

pub(crate) fn profile_from_user(user: &User) -> Option<CoreUserProfile> {
    if user.bot {
        return None;
    }

    let discord_id = match i64::try_from(user.id.get()) {
        Ok(discord_id) => discord_id,
        Err(_) => {
            tracing::warn!(
                discord_id = user.id.get(),
                "Discord-ID passt nicht in core.users.discord_id"
            );
            return None;
        }
    };

    Some(CoreUserProfile {
        discord_id,
        username: non_empty(user.name.clone()),
        global_name: user.global_name.clone().and_then(non_empty),
        avatar: user.avatar_url().and_then(non_empty),
    })
}

pub(crate) fn profile_from_interaction(interaction: &Interaction) -> Option<CoreUserProfile> {
    match interaction {
        Interaction::Command(command) | Interaction::Autocomplete(command) => {
            profile_from_user(&command.user)
        }
        Interaction::Component(component) => profile_from_user(&component.user),
        Interaction::Modal(modal) => profile_from_user(&modal.user),
        Interaction::Ping(_) => None,
        _ => None,
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::{mpsc, Notify};

    fn lazy_pool() -> PgPool {
        PgPoolOptions::new()
            .connect_lazy("postgres://core-user-sync-test.invalid/deadlock")
            .expect("lazy pg pool")
    }

    #[cfg(feature = "testing")]
    async fn read_core_user_optional(
        pool: &PgPool,
        discord_id: i64,
    ) -> Option<(Option<String>, Option<String>, Option<String>)> {
        sqlx::query_as::<_, (Option<String>, Option<String>, Option<String>)>(
            "SELECT username, global_name, avatar FROM core.users WHERE discord_id = $1",
        )
        .bind(discord_id)
        .fetch_optional(pool)
        .await
        .expect("core user lookup")
    }

    #[cfg(feature = "testing")]
    async fn wait_for_core_user(
        pool: &PgPool,
        discord_id: i64,
    ) -> (Option<String>, Option<String>, Option<String>) {
        for _ in 0..100 {
            if let Some(user) = read_core_user_optional(pool, discord_id).await {
                return user;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("core user row {discord_id} was not written");
    }

    struct BlockingUpsert {
        started: mpsc::UnboundedSender<i64>,
        finished: mpsc::UnboundedSender<i64>,
        release: Arc<Notify>,
        released: Arc<AtomicBool>,
        calls: Arc<AtomicUsize>,
    }

    impl CoreUserUpsert for BlockingUpsert {
        fn upsert(&self, _pool: PgPool, profile: CoreUserProfile) -> CoreUserUpsertFuture {
            let started = self.started.clone();
            let finished = self.finished.clone();
            let release = Arc::clone(&self.release);
            let released = Arc::clone(&self.released);
            let calls = Arc::clone(&self.calls);
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let _ = started.send(profile.discord_id);
                if !released.load(Ordering::SeqCst) {
                    release.notified().await;
                }
                let _ = finished.send(profile.discord_id);
                Err(CentralDbError::TestHarness(
                    "simulierter Upsert-Fehler".to_string(),
                ))
            })
        }
    }

    #[test]
    fn event_kind_labels_match_gateway_names() {
        assert_eq!(CoreUserEventKind::MessageCreate.as_str(), "message_create");
        assert_eq!(
            CoreUserEventKind::InteractionCreate.as_str(),
            "interaction_create"
        );
        assert_eq!(
            CoreUserEventKind::GuildMemberAdd.as_str(),
            "guild_member_add"
        );
        assert_eq!(
            CoreUserEventKind::GuildMemberUpdate.as_str(),
            "guild_member_update"
        );
    }

    #[tokio::test]
    async fn record_returns_before_slow_upsert_finishes() {
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        let (finished_tx, mut finished_rx) = mpsc::unbounded_channel();
        let release = Arc::new(Notify::new());
        let released = Arc::new(AtomicBool::new(false));
        let calls = Arc::new(AtomicUsize::new(0));
        let sync = Arc::new(CoreUserSync::with_upsert(
            lazy_pool(),
            DEFAULT_COOLDOWN,
            Arc::new(BlockingUpsert {
                started: started_tx,
                finished: finished_tx,
                release: Arc::clone(&release),
                released: Arc::clone(&released),
                calls: Arc::clone(&calls),
            }),
        ));
        let profile = CoreUserProfile {
            discord_id: 9_101_000_010,
            username: Some("slow-user".to_string()),
            global_name: None,
            avatar: None,
        };

        tokio::time::timeout(Duration::from_millis(250), async {
            assert!(sync.record(CoreUserEventKind::MessageCreate, profile.clone()));
        })
        .await
        .expect("record must not wait for the upsert future");

        assert_eq!(
            started_rx.recv().await,
            Some(profile.discord_id),
            "spawned task should start the DB write"
        );
        assert_eq!(
            finished_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty),
            "upsert is still blocked, but record already returned"
        );
        assert!(!sync.record(CoreUserEventKind::MessageCreate, profile.clone()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        released.store(true, Ordering::SeqCst);
        release.notify_waiters();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(1), finished_rx.recv())
                .await
                .expect("upsert task should finish after release"),
            Some(profile.discord_id)
        );
        for _ in 0..100 {
            if sync.record(CoreUserEventKind::MessageCreate, profile.clone()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("failed upsert should clear the reservation");
    }

    #[tokio::test]
    async fn clear_reservation_keeps_newer_reservation() {
        let sync = CoreUserSync::with_cooldown(lazy_pool(), Duration::ZERO);
        let discord_id = 9_101_000_011;
        let older = Instant::now();
        let newer = older + Duration::from_millis(1);

        assert!(sync.reserve(discord_id, older));
        assert!(sync.reserve(discord_id, newer));
        sync.clear_reservation(discord_id, older);

        let guard = sync
            .last_write
            .lock()
            .expect("core user cooldown mutex poisoned");
        assert_eq!(guard.get(&discord_id), Some(&newer));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn message_create_and_interaction_create_upsert_core_users() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("central test db");
        let sync = Arc::new(CoreUserSync::with_cooldown(
            db.pool().clone(),
            DEFAULT_COOLDOWN,
        ));

        let message_user = CoreUserProfile {
            discord_id: 9_101_000_001,
            username: Some("message-user".to_string()),
            global_name: Some("Message User".to_string()),
            avatar: Some("https://cdn.discordapp.com/avatars/9101000001/msg.png".to_string()),
        };
        let interaction_user = CoreUserProfile {
            discord_id: 9_101_000_002,
            username: Some("interaction-user".to_string()),
            global_name: Some("Interaction User".to_string()),
            avatar: Some("https://cdn.discordapp.com/avatars/9101000002/int.png".to_string()),
        };

        assert!(sync.record(CoreUserEventKind::MessageCreate, message_user.clone()));
        assert!(sync.record(
            CoreUserEventKind::InteractionCreate,
            interaction_user.clone()
        ));

        assert_eq!(
            wait_for_core_user(db.pool(), message_user.discord_id).await,
            (
                message_user.username,
                message_user.global_name,
                message_user.avatar
            )
        );
        assert_eq!(
            wait_for_core_user(db.pool(), interaction_user.discord_id).await,
            (
                interaction_user.username,
                interaction_user.global_name,
                interaction_user.avatar
            )
        );
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn repeated_events_inside_cooldown_do_not_write_again() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("central test db");
        let sync = Arc::new(CoreUserSync::with_cooldown(
            db.pool().clone(),
            DEFAULT_COOLDOWN,
        ));
        let discord_id = 9_101_000_003;

        assert!(sync.record(
            CoreUserEventKind::MessageCreate,
            CoreUserProfile {
                discord_id,
                username: Some("first".to_string()),
                global_name: Some("First".to_string()),
                avatar: Some("avatar-first".to_string()),
            },
        ));
        assert!(!sync.record(
            CoreUserEventKind::MessageCreate,
            CoreUserProfile {
                discord_id,
                username: Some("second".to_string()),
                global_name: Some("Second".to_string()),
                avatar: Some("avatar-second".to_string()),
            },
        ));

        assert_eq!(
            wait_for_core_user(db.pool(), discord_id).await,
            (
                Some("first".to_string()),
                Some("First".to_string()),
                Some("avatar-first".to_string())
            )
        );
    }
}
