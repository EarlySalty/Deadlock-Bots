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

use serde_json::{Map, Value};
use serenity::all::{GuildId, Http, InviteCreateEvent, Member};
use tokio::sync::Mutex;

#[derive(Clone)]
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
#[derive(Default)]
pub struct InviteTracker {
    by_guild: Mutex<HashMap<u64, HashMap<String, InviteSnap>>>,
}

impl InviteTracker {
    pub fn new() -> Self {
        Self::default()
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

    /// Primt den Cache einer Gilde (API-Fetch). Beim Start für alle Gilden.
    pub async fn prime(&self, http: &Http, guild_id: u64) {
        if let Some(map) = Self::fetch(http, guild_id).await {
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
        self.by_guild
            .lock()
            .await
            .entry(guild_id.get())
            .or_default()
            .insert(ev.code.to_string(), snap);
    }

    /// Entfernt einen gelöschten Invite aus dem Cache.
    pub async fn on_invite_delete(&self, guild_id: u64, code: &str) {
        if let Some(g) = self.by_guild.lock().await.get_mut(&guild_id) {
            g.remove(code);
        }
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

        let mut guard = self.by_guild.lock().await;
        let before = guard.get(&guild_id).cloned();
        let after = Self::fetch(http, guild_id).await;

        match (before, after) {
            (_, None) => {
                // Invites nicht abrufbar (fehlende MANAGE_GUILD-Rechte o. Ä.).
                meta.insert(
                    "join_source_reason".into(),
                    Value::from("invite_snapshot_unavailable"),
                );
            }
            (None, Some(after_map)) => {
                // Kein Baseline-Snapshot → unbekannt; Cache jetzt setzen.
                guard.insert(guild_id, after_map);
                meta.insert("join_source_reason".into(), Value::from("baseline_missing"));
            }
            (Some(before_map), Some(after_map)) => {
                // Code mit grösstem positivem Delta (Tie-Break: kleinerer Code).
                let mut best: Option<(String, u64)> = None;
                for (code, snap) in &after_map {
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
                    } else {
                        meta.insert("join_source_bucket".into(), Value::from("personal"));
                        meta.insert("join_source_kind".into(), Value::from("invite_link"));
                        meta.insert(
                            "join_source_label".into(),
                            Value::from("Persönliche Einladung"),
                        );
                    }
                } else {
                    // Kein Invite-Delta → Discovery/Vanity (Bucket public).
                    meta.insert("join_source_bucket".into(), Value::from("public"));
                    meta.insert("join_source_kind".into(), Value::from("server_discovery"));
                    meta.insert(
                        "join_source_label".into(),
                        Value::from("Public: Server entdecken"),
                    );
                    meta.insert("join_source_confidence".into(), Value::from("medium"));
                }
                guard.insert(guild_id, after_map);
            }
        }

        Value::Object(meta)
    }
}
