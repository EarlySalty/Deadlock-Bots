-- Zeitindex fuer core.discord_audit_log. Die Tabelle waechst INSERT-only und
-- bewusst ohne Retention; jede Auswertung liest pro Guild einen Zeitraum
-- (Dashboard-Audit, brain-feeder-Aggregation) und lief bisher als Seq Scan.
CREATE INDEX IF NOT EXISTS idx_discord_audit_log_guild_time
    ON core.discord_audit_log (guild_id, occurred_at);
