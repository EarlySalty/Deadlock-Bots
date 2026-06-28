//! Generische Reaction-Roles.
//!
//! Eine konfigurierte Reaktion auf eine Nachricht vergibt eine Rolle, optional
//! mit einmaliger DM. Entfernt der User die Reaktion, kann die Rolle wieder
//! entzogen werden. Backfills laufen über Discord-REST und verwenden dieselbe
//! Rollen-/DM-Logik wie Live-Events.

use std::sync::Arc;
use std::time::Duration;

use dl_db::{Db, DbError};
use rusqlite::{params, OptionalExtension};
use serenity::all::{EmojiId, ReactionType};

#[cfg(test)]
const BACKFILL_DM_DELAY: Duration = Duration::from_millis(0);
#[cfg(not(test))]
const BACKFILL_DM_DELAY: Duration = Duration::from_secs(1);

pub const SCRIM_COACHING_REACTION_ROLE_DM_TEXT: &str = "Hey! 👋 Schön, dass du beim Scrim-Coaching dabei bist.\n\nKurz worum's geht: Wir (Leo & deniz) stellen feste Teams mit einem festen Coach zusammen, spielen regelmäßig Showmatches gegeneinander und setzen uns zwischendurch zusammen, um an euren Punkten zu arbeiten — Schritt für Schritt besser werden, als Team.\n\nDamit wir die Teams gut zusammenbekommen, schreib uns am besten direkt in <#1520842755037855975>:\n• deinen aktuellen Rang\n• deine bevorzugte Lane/Rolle\n• wann du grob Zeit hast (Wochentag/Uhrzeit)\n\nWir melden uns bei dir, sobald die Gruppen stehen. Bis gleich! 🎮";

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
    Db(#[from] DbError),
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
    db: Arc<Db>,
    port: Arc<dyn ReactionRolePort>,
}

impl ReactionRoleService {
    pub fn new(db: Arc<Db>, port: Arc<dyn ReactionRolePort>) -> Arc<Self> {
        Arc::new(Self { db, port })
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
        let canonical = canonical_emoji(emoji);
        let Some(mapping) = self.mapping_for_message(message_id, &canonical).await? else {
            return Ok(());
        };
        self.apply_mapping_add(&mapping, guild_id, user_id).await?;
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
                        .apply_mapping_add(&mapping, mapping.guild_id, user.id)
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
    ) -> Result<ApplyMappingAddOutcome, ReactionRoleError> {
        let mut outcome = ApplyMappingAddOutcome::default();
        if let Err(err) = self.port.add_role(guild_id, user_id, mapping.role_id).await {
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

    async fn mapping_for_message(
        &self,
        message_id: u64,
        emoji: &str,
    ) -> Result<Option<ReactionRoleMapping>, DbError> {
        let emoji = emoji.to_string();
        self.db
            .read(move |conn| {
                conn.query_row(
                    "SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
                            dm_enabled, dm_text, remove_on_unreact
                       FROM reaction_role_mappings
                      WHERE message_id = ?1 AND emoji = ?2 AND active = 1
                      LIMIT 1",
                    params![message_id, emoji],
                    row_to_mapping,
                )
                .optional()
            })
            .await
    }

    async fn pending_backfill_mappings(&self) -> Result<Vec<ReactionRoleMapping>, DbError> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, guild_id, source_channel_id, message_id, emoji, role_id,
                            dm_enabled, dm_text, remove_on_unreact
                       FROM reaction_role_mappings
                      WHERE backfill_pending = 1 AND active = 1
                      ORDER BY id ASC",
                )?;
                let rows = stmt.query_map([], row_to_mapping)?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn reserve_dm_log(&self, mapping_id: i64, user_id: u64) -> Result<bool, DbError> {
        let sent_at = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                let inserted = conn.execute(
                    "INSERT OR IGNORE INTO reaction_role_dm_log(mapping_id, user_id, sent_at)
                     VALUES(?1, ?2, ?3)",
                    params![mapping_id, user_id, sent_at],
                )?;
                Ok(inserted == 1)
            })
            .await
    }

    #[cfg(test)]
    async fn mark_dm_logged(&self, mapping_id: i64, user_id: u64) -> Result<(), DbError> {
        let sent_at = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO reaction_role_dm_log(mapping_id, user_id, sent_at)
                     VALUES(?1, ?2, ?3)
                     ON CONFLICT(mapping_id, user_id) DO UPDATE SET sent_at = excluded.sent_at",
                    params![mapping_id, user_id, sent_at],
                )?;
                Ok(())
            })
            .await
    }

    async fn touch_dm_logged(&self, mapping_id: i64, user_id: u64) -> Result<(), DbError> {
        let sent_at = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE reaction_role_dm_log
                        SET sent_at = ?1
                      WHERE mapping_id = ?2 AND user_id = ?3",
                    params![sent_at, mapping_id, user_id],
                )?;
                Ok(())
            })
            .await
    }

    async fn release_dm_reservation(&self, mapping_id: i64, user_id: u64) -> Result<(), DbError> {
        self.db
            .write(move |conn| {
                conn.execute(
                    "DELETE FROM reaction_role_dm_log
                      WHERE mapping_id = ?1 AND user_id = ?2",
                    params![mapping_id, user_id],
                )?;
                Ok(())
            })
            .await
    }

    async fn clear_backfill_pending(&self, mapping_id: i64) -> Result<(), DbError> {
        let updated_at = chrono::Utc::now().timestamp();
        self.db
            .write(move |conn| {
                conn.execute(
                    "UPDATE reaction_role_mappings
                        SET backfill_pending = 0, updated_at = ?1
                      WHERE id = ?2",
                    params![updated_at, mapping_id],
                )?;
                Ok(())
            })
            .await
    }
}

fn row_to_mapping(row: &rusqlite::Row<'_>) -> rusqlite::Result<ReactionRoleMapping> {
    Ok(ReactionRoleMapping {
        id: row.get(0)?,
        guild_id: row.get(1)?,
        source_channel_id: row.get(2)?,
        message_id: row.get(3)?,
        emoji: row.get(4)?,
        role_id: row.get(5)?,
        dm_enabled: row.get::<_, i64>(6)? != 0,
        dm_text: row.get(7)?,
        remove_on_unreact: row.get::<_, i64>(8)? != 0,
    })
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::{Mutex, MutexGuard};

    use super::*;

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

    async fn test_service() -> Result<
        (
            tempfile::TempDir,
            Arc<Db>,
            Arc<MockPort>,
            Arc<ReactionRoleService>,
        ),
        Box<dyn std::error::Error>,
    > {
        let dir = tempfile::tempdir()?;
        let db = Arc::new(Db::open_creating(
            dir.path().join("reaction_roles.sqlite3"),
        )?);
        db.bootstrap_schema().await?;
        let port = Arc::new(MockPort::default());
        let service = ReactionRoleService::new(db.clone(), port.clone());
        Ok((dir, db, port, service))
    }

    struct MappingInsert {
        message_id: u64,
        emoji: String,
        role_id: u64,
        dm_enabled: bool,
        remove_on_unreact: bool,
        backfill_pending: bool,
    }

    async fn insert_mapping(db: &Db, insert: MappingInsert) -> Result<i64, DbError> {
        db.write(move |conn| {
            conn.execute(
                "INSERT INTO reaction_role_mappings(
                    guild_id, source_channel_id, message_id, emoji, role_id,
                    dm_enabled, dm_text, remove_on_unreact, backfill_pending,
                    active, created_at, updated_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, 10, 10)",
                params![
                    42_u64,
                    43_u64,
                    insert.message_id,
                    insert.emoji,
                    insert.role_id,
                    insert.dm_enabled as i64,
                    "DM",
                    insert.remove_on_unreact as i64,
                    insert.backfill_pending as i64,
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
        .await
    }

    async fn dm_log_count(db: &Db, mapping_id: i64, user_id: u64) -> Result<i64, DbError> {
        db.read(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM reaction_role_dm_log
                  WHERE mapping_id = ?1 AND user_id = ?2",
                params![mapping_id, user_id],
                |row| row.get(0),
            )
        })
        .await
    }

    async fn backfill_pending(db: &Db, mapping_id: i64) -> Result<bool, DbError> {
        db.read(move |conn| {
            conn.query_row(
                "SELECT backfill_pending FROM reaction_role_mappings WHERE id = ?1",
                params![mapping_id],
                |row| row.get::<_, i64>(0).map(|value| value != 0),
            )
        })
        .await
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
