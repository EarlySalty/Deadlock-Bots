//! Invite-Snapshot-Cache für die Beitrittsquellen-Erkennung.
//!
//! Hält pro Gilde die zuletzt bekannten Invite-Nutzungszähler. Bei einem Join
//! wird der aktuelle Stand geholt und gegen den Cache differenziert: Der Code
//! mit dem grössten positiven Delta ist der genutzte Invite (Port der
//! Snapshot-/Diff-Logik aus `cogs/user_activity_analyzer.py`
//! `_collect_join_invite_snapshot` / `_classify_join_source`).
//!
//! Die produzierten Metadaten sind ROH (`join_source_bucket` = personal /
//! bot_invite / public / unknown); die Twitch-/Website-Verfeinerung macht der
//! Writer über `dl_activity::join_source::classify`. Vanity-Nutzungszähler
//! werden NICHT getrackt (serenity liefert keinen) — Vanity-Joins fallen in die
//! Discovery-Heuristik (Bucket `public`), was den Bucket-Count nicht verändert.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serenity::all::{GuildId, Http, InviteCreateEvent, Member};
use sqlx::PgPool;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InviteSnap {
    uses: u64,
    url: String,
    inviter_id: Option<u64>,
    inviter_name: String,
    inviter_bot: bool,
    channel_id: Option<u64>,
    channel_name: String,
}

/// Pro-Gilde-Cache der Invite-Stände (`code → Snapshot`).
pub struct InviteTracker {
    by_guild: Mutex<HashMap<u64, HashMap<String, InviteSnap>>>,
    pool: PgPool,
}

fn should_retry_join_source(has_baseline: bool, kind: &str) -> bool {
    has_baseline && matches!(kind, "server_discovery" | "unknown")
}

fn classify_snapshots(
    meta: &mut Map<String, Value>,
    before_map: Option<&HashMap<String, InviteSnap>>,
    after_map: Option<&HashMap<String, InviteSnap>>,
) -> String {
    let Some(after_map) = after_map else {
        meta.insert(
            "join_source_reason".into(),
            Value::from(if before_map.is_none() {
                "baseline_missing"
            } else {
                "invite_snapshot_unavailable"
            }),
        );
        return "unknown".to_string();
    };
    let Some(before_map) = before_map else {
        meta.insert("join_source_reason".into(), Value::from("baseline_missing"));
        return "unknown".to_string();
    };

    let mut best: Option<(String, u64)> = None;
    for (code, snap) in after_map {
        let before_uses = before_map.get(code).map(|s| s.uses).unwrap_or(0);
        if snap.uses > before_uses {
            let delta = snap.uses - before_uses;
            let take = match &best {
                None => true,
                Some((bc, bd)) => delta > *bd || (delta == *bd && code < bc),
            };
            if take {
                best = Some((code.clone(), delta));
            }
        }
    }

    if let Some((code, _)) = best {
        let snap = &after_map[&code];
        meta.insert("invite_code".into(), Value::from(code.clone()));
        meta.insert("invite_url".into(), Value::from(snap.url.clone()));
        if let Some(iid) = snap.inviter_id {
            meta.insert("inviter_id".into(), Value::from(iid));
        }
        meta.insert(
            "inviter_name".into(),
            Value::from(snap.inviter_name.clone()),
        );
        meta.insert("inviter_bot".into(), Value::from(snap.inviter_bot));
        if let Some(cid) = snap.channel_id {
            meta.insert("invite_channel_id".into(), Value::from(cid));
        }
        meta.insert(
            "invite_channel_name".into(),
            Value::from(snap.channel_name.clone()),
        );
        meta.insert("join_source_confidence".into(), Value::from("high"));
        if snap.inviter_bot {
            meta.insert("join_source_bucket".into(), Value::from("bot_invite"));
            meta.insert("join_source_kind".into(), Value::from("bot_invite"));
            meta.insert(
                "join_source_label".into(),
                Value::from(format!("Bot Invite: {}", snap.inviter_name)),
            );
            "bot_invite".to_string()
        } else {
            meta.insert("join_source_bucket".into(), Value::from("personal"));
            meta.insert("join_source_kind".into(), Value::from("invite_link"));
            meta.insert(
                "join_source_label".into(),
                Value::from("Persönliche Einladung"),
            );
            "invite_link".to_string()
        }
    } else {
        meta.insert("join_source_bucket".into(), Value::from("public"));
        meta.insert("join_source_kind".into(), Value::from("server_discovery"));
        meta.insert(
            "join_source_label".into(),
            Value::from("Public: Server entdecken"),
        );
        meta.insert("join_source_confidence".into(), Value::from("medium"));
        "server_discovery".to_string()
    }
}

impl InviteTracker {
    pub fn new(pool: PgPool) -> Self {
        Self {
            by_guild: Mutex::new(HashMap::new()),
            pool,
        }
    }

    async fn fetch(http: &Http, guild_id: u64) -> Option<HashMap<String, InviteSnap>> {
        let invites = http.get_guild_invites(GuildId::new(guild_id)).await.ok()?;
        let mut map = HashMap::new();
        for inv in invites {
            map.insert(
                inv.code.to_string(),
                InviteSnap {
                    uses: inv.uses,
                    url: format!("https://discord.gg/{}", inv.code),
                    inviter_id: inv.inviter.as_ref().map(|u| u.id.get()),
                    inviter_name: inv
                        .inviter
                        .as_ref()
                        .map(|u| u.name.to_string())
                        .unwrap_or_default(),
                    inviter_bot: inv.inviter.as_ref().map(|u| u.bot).unwrap_or(false),
                    channel_id: Some(inv.channel.id.get()),
                    channel_name: inv.channel.name.to_string(),
                },
            );
        }
        Some(map)
    }

    async fn load_snapshot_from_db(&self, guild_id: u64) -> Option<HashMap<String, InviteSnap>> {
        let guild_id = i64::try_from(guild_id).ok()?;
        let row = sqlx::query!(
            r#"
            SELECT snapshot_json::text AS "snapshot_json!"
            FROM bot.invite_snapshot_cache
            WHERE guild_id = $1
            "#,
            guild_id,
        )
        .fetch_optional(&self.pool)
        .await
        .ok()?;

        let raw = row?.snapshot_json;
        let value: Value = match serde_json::from_str(&raw) {
            Ok(value) => value,
            Err(_) => return None,
        };
        value
            .get("invites")
            .cloned()
            .and_then(|v| serde_json::from_value::<HashMap<String, InviteSnap>>(v).ok())
    }

    async fn save_snapshot_to_db(&self, guild_id: u64, snapshot: &HashMap<String, InviteSnap>) {
        let guild_id = match i64::try_from(guild_id) {
            Ok(guild_id) => guild_id,
            Err(err) => {
                tracing::debug!(%err, guild_id, "Invite-Snapshot-Guild-ID passt nicht in BIGINT");
                return;
            }
        };
        let payload = serde_json::json!({
            "invites": snapshot,
            "vanity": {},
        });
        let Ok(payload) = serde_json::to_string(&payload) else {
            return;
        };
        let result = sqlx::query!(
            r#"
            INSERT INTO bot.invite_snapshot_cache (guild_id, snapshot_json, updated_at)
            VALUES ($1, $2::text::jsonb, now())
            ON CONFLICT (guild_id) DO UPDATE SET
              snapshot_json = EXCLUDED.snapshot_json,
              updated_at = EXCLUDED.updated_at
            "#,
            guild_id,
            payload,
        )
        .execute(&self.pool)
        .await;
        if let Err(err) = result {
            tracing::debug!(%err, guild_id, "Invite-Snapshot konnte nicht persistiert werden");
        }
    }

    async fn restore_from_db(&self, guild_id: u64) -> bool {
        let Some(snapshot) = self.load_snapshot_from_db(guild_id).await else {
            return false;
        };
        self.by_guild.lock().await.insert(guild_id, snapshot);
        true
    }

    /// Primt den Cache einer Gilde (API-Fetch). Beim Start für alle Gilden.
    pub async fn prime(&self, http: &Http, guild_id: u64) {
        let has_cache = self.by_guild.lock().await.contains_key(&guild_id);
        if !has_cache && self.restore_from_db(guild_id).await {
            return;
        }
        if let Some(map) = Self::fetch(http, guild_id).await {
            self.save_snapshot_to_db(guild_id, &map).await;
            self.by_guild.lock().await.insert(guild_id, map);
        }
    }

    /// Übernimmt einen frisch erstellten Invite in den Cache (uses = 0).
    pub async fn on_invite_create(&self, ev: &InviteCreateEvent) {
        let Some(guild_id) = ev.guild_id else {
            return;
        };
        let snap = InviteSnap {
            uses: 0,
            url: format!("https://discord.gg/{}", ev.code),
            inviter_id: ev.inviter.as_ref().map(|u| u.id.get()),
            inviter_name: ev
                .inviter
                .as_ref()
                .map(|u| u.name.to_string())
                .unwrap_or_default(),
            inviter_bot: ev.inviter.as_ref().map(|u| u.bot).unwrap_or(false),
            channel_id: Some(ev.channel_id.get()),
            channel_name: String::new(),
        };
        let guild_id = guild_id.get();
        let snapshot = {
            let mut guard = self.by_guild.lock().await;
            let entry = guard.entry(guild_id).or_default();
            entry.insert(ev.code.to_string(), snap);
            entry.clone()
        };
        self.save_snapshot_to_db(guild_id, &snapshot).await;
    }

    /// Entfernt einen gelöschten Invite aus dem Cache.
    pub async fn on_invite_delete(&self, guild_id: u64, code: &str) {
        let snapshot = {
            let mut guard = self.by_guild.lock().await;
            let Some(g) = guard.get_mut(&guild_id) else {
                return;
            };
            g.remove(code);
            g.clone()
        };
        self.save_snapshot_to_db(guild_id, &snapshot).await;
    }

    /// Detektiert die Beitrittsquelle: aktuellen Invite-Stand holen, gegen den
    /// Cache differenzieren, Cache aktualisieren, rohe `join_source_*`-Metadaten
    /// zurückgeben.
    pub async fn on_join(&self, http: &Http, member: &Member) -> Value {
        let guild_id = member.guild_id.get();
        let mut meta = Map::new();
        meta.insert(
            "avatar_url".into(),
            member
                .user
                .avatar_url()
                .map(Value::from)
                .unwrap_or(Value::Null),
        );
        meta.insert("is_pending".into(), Value::from(member.pending));
        meta.insert("join_source_bucket".into(), Value::from("unknown"));
        meta.insert("join_source_kind".into(), Value::from("unknown"));
        meta.insert("join_source_label".into(), Value::from("Unbekannt"));
        meta.insert("join_source_confidence".into(), Value::from("low"));

        let before = self.by_guild.lock().await.get(&guild_id).cloned();
        let has_baseline = before.is_some();
        let attempts = if has_baseline { 2 } else { 1 };
        let mut latest_after: Option<HashMap<String, InviteSnap>> = None;

        for attempt in 0..attempts {
            meta.insert("join_source_bucket".into(), Value::from("unknown"));
            meta.insert("join_source_kind".into(), Value::from("unknown"));
            meta.insert("join_source_label".into(), Value::from("Unbekannt"));
            meta.insert("join_source_confidence".into(), Value::from("low"));
            meta.remove("join_source_reason");

            latest_after = Self::fetch(http, guild_id).await;
            let kind = classify_snapshots(&mut meta, before.as_ref(), latest_after.as_ref());
            if !should_retry_join_source(has_baseline, &kind) || attempt + 1 >= attempts {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        if let Some(after_map) = latest_after {
            self.save_snapshot_to_db(guild_id, &after_map).await;
            self.by_guild.lock().await.insert(guild_id, after_map);
        }

        Value::Object(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "testing")]
    async fn invite_db() -> dl_central_db::TestDb {
        dl_central_db::testing::test_pool()
            .await
            .expect("central test db")
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn snapshot_cache_roundtrip_in_db() {
        let db = invite_db().await;
        let tracker = InviteTracker::new(db.pool().clone());
        let mut snapshot = HashMap::new();
        snapshot.insert(
            "abc".to_string(),
            InviteSnap {
                uses: 7,
                url: "https://discord.gg/abc".to_string(),
                inviter_id: Some(42),
                inviter_name: "Inviter".to_string(),
                inviter_bot: false,
                channel_id: Some(99),
                channel_name: "willkommen".to_string(),
            },
        );

        tracker.save_snapshot_to_db(1, &snapshot).await;
        let restored = tracker.load_snapshot_from_db(1).await.expect("snapshot");
        assert_eq!(restored.get("abc").map(|snap| snap.uses), Some(7));
        tracker.restore_from_db(1).await;
        assert!(tracker.by_guild.lock().await.contains_key(&1));
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn snapshot_cache_missing_guild_returns_none_in_db() {
        let db = invite_db().await;
        let tracker = InviteTracker::new(db.pool().clone());
        let missing_guild_id = 9_876_543_210_u64;

        assert!(tracker
            .load_snapshot_from_db(missing_guild_id)
            .await
            .is_none());
        assert!(!tracker.restore_from_db(missing_guild_id).await);
        assert!(!tracker
            .by_guild
            .lock()
            .await
            .contains_key(&missing_guild_id));

        let mut snapshot = HashMap::new();
        snapshot.insert(
            "negative-path-jsonb".to_string(),
            InviteSnap {
                uses: 1,
                url: "https://discord.gg/negative-path-jsonb".to_string(),
                inviter_id: None,
                inviter_name: String::new(),
                inviter_bot: false,
                channel_id: None,
                channel_name: String::new(),
            },
        );

        tracker
            .save_snapshot_to_db(missing_guild_id, &snapshot)
            .await;
        let snapshot_json_type: String = sqlx::query_scalar(
            "SELECT jsonb_typeof(snapshot_json) FROM bot.invite_snapshot_cache WHERE guild_id = $1",
        )
        .bind(i64::try_from(missing_guild_id).expect("test guild id fits bigint"))
        .fetch_one(db.pool())
        .await
        .expect("snapshot_json type");
        assert_eq!(snapshot_json_type, "object");
    }

    #[test]
    fn retry_nur_bei_baseline_und_unbarer_erkennung() {
        assert!(should_retry_join_source(true, "server_discovery"));
        assert!(should_retry_join_source(true, "unknown"));
        assert!(!should_retry_join_source(false, "server_discovery"));
        assert!(!should_retry_join_source(true, "invite_link"));
    }
}
