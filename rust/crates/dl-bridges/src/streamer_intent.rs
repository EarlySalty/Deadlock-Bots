//! Streamer-Partner-Verknüpfung (vereinfacht) — Scheibe 1: Absicht erfassen.
//!
//! Ersetzt den alten 2-Schritt-Twitch-Autorisierungs-Flow (`step_streamer.py`)
//! durch einen schlanken Weg: `/streamer` verweist auf die Website (dort
//! verbindet der User seinen Twitch-Account) und merkt sich die Discord-ID mit
//! einem 1-Stunden-Fenster. Ein Watcher (Scheibe 2) verknüpft die gemerkte
//! Discord-ID dann mit einem Streamer, der in genau diesem Fenster neu auf der
//! Twitch-Bot-Seite auftaucht — die zeitliche Nähe ist das Korrelationssignal.

use std::collections::HashSet;
use std::num::TryFromIntError;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Value};
use sqlx::PgPool;

use crate::matcher::{norm_key, similarity};
use crate::twitch::TwitchApiClient;

/// Website, auf der der Streamer seinen Twitch-Account verbindet.
pub const STREAMER_ONBOARDING_URL: &str =
    "https://deutsche-deadlock-community.de/streamer/onboarding/";

/// Korrelationsfenster: so lange nach `/streamer` gilt ein neu auftauchender
/// Streamer als „derselbe" und wird verknüpft (Wunsch: max 1 Stunde).
pub const INTENT_TTL_SECS: i64 = 3600;

/// Poll-Takt des Watchers — deutlich häufiger als der 6-h-Matcher, damit das
/// 1-h-Fenster zuverlässig getroffen wird.
const POLL_INTERVAL: Duration = Duration::from_secs(180);

/// Namens-Ähnlichkeits-Schwelle für die Disambiguierung bei mehreren offenen
/// Absichten (gleicher Boden wie der Matcher).
const FUZZY_FLOOR: f64 = 0.62;

#[derive(Debug, thiserror::Error)]
pub enum StreamerIntentError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error("Discord-ID {value} passt nicht in PostgreSQL BIGINT")]
    DiscordIdOutOfRange { value: u64, source: TryFromIntError },
    #[error("Unix-Zeitstempel ist ausserhalb des chrono-Bereichs: {0}")]
    TimestampOutOfRange(i64),
}

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
    pool: PgPool,
}

impl StreamerIntents {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Prüft, dass die zentrale Migration die Tabelle bereitgestellt hat.
    pub async fn ensure_schema(&self) -> Result<(), StreamerIntentError> {
        sqlx::query!(
            r#"
            SELECT 1 AS "ok!"
            FROM bot.streamer_link_intents
            LIMIT 0
            "#
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(())
    }

    /// Merkt sich (oder erneuert) die Absicht eines Users, Streamer zu werden.
    /// Ein erneuter `/streamer`-Aufruf frischt das 1-h-Fenster wieder auf.
    pub async fn record_intent(
        &self,
        discord_id: u64,
        discord_name: String,
        now: i64,
    ) -> Result<(), StreamerIntentError> {
        let discord_id = discord_id_to_i64(discord_id)?;
        let created_at = utc_from_unix_seconds(now)?;
        let expires_at = expires_at_from(now)?;

        sqlx::query!(
            r#"
            INSERT INTO bot.streamer_link_intents
                (discord_id, discord_name, created_at, expires_at, status, linked_login)
            VALUES ($1, $2, $3, $4, 'pending', NULL)
            ON CONFLICT (discord_id) DO UPDATE
            SET discord_name = EXCLUDED.discord_name,
                created_at = EXCLUDED.created_at,
                expires_at = EXCLUDED.expires_at,
                status = 'pending',
                linked_login = NULL
            "#,
            discord_id,
            discord_name,
            created_at,
            expires_at,
        )
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Noch offene Absichten im Fenster (jüngste zuerst — wer gerade `/streamer`
    /// gemacht hat, ist am wahrscheinlichsten der, der gerade autorisiert).
    pub async fn pending_intents(&self, now: i64) -> Vec<StreamerIntent> {
        let Ok(now) = utc_from_unix_seconds(now) else {
            return Vec::new();
        };

        let rows = sqlx::query!(
            r#"
            SELECT discord_id, discord_name, created_at
            FROM bot.streamer_link_intents
            WHERE status = 'pending'
              AND expires_at >= $1
            ORDER BY created_at DESC
            "#,
            now,
        )
        .fetch_all(&self.pool)
        .await;

        match rows {
            Ok(rows) => rows
                .into_iter()
                .filter_map(|row| {
                    let discord_id = u64::try_from(row.discord_id).ok()?;
                    Some(StreamerIntent {
                        discord_id,
                        discord_name: row.discord_name,
                        created_at: row.created_at.timestamp(),
                    })
                })
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Markiert eine Absicht als verknüpft (vom Watcher aufgerufen).
    pub async fn mark_linked(&self, discord_id: u64, login: &str) {
        let Ok(discord_id) = discord_id_to_i64(discord_id) else {
            return;
        };
        let login = login.to_string();
        let _ = sqlx::query!(
            r#"
            UPDATE bot.streamer_link_intents
            SET status = 'linked',
                linked_login = $2
            WHERE discord_id = $1
            "#,
            discord_id,
            login,
        )
        .execute(&self.pool)
        .await;
    }

    /// Schließt abgelaufene Absichten (`pending` → `expired`).
    pub async fn expire_old(&self, now: i64) {
        let Ok(now) = utc_from_unix_seconds(now) else {
            return;
        };

        let _ = sqlx::query!(
            r#"
            UPDATE bot.streamer_link_intents
            SET status = 'expired'
            WHERE status = 'pending'
              AND expires_at < $1
            "#,
            now,
        )
        .execute(&self.pool)
        .await;
    }
}

fn discord_id_to_i64(discord_id: u64) -> Result<i64, StreamerIntentError> {
    i64::try_from(discord_id).map_err(|source| StreamerIntentError::DiscordIdOutOfRange {
        value: discord_id,
        source,
    })
}

fn utc_from_unix_seconds(value: i64) -> Result<DateTime<Utc>, StreamerIntentError> {
    DateTime::from_timestamp(value, 0).ok_or(StreamerIntentError::TimestampOutOfRange(value))
}

fn expires_at_from(now: i64) -> Result<DateTime<Utc>, StreamerIntentError> {
    let expires = now
        .checked_add(INTENT_TTL_SECS)
        .ok_or(StreamerIntentError::TimestampOutOfRange(now))?;
    utc_from_unix_seconds(expires)
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

/// Ordnet einem neu aufgetauchten Twitch-Login die passende offene Absicht zu.
///
/// Primär über Namens-Ähnlichkeit (entscheidet, wenn mehrere Leute warten); gibt
/// es keinen Namens-Treffer über der Schwelle, greift die reine Zeit-Korrelation
/// nur, wenn GENAU eine Absicht offen ist — bei mehreren wäre eine Zuordnung zu
/// unsicher (→ `None`, der 6-h-Matcher übernimmt später per Namensabgleich).
pub fn correlate(login: &str, pending: &[StreamerIntent], fuzzy_floor: f64) -> Option<u64> {
    let login_key = norm_key(login);
    let best = pending
        .iter()
        .map(|intent| {
            (
                intent.discord_id,
                similarity(&login_key, &norm_key(&intent.discord_name)),
            )
        })
        .filter(|(_, score)| *score >= fuzzy_floor)
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    if let Some((discord_id, _)) = best {
        return Some(discord_id);
    }
    if pending.len() == 1 {
        return Some(pending[0].discord_id);
    }
    None
}

/// Extrahiert die Twitch-Logins aus den `link_candidates`-Einträgen
/// (Feld `twitch_login`, normalisiert wie der Matcher).
fn candidate_logins(candidates: &[Value]) -> Vec<String> {
    candidates
        .iter()
        .filter_map(|entry| entry.get("twitch_login").and_then(Value::as_str))
        .map(|login| login.trim().to_lowercase())
        .filter(|login| !login.is_empty())
        .collect()
}

/// Watcher (Scheibe 2): pollt die Link-Candidates der Twitch-Bot-Seite, erkennt
/// neu auftauchende Streamer und verknüpft sie mit einer offenen `/streamer`-
/// Absicht im 1-h-Fenster. `link-candidates` liefert nur UNverknüpfte Streamer
/// — sobald verknüpft, verschwindet ein Login aus der Liste, also kein Konflikt
/// mit dem 6-h-Matcher.
pub fn spawn_watcher(
    intents: StreamerIntents,
    client: Arc<TwitchApiClient>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Beim ersten Lauf alle bestehenden Candidates als „gesehen" markieren,
        // damit ein Bot-Start keine Alt-Streamer fälschlich als „neu" verknüpft.
        let mut seen: HashSet<String> = HashSet::new();
        let mut baselined = false;
        loop {
            let now = chrono::Utc::now().timestamp();
            intents.expire_old(now).await;

            match client.link_candidates().await {
                Ok(candidates) => {
                    let logins = candidate_logins(&candidates);
                    if !baselined {
                        seen.extend(logins);
                        baselined = true;
                    } else {
                        let mut pending = intents.pending_intents(now).await;
                        for login in logins {
                            if !seen.insert(login.clone()) {
                                continue; // schon bekannt
                            }
                            if pending.is_empty() {
                                continue;
                            }
                            let Some(discord_id) = correlate(&login, &pending, FUZZY_FLOOR) else {
                                continue;
                            };
                            let name = pending
                                .iter()
                                .find(|i| i.discord_id == discord_id)
                                .map(|i| i.discord_name.clone())
                                .unwrap_or_default();
                            match client.link_discord_profile(&login, discord_id, &name).await {
                                Ok(_) => {
                                    intents.mark_linked(discord_id, &login).await;
                                    pending.retain(|i| i.discord_id != discord_id);
                                    tracing::info!(
                                        login = %login,
                                        discord_id,
                                        "Streamer-Absicht automatisch verknüpft"
                                    );
                                }
                                Err(err) => tracing::warn!(
                                    %err, login = %login,
                                    "Streamer-Verknüpfung fehlgeschlagen"
                                ),
                            }
                        }
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, "link-candidates-Abruf fehlgeschlagen");
                }
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "testing")]
    async fn mk() -> Result<(dl_central_db::TestDb, StreamerIntents), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let intents = StreamerIntents::new(db.pool().clone());
        intents.ensure_schema().await.expect("schema");
        Ok((db, intents))
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn record_und_pending() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        let now = 1_000_000_000;
        store.record_intent(42, "Alice".into(), now).await?;
        let pending = store.pending_intents(now).await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].discord_id, 42);
        assert_eq!(pending[0].discord_name, "Alice");
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn abgelaufene_fallen_aus_dem_fenster() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        let now = 1_000_000_000;
        store.record_intent(7, "Bob".into(), now).await?;
        // 1 Sekunde nach Ablauf des 1-h-Fensters.
        let later = now + INTENT_TTL_SECS + 1;
        assert!(store.pending_intents(later).await.is_empty());
        store.expire_old(later).await;
        // Auch nach expire_old bleibt sie draußen (jetzt status='expired').
        assert!(store.pending_intents(now).await.is_empty());
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn mark_linked_entfernt_aus_pending() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        let now = 1_000_000_000;
        store.record_intent(5, "Cara".into(), now).await?;
        store.mark_linked(5, "cara_ttv").await;
        assert!(store.pending_intents(now).await.is_empty());
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn erneuter_aufruf_frischt_fenster_auf() -> Result<(), Box<dyn std::error::Error>> {
        let (_db, store) = mk().await?;
        let first = 1_000_000_000;
        store.record_intent(9, "Dee".into(), first).await?;
        // Kurz vor Ablauf erneut /streamer → Fenster startet neu.
        let again = first + INTENT_TTL_SECS - 10;
        store.record_intent(9, "Dee".into(), again).await?;
        // Zu einem Zeitpunkt, der nur dank Auffrischung noch im Fenster liegt.
        let check = first + INTENT_TTL_SECS + 100;
        let pending = store.pending_intents(check).await;
        assert_eq!(
            pending.len(),
            1,
            "Fenster wurde durch den 2. Aufruf erneuert"
        );
        assert_eq!(pending[0].created_at, again);
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn zu_grosse_discord_id_schreibt_keine_absicht() -> Result<(), Box<dyn std::error::Error>>
    {
        let (db, store) = mk().await?;
        let too_large = i64::MAX as u64 + 1;

        let err = store
            .record_intent(too_large, "Overflow".into(), 1_000_000_000)
            .await
            .expect_err("discord id above PG BIGINT must fail");

        assert!(matches!(
            err,
            StreamerIntentError::DiscordIdOutOfRange { value, .. } if value == too_large
        ));
        let row = sqlx::query!(
            r#"
            SELECT COUNT(*) AS "count!"
            FROM bot.streamer_link_intents
            "#
        )
        .fetch_one(db.pool())
        .await?;
        assert_eq!(row.count, 0);
        Ok(())
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn pg_constraint_rejects_missing_discord_name() -> Result<(), Box<dyn std::error::Error>>
    {
        let (db, _store) = mk().await?;
        let now = utc_from_unix_seconds(1_000_000_000)?;
        let expires_at = expires_at_from(1_000_000_000)?;

        let err = sqlx::query!(
            r#"
            INSERT INTO bot.streamer_link_intents
                (discord_id, discord_name, created_at, expires_at)
            VALUES ($1, $2, $3, $4)
            "#,
            1_i64,
            Option::<&str>::None,
            now,
            expires_at,
        )
        .execute(db.pool())
        .await
        .expect_err("discord_name NOT NULL constraint must reject NULL");

        let code = err.as_database_error().and_then(|db_err| db_err.code());
        assert_eq!(code.as_deref(), Some("23502"));
        Ok(())
    }

    fn intent(id: u64, name: &str) -> StreamerIntent {
        StreamerIntent {
            discord_id: id,
            discord_name: name.into(),
            created_at: 0,
        }
    }

    #[test]
    fn correlate_einzelne_absicht_greift_per_zeit() {
        // Kein Namens-Bezug, aber genau eine offene Absicht → Zeit-Korrelation.
        let pending = vec![intent(1, "wwwwwwww")];
        assert_eq!(correlate("dragskope", &pending, FUZZY_FLOOR), Some(1));
    }

    #[test]
    fn correlate_mehrere_disambiguiert_per_name() {
        let pending = vec![intent(1, "wwwwwwww"), intent(2, "dragskope")];
        assert_eq!(correlate("dragskope", &pending, FUZZY_FLOOR), Some(2));
    }

    #[test]
    fn correlate_mehrere_ohne_namenstreffer_ist_none() {
        let pending = vec![intent(1, "wwwwwwww"), intent(2, "vvvvvvvv")];
        assert_eq!(correlate("dragskope", &pending, FUZZY_FLOOR), None);
    }

    #[test]
    fn correlate_ohne_absicht_ist_none() {
        assert_eq!(correlate("dragskope", &[], FUZZY_FLOOR), None);
    }

    #[test]
    fn candidate_logins_extrahiert_und_normalisiert() {
        let candidates = vec![
            json!({ "twitch_login": " DragSkope " }),
            json!({ "twitch_login": "" }),
            json!({ "foo": "bar" }),
            json!({ "twitch_login": "Live_TTV" }),
        ];
        assert_eq!(
            candidate_logins(&candidates),
            vec!["dragskope".to_string(), "live_ttv".to_string()]
        );
    }
}
