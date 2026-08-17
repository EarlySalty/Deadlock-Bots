use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use dl_community::tags::{TagEvent, TagService};
use serde_json::json;
use serenity::all::{GuildId, UserId};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::Mutex;

pub const NATIVE_ONBOARDING_DEDUPE_NS: &str = "native_onboarding:completed";
const NATIVE_ONBOARDING_PROCESS_DEDUPE_LIMIT: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeOnboardingRole {
    pub role_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeOnboardingChoice {
    InviteGuest,
    Freshling,
    Player,
}

impl NativeOnboardingChoice {
    fn as_str(self) -> &'static str {
        match self {
            Self::InviteGuest => "invite_guest",
            Self::Freshling => "freshling",
            Self::Player => "player_lfg",
        }
    }
}

struct NativeOnboardingProcessDedupe {
    limit: usize,
    inner: Mutex<NativeOnboardingProcessDedupeInner>,
}

#[derive(Default)]
struct NativeOnboardingProcessDedupeInner {
    keys: HashSet<String>,
    order: VecDeque<String>,
}

impl NativeOnboardingProcessDedupe {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            inner: Mutex::new(NativeOnboardingProcessDedupeInner::default()),
        }
    }

    async fn mark_unknown(&self, key: String) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.keys.contains(&key) {
            return false;
        }
        inner.keys.insert(key.clone());
        inner.order.push_back(key);
        while inner.keys.len() > self.limit {
            if let Some(oldest) = inner.order.pop_front() {
                inner.keys.remove(&oldest);
            } else {
                break;
            }
        }
        true
    }

    async fn forget(&self, key: &str) {
        let mut inner = self.inner.lock().await;
        inner.keys.remove(key);
    }
}

#[async_trait]
pub trait NativeOnboardingLookup: Send + Sync {
    async fn member_roles_and_guild_roles(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<(Vec<u64>, Vec<NativeOnboardingRole>), String>;
}

struct SerenityNativeOnboardingLookup {
    adapter: Arc<dl_discord::DiscordAdapter>,
}

#[async_trait]
impl NativeOnboardingLookup for SerenityNativeOnboardingLookup {
    async fn member_roles_and_guild_roles(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<(Vec<u64>, Vec<NativeOnboardingRole>), String> {
        let guild = GuildId::new(guild_id);
        let member = self
            .adapter
            .http
            .get_member(guild, UserId::new(user_id))
            .await
            .map_err(|err| err.to_string())?;
        let mut member_role_ids = member
            .roles
            .iter()
            .map(|role| role.get())
            .collect::<Vec<_>>();
        member_role_ids.sort_unstable();
        let roles = self
            .adapter
            .http
            .get_guild_roles(guild)
            .await
            .map_err(|err| err.to_string())?
            .into_iter()
            .map(|role| NativeOnboardingRole {
                role_id: role.id.get(),
                name: role.name,
            })
            .collect();
        Ok((member_role_ids, roles))
    }
}

const WEICHE_ROLE_CHOICES: &[(u64, &str)] = &[
    (
        dl_community::onboarding::ROLE_STREAMER_ONBOARD_ID,
        "streamer_onboarding",
    ),
    (
        dl_community::onboarding::ROLE_STREAMER_PARTNER_ID,
        "streamer_partner",
    ),
    (dl_community::onboarding::ROLE_LFG_PING_ID, "lfg_ping"),
    (
        dl_community::onboarding::ROLE_CUSTOM_GAMES_PING_ID,
        "custom_games_ping",
    ),
    (
        dl_community::onboarding::ROLE_PATCHNOTES_PING_ID,
        "patchnotes_ping",
    ),
    (dl_community::onboarding::ROLE_RANKED_ID, "ranked"),
    (dl_community::onboarding::ROLE_CASUAL_ID, "casual"),
];

fn weiche_choice_for_role(role_id: u64) -> Option<&'static str> {
    WEICHE_ROLE_CHOICES
        .iter()
        .find_map(|(candidate, choice)| (*candidate == role_id).then_some(*choice))
}

pub fn classify_native_onboarding_choice(
    member_role_ids: &[u64],
    roles: &[NativeOnboardingRole],
) -> NativeOnboardingChoice {
    if has_named_role(member_role_ids, roles, "Invite-Gast") {
        NativeOnboardingChoice::InviteGuest
    } else if has_named_role(member_role_ids, roles, "Frischling") {
        NativeOnboardingChoice::Freshling
    } else {
        NativeOnboardingChoice::Player
    }
}

fn has_named_role(member_role_ids: &[u64], roles: &[NativeOnboardingRole], expected: &str) -> bool {
    let expected = normalized_role_name(expected);
    roles.iter().any(|role| {
        member_role_ids.contains(&role.role_id) && normalized_role_name(&role.name) == expected
    })
}

fn normalized_role_name(name: &str) -> String {
    name.trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .trim()
        .to_lowercase()
}

#[cfg(test)]
pub async fn claim_native_onboarding_once(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await?;
    if dl_activity::journey::lock_user_privacy_and_is_opted_out_tx(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }
    let claimed = claim_native_onboarding_once_tx(&mut tx, guild_id, user_id).await?;
    tx.commit().await?;
    Ok(claimed)
}

fn native_onboarding_dedupe_key(guild_id: u64, user_id: u64) -> String {
    format!("{guild_id}:{user_id}")
}

async fn native_onboarding_claim_exists(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar::<_, i32>(
        "SELECT 1
           FROM bot.kv_store
          WHERE ns = $1 AND k = $2
          LIMIT 1",
    )
    .bind(NATIVE_ONBOARDING_DEDUPE_NS)
    .bind(native_onboarding_dedupe_key(guild_id, user_id))
    .fetch_optional(pool)
    .await
    .map(|row| row.is_some())
}

async fn claim_native_onboarding_once_tx(
    tx: &mut Transaction<'_, Postgres>,
    guild_id: u64,
    user_id: u64,
) -> Result<bool, sqlx::Error> {
    let key = native_onboarding_dedupe_key(guild_id, user_id);
    let value = json!({
        "guild_id": guild_id,
        "user_id": user_id,
        "claimed_at": Utc::now().to_rfc3339(),
    })
    .to_string();
    sqlx::query(
        "INSERT INTO bot.kv_store(ns, k, v)
         VALUES($1, $2, $3)
         ON CONFLICT(ns, k) DO NOTHING",
    )
    .bind(NATIVE_ONBOARDING_DEDUPE_NS)
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await
    .map(|result| result.rows_affected() > 0)
}

async fn record_native_onboarding_completed_once(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    choice: NativeOnboardingChoice,
    matched_role_id: Option<u64>,
    member_role_count: usize,
) -> anyhow::Result<bool> {
    let occurred_at = Utc::now();
    let mut tx = pool.begin().await?;
    if dl_activity::journey::lock_user_privacy_and_is_opted_out_tx(&mut tx, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }
    if !claim_native_onboarding_once_tx(&mut tx, guild_id, user_id).await? {
        tx.commit().await?;
        return Ok(false);
    }

    let mut input = dl_activity::journey::JourneyEventInput::new(
        user_id,
        guild_id,
        dl_activity::journey::JourneyEventType::NativeOnboardingCompleted,
        occurred_at,
    );
    input.event_source = "member_flags_completed_onboarding";
    input.metadata = json!({
        "weiche_choice": choice.as_str(),
        "matched_role_id": matched_role_id,
        "member_role_count": member_role_count,
    });
    let recorded = dl_activity::journey::record_journey_event_tx(&mut tx, input).await?;
    tx.commit().await?;
    Ok(recorded)
}

async fn handle_native_onboarding_completed(
    pool: PgPool,
    lookup: Arc<dyn NativeOnboardingLookup>,
    process_dedupe: Arc<NativeOnboardingProcessDedupe>,
    dedupe_key: String,
    guild_id: u64,
    user_id: u64,
    reread_delay: Duration,
) {
    tokio::time::sleep(reread_delay).await;
    let (member_role_ids, roles) = match lookup
        .member_roles_and_guild_roles(guild_id, user_id)
        .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::warn!(%err, guild_id, user_id, "Native-Onboarding Rollen-Nachlese fehlgeschlagen");
            process_dedupe.forget(&dedupe_key).await;
            return;
        }
    };
    let choice = classify_native_onboarding_choice(&member_role_ids, &roles);
    let matched_role_id = match choice {
        NativeOnboardingChoice::InviteGuest => {
            role_id_by_name(&member_role_ids, &roles, "Invite-Gast")
        }
        NativeOnboardingChoice::Freshling => {
            role_id_by_name(&member_role_ids, &roles, "Frischling")
        }
        NativeOnboardingChoice::Player => None,
    };
    if let Err(err) = record_native_onboarding_completed_once(
        &pool,
        guild_id,
        user_id,
        choice,
        matched_role_id,
        member_role_ids.len(),
    )
    .await
    {
        tracing::warn!(%err, guild_id, user_id, "Native-Onboarding Claim+Journey-Record fehlgeschlagen");
        process_dedupe.forget(&dedupe_key).await;
    }
}

fn role_id_by_name(
    member_role_ids: &[u64],
    roles: &[NativeOnboardingRole],
    expected: &str,
) -> Option<u64> {
    let expected = normalized_role_name(expected);
    roles
        .iter()
        .find(|role| {
            member_role_ids.contains(&role.role_id) && normalized_role_name(&role.name) == expected
        })
        .map(|role| role.role_id)
}

pub fn spawn_native_onboarding_completed(
    pool: PgPool,
    adapter: Arc<dl_discord::DiscordAdapter>,
    dispatcher: &dl_discord::Dispatcher,
    guild_id_filter: u64,
) -> tokio::task::JoinHandle<()> {
    spawn_native_onboarding_completed_with_lookup(
        pool,
        Arc::new(SerenityNativeOnboardingLookup { adapter }),
        dispatcher,
        guild_id_filter,
        Duration::from_secs(7),
    )
}

pub fn spawn_native_onboarding_completed_with_lookup(
    pool: PgPool,
    lookup: Arc<dyn NativeOnboardingLookup>,
    dispatcher: &dl_discord::Dispatcher,
    guild_id_filter: u64,
    reread_delay: Duration,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_members();
    let process_dedupe = Arc::new(NativeOnboardingProcessDedupe::new(
        NATIVE_ONBOARDING_PROCESS_DEDUPE_LIMIT,
    ));
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::MemberEvent::NativeOnboardingCompleted { guild_id, user_id })
                    if guild_id == guild_id_filter =>
                {
                    let dedupe_key = native_onboarding_dedupe_key(guild_id, user_id);
                    if !process_dedupe.mark_unknown(dedupe_key.clone()).await {
                        continue;
                    }
                    match native_onboarding_claim_exists(&pool, guild_id, user_id).await {
                        Ok(true) => continue,
                        Ok(false) => {}
                        Err(err) => {
                            tracing::warn!(%err, guild_id, user_id, "Native-Onboarding KV-Dedupe-Vorabpruefung fehlgeschlagen");
                            process_dedupe.forget(&dedupe_key).await;
                            continue;
                        }
                    }
                    let pool = pool.clone();
                    let lookup = lookup.clone();
                    let process_dedupe = process_dedupe.clone();
                    tokio::spawn(handle_native_onboarding_completed(
                        pool,
                        lookup,
                        process_dedupe,
                        dedupe_key,
                        guild_id,
                        user_id,
                        reread_delay,
                    ));
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Journey Native-Onboarding-Events verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

async fn record_role_delta(
    pool: &PgPool,
    guild_id: u64,
    user_id: u64,
    role_id: u64,
    change: &'static str,
) {
    if change == "gained" && role_id == dl_community::onboarding::ONBOARD_COMPLETE_ROLE_ID {
        if let Err(err) = dl_activity::journey::record_native_onboarding_completed_event(
            pool,
            user_id,
            guild_id,
            "role_event",
            Utc::now(),
            json!({
                "completed_role_id": role_id,
                "change": change,
            }),
        )
        .await
        {
            tracing::warn!(%err, guild_id, user_id, role_id, "Journey native_onboarding_completed fehlgeschlagen");
        }
    }

    if let Some(choice) = weiche_choice_for_role(role_id) {
        if let Err(err) = dl_activity::journey::record_weiche_changed_event(
            pool,
            user_id,
            guild_id,
            choice,
            change,
            "role_event",
            Utc::now(),
        )
        .await
        {
            tracing::warn!(%err, guild_id, user_id, role_id, "Journey weiche_changed aus RoleEvent fehlgeschlagen");
        }
    }
}

pub fn spawn_role_events(
    pool: PgPool,
    dispatcher: &dl_discord::Dispatcher,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_roles();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(dl_discord::RoleEvent::Gained {
                    guild_id,
                    user_id,
                    role_ids,
                }) => {
                    for role_id in role_ids {
                        record_role_delta(&pool, guild_id, user_id, role_id, "gained").await;
                    }
                }
                Ok(dl_discord::RoleEvent::Removed {
                    guild_id,
                    user_id,
                    role_ids,
                }) => {
                    for role_id in role_ids {
                        record_role_delta(&pool, guild_id, user_id, role_id, "removed").await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Journey RoleEvents verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

pub fn spawn_tag_events(
    pool: PgPool,
    tags: Arc<TagService>,
    guild_id: u64,
) -> tokio::task::JoinHandle<()> {
    let mut events = tags.subscribe();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(TagEvent::UserTagChanged {
                    user_id, key, new, ..
                }) if key == "age" || key == "tone" => {
                    let choice = new
                        .map(|value| format!("{key}:{value}"))
                        .unwrap_or_else(|| format!("{key}:cleared"));
                    if let Err(err) = dl_activity::journey::record_weiche_changed_event(
                        &pool,
                        user_id,
                        guild_id,
                        &choice,
                        "tag_changed",
                        "tag_service",
                        Utc::now(),
                    )
                    .await
                    {
                        tracing::warn!(%err, guild_id, user_id, key, "Journey weiche_changed aus TagEvent fehlgeschlagen");
                    }
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "Journey TagEvents verpasst");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn native_onboarding_weiche_klassifikation_priorisiert_invite_vor_frischling() {
        let roles = vec![
            NativeOnboardingRole {
                role_id: 10,
                name: "Frischling".to_string(),
            },
            NativeOnboardingRole {
                role_id: 11,
                name: "Invite-Gast".to_string(),
            },
        ];

        assert_eq!(
            classify_native_onboarding_choice(&[10, 11], &roles),
            NativeOnboardingChoice::InviteGuest
        );
        assert_eq!(
            classify_native_onboarding_choice(&[10], &roles),
            NativeOnboardingChoice::Freshling
        );
        assert_eq!(
            classify_native_onboarding_choice(&[], &roles),
            NativeOnboardingChoice::Player
        );
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn native_onboarding_dedupe_marker_ist_persistent_und_einmalig(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        assert!(claim_native_onboarding_once(pool, 1, 42).await?);
        assert!(!claim_native_onboarding_once(pool, 1, 42).await?);
        assert!(claim_native_onboarding_once(pool, 2, 42).await?);

        Ok(())
    }

    struct CountingLookup {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl NativeOnboardingLookup for CountingLookup {
        async fn member_roles_and_guild_roles(
            &self,
            _guild_id: u64,
            _user_id: u64,
        ) -> Result<(Vec<u64>, Vec<NativeOnboardingRole>), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok((Vec::new(), Vec::new()))
        }
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn native_onboarding_vorab_dedupe_verhindert_rest_duplikate(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool().clone();
        let dispatcher = dl_discord::Dispatcher::new();
        let lookup = Arc::new(CountingLookup {
            calls: AtomicUsize::new(0),
        });
        let task = spawn_native_onboarding_completed_with_lookup(
            pool,
            lookup.clone(),
            &dispatcher,
            1,
            Duration::from_millis(10),
        );

        for _ in 0..5 {
            dispatcher.publish_member(dl_discord::MemberEvent::NativeOnboardingCompleted {
                guild_id: 1,
                user_id: 42,
            });
        }

        tokio::time::sleep(Duration::from_millis(150)).await;
        task.abort();

        assert_eq!(lookup.calls.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn native_onboarding_claim_und_journey_record_committen_atomar(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        assert!(
            record_native_onboarding_completed_once(
                pool,
                1,
                42,
                NativeOnboardingChoice::Player,
                None,
                0,
            )
            .await?
        );
        assert!(
            !record_native_onboarding_completed_once(
                pool,
                1,
                42,
                NativeOnboardingChoice::Player,
                None,
                0,
            )
            .await?
        );

        let kv_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*)::int8 FROM bot.kv_store WHERE ns = $1 AND k = $2")
                .bind(NATIVE_ONBOARDING_DEDUPE_NS)
                .bind(native_onboarding_dedupe_key(1, 42))
                .fetch_one(pool)
                .await?;
        let event_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8
               FROM activity.journey_events
              WHERE guild_id = 1
                AND user_id = 42
                AND event_type = 'native_onboarding_completed'",
        )
        .fetch_one(pool)
        .await?;

        assert_eq!(kv_count, 1);
        assert_eq!(event_count, 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn native_onboarding_tombstone_blockiert_claim_und_journey_gemeinsam(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        sqlx::query(
            "INSERT INTO core.user_privacy(user_id, opted_out, updated_at)
             VALUES(42, TRUE, now())",
        )
        .execute(pool)
        .await?;

        assert!(
            !record_native_onboarding_completed_once(
                pool,
                1,
                42,
                NativeOnboardingChoice::Player,
                None,
                0,
            )
            .await?
        );

        let kv_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*)::int8 FROM bot.kv_store WHERE ns = $1 AND k = $2")
                .bind(NATIVE_ONBOARDING_DEDUPE_NS)
                .bind(native_onboarding_dedupe_key(1, 42))
                .fetch_one(pool)
                .await?;
        let event_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::int8
               FROM activity.journey_events
              WHERE guild_id = 1
                AND user_id = 42
                AND event_type = 'native_onboarding_completed'",
        )
        .fetch_one(pool)
        .await?;

        assert_eq!((kv_count, event_count), (0, 0));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn native_onboarding_record_fehler_rollt_kv_claim_zurueck(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();
        let user_id = u64::MAX;
        let key = native_onboarding_dedupe_key(1, user_id);

        let err = record_native_onboarding_completed_once(
            pool,
            1,
            user_id,
            NativeOnboardingChoice::Player,
            None,
            0,
        )
        .await
        .expect_err("out-of-range user id must fail after tx claim attempt");
        assert!(err.to_string().contains("passt nicht in PostgreSQL BIGINT"));

        let kv_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*)::int8 FROM bot.kv_store WHERE ns = $1 AND k = $2")
                .bind(NATIVE_ONBOARDING_DEDUPE_NS)
                .bind(key)
                .fetch_one(pool)
                .await?;
        assert_eq!(kv_count, 0);
        Ok(())
    }
}
