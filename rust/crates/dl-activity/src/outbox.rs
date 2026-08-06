//! Generischer Zusteller für `bot.action_outbox`.
//!
//! Der Dispatcher kennt keine Fachlogik. Er pollt ausschließlich Zeilen, für
//! deren `action_type` ein Handler registriert ist, ruft den Handler in der
//! bereits offenen Transaktion auf und schreibt erst danach den Status. Eine
//! Zeile wechselt nur nach nachweislich erfolgreicher Verarbeitung auf `sent`;
//! bricht der Handler ab, rollt die Transaktion zurück und die Zeile bleibt
//! `pending`.
//!
//! Zeilen mit einem `action_type` ohne Handler werden nie angefasst: sie
//! bleiben `pending`, werden aber pro Poll-Zyklus gezählt und gemeldet. Damit
//! ist ein fehlender Handler ein sichtbarer Befund und kein stiller Verlust.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Row, Transaction};

/// Abstand zwischen zwei Poll-Zyklen des Hintergrund-Tasks.
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

/// Obergrenze für Fehler- und Unterdrückungstexte, die in die Tabelle gehen.
const REASON_MAX_CHARS: usize = 1_000;

#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// Eine Outbox-Zeile in der Form, in der ein Handler sie sieht.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxRow {
    pub id: i64,
    pub action_type: String,
    pub user_id: i64,
    pub guild_id: i64,
    pub payload: Value,
}

/// Urteil eines Handlers über genau eine Zeile.
///
/// Der Handler entscheidet, der Dispatcher schreibt — so kann kein Handler
/// vergessen, den Status zu setzen, und kein Status entsteht ohne Urteil.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerOutcome {
    /// Aktion nachweislich ausgeführt.
    Sent,
    /// Aktion bewusst nicht ausgeführt, weil ein Gate sie verboten hat.
    Suppressed { reason: String },
    /// Aktion versucht und fehlgeschlagen.
    Failed { error: String },
}

/// Fachlogik für genau einen `action_type`.
///
/// Der Handler bekommt die offene Transaktion, damit seine eigenen Schreibvorgänge
/// (Zähler, Buchungen) mit dem Statuswechsel der Zeile zusammen committen.
#[async_trait::async_trait]
pub trait OutboxHandler: Send + Sync {
    async fn handle(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        row: &OutboxRow,
        now: DateTime<Utc>,
    ) -> Result<HandlerOutcome, OutboxError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchOutcome {
    Empty,
    Sent,
    Suppressed,
    Failed,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DrainSummary {
    pub sent: u64,
    pub suppressed: u64,
    pub failed: u64,
}

impl DrainSummary {
    pub fn total(&self) -> u64 {
        self.sent + self.suppressed + self.failed
    }
}

/// Zählung der fälligen Zeilen, für die kein Handler registriert ist.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UnhandledReport {
    pub total: i64,
    pub by_action_type: Vec<(String, i64)>,
}

impl UnhandledReport {
    /// Kompakte Auflistung für Logzeilen: `lfg_invite=12, connect_suggest=3`.
    pub fn summary(&self) -> String {
        self.by_action_type
            .iter()
            .map(|(action_type, count)| format!("{action_type}={count}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub struct OutboxDispatcher {
    pool: PgPool,
    handlers: BTreeMap<String, Arc<dyn OutboxHandler>>,
}

impl OutboxDispatcher {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            handlers: BTreeMap::new(),
        }
    }

    /// Registriert den Handler für einen `action_type`.
    ///
    /// Ohne Registrierung wird ein `action_type` nie zugestellt — das ist die
    /// Sperre gegen ungewollte Außenwirkung, nicht ein Env-Flag.
    #[must_use]
    pub fn register(
        mut self,
        action_type: impl Into<String>,
        handler: Arc<dyn OutboxHandler>,
    ) -> Self {
        let action_type = action_type.into();
        if self.handlers.insert(action_type.clone(), handler).is_some() {
            tracing::warn!(
                %action_type,
                "Outbox-Handler doppelt registriert, die spätere Registrierung gewinnt"
            );
        }
        self
    }

    pub fn registered_action_types(&self) -> Vec<String> {
        self.handlers.keys().cloned().collect()
    }

    /// Holt genau eine fällige Zeile mit registriertem Handler und verarbeitet sie.
    pub async fn dispatch_one(&self, now: DateTime<Utc>) -> Result<DispatchOutcome, OutboxError> {
        let action_types = self.registered_action_types();
        if action_types.is_empty() {
            return Ok(DispatchOutcome::Empty);
        }
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT id, action_type, user_id, guild_id, payload
               FROM bot.action_outbox
              WHERE status = 'pending'
                AND scheduled_for <= $1
                AND action_type = ANY($2)
              ORDER BY priority DESC, scheduled_for, id
              FOR UPDATE SKIP LOCKED
              LIMIT 1",
        )
        .bind(now)
        .bind(&action_types[..])
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(DispatchOutcome::Empty);
        };
        let row = OutboxRow {
            id: row.try_get("id")?,
            action_type: row.try_get("action_type")?,
            user_id: row.try_get("user_id")?,
            guild_id: row.try_get("guild_id")?,
            payload: row.try_get("payload")?,
        };
        let Some(handler) = self.handlers.get(&row.action_type) else {
            tx.commit().await?;
            tracing::warn!(
                id = row.id,
                action_type = %row.action_type,
                "Outbox-Zeile ohne Handler ausgewählt, bleibt pending"
            );
            return Ok(DispatchOutcome::Empty);
        };
        let outcome = match handler.handle(&mut tx, &row, now).await {
            Ok(outcome) => outcome,
            Err(error) => {
                tracing::warn!(
                    id = row.id,
                    action_type = %row.action_type,
                    user_id = row.user_id,
                    %error,
                    "Outbox-Handler brach ab, Zeile bleibt pending"
                );
                return Err(error);
            }
        };
        match &outcome {
            HandlerOutcome::Sent => {
                sqlx::query(
                    "UPDATE bot.action_outbox
                        SET status = 'sent', sent_at = $2, error = NULL
                      WHERE id = $1",
                )
                .bind(row.id)
                .bind(now)
                .execute(&mut *tx)
                .await?;
            }
            HandlerOutcome::Suppressed { reason } => {
                let reason = truncate(reason);
                sqlx::query(
                    "UPDATE bot.action_outbox
                        SET status = 'suppressed', suppress_reason = $2
                      WHERE id = $1",
                )
                .bind(row.id)
                .bind(&reason)
                .execute(&mut *tx)
                .await?;
            }
            HandlerOutcome::Failed { error } => {
                let error = truncate(error);
                sqlx::query("UPDATE bot.action_outbox SET status = 'failed', error = $2 WHERE id = $1")
                    .bind(row.id)
                    .bind(&error)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        tx.commit().await?;
        match outcome {
            HandlerOutcome::Sent => {
                tracing::info!(
                    id = row.id,
                    action_type = %row.action_type,
                    user_id = row.user_id,
                    decision = "gesendet",
                    "Outbox-Zustellung"
                );
                Ok(DispatchOutcome::Sent)
            }
            HandlerOutcome::Suppressed { reason } => {
                tracing::info!(
                    id = row.id,
                    action_type = %row.action_type,
                    user_id = row.user_id,
                    decision = "unterdrückt",
                    reason = %reason,
                    "Outbox-Zustellung"
                );
                Ok(DispatchOutcome::Suppressed)
            }
            HandlerOutcome::Failed { error } => {
                tracing::warn!(
                    id = row.id,
                    action_type = %row.action_type,
                    user_id = row.user_id,
                    decision = "fehlgeschlagen",
                    grund = %error,
                    "Outbox-Zustellung"
                );
                Ok(DispatchOutcome::Failed)
            }
        }
    }

    /// Verarbeitet alle fälligen Zeilen mit Handler, bis nichts mehr übrig ist.
    pub async fn drain(&self, now: DateTime<Utc>) -> Result<DrainSummary, OutboxError> {
        let mut summary = DrainSummary::default();
        loop {
            match self.dispatch_one(now).await? {
                DispatchOutcome::Empty => break,
                DispatchOutcome::Sent => summary.sent += 1,
                DispatchOutcome::Suppressed => summary.suppressed += 1,
                DispatchOutcome::Failed => summary.failed += 1,
            }
        }
        Ok(summary)
    }

    /// Zählt fällige Zeilen, für die kein Handler registriert ist.
    ///
    /// Diese Zeilen werden nicht angefasst — weder gesendet noch als `failed`
    /// markiert. Die Zählung ist der Beweis, dass sie nicht verloren gehen.
    pub async fn report_unhandled(
        &self,
        now: DateTime<Utc>,
    ) -> Result<UnhandledReport, OutboxError> {
        let action_types = self.registered_action_types();
        let rows = sqlx::query(
            "SELECT action_type, COUNT(*)::BIGINT AS pending
               FROM bot.action_outbox
              WHERE status = 'pending'
                AND scheduled_for <= $1
                AND NOT (action_type = ANY($2))
              GROUP BY action_type
              ORDER BY action_type",
        )
        .bind(now)
        .bind(&action_types[..])
        .fetch_all(&self.pool)
        .await?;
        let mut by_action_type = Vec::with_capacity(rows.len());
        for row in rows {
            by_action_type.push((row.try_get("action_type")?, row.try_get("pending")?));
        }
        let total = by_action_type.iter().map(|(_, count)| *count).sum();
        let report = UnhandledReport {
            total,
            by_action_type,
        };
        if report.total > 0 {
            tracing::debug!(
                pending_without_handler = report.total,
                action_types = %report.summary(),
                "Outbox: Zeilen ohne Handler"
            );
        }
        Ok(report)
    }
}

fn truncate(value: &str) -> String {
    value.chars().take(REASON_MAX_CHARS).collect()
}

/// Startet den Dispatcher als eigenen Hintergrund-Task.
///
/// Der Task hat einen eigenen Lebenszyklus und hängt an keinem Feature-Flag:
/// welche Aktionen tatsächlich zugestellt werden, entscheidet allein die
/// Handler-Registrierung.
pub fn spawn(dispatcher: OutboxDispatcher) -> tokio::task::JoinHandle<()> {
    let action_types = dispatcher.registered_action_types();
    tokio::spawn(async move {
        if action_types.is_empty() {
            tracing::warn!("Outbox-Dispatcher ohne Handler gestartet, es wird nichts zugestellt");
        } else {
            tracing::info!(
                handlers = %action_types.join(", "),
                "Outbox-Dispatcher gestartet"
            );
        }
        let mut timer = tokio::time::interval(POLL_INTERVAL);
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Der Zähler der Zeilen ohne Handler steht bei stehendem Rückstand still.
        // Damit das Journal nicht zuläuft, wird nur die Änderung als Warnung
        // gemeldet; die Zählung selbst läuft in jedem Zyklus.
        let mut last_unhandled: Option<UnhandledReport> = None;
        loop {
            timer.tick().await;
            let now = Utc::now();
            match dispatcher.drain(now).await {
                Ok(summary) if summary.total() > 0 => tracing::info!(
                    sent = summary.sent,
                    suppressed = summary.suppressed,
                    failed = summary.failed,
                    "Outbox-Zyklus abgeschlossen"
                ),
                Ok(_) => {}
                Err(error) => tracing::warn!(%error, "Outbox-Zyklus abgebrochen"),
            }
            match dispatcher.report_unhandled(now).await {
                Ok(report) => {
                    if report.total > 0 && last_unhandled.as_ref() != Some(&report) {
                        tracing::warn!(
                            pending_without_handler = report.total,
                            "Outbox: {} Zeilen ohne Handler, action_types: {}",
                            report.total,
                            report.summary()
                        );
                    }
                    last_unhandled = Some(report);
                }
                Err(error) => tracing::warn!(
                    %error,
                    "Outbox: Zeilen ohne Handler konnten nicht gezählt werden"
                ),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "testing")]
    use crate::survey_pulse::{
        SurveyPulseConfig, SurveyPulseOutboxHandler, SurveyPulsePort, SURVEY_PULSE_ACTION_TYPE,
    };
    #[cfg(feature = "testing")]
    use chrono::TimeZone;
    #[cfg(feature = "testing")]
    use std::sync::Mutex;

    #[test]
    fn summary_listet_jeden_typ_mit_anzahl() {
        let report = UnhandledReport {
            total: 15,
            by_action_type: vec![("connect_suggest".into(), 3), ("lfg_invite".into(), 12)],
        };
        assert_eq!(report.summary(), "connect_suggest=3, lfg_invite=12");
        assert_eq!(UnhandledReport::default().summary(), "");
    }

    #[cfg(feature = "testing")]
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0)
            .single()
            .expect("valid timestamp")
    }

    #[cfg(feature = "testing")]
    #[derive(Default)]
    struct MockPort {
        sent: Mutex<Vec<u64>>,
    }

    #[cfg(feature = "testing")]
    #[async_trait::async_trait]
    impl SurveyPulsePort for MockPort {
        async fn send_dm(&self, user_id: u64, _body: Value) -> Result<(), String> {
            self.sent.lock().expect("sent").push(user_id);
            Ok(())
        }
    }

    #[cfg(feature = "testing")]
    async fn seed_eligible_user(pool: &PgPool, user_id: i64, guild_id: i64) {
        sqlx::query(
            "INSERT INTO activity.user_retention_tracking(
                user_id, guild_id, first_seen_at, last_active_at,
                total_active_days, opted_out, updated_at
             ) VALUES ($1, $2, $3, $3, 1, FALSE, $3)",
        )
        .bind(user_id)
        .bind(guild_id)
        .bind(now())
        .execute(pool)
        .await
        .expect("retention row");
    }

    #[cfg(feature = "testing")]
    async fn insert_outbox(pool: &PgPool, action_type: &str, user_id: i64, key: &str) {
        sqlx::query(
            "INSERT INTO bot.action_outbox(
                action_type, user_id, guild_id, payload, anchor, idempotency_key, scheduled_for
             ) VALUES ($1, $2, 99, jsonb_build_object('wave_id', 7), 'anchor', $3, $4)",
        )
        .bind(action_type)
        .bind(user_id)
        .bind(key)
        .bind(now())
        .execute(pool)
        .await
        .expect("outbox row");
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn dispatcher_stellt_registrierte_typen_zu_und_zaehlt_den_rest() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        seed_eligible_user(pool, 1, 99).await;
        seed_eligible_user(pool, 2, 99).await;
        seed_eligible_user(pool, 3, 99).await;
        insert_outbox(pool, SURVEY_PULSE_ACTION_TYPE, 1, "survey-1").await;
        insert_outbox(pool, "lfg_invite", 2, "lfg-2").await;
        insert_outbox(pool, "connect_suggest", 3, "connect-3").await;

        // Der Dispatcher bekommt keine SurveyPulseConfig: das Wellen-Flag kann
        // nicht beeinflussen, ob zugestellt wird.
        assert!(
            !SurveyPulseConfig::from_lookup(|_| None).enabled,
            "SURVEY_PULSE_ENABLED ist ohne Env aus — genau dieser Fall wird geprüft"
        );
        let port = Arc::new(MockPort::default());
        let dispatcher = OutboxDispatcher::new(pool.clone()).register(
            SURVEY_PULSE_ACTION_TYPE,
            SurveyPulseOutboxHandler::new(port.clone()),
        );

        let summary = dispatcher.drain(now()).await.expect("drain");
        assert_eq!(summary.sent, 1);
        assert_eq!(summary.suppressed, 0);
        assert_eq!(summary.failed, 0);
        assert_eq!(*port.sent.lock().expect("sent"), vec![1_u64]);

        let statuses: Vec<(String, String)> = sqlx::query_as(
            "SELECT action_type, status FROM bot.action_outbox ORDER BY action_type",
        )
        .fetch_all(pool)
        .await
        .expect("statuses");
        assert_eq!(
            statuses,
            vec![
                ("connect_suggest".to_string(), "pending".to_string()),
                ("lfg_invite".to_string(), "pending".to_string()),
                ("survey_pulse".to_string(), "sent".to_string()),
            ],
            "Typen ohne Handler bleiben pending, werden nie failed"
        );

        let report = dispatcher.report_unhandled(now()).await.expect("report");
        assert_eq!(report.total, 2);
        assert_eq!(
            report.by_action_type,
            vec![("connect_suggest".to_string(), 1), ("lfg_invite".to_string(), 1)]
        );
        assert_eq!(report.summary(), "connect_suggest=1, lfg_invite=1");
    }

    #[tokio::test]
    #[cfg(feature = "testing")]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn ohne_handler_wird_nichts_zugestellt_und_alles_gezaehlt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("disposable postgres");
        let pool = db.pool();
        seed_eligible_user(pool, 1, 99).await;
        insert_outbox(pool, SURVEY_PULSE_ACTION_TYPE, 1, "survey-1").await;
        let dispatcher = OutboxDispatcher::new(pool.clone());

        assert_eq!(dispatcher.registered_action_types(), Vec::<String>::new());
        assert_eq!(
            dispatcher.drain(now()).await.expect("drain"),
            DrainSummary::default()
        );
        let report = dispatcher.report_unhandled(now()).await.expect("report");
        assert_eq!(report.total, 1);
        assert_eq!(report.summary(), "survey_pulse=1");
    }
}
