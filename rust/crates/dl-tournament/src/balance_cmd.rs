//! `!balance`-Admin-Befehlsschicht — Port von `cogs/deadlock_team_balancer.py`.
//!
//! Erste Scheibe: der `!balance`-Prefix-Listener + `auto`/`voice` (read-only
//! Team-Vorschau, kein Channel-Move) + die Hilfe. Die zustandsverändernden
//! Subcommands (`start`/`manual`/`status`/`matches`/`end`/`cleanup`) folgen.
//! Der Balancing-Algorithmus liegt schon in [`crate::balancer`].
//!
//! `auto`/`voice` sind wie im Original NICHT permission-gated (reine Vorschau);
//! die gateenden Subcommands prüfen ihre Rechte später selbst.

use std::sync::Arc;

use dl_discord::{ChannelSender, Dispatcher};
use serde_json::{json, Value};

use crate::balancer::best_split;

/// Rang-Namen nach Wert (0..=11), wie `DEADLOCK_RANKS` im Original.
const RANK_NAMES: [&str; 12] = [
    "Obscurus",
    "Initiate",
    "Seeker",
    "Alchemist",
    "Arcanist",
    "Ritualist",
    "Emissary",
    "Archon",
    "Oracle",
    "Phantom",
    "Ascendant",
    "Eternus",
];

/// Discord-Rollen-ID → Rang-Wert (wie `DISCORD_RANK_ROLES`).
const DISCORD_RANK_ROLES: [(u64, i64); 11] = [
    (1331457571118387210, 1),
    (1331457652877955072, 2),
    (1331457699992436829, 3),
    (1331457724848017539, 4),
    (1331457879345070110, 5),
    (1331457898781474836, 6),
    (1331457949654319114, 7),
    (1316966867033653338, 8),
    (1331458016356208680, 9),
    (1331458049637875785, 10),
    (1331458087349129296, 11),
];

/// Höchster Rang-Wert aus den Rollen des Mitglieds (`_rank_from_roles`); 0, wenn
/// keine Rang-Rolle vorhanden ist.
pub fn rank_from_roles(role_ids: &[u64]) -> i64 {
    role_ids
        .iter()
        .filter_map(|rid| {
            DISCORD_RANK_ROLES
                .iter()
                .find(|(id, _)| id == rid)
                .map(|(_, value)| *value)
        })
        .max()
        .unwrap_or(0)
}

fn rank_name(value: i64) -> &'static str {
    // Wie Python: unbekannter/außerhalb liegender Wert → "Obscurus".
    usize::try_from(value)
        .ok()
        .and_then(|i| RANK_NAMES.get(i))
        .copied()
        .unwrap_or("Obscurus")
}

/// Ein Mitglied im Voice-Channel des Aufrufers.
pub struct VoiceMember {
    pub user_id: u64,
    pub display_name: String,
    pub role_ids: Vec<u64>,
}

/// Discord-Anbindung des Balancers (vom Bot über den Cache erfüllt).
#[async_trait::async_trait]
pub trait BalancePort: Send + Sync {
    /// Nicht-Bot-Mitglieder im Voice-Channel des Aufrufers (leer, wenn er in
    /// keinem ist).
    async fn caller_voice_members(&self, guild_id: u64, user_id: u64) -> Vec<VoiceMember>;
}

/// Eine Antwort des Listeners (Text und/oder Embed).
pub struct BalanceReply {
    pub content: Option<String>,
    pub embeds: Vec<Value>,
}

impl BalanceReply {
    fn text(content: impl Into<String>) -> Self {
        Self {
            content: Some(content.into()),
            embeds: vec![],
        }
    }
    fn embed(embed: Value) -> Self {
        Self {
            content: None,
            embeds: vec![embed],
        }
    }
}

pub struct BalanceCommands {
    pub port: Arc<dyn BalancePort>,
}

impl BalanceCommands {
    /// Verarbeitet eine Nachricht; `None`, wenn es kein `!balance`-Befehl ist.
    pub async fn reply_for(&self, content: &str, guild_id: u64, user_id: u64) -> Option<BalanceReply> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_lowercase();
        if !matches!(root.as_str(), "!balance" | "!bal" | "!teams") {
            return None;
        }
        let sub = parts.next().unwrap_or_default().to_lowercase();
        let reply = match sub.as_str() {
            "" => BalanceReply::embed(help_embed()),
            "auto" | "voice" => self.auto_preview(guild_id, user_id).await,
            other => BalanceReply::text(format!(
                "`!balance {other}` ist in Rust noch nicht verfügbar (folgt)."
            )),
        };
        Some(reply)
    }

    /// `auto`/`voice`: Voice-Member → Ränge → beste 2 Teams → Vorschau-Embed.
    async fn auto_preview(&self, guild_id: u64, user_id: u64) -> BalanceReply {
        let members = self.port.caller_voice_members(guild_id, user_id).await;
        if members.len() < 4 {
            return BalanceReply::text(format!(
                "❌ Mindestens 4 Spieler benötigt (aktuell: {})",
                members.len()
            ));
        }
        // (user_id, name, rang_wert) — höchster Rang zuerst, wie das Original.
        let mut players: Vec<(String, i64)> = members
            .into_iter()
            .map(|m| (m.display_name, rank_from_roles(&m.role_ids)))
            .collect();
        players.sort_by_key(|p| std::cmp::Reverse(p.1));

        let values: Vec<i64> = players.iter().map(|(_, v)| *v).collect();
        let (idx_a, idx_b) = best_split(&values);
        let team_a: Vec<(&str, i64)> = idx_a.iter().map(|&i| (players[i].0.as_str(), players[i].1)).collect();
        let team_b: Vec<(&str, i64)> = idx_b.iter().map(|&i| (players[i].0.as_str(), players[i].1)).collect();
        BalanceReply::embed(team_embed(&team_a, &team_b, "🎯 Vorschau – Team Balance (ohne Move)"))
    }
}

/// Hilfe-Embed der `!balance`-Gruppe (Port des Root-Embeds).
fn help_embed() -> Value {
    json!({
        "title": "⚖️ Deadlock Team Balancer",
        "description": "Teilt Spieler in **2 Teams** nach Rang auf.",
        "color": 0x0099FF,
        "fields": [
            {
                "name": "Befehle",
                "value": "`!balance auto` – nur Anzeige (keine Channels)\n\
                          `!balance voice` – Alias von auto\n\
                          `!balance start` – Channels erstellen & Spieler moven (folgt)\n\
                          `!balance manual @u1 …` – manuelle Auswahl (folgt)\n\
                          `!balance status [@user]` – Rank-Status (folgt)\n\
                          `!balance matches` – aktive Matches (folgt)\n\
                          `!balance end <id>` – Match beenden (folgt)\n\
                          `!balance cleanup <hours>` – alte Matches löschen (folgt)",
                "inline": false
            },
            {
                "name": "Spielerzahl",
                "value": "Min. 4 Spieler, max. 12 (6v6).",
                "inline": false
            }
        ]
    })
}

/// Vorschau-Embed zweier Teams (Port von `_team_embed`).
fn team_embed(team_a: &[(&str, i64)], team_b: &[(&str, i64)], title: &str) -> Value {
    let fmt = |team: &[(&str, i64)]| -> (String, f64, f64) {
        if team.is_empty() {
            return ("—".to_string(), 0.0, 0.0);
        }
        let avg = team.iter().map(|&(_, v)| v).sum::<i64>() as f64 / team.len() as f64;
        let var = team.iter().map(|&(_, v)| (v as f64 - avg).powi(2)).sum::<f64>() / team.len() as f64;
        let lines = team
            .iter()
            .map(|&(name, v)| format!("• **{name}** — {} ({v})", rank_name(v)))
            .collect::<Vec<_>>()
            .join("\n");
        (lines, avg, var)
    };
    let (a_txt, a_avg, a_var) = fmt(team_a);
    let (b_txt, b_avg, b_var) = fmt(team_b);
    let diff = (a_avg - b_avg).abs();
    let mark = if diff < 1.0 {
        "✅"
    } else if diff < 2.0 {
        "⚠️"
    } else {
        "❌"
    };
    json!({
        "title": title,
        "color": 0x00CC88,
        "fields": [
            { "name": format!("🟠 Team Amber (Ø {a_avg:.1})"), "value": a_txt, "inline": true },
            { "name": format!("🔵 Team Sapphire (Ø {b_avg:.1})"), "value": b_txt, "inline": true },
            {
                "name": "📊 Balance",
                "value": format!(
                    "{mark} Ø-Unterschied: **{diff:.2}**\nVarianz A: {a_var:.2} | Varianz B: {b_var:.2}"
                ),
                "inline": false
            }
        ]
    })
}

/// Message-Listener für `!balance` (Muster `spawn_admin_command_listener`):
/// parst den Befehl und schickt die Antwort in denselben Kanal.
pub fn spawn(
    commands: Arc<BalanceCommands>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut events = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    let content = event.content.trim();
                    let Some(reply) = commands.reply_for(content, guild_id, event.author_id).await
                    else {
                        continue;
                    };
                    let _ = sender
                        .send_to_channel(event.channel_id, reply.content.as_deref(), &reply.embeds)
                        .await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rang_aus_rollen_nimmt_hoechsten() {
        // Eternus (11) + Seeker (2) → 11.
        assert_eq!(rank_from_roles(&[1331457652877955072, 1331458087349129296]), 11);
        // keine Rang-Rolle → 0.
        assert_eq!(rank_from_roles(&[999, 1000]), 0);
        assert_eq!(rank_from_roles(&[]), 0);
    }

    #[test]
    fn rang_name_nach_wert() {
        assert_eq!(rank_name(0), "Obscurus");
        assert_eq!(rank_name(11), "Eternus");
        assert_eq!(rank_name(99), "Obscurus"); // außerhalb → clamp
    }

    #[test]
    fn team_embed_markiert_balance() {
        // Perfekt ausgeglichen → ✅, Ø-Unterschied 0.
        let a = [("A", 5), ("B", 5)];
        let b = [("C", 5), ("D", 5)];
        let embed = team_embed(&a, &b, "T");
        let balance = embed["fields"][2]["value"].as_str().unwrap();
        assert!(balance.contains("✅"));
        assert!(balance.contains("0.00"));
    }

    struct MockPort(Vec<(u64, i64)>);

    #[async_trait::async_trait]
    impl BalancePort for MockPort {
        async fn caller_voice_members(&self, _g: u64, _u: u64) -> Vec<VoiceMember> {
            self.0
                .iter()
                .map(|&(id, val)| VoiceMember {
                    user_id: id,
                    display_name: format!("U{id}"),
                    // Wert val direkt als „Rolle" abbilden geht nicht — wir geben
                    // die passende Rang-Rollen-ID zurück.
                    role_ids: DISCORD_RANK_ROLES
                        .iter()
                        .find(|(_, v)| *v == val)
                        .map(|(rid, _)| vec![*rid])
                        .unwrap_or_default(),
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn auto_zu_wenige_spieler() {
        let cmds = BalanceCommands {
            port: Arc::new(MockPort(vec![(1, 5), (2, 5), (3, 5)])),
        };
        let reply = cmds.reply_for("!balance auto", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("Mindestens 4"));
    }

    #[tokio::test]
    async fn auto_baut_zwei_teams() {
        let cmds = BalanceCommands {
            port: Arc::new(MockPort(vec![(1, 11), (2, 1), (3, 11), (4, 1)])),
        };
        let reply = cmds.reply_for("!balance auto", 1, 1).await.unwrap();
        assert_eq!(reply.embeds.len(), 1);
        let balance = reply.embeds[0]["fields"][2]["value"].as_str().unwrap();
        // 11+1 vs 11+1 → perfekt ausgeglichen.
        assert!(balance.contains("✅"), "balance feld: {balance}");
    }

    #[tokio::test]
    async fn nicht_balance_befehl_ist_none() {
        let cmds = BalanceCommands {
            port: Arc::new(MockPort(vec![])),
        };
        assert!(cmds.reply_for("hallo welt", 1, 1).await.is_none());
    }

    #[tokio::test]
    async fn unbekannter_subcommand_meldet_folgt() {
        let cmds = BalanceCommands {
            port: Arc::new(MockPort(vec![])),
        };
        let reply = cmds.reply_for("!balance start", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("noch nicht verfügbar"));
    }
}
