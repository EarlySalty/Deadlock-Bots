//! Generische Reaction-Roles.
//!
//! Eine konfigurierte Reaktion auf eine Nachricht vergibt eine Rolle, optional
//! mit einmaliger DM. Entfernt der User die Reaktion, kann die Rolle wieder
//! entzogen werden. Backfills laufen über Discord-REST und verwenden dieselbe
//! Rollen-/DM-Logik wie Live-Events.

use std::sync::Arc;
use std::time::Duration;

use dl_squads::store::PARTICIPANTS_LOCK;
use serenity::all::{EmojiId, ReactionType};
use sqlx::PgPool;

use crate::db::{advisory_lock, pg_i64_to_u64, u64_to_i64, CommunityDbError, CommunityDbResult};

#[cfg(test)]
const BACKFILL_DM_DELAY: Duration = Duration::from_millis(0);
#[cfg(not(test))]
const BACKFILL_DM_DELAY: Duration = Duration::from_secs(1);

pub const SCRIM_SIGNUP_ROLE_ID: u64 = 1_520_849_762_851_618_817;

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum PortErr {
    #[error("discord error: {0}")]
    Discord(String),
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum DmErr {
    #[error("permanent DM error: {0}")]
    Permanent(String),
    #[error("transient DM error: {0}")]
    Transient(String),
}

impl DmErr {
    pub fn is_permanent(&self) -> bool {
        matches!(self, Self::Permanent(_))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReactionRoleError {
    #[error(transparent)]
    Db(#[from] CommunityDbError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReactedUser {
    pub id: u64,
    pub is_bot: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ApplyMappingAddOutcome {
    dm_attempted: bool,
    retryable_failure: bool,
}

#[async_trait::async_trait]
pub trait ReactionRolePort: Send + Sync {
    async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> Result<(), PortErr>;
    async fn remove_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> Result<(), PortErr>;
    async fn send_dm(&self, user_id: u64, content: &str) -> Result<(), DmErr>;
    /// Liefert eine rohe Discord-Seite Reaktoren, maximal 100 User pro Seite.
    async fn reaction_users(
        &self,
        channel_id: u64,
        message_id: u64,
        emoji: &ReactionType,
        after: Option<u64>,
    ) -> Result<Vec<ReactedUser>, PortErr>;
}

#[derive(Debug, Clone)]
struct ReactionRoleMapping {
    id: i64,
    guild_id: u64,
    source_channel_id: u64,
    message_id: u64,
    emoji: String,
    role_id: u64,
    dm_enabled: bool,
    dm_text: Option<String>,
    remove_on_unreact: bool,
}

pub struct ReactionRoleService {
    pool: PgPool,
    port: Arc<dyn ReactionRolePort>,
}

impl ReactionRoleService {
    pub fn new(pool: PgPool, port: Arc<dyn ReactionRolePort>) -> Arc<Self> {
        Arc::new(Self { pool, port })
    }

    pub async fn handle_reaction_add(
        &self,
        guild_id: u64,
        _channel_id: u64,
        message_id: u64,
        user_id: u64,
        emoji: &ReactionType,
        is_bot: bool,
    ) -> Result<(), ReactionRoleError> {
        if is_bot {
            return Ok(());
        }
        self.handle_reaction_add_with_display_name(
            guild_id, message_id, user_id, None, emoji, is_bot,
        )
        .await
    }

    pub async fn handle_reaction_add_with_display_name(
        &self,
        guild_id: u64,
        message_id: u64,
        user_id: u64,
        display_name: Option<&str>,
        emoji: &ReactionType,
        is_bot: bool,
    ) -> Result<(), ReactionRoleError> {
        if is_bot {
            return Ok(());
        }
        let canonical = canonical_emoji(emoji);
        let Some(mapping) = self.mapping_for_message(message_id, &canonical).await? else {
            return Ok(());
        };
        self.apply_mapping_add(&mapping, guild_id, user_id, display_name, true)
            .await?;
        Ok(())
    }

    pub async fn handle_reaction_remove(
        &self,
        guild_id: u64,
        _channel_id: u64,
        message_id: u64,
        user_id: u64,
        emoji: &ReactionType,
        is_bot: bool,
    ) -> Result<(), ReactionRoleError> {
        if is_bot {
            return Ok(());
        }
        let canonical = canonical_emoji(emoji);
        let Some(mapping) = self.mapping_for_message(message_id, &canonical).await? else {
            return Ok(());
        };
        if mapping.remove_on_unreact {
            if let Err(err) = self
                .port
                .remove_role(guild_id, user_id, mapping.role_id)
                .await
            {
                tracing::warn!(
                    %err,
                    mapping_id = mapping.id,
                    guild_id,
                    user_id,
                    role_id = mapping.role_id,
                    "Reaction-Role: Rolle konnte nicht entzogen werden"
                );
            }
        }
        Ok(())
    }

    pub async fn run_pending_backfills(&self) -> Result<(), ReactionRoleError> {
        let mappings = self.pending_backfill_mappings().await?;
        for mapping in mappings {
            let emoji = reaction_type_from_canonical(&mapping.emoji);
            let mut after = None;
            let mut failed = false;
            loop {
                let page = match self
                    .port
                    .reaction_users(mapping.source_channel_id, mapping.message_id, &emoji, after)
                    .await
                {
                    Ok(page) => page,
                    Err(err) => {
                        tracing::warn!(
                            %err,
                            mapping_id = mapping.id,
                            channel_id = mapping.source_channel_id,
                            message_id = mapping.message_id,
                            emoji = %mapping.emoji,
                            "Reaction-Role-Backfill: Reaktoren konnten nicht geladen werden"
                        );
                        failed = true;
                        break;
                    }
                };
                let page_len = page.len();
                let next_after = page.last().map(|user| user.id);
                for user in page {
                    if user.is_bot {
                        continue;
                    }
                    let outcome = self
                        .apply_mapping_add(&mapping, mapping.guild_id, user.id, None, false)
                        .await?;
                    if outcome.retryable_failure {
                        failed = true;
                    }
                    if outcome.dm_attempted && !BACKFILL_DM_DELAY.is_zero() {
                        tokio::time::sleep(BACKFILL_DM_DELAY).await;
                    }
                }
                after = next_after;
                if page_len < 100 {
                    break;
                }
            }
            if !failed {
                self.clear_backfill_pending(mapping.id).await?;
            }
        }
        Ok(())
    }

    async fn apply_mapping_add(
        &self,
        mapping: &ReactionRoleMapping,
        guild_id: u64,
        user_id: u64,
        display_name: Option<&str>,
        allow_scrim_pool_upsert: bool,
    ) -> Result<ApplyMappingAddOutcome, ReactionRoleError> {
        let mut outcome = ApplyMappingAddOutcome::default();
        match self.port.add_role(guild_id, user_id, mapping.role_id).await {
            Ok(()) => {
                if allow_scrim_pool_upsert {
                    self.upsert_scrim_signup_participant(mapping, user_id, display_name)
                        .await;
                }
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    mapping_id = mapping.id,
                    guild_id,
                    user_id,
                    role_id = mapping.role_id,
                    "Reaction-Role: Rolle konnte nicht vergeben werden"
                );
                outcome.retryable_failure = true;
            }
        }

        if !mapping.dm_enabled || !self.reserve_dm_log(mapping.id, user_id).await? {
            return Ok(outcome);
        }
        outcome.dm_attempted = true;
        let dm_text = mapping.dm_text.as_deref().unwrap_or("");
        match self.port.send_dm(user_id, dm_text).await {
            Ok(()) => {
                self.touch_dm_logged(mapping.id, user_id).await?;
            }
            Err(err) if err.is_permanent() => {
                tracing::info!(
                    %err,
                    mapping_id = mapping.id,
                    user_id,
                    "Reaction-Role: permanente DM-Ablehnung protokolliert"
                );
            }
            Err(err) => {
                tracing::warn!(
                    %err,
                    mapping_id = mapping.id,
                    user_id,
                    "Reaction-Role: transienter DM-Fehler, kein dm_log"
                );
                self.release_dm_reservation(mapping.id, user_id).await?;
                outcome.retryable_failure = true;
            }
        }
        Ok(outcome)
    }

    async fn upsert_scrim_signup_participant(
        &self,
        mapping: &ReactionRoleMapping,
        user_id: u64,
        display_name: Option<&str>,
    ) {
        if mapping.role_id != SCRIM_SIGNUP_ROLE_ID {
            return;
        }
        let Ok(discord_id) = i64::try_from(user_id) else {
            tracing::warn!(
                mapping_id = mapping.id,
                user_id,
                "Scrim-Signup: Discord-ID passt nicht in i64"
            );
            return;
        };
        let Some(display_name) = display_name.map(str::trim).filter(|name| !name.is_empty()) else {
            return;
        };
        if let Err(err) =
            upsert_scrim_participant_by_discord(&self.pool, discord_id, display_name).await
        {
            tracing::warn!(
                %err,
                mapping_id = mapping.id,
                user_id,
                "Scrim-Signup: Pool-Upsert fehlgeschlagen"
            );
        }
    }

    async fn mapping_for_message(
        &self,
        message_id: u64,
        emoji: &str,
    ) -> CommunityDbResult<Option<ReactionRoleMapping>> {
        let message_id = u64_to_i64(message_id, "message_id")?;
        let row = sqlx::query!(
            r#"
            SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
                   dm_enabled, dm_text, remove_on_unreact
              FROM bot.reaction_role_mappings
             WHERE message_id = $1 AND emoji = $2 AND active = TRUE
             LIMIT 1
            "#,
            message_id,
            emoji,
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(ReactionRoleMapping {
                id: row.id,
                guild_id: pg_i64_to_u64(row.guild_id, "guild_id")?,
                source_channel_id: pg_i64_to_u64(row.source_channel_id, "source_channel_id")?,
                message_id: pg_i64_to_u64(row.message_id, "message_id")?,
                emoji: row.emoji,
                role_id: pg_i64_to_u64(row.role_id, "role_id")?,
                dm_enabled: row.dm_enabled,
                dm_text: row.dm_text,
                remove_on_unreact: row.remove_on_unreact,
            })
        })
        .transpose()
    }

    async fn pending_backfill_mappings(&self) -> CommunityDbResult<Vec<ReactionRoleMapping>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
                   dm_enabled, dm_text, remove_on_unreact
              FROM bot.reaction_role_mappings
             WHERE backfill_pending = TRUE AND active = TRUE
             ORDER BY id ASC
            "#
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(ReactionRoleMapping {
                    id: row.id,
                    guild_id: pg_i64_to_u64(row.guild_id, "guild_id")?,
                    source_channel_id: pg_i64_to_u64(row.source_channel_id, "source_channel_id")?,
                    message_id: pg_i64_to_u64(row.message_id, "message_id")?,
                    emoji: row.emoji,
                    role_id: pg_i64_to_u64(row.role_id, "role_id")?,
                    dm_enabled: row.dm_enabled,
                    dm_text: row.dm_text,
                    remove_on_unreact: row.remove_on_unreact,
                })
            })
            .collect()
    }

    async fn reserve_dm_log(&self, mapping_id: i64, user_id: u64) -> CommunityDbResult<bool> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        let result = sqlx::query!(
            r#"
            INSERT INTO bot.reaction_role_dm_log(mapping_id, user_id, sent_at)
            VALUES ($1, $2, $3)
            ON CONFLICT(mapping_id, user_id) DO NOTHING
            "#,
            mapping_id,
            user_id,
            chrono::Utc::now(),
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    #[cfg(all(test, feature = "testing"))]
    async fn mark_dm_logged(&self, mapping_id: i64, user_id: u64) -> CommunityDbResult<()> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        sqlx::query!(
            r#"
            INSERT INTO bot.reaction_role_dm_log(mapping_id, user_id, sent_at)
            VALUES ($1, $2, $3)
            ON CONFLICT(mapping_id, user_id) DO UPDATE SET sent_at = excluded.sent_at
            "#,
            mapping_id,
            user_id,
            chrono::Utc::now(),
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn touch_dm_logged(&self, mapping_id: i64, user_id: u64) -> CommunityDbResult<()> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        sqlx::query!(
            r#"
            UPDATE bot.reaction_role_dm_log
               SET sent_at = $1
             WHERE mapping_id = $2 AND user_id = $3
            "#,
            chrono::Utc::now(),
            mapping_id,
            user_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn release_dm_reservation(&self, mapping_id: i64, user_id: u64) -> CommunityDbResult<()> {
        let user_id = u64_to_i64(user_id, "user_id")?;
        sqlx::query!(
            r#"
            DELETE FROM bot.reaction_role_dm_log
             WHERE mapping_id = $1 AND user_id = $2
            "#,
            mapping_id,
            user_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn clear_backfill_pending(&self, mapping_id: i64) -> CommunityDbResult<()> {
        sqlx::query!(
            r#"
            UPDATE bot.reaction_role_mappings
               SET backfill_pending = FALSE,
                   updated_at = $1
             WHERE id = $2
            "#,
            chrono::Utc::now(),
            mapping_id,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

async fn upsert_scrim_participant_by_discord(
    pool: &PgPool,
    discord_id: i64,
    display_name: &str,
) -> CommunityDbResult<i64> {
    let display_name = display_name.trim().to_string();
    let now = chrono::Utc::now();
    let mut tx = pool.begin().await?;
    advisory_lock(&mut tx, PARTICIPANTS_LOCK).await?;

    let existing_id = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!: i32"
          FROM scrim.participants
         WHERE discord_id = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
        discord_id,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = existing_id {
        sqlx::query!(
            r#"
            UPDATE scrim.participants
               SET display_name = $2,
                   updated_at = $3
             WHERE id = $1
            "#,
            id,
            display_name,
            now,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(i64::from(id));
    }

    let name_id = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!: i32"
          FROM scrim.participants
         WHERE display_name = $1
         ORDER BY id ASC
         LIMIT 1
        "#,
        display_name,
    )
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = name_id {
        sqlx::query!(
            r#"
            UPDATE scrim.participants
               SET discord_id = COALESCE(discord_id, $2),
                   updated_at = $3
             WHERE id = $1
            "#,
            id,
            discord_id,
            now,
        )
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        return Ok(i64::from(id));
    }

    let next_id = sqlx::query_scalar!(
        r#"
        SELECT COALESCE(MAX(id), 0) + 1 AS "next_id!: i32"
          FROM scrim.participants
        "#
    )
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query!(
        r#"
        INSERT INTO scrim.participants(
            id, discord_id, display_name, rank, rank_source, rank_verified,
            roles, availability, status, source, created_at, updated_at
        )
        VALUES ($1, $2, $3, NULL, 'self', FALSE, NULL, NULL, 'new', 'discord_reaction', $4, $4)
        "#,
        next_id,
        discord_id,
        display_name,
        now,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(i64::from(next_id))
}

pub fn canonical_emoji(emoji: &ReactionType) -> String {
    match emoji {
        ReactionType::Unicode(value) => value.clone(),
        ReactionType::Custom { id, name, .. } => {
            format!("{}:{}", name.as_deref().unwrap_or_default(), id.get())
        }
        _ => String::new(),
    }
}

pub fn canonical_emoji_input(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(inner) = trimmed
        .strip_prefix("<:")
        .or_else(|| trimmed.strip_prefix("<a:"))
        .and_then(|value| value.strip_suffix('>'))
    {
        if let Some((name, id)) = inner.rsplit_once(':') {
            if id.parse::<u64>().is_ok() {
                return format!("{name}:{id}");
            }
        }
    }
    trimmed.to_string()
}

fn reaction_type_from_canonical(value: &str) -> ReactionType {
    if let Some((name, id_raw)) = value.rsplit_once(':') {
        if let Ok(id) = id_raw.parse::<u64>() {
            return ReactionType::Custom {
                animated: false,
                id: EmojiId::new(id),
                name: Some(name.to_string()),
            };
        }
    }
    ReactionType::Unicode(value.to_string())
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Mutex, MutexGuard};

    use super::*;
    use dl_central_db::testing::{test_pool, TestDb};

    #[derive(Default)]
    struct MockState {
        add_roles: Vec<(u64, u64, u64)>,
        remove_roles: Vec<(u64, u64, u64)>,
        dms: Vec<(u64, String)>,
        reaction_fetches: Vec<(u64, u64, String, Option<u64>)>,
    }

    #[derive(Default)]
    struct MockPort {
        state: Mutex<MockState>,
        add_role_results: Mutex<VecDeque<Result<(), PortErr>>>,
        dm_results: Mutex<VecDeque<Result<(), DmErr>>>,
        reaction_pages: Mutex<VecDeque<Result<Vec<ReactedUser>, PortErr>>>,
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    impl MockPort {
        fn push_add_role_result(&self, result: Result<(), PortErr>) {
            lock(&self.add_role_results).push_back(result);
        }

        fn push_dm_result(&self, result: Result<(), DmErr>) {
            lock(&self.dm_results).push_back(result);
        }

        fn push_reaction_page(&self, result: Result<Vec<ReactedUser>, PortErr>) {
            lock(&self.reaction_pages).push_back(result);
        }
    }

    fn reacted_user(id: u64) -> ReactedUser {
        ReactedUser { id, is_bot: false }
    }

    fn reacted_bot(id: u64) -> ReactedUser {
        ReactedUser { id, is_bot: true }
    }

    #[async_trait::async_trait]
    impl ReactionRolePort for MockPort {
        async fn add_role(&self, guild_id: u64, user_id: u64, role_id: u64) -> Result<(), PortErr> {
            lock(&self.state)
                .add_roles
                .push((guild_id, user_id, role_id));
            lock(&self.add_role_results).pop_front().unwrap_or(Ok(()))
        }

        async fn remove_role(
            &self,
            guild_id: u64,
            user_id: u64,
            role_id: u64,
        ) -> Result<(), PortErr> {
            lock(&self.state)
                .remove_roles
                .push((guild_id, user_id, role_id));
            Ok(())
        }

        async fn send_dm(&self, user_id: u64, content: &str) -> Result<(), DmErr> {
            lock(&self.state).dms.push((user_id, content.to_string()));
            lock(&self.dm_results).pop_front().unwrap_or(Ok(()))
        }

        async fn reaction_users(
            &self,
            channel_id: u64,
            message_id: u64,
            emoji: &ReactionType,
            after: Option<u64>,
        ) -> Result<Vec<ReactedUser>, PortErr> {
            lock(&self.state).reaction_fetches.push((
                channel_id,
                message_id,
                canonical_emoji(emoji),
                after,
            ));
            lock(&self.reaction_pages)
                .pop_front()
                .unwrap_or_else(|| Ok(Vec::new()))
        }
    }

    fn mock_state(port: &MockPort) -> MockState {
        let state = lock(&port.state);
        MockState {
            add_roles: state.add_roles.clone(),
            remove_roles: state.remove_roles.clone(),
            dms: state.dms.clone(),
            reaction_fetches: state.reaction_fetches.clone(),
        }
    }

    async fn test_service(
    ) -> Result<(TestDb, PgPool, Arc<MockPort>, Arc<ReactionRoleService>), Box<dyn std::error::Error>>
    {
        let test_db = test_pool().await?;
        let pool = test_db.pool().clone();
        let port = Arc::new(MockPort::default());
        let service = ReactionRoleService::new(pool.clone(), port.clone());
        Ok((test_db, pool, port, service))
    }

    struct MappingInsert {
        message_id: u64,
        emoji: String,
        role_id: u64,
        dm_enabled: bool,
        remove_on_unreact: bool,
        backfill_pending: bool,
    }

    async fn insert_mapping(
        pool: &PgPool,
        insert: MappingInsert,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let row = sqlx::query!(
            r#"
            INSERT INTO bot.reaction_role_mappings(
                guild_id, source_channel_id, message_id, emoji, role_id,
                dm_enabled, dm_text, remove_on_unreact, backfill_pending,
                active, created_at, updated_at
            )
            VALUES (42, 43, $1, $2, $3, $4, 'DM', $5, $6, TRUE, now(), now())
            RETURNING id
            "#,
            u64_to_i64(insert.message_id, "message_id")?,
            insert.emoji,
            u64_to_i64(insert.role_id, "role_id")?,
            insert.dm_enabled,
            insert.remove_on_unreact,
            insert.backfill_pending,
        )
        .fetch_one(pool)
        .await?;
        Ok(row.id)
    }

    async fn dm_log_count(
        pool: &PgPool,
        mapping_id: i64,
        user_id: u64,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) AS "count!"
              FROM bot.reaction_role_dm_log
             WHERE mapping_id = $1 AND user_id = $2
            "#,
            mapping_id,
            u64_to_i64(user_id, "user_id")?,
        )
        .fetch_one(pool)
        .await?;
        Ok(count)
    }

    async fn backfill_pending(
        pool: &PgPool,
        mapping_id: i64,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        let pending = sqlx::query_scalar!(
            "SELECT backfill_pending FROM bot.reaction_role_mappings WHERE id = $1",
            mapping_id,
        )
        .fetch_one(pool)
        .await?;
        Ok(pending)
    }

    async fn scrim_participants(
        pool: &PgPool,
    ) -> Result<Vec<(i64, Option<i64>, String, String)>, Box<dyn std::error::Error>> {
        let rows = sqlx::query!(
            r#"
            SELECT id, discord_id, display_name, source
              FROM scrim.participants
             ORDER BY id ASC
            "#
        )
        .fetch_all(pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    i64::from(row.id),
                    row.discord_id,
                    row.display_name,
                    row.source,
                )
            })
            .collect())
    }

    #[test]
    fn canonical_emoji_unterscheidet_unicode_und_custom() {
        assert_eq!(
            canonical_emoji(&ReactionType::Unicode("✅".to_string())),
            "✅"
        );
        assert_eq!(
            canonical_emoji(&ReactionType::Custom {
                animated: false,
                id: EmojiId::new(123_456),
                name: Some("pog".to_string()),
            }),
            "pog:123456"
        );
    }

    #[test]
    fn canonical_emoji_input_kanonisiert_discord_custom_und_unicode() {
        assert_eq!(canonical_emoji_input("<:pog:123>"), "pog:123");
        assert_eq!(canonical_emoji_input("<a:spin:456>"), "spin:456");
        assert_eq!(canonical_emoji_input("✅"), "✅");
        assert_eq!(canonical_emoji_input("name:789"), "name:789");
    }

    #[tokio::test]
    async fn add_vergibt_rolle_und_dm_nur_einmal() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;

        let emoji = ReactionType::Unicode("✅".to_string());
        service
            .handle_reaction_add(42, 43, 100, 900, &emoji, false)
            .await?;
        service
            .handle_reaction_add(42, 43, 100, 900, &emoji, false)
            .await?;

        let state = mock_state(&port);
        assert_eq!(state.add_roles, vec![(42, 900, 500), (42, 900, 500)]);
        assert_eq!(state.dms, vec![(900, "DM".to_string())]);
        assert_eq!(dm_log_count(&db, mapping_id, 900).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn scrim_signup_rolle_schreibt_pool_eintrag() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: SCRIM_SIGNUP_ROLE_ID,
                dm_enabled: false,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;

        service
            .handle_reaction_add_with_display_name(
                42,
                100,
                900,
                Some("Vicky"),
                &ReactionType::Unicode("✅".to_string()),
                false,
            )
            .await?;

        let participants = scrim_participants(&db).await?;
        assert_eq!(participants.len(), 1);
        assert_eq!(
            participants,
            vec![(
                participants[0].0,
                Some(900),
                "Vicky".to_string(),
                "discord_reaction".to_string()
            )]
        );
        let state = mock_state(&port);
        assert_eq!(state.add_roles, vec![(42, 900, SCRIM_SIGNUP_ROLE_ID)]);
        Ok(())
    }

    #[tokio::test]
    async fn scrim_signup_backfill_schreibt_keinen_pool_eintrag(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: SCRIM_SIGNUP_ROLE_ID,
                dm_enabled: false,
                remove_on_unreact: true,
                backfill_pending: true,
            },
        )
        .await?;
        port.push_reaction_page(Ok(vec![reacted_user(900)]));

        service.run_pending_backfills().await?;

        assert!(scrim_participants(&db).await?.is_empty());
        assert!(!backfill_pending(&db, mapping_id).await?);
        let state = mock_state(&port);
        assert_eq!(state.add_roles, vec![(42, 900, SCRIM_SIGNUP_ROLE_ID)]);
        Ok(())
    }

    #[tokio::test]
    async fn andere_reaction_rolle_schreibt_keinen_pool_eintrag(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, _port, service) = test_service().await?;
        insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: false,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;

        service
            .handle_reaction_add(
                42,
                43,
                100,
                900,
                &ReactionType::Unicode("✅".to_string()),
                false,
            )
            .await?;

        assert!(scrim_participants(&db).await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn scrim_pool_upsert_fehler_bleibt_fail_open() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: SCRIM_SIGNUP_ROLE_ID,
                dm_enabled: false,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;
        sqlx::query("DROP TABLE scrim.participants")
            .execute(&db)
            .await?;

        service
            .handle_reaction_add_with_display_name(
                42,
                100,
                900,
                Some("Vicky"),
                &ReactionType::Unicode("✅".to_string()),
                false,
            )
            .await?;

        let state = mock_state(&port);
        assert_eq!(state.add_roles, vec![(42, 900, SCRIM_SIGNUP_ROLE_ID)]);
        Ok(())
    }

    #[tokio::test]
    async fn permanente_dm_fehler_loggen_transiente_nicht() -> Result<(), Box<dyn std::error::Error>>
    {
        let (_dir, db, port, service) = test_service().await?;
        let permanent_mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;
        let transient_mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 101,
                emoji: "🔥".to_string(),
                role_id: 501,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;

        port.push_dm_result(Err(DmErr::Permanent("closed".to_string())));
        service
            .handle_reaction_add(
                42,
                43,
                100,
                900,
                &ReactionType::Unicode("✅".to_string()),
                false,
            )
            .await?;
        service
            .handle_reaction_add(
                42,
                43,
                100,
                900,
                &ReactionType::Unicode("✅".to_string()),
                false,
            )
            .await?;

        port.push_dm_result(Err(DmErr::Transient("timeout".to_string())));
        port.push_dm_result(Ok(()));
        service
            .handle_reaction_add(
                42,
                43,
                101,
                901,
                &ReactionType::Unicode("🔥".to_string()),
                false,
            )
            .await?;
        assert_eq!(dm_log_count(&db, transient_mapping_id, 901).await?, 0);
        service
            .handle_reaction_add(
                42,
                43,
                101,
                901,
                &ReactionType::Unicode("🔥".to_string()),
                false,
            )
            .await?;

        let state = mock_state(&port);
        assert_eq!(
            state.dms,
            vec![
                (900, "DM".to_string()),
                (901, "DM".to_string()),
                (901, "DM".to_string()),
            ]
        );
        assert_eq!(dm_log_count(&db, permanent_mapping_id, 900).await?, 1);
        assert_eq!(dm_log_count(&db, transient_mapping_id, 901).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn dm_reservierung_verhindert_doppelte_sends_und_gibt_transient_frei(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;

        let emoji = ReactionType::Unicode("✅".to_string());
        let first = service.handle_reaction_add(42, 43, 100, 900, &emoji, false);
        let second = service.handle_reaction_add(42, 43, 100, 900, &emoji, false);
        tokio::try_join!(first, second)?;

        let state = mock_state(&port);
        assert_eq!(state.dms, vec![(900, "DM".to_string())]);
        assert_eq!(dm_log_count(&db, mapping_id, 900).await?, 1);

        let transient_mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 101,
                emoji: "🔥".to_string(),
                role_id: 501,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;
        port.push_dm_result(Err(DmErr::Transient("timeout".to_string())));
        service
            .handle_reaction_add(
                42,
                43,
                101,
                901,
                &ReactionType::Unicode("🔥".to_string()),
                false,
            )
            .await?;
        assert_eq!(dm_log_count(&db, transient_mapping_id, 901).await?, 0);

        port.push_dm_result(Ok(()));
        service
            .handle_reaction_add(
                42,
                43,
                101,
                901,
                &ReactionType::Unicode("🔥".to_string()),
                false,
            )
            .await?;
        let state = mock_state(&port);
        assert_eq!(
            state.dms,
            vec![
                (900, "DM".to_string()),
                (901, "DM".to_string()),
                (901, "DM".to_string()),
            ]
        );
        assert_eq!(dm_log_count(&db, transient_mapping_id, 901).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn remove_entzieht_nur_wenn_aktiviert_und_ruert_dm_log_nicht_an(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;
        insert_mapping(
            &db,
            MappingInsert {
                message_id: 101,
                emoji: "🔥".to_string(),
                role_id: 501,
                dm_enabled: true,
                remove_on_unreact: false,
                backfill_pending: false,
            },
        )
        .await?;
        service.mark_dm_logged(mapping_id, 900).await?;

        service
            .handle_reaction_remove(
                42,
                43,
                100,
                900,
                &ReactionType::Unicode("✅".to_string()),
                false,
            )
            .await?;
        service
            .handle_reaction_remove(
                42,
                43,
                101,
                901,
                &ReactionType::Unicode("🔥".to_string()),
                false,
            )
            .await?;

        let state = mock_state(&port);
        assert_eq!(state.remove_roles, vec![(42, 900, 500)]);
        assert_eq!(dm_log_count(&db, mapping_id, 900).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn backfill_paginiert_vergibt_dm_once_und_cleart_pending(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: true,
            },
        )
        .await?;
        service.mark_dm_logged(mapping_id, 1_000).await?;
        port.push_reaction_page(Ok((1_000..1_100).map(reacted_user).collect()));
        port.push_reaction_page(Ok(vec![reacted_user(2_000), reacted_user(2_001)]));

        service.run_pending_backfills().await?;

        let state = mock_state(&port);
        assert_eq!(state.reaction_fetches.len(), 2);
        assert_eq!(state.reaction_fetches[0], (43, 100, "✅".to_string(), None));
        assert_eq!(
            state.reaction_fetches[1],
            (43, 100, "✅".to_string(), Some(1_099))
        );
        assert_eq!(state.add_roles.len(), 102);
        assert_eq!(state.dms.len(), 101);
        assert!(!backfill_pending(&db, mapping_id).await?);
        assert_eq!(dm_log_count(&db, mapping_id, 2_001).await?, 1);
        Ok(())
    }

    #[tokio::test]
    async fn backfill_paginiert_nach_roher_seitenlaenge_und_filtert_bots_selbst(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: false,
                remove_on_unreact: true,
                backfill_pending: true,
            },
        )
        .await?;
        let mut first_page: Vec<ReactedUser> = (1_000..1_099).map(reacted_user).collect();
        first_page.push(reacted_bot(9_999));
        port.push_reaction_page(Ok(first_page));
        port.push_reaction_page(Ok(vec![reacted_user(2_000), reacted_user(2_001)]));

        service.run_pending_backfills().await?;

        let state = mock_state(&port);
        assert_eq!(state.reaction_fetches.len(), 2);
        assert_eq!(
            state.reaction_fetches[1],
            (43, 100, "✅".to_string(), Some(9_999))
        );
        assert_eq!(state.add_roles.len(), 101);
        assert!(!state
            .add_roles
            .iter()
            .any(|(_, user_id, _)| *user_id == 9_999));
        assert!(!backfill_pending(&db, mapping_id).await?);
        Ok(())
    }

    #[tokio::test]
    async fn backfill_pending_bleibt_bei_retryable_fehlern_gesetzt(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        let add_error_mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: false,
                remove_on_unreact: true,
                backfill_pending: true,
            },
        )
        .await?;
        let transient_dm_mapping_id = insert_mapping(
            &db,
            MappingInsert {
                message_id: 101,
                emoji: "🔥".to_string(),
                role_id: 501,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: true,
            },
        )
        .await?;
        port.push_reaction_page(Ok(vec![reacted_user(900)]));
        port.push_reaction_page(Ok(vec![reacted_user(901)]));
        port.push_add_role_result(Err(PortErr::Discord("rate limit".to_string())));
        port.push_add_role_result(Ok(()));
        port.push_dm_result(Err(DmErr::Transient("timeout".to_string())));

        service.run_pending_backfills().await?;

        assert!(backfill_pending(&db, add_error_mapping_id).await?);
        assert!(backfill_pending(&db, transient_dm_mapping_id).await?);
        assert_eq!(dm_log_count(&db, transient_dm_mapping_id, 901).await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn bot_events_werden_ignoriert() -> Result<(), Box<dyn std::error::Error>> {
        let (_dir, db, port, service) = test_service().await?;
        insert_mapping(
            &db,
            MappingInsert {
                message_id: 100,
                emoji: "✅".to_string(),
                role_id: 500,
                dm_enabled: true,
                remove_on_unreact: true,
                backfill_pending: false,
            },
        )
        .await?;

        let emoji = ReactionType::Unicode("✅".to_string());
        service
            .handle_reaction_add(42, 43, 100, 900, &emoji, true)
            .await?;
        service
            .handle_reaction_remove(42, 43, 100, 900, &emoji, true)
            .await?;

        let state = mock_state(&port);
        assert!(state.add_roles.is_empty());
        assert!(state.remove_roles.is_empty());
        assert!(state.dms.is_empty());
        Ok(())
    }
}
