use std::collections::{HashMap, HashSet};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{TimeDelta, Utc};
use clap::Parser;
use dl_brain_community::{
    decide, render_report, Candidate, Decision, GateData, LedgerEntry, RenderedReport, WeeklyPulse,
};
use serde_json::json;
use sqlx::{PgPool, Postgres, Transaction};

#[derive(Debug, Parser)]
#[command(
    name = "dl-brain-community",
    about = "Persistiert Community-Entscheidungen im festen Shadow-Modus"
)]
struct Args {}

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let _args = Args::parse();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "dl-brain-community fehlgeschlagen");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let dsn = dl_central_db::dsn_from_env().context("zentrale DB-Konfiguration")?;
    let pool = dl_central_db::connect_pool(&dsn)
        .await
        .context("zentrale DB verbinden")?;

    let mut load_errors = Vec::new();
    let pulses = match load_weekly_pulse(&pool).await {
        Ok(pulses) => pulses,
        Err(error) => {
            tracing::error!(%error, "weekly_pulse konnte nicht geladen werden");
            load_errors.push(error_entry(
                "brain.report",
                "load_weekly_pulse",
                "load=activity.weekly_pulse",
                None,
            ));
            Vec::new()
        }
    };

    let candidates = match load_candidates(&pool).await {
        Ok(candidates) => candidates,
        Err(error) => {
            tracing::error!(%error, "at_risk_members konnten nicht geladen werden");
            load_errors.push(error_entry(
                "brain.activation",
                "load_at_risk_members",
                "load=activity.at_risk_members",
                None,
            ));
            Vec::new()
        }
    };

    let mut decisions = if candidates.is_empty() {
        Vec::new()
    } else {
        match load_gate_data(&pool, &candidates).await {
            Ok(gates) => decide(&candidates, &gates),
            Err((reason, error)) => {
                tracing::error!(%error, reason, "Aktivierungs-Gates konnten nicht geladen werden");
                candidates
                    .iter()
                    .map(|candidate| {
                        error_entry(
                            "brain.activation",
                            reason,
                            &format!("load={reason};candidate=at_risk"),
                            Some(candidate),
                        )
                    })
                    .collect()
            }
        }
    };
    decisions.extend(load_errors);

    let report = render_report(&pulses, &candidates, &decisions);
    let now = Utc::now();
    let period_start = pulses
        .iter()
        .map(|pulse| pulse.period_start)
        .min()
        .unwrap_or(now - TimeDelta::days(7));
    let period_end = pulses
        .iter()
        .map(|pulse| pulse.period_end)
        .max()
        .unwrap_or(now);

    persist_run(&pool, &decisions, &report, period_start, period_end)
        .await
        .context("Ledger und Report persistieren")?;

    if let Err(error) = deliver_staff_report(&report.report_text).await {
        tracing::error!(%error, "Staff-Report konnte nicht an Discord zugestellt werden");
        println!("{}", report.report_text);
    }

    Ok(())
}

async fn load_weekly_pulse(pool: &PgPool) -> Result<Vec<WeeklyPulse>, sqlx::Error> {
    sqlx::query_as::<_, WeeklyPulse>(
        "SELECT guild_id, period_start, period_end,
                voice_wau, text_wau, new_members,
                open_lfg_watches, fired_lfg_watches,
                voice_minutes::DOUBLE PRECISION AS voice_minutes
           FROM activity.weekly_pulse
          ORDER BY guild_id",
    )
    .fetch_all(pool)
    .await
}

async fn load_candidates(pool: &PgPool) -> Result<Vec<Candidate>, sqlx::Error> {
    sqlx::query_as::<_, (i64, i64, i32)>(
        "SELECT user_id, guild_id, inactive_days
           FROM activity.at_risk_members
          ORDER BY user_id",
    )
    .fetch_all(pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(user_id, guild_id, inactive_days)| Candidate {
                user_id,
                guild_id,
                inactive_days,
            })
            .collect()
    })
}

async fn load_gate_data(
    pool: &PgPool,
    candidates: &[Candidate],
) -> Result<GateData, (&'static str, sqlx::Error)> {
    let user_ids: Vec<i64> = candidates
        .iter()
        .map(|candidate| candidate.user_id)
        .collect();
    let opted_out_users = sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT candidate.user_id
           FROM unnest($1::BIGINT[]) AS candidate(user_id)
           LEFT JOIN core.user_privacy AS privacy
             ON privacy.user_id = candidate.user_id
           LEFT JOIN activity.user_retention_tracking AS retention
             ON retention.user_id = candidate.user_id
          WHERE COALESCE(privacy.opted_out, FALSE)
             OR COALESCE(retention.opted_out, FALSE)",
    )
    .bind(&user_ids)
    .fetch_all(pool)
    .await
    .map_err(|error| ("load_opted_out", error))?
    .into_iter()
    .collect::<HashSet<_>>();

    let budgeted_users = sqlx::query_scalar::<_, i64>(
        "SELECT DISTINCT user_id
           FROM bot.action_outbox
          WHERE user_id = ANY($1::BIGINT[])
            AND status IN ('pending', 'sent')
            AND created_at >= now() - INTERVAL '14 days'",
    )
    .bind(&user_ids)
    .fetch_all(pool)
    .await
    .map_err(|error| ("load_budget_14d", error))?
    .into_iter()
    .collect::<HashSet<_>>();

    // ponytail: Es gibt noch keine autoritative Quelle für konkrete offene Slots/Events.
    let anchors = HashMap::new();

    Ok(GateData {
        opted_out_users,
        budgeted_users,
        anchors,
    })
}

fn error_entry(
    source: &'static str,
    reason: &str,
    input_summary: &str,
    candidate: Option<&Candidate>,
) -> LedgerEntry {
    LedgerEntry {
        source,
        subject_user_id: candidate.map(|value| value.user_id),
        guild_id: candidate.map(|value| value.guild_id),
        input_summary: input_summary.to_string(),
        decision: Decision::Error,
        confidence: None,
        reason: reason.to_string(),
        action_taken: "shadow",
        payload: json!({}),
    }
}

async fn persist_run(
    pool: &PgPool,
    decisions: &[LedgerEntry],
    report: &RenderedReport,
    period_start: chrono::DateTime<Utc>,
    period_end: chrono::DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    let mut transaction = pool.begin().await?;
    for entry in decisions {
        insert_ledger_entry(&mut transaction, entry).await?;
    }
    sqlx::query(
        "INSERT INTO bot.brain_reports(period_start, period_end, kpis, report_text)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(period_start)
    .bind(period_end)
    .bind(&report.kpis)
    .bind(&report.report_text)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await
}

async fn insert_ledger_entry(
    transaction: &mut Transaction<'_, Postgres>,
    entry: &LedgerEntry,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO bot.ai_decision_ledger(
             source, subject_user_id, guild_id, input_summary, decision,
             confidence, reason, action_taken, payload
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(entry.source)
    .bind(entry.subject_user_id)
    .bind(entry.guild_id)
    .bind(&entry.input_summary)
    .bind(entry.decision.as_str())
    .bind(entry.confidence)
    .bind(&entry.reason)
    .bind(entry.action_taken)
    .bind(&entry.payload)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn deliver_staff_report(report_text: &str) -> Result<()> {
    let channel_id = match std::env::var("DL_BRAIN_STAFF_CHANNEL_ID") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            println!("{report_text}");
            return Ok(());
        }
    };
    let token = std::env::var("DISCORD_TOKEN")
        .or_else(|_| std::env::var("BOT_TOKEN"))
        .context("DISCORD_TOKEN oder BOT_TOKEN fehlt")?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("Discord HTTP-Client")?
        .post(format!(
            "https://discord.com/api/v10/channels/{channel_id}/messages"
        ))
        .header("Authorization", format!("Bot {token}"))
        .header("User-Agent", "dl-brain-community/0.1")
        .json(&json!({
            "content": report_text,
            "allowed_mentions": {"parse": []},
        }))
        .send()
        .await
        .context("Discord Staff-Report senden")?;
    response
        .error_for_status()
        .context("Discord Staff-Report abgelehnt")?;
    Ok(())
}
