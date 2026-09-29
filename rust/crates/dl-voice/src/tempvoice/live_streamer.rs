//! Temporäre Panel-Rechte für live streamende Mitglieder mit Streamer-Rolle.
//! Keine Discord-Permissions oder Rollen werden dafür vergeben.

use std::{sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use dl_bridges::twitch::TwitchApiClient;
use dl_discord::DiscordAdapter;
use serde_json::Value;
use serenity::all::{GuildId, UserId};

/// Der bestehende Twitch-Poller aktualisiert diesen Zeitpunkt auch bei anderen Spielen.
const MAX_LIVE_AGE_SECONDS: i64 = 180;
/// Auch Buttons, die ein Modal öffnen, müssen innerhalb von drei Sekunden antworten.
const ACCESS_CHECK_TIMEOUT: Duration = Duration::from_secs(2);

pub struct LiveStreamerAccess {
    client: Arc<TwitchApiClient>,
    role_id: u64,
}

impl LiveStreamerAccess {
    pub fn new(client: Arc<TwitchApiClient>, role_id: u64) -> Self {
        Self { client, role_id }
    }

    pub async fn allows(&self, adapter: &DiscordAdapter, guild_id: u64, user_id: u64) -> bool {
        if guild_id == 0
            || user_id == 0
            || self.role_id == 0
            || adapter.voice_cache_snapshot(guild_id).is_none()
        {
            return false;
        }
        // Ein alter Rollen-Cache oder eine gecachte positive Freigabe darf nach
        // Rollenentzug nicht weiter berechtigen. Nur serverseitige Quellen nutzen.
        let check = async {
            let member = adapter
                .http
                .get_member(GuildId::new(guild_id), UserId::new(user_id))
                .await
                .ok()?;
            if member.user.bot || !member.roles.iter().any(|role| role.get() == self.role_id) {
                return Some(false);
            }
            let status = self.client.diagnose_discord_user(user_id).await.ok()?;
            Some(fresh_live_status(&status, Utc::now()))
        };
        tokio::time::timeout(ACCESS_CHECK_TIMEOUT, check)
            .await
            .ok()
            .flatten()
            .unwrap_or(false)
            && adapter.voice_cache_snapshot(guild_id).is_some()
    }
}

fn fresh_live_status(status: &Value, now: DateTime<Utc>) -> bool {
    if status.get("ok").and_then(Value::as_bool) != Some(true)
        || status.get("found").and_then(Value::as_bool) != Some(true)
        || status.get("is_live").and_then(Value::as_bool) != Some(true)
        || status
            .get("twitch_login")
            .and_then(Value::as_str)
            .is_none_or(|login| login.trim().is_empty())
    {
        return false;
    }
    let Some(last_seen) = status
        .get("last_seen_at")
        .and_then(Value::as_str)
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.with_timezone(&Utc))
    else {
        return false;
    };
    let age = now.signed_duration_since(last_seen);
    age >= chrono::Duration::zero() && age <= chrono::Duration::seconds(MAX_LIVE_AGE_SECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-09-24T20:00:00Z")
            .expect("valid test fixture")
            .with_timezone(&Utc)
    }

    fn live() -> Value {
        json!({"ok": true, "found": true, "is_live": true,
            "twitch_login": "streamer", "last_seen_at": "2026-09-24T19:59:30Z"})
    }

    #[test]
    fn live_streamer_accepts_every_game_and_no_partner_gate() {
        for game in ["Deadlock", "Minecraft", "Just Chatting", ""] {
            let mut status = live();
            status["last_game"] = json!(game);
            status["is_partner_active"] = json!(false);
            assert!(fresh_live_status(&status, now()), "game: {game}");
        }
    }

    #[test]
    fn live_streamer_requires_explicit_live_linked_success() {
        for key in ["ok", "found", "is_live"] {
            for value in [Value::Null, json!(false), json!(1), json!("true")] {
                let mut status = live();
                status[key] = value;
                assert!(!fresh_live_status(&status, now()), "field: {key}");
            }
        }
        for value in [Value::Null, json!(""), json!(" "), json!(123)] {
            let mut status = live();
            status["twitch_login"] = value;
            assert!(!fresh_live_status(&status, now()));
        }
    }

    #[test]
    fn live_streamer_rejects_stale_unknown_and_future_status() {
        for timestamp in [
            Value::Null,
            json!("invalid"),
            json!("2026-09-24T19:56:59Z"),
            json!("2026-09-24T20:00:01Z"),
            json!("2026-09-24T20:00:00.001Z"),
        ] {
            let mut status = live();
            status["last_seen_at"] = timestamp;
            assert!(!fresh_live_status(&status, now()));
        }
        let mut old_server = live();
        old_server
            .as_object_mut()
            .expect("valid test fixture")
            .remove("last_seen_at");
        assert!(!fresh_live_status(&old_server, now()));
    }

    #[test]
    fn live_streamer_checks_exact_freshness_boundary_and_timezone() {
        for timestamp in [
            "2026-09-24T19:57:00Z",
            "2026-09-24T20:00:00Z",
            "2026-09-24T21:59:30+02:00",
        ] {
            let mut status = live();
            status["last_seen_at"] = json!(timestamp);
            assert!(fresh_live_status(&status, now()));
        }
        let mut status = live();
        status["last_seen_at"] = json!("2026-09-24T19:56:59.999Z");
        assert!(!fresh_live_status(&status, now()));
    }
}
