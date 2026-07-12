use std::path::Path;

fn migration() -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations/2026071260_zweitgehirn_foundation.sql"),
    )
    .expect("read zweitgehirn foundation migration")
}

#[test]
fn migration_defines_anchored_idempotent_action_outbox() {
    let sql = migration();

    for contract in [
        "CREATE TABLE bot.action_outbox",
        "anchor TEXT NOT NULL",
        "idempotency_key TEXT UNIQUE NOT NULL",
        "status TEXT NOT NULL DEFAULT 'pending'",
        "CHECK (status IN ('pending', 'sent', 'failed', 'suppressed'))",
    ] {
        assert!(
            sql.contains(contract),
            "missing outbox contract: {contract}"
        );
    }
    assert!(!sql.contains("ALTER TABLE bot.notification_queue"));
}

#[test]
fn migration_defines_weekly_pulse_and_at_risk_views() {
    let sql = migration();

    for contract in [
        "CREATE VIEW activity.weekly_pulse",
        "voice_wau",
        "text_wau",
        "new_members",
        "open_lfg_watches",
        "fired_lfg_watches",
        "voice_minutes",
        "SUM(COALESCE(events.duration_seconds, 0)) / 60.0",
        "CREATE VIEW activity.at_risk_members",
        "NOT retention.opted_out",
        "patterns.last_pinged_at <= now() - INTERVAL '14 days'",
        "COALESCE(patterns.ping_count_30d, 0) < 2",
    ] {
        assert!(sql.contains(contract), "missing view contract: {contract}");
    }
    assert!(!sql.contains("aggregates.day >= current_date - 6"));
}
