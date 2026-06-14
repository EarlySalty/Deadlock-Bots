//! Streamer-Partner-Verknüpfung (vereinfacht) — Scheibe 1: Absicht erfassen.
//!
//! Ersetzt den alten 2-Schritt-Twitch-Autorisierungs-Flow (`step_streamer.py`)
//! durch einen schlanken Weg: `/streamer` verweist auf die Website (dort
//! verbindet der User seinen Twitch-Account) und merkt sich die Discord-ID mit
//! einem 1-Stunden-Fenster. Ein Watcher (Scheibe 2) verknüpft die gemerkte
//! Discord-ID dann mit einem Streamer, der in genau diesem Fenster neu auf der
//! Twitch-Bot-Seite auftaucht — die zeitliche Nähe ist das Korrelationssignal.

use std::sync::Arc;

use dl_db::{Db, DbError};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use rusqlite::params;
use serde_json::json;

/// Website, auf der der Streamer seinen Twitch-Account verbindet.
pub const STREAMER_ONBOARDING_URL: &str =
    "https://deutsche-deadlock-community.de/streamer/onboarding/";

/// Korrelationsfenster: so lange nach `/streamer` gilt ein neu auftauchender
/// Streamer als „derselbe" und wird verknüpft (Wunsch: max 1 Stunde).
pub const INTENT_TTL_SECS: i64 = 3600;

/// Eine offene Verknüpfungs-Absicht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamerIntent {
    pub discord_id: u64,
    pub discord_name: String,
    pub created_at: i64,
}

/// Store über `streamer_link_intents`.
#[derive(Clone)]
pub struct StreamerIntents {
    db: Db,
}

impl StreamerIntents {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// Legt die Tabelle an (idempotent).
    pub async fn ensure_schema(&self) -> Result<(), DbError> {
        self.db
            .write(|conn| {
                conn.execute_batch(
                    "CREATE TABLE IF NOT EXISTS streamer_link_intents(
                       discord_id   INTEGER PRIMARY KEY,
                       discord_name TEXT NOT NULL,
                       created_at   INTEGER NOT NULL,
                       expires_at   INTEGER NOT NULL,
                       status       TEXT NOT NULL DEFAULT 'pending',
                       linked_login TEXT
                     );",
                )?;
                Ok(())
            })
            .await
    }

    /// Merkt sich (oder erneuert) die Absicht eines Users, Streamer zu werden.
    /// Ein erneuter `/streamer`-Aufruf frischt das 1-h-Fenster wieder auf.
    pub async fn record_intent(
        &self,
        discord_id: u64,
        discord_name: String,
        now: i64,
    ) -> Result<(), DbError> {
        let expires = now + INTENT_TTL_SECS;
        self.db
            .write(move |conn| {
                conn.execute(
                    "INSERT INTO streamer_link_intents
                       (discord_id, discord_name, created_at, expires_at, status, linked_login)
                     VALUES (?1, ?2, ?3, ?4, 'pending', NULL)
                     ON CONFLICT(discord_id) DO UPDATE SET
                       discord_name = excluded.discord_name,
                       created_at   = excluded.created_at,
                       expires_at   = excluded.expires_at,
                       status       = 'pending',
                       linked_login = NULL",
                    params![discord_id, discord_name, now, expires],
                )
                .map(|_| ())
            })
            .await
    }

    /// Noch offene Absichten im Fenster (jüngste zuerst — wer gerade `/streamer`
    /// gemacht hat, ist am wahrscheinlichsten der, der gerade autorisiert).
    pub async fn pending_intents(&self, now: i64) -> Vec<StreamerIntent> {
        self.db
            .read(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT discord_id, discord_name, created_at
                       FROM streamer_link_intents
                      WHERE status = 'pending' AND expires_at >= ?1
                      ORDER BY created_at DESC",
                )?;
                let rows = stmt.query_map(params![now], |r| {
                    Ok(StreamerIntent {
                        discord_id: r.get::<_, i64>(0)? as u64,
                        discord_name: r.get(1)?,
                        created_at: r.get(2)?,
                    })
                })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
            .unwrap_or_default()
    }

    /// Markiert eine Absicht als verknüpft (vom Watcher aufgerufen).
    pub async fn mark_linked(&self, discord_id: u64, login: &str) {
        let login = login.to_string();
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE streamer_link_intents
                        SET status = 'linked', linked_login = ?2
                      WHERE discord_id = ?1",
                    params![discord_id, login],
                )
                .map(|_| ())
            })
            .await;
    }

    /// Schließt abgelaufene Absichten (`pending` → `expired`).
    pub async fn expire_old(&self, now: i64) {
        let _ = self
            .db
            .write(move |conn| {
                conn.execute(
                    "UPDATE streamer_link_intents
                        SET status = 'expired'
                      WHERE status = 'pending' AND expires_at < ?1",
                    params![now],
                )
                .map(|_| ())
            })
            .await;
    }
}

struct StreamerHandler {
    intents: StreamerIntents,
}

#[async_trait::async_trait]
impl InteractionHandler for StreamerHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        if interaction.command == "streamer" {
            let now = chrono::Utc::now().timestamp();
            if let Err(err) = self
                .intents
                .record_intent(interaction.user_id, interaction.author_name.clone(), now)
                .await
            {
                tracing::warn!(%err, "Streamer-Intent konnte nicht gespeichert werden");
            }
            return BridgeReply::ephemeral_text(format!(
                "🎥 **Streamer-Partner werden**\n\n\
                 Verbinde deinen Twitch-Account hier:\n{STREAMER_ONBOARDING_URL}\n\n\
                 Sobald du dich in der nächsten **Stunde** dort verbindest, verknüpfen wir \
                 deinen Discord- und Twitch-Account **automatisch** — du musst nichts weiter tun."
            ));
        }
        BridgeReply::ephemeral_text("Unbekannte Aktion.")
    }
}

/// Registriert die `/streamer`-Slash-Command.
pub fn register(router: &mut InteractionRouter, intents: StreamerIntents) {
    let handler = Arc::new(StreamerHandler { intents });
    router.on_command(
        "streamer",
        CommandSpec {
            definition: json!({
                "name": "streamer",
                "description": "Streamer-Partner werden – verbinde deinen Twitch-Account.",
                "type": 1,
            }),
        },
        handler,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mk() -> (tempfile::TempDir, StreamerIntents) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Db::open_creating(dir.path().join("s.sqlite3")).expect("db");
        let intents = StreamerIntents::new(db);
        intents.ensure_schema().await.expect("schema");
        (dir, intents)
    }

    #[tokio::test]
    async fn record_und_pending() {
        let (_d, store) = mk().await;
        let now = 1_000_000_000;
        store.record_intent(42, "Alice".into(), now).await.unwrap();
        let pending = store.pending_intents(now).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].discord_id, 42);
        assert_eq!(pending[0].discord_name, "Alice");
    }

    #[tokio::test]
    async fn abgelaufene_fallen_aus_dem_fenster() {
        let (_d, store) = mk().await;
        let now = 1_000_000_000;
        store.record_intent(7, "Bob".into(), now).await.unwrap();
        // 1 Sekunde nach Ablauf des 1-h-Fensters.
        let later = now + INTENT_TTL_SECS + 1;
        assert!(store.pending_intents(later).await.is_empty());
        store.expire_old(later).await;
        // Auch nach expire_old bleibt sie draußen (jetzt status='expired').
        assert!(store.pending_intents(now).await.is_empty());
    }

    #[tokio::test]
    async fn mark_linked_entfernt_aus_pending() {
        let (_d, store) = mk().await;
        let now = 1_000_000_000;
        store.record_intent(5, "Cara".into(), now).await.unwrap();
        store.mark_linked(5, "cara_ttv").await;
        assert!(store.pending_intents(now).await.is_empty());
    }

    #[tokio::test]
    async fn erneuter_aufruf_frischt_fenster_auf() {
        let (_d, store) = mk().await;
        let first = 1_000_000_000;
        store.record_intent(9, "Dee".into(), first).await.unwrap();
        // Kurz vor Ablauf erneut /streamer → Fenster startet neu.
        let again = first + INTENT_TTL_SECS - 10;
        store.record_intent(9, "Dee".into(), again).await.unwrap();
        // Zu einem Zeitpunkt, der nur dank Auffrischung noch im Fenster liegt.
        let check = first + INTENT_TTL_SECS + 100;
        let pending = store.pending_intents(check).await;
        assert_eq!(pending.len(), 1, "Fenster wurde durch den 2. Aufruf erneuert");
        assert_eq!(pending[0].created_at, again);
    }
}
