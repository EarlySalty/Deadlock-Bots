//! `!balance`-Admin-Befehlsschicht — Port von `cogs/deadlock_team_balancer.py`.
//!
//! Erste Scheibe: der `!balance`-Prefix-Listener + `auto`/`voice` (read-only
//! Team-Vorschau, kein Channel-Move) + die Hilfe. Die zustandsverändernden
//! Subcommands (`start`/`manual`/`status`/`matches`/`end`/`cleanup`) folgen.
//! Der Balancing-Algorithmus liegt schon in [`crate::balancer`].
//!
//! `auto`/`voice` sind wie im Original NICHT permission-gated (reine Vorschau);
//! die gateenden Subcommands prüfen ihre Rechte später selbst.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dl_discord::{ChannelSender, Dispatcher};
use serde_json::{json, Value};

use crate::balancer::best_split;

/// Pflicht-Kategorie für die beiden Team-Voice-Channels (`MATCH_CATEGORY_ID`).
const MATCH_CATEGORY_ID: u64 = 1289721245281292290;
/// Pause zwischen den zwei Channel-Erstellungen (Rate-Limit-Schoner).
const CREATE_SLEEP: Duration = Duration::from_millis(400);
/// Pause zwischen einzelnen Member-Moves (`MOVE_SLEEP`).
const MOVE_SLEEP: Duration = Duration::from_millis(350);

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

/// Ergebnis eines Member-Moves.
pub enum MoveOutcome {
    Moved,
    NotInVoice,
    Failed(String),
}

/// Discord-Anbindung des Balancers (vom Bot über den Cache/HTTP erfüllt).
#[async_trait::async_trait]
pub trait BalancePort: Send + Sync {
    /// Nicht-Bot-Mitglieder im Voice-Channel des Aufrufers (leer, wenn er in
    /// keinem ist).
    async fn caller_voice_members(&self, guild_id: u64, user_id: u64) -> Vec<VoiceMember>;
    /// Hat der Aufrufer `Server verwalten` (für `!balance start`)?
    async fn is_admin(&self, guild_id: u64, user_id: u64) -> bool;
    /// Aktueller Voice-Channel des Aufrufers (für die Rück-Move-Referenz).
    async fn caller_voice_channel(&self, guild_id: u64, user_id: u64) -> Option<u64>;
    /// Erstellt einen Voice-Channel unter `category_id`; `None` bei Fehler oder
    /// wenn die Kategorie fehlt. Gibt die Channel-ID zurück.
    async fn create_match_channel(
        &self,
        guild_id: u64,
        name: &str,
        category_id: u64,
    ) -> Option<u64>;
    /// Verschiebt ein Mitglied in den Channel (nur wenn es in Voice ist).
    async fn move_member(&self, guild_id: u64, user_id: u64, channel_id: u64) -> MoveOutcome;
    /// Name + Anzahl Nicht-Bot-Mitglieder eines Channels; `None`, wenn der
    /// Channel nicht (mehr) existiert.
    async fn channel_member_count(&self, guild_id: u64, channel_id: u64) -> Option<(String, usize)>;
    /// Löscht einen Channel; `true` bei Erfolg.
    async fn delete_channel(&self, channel_id: u64) -> bool;
    /// Hat der Aufrufer `Kanäle verwalten` (für `!balance cleanup`)?
    async fn can_manage_channels(&self, guild_id: u64, user_id: u64) -> bool;
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

/// Ein laufendes Match (in-memory, wie Pythons `active_matches`).
struct MatchInfo {
    guild_id: u64,
    team1_channel_id: u64,
    team2_channel_id: u64,
    players: Vec<u64>,
    started_at: i64,
    /// Wird wie im Original gespeichert, aber nie gelesen (Legacy-Feld).
    #[allow(dead_code)]
    original_channel_id: Option<u64>,
}

pub struct BalanceCommands {
    pub port: Arc<dyn BalancePort>,
    matches: Mutex<HashMap<String, MatchInfo>>,
    counter: Mutex<u32>,
}

impl BalanceCommands {
    pub fn new(port: Arc<dyn BalancePort>) -> Self {
        Self {
            port,
            matches: Mutex::new(HashMap::new()),
            counter: Mutex::new(0),
        }
    }

    fn next_match_id(&self) -> String {
        let mut c = self.counter.lock().expect("balance counter");
        *c += 1;
        format!("{:03}", *c)
    }

    /// Verarbeitet eine Nachricht; `None`, wenn es kein `!balance`-Befehl ist.
    pub async fn reply_for(&self, content: &str, guild_id: u64, user_id: u64) -> Option<BalanceReply> {
        let mut parts = content.split_whitespace();
        let root = parts.next()?.to_lowercase();
        if !matches!(root.as_str(), "!balance" | "!bal" | "!teams") {
            return None;
        }
        let sub = parts.next().unwrap_or_default().to_lowercase();
        let args: Vec<&str> = parts.collect();
        let reply = match sub.as_str() {
            "" => BalanceReply::embed(help_embed()),
            "auto" | "voice" => self.auto_preview(guild_id, user_id).await,
            "start" => self.start(guild_id, user_id).await,
            "matches" => self.matches_list().await,
            "cleanup" => self.cleanup(guild_id, user_id, args.first().copied()).await,
            "end" => self.end(guild_id, args.first().copied(), args.get(1).copied()).await,
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

    /// `start`: erstellt 2 Match-Voice-Channels, verschiebt die Teams und merkt
    /// sich das Match (Port von `_run_balance_and_start`). `manage_guild`-gated.
    async fn start(&self, guild_id: u64, user_id: u64) -> BalanceReply {
        if !self.port.is_admin(guild_id, user_id).await {
            return BalanceReply::text("❌ Dafür brauchst du die Berechtigung „Server verwalten“.");
        }
        let members = self.port.caller_voice_members(guild_id, user_id).await;
        if members.len() < 4 {
            return BalanceReply::text(format!(
                "❌ Mindestens 4 Spieler benötigt (aktuell: {})",
                members.len()
            ));
        }
        let mut players: Vec<(u64, String, i64)> = members
            .into_iter()
            .map(|m| (m.user_id, m.display_name, rank_from_roles(&m.role_ids)))
            .collect();
        players.sort_by_key(|p| std::cmp::Reverse(p.2));
        let values: Vec<i64> = players.iter().map(|p| p.2).collect();
        let (idx_a, idx_b) = best_split(&values);

        // Zwei Channels in der Match-Kategorie erstellen (Pflicht).
        let match_id = self.next_match_id();
        let Some(ch1) = self
            .port
            .create_match_channel(guild_id, &format!("🟠 Team Amber • {match_id}"), MATCH_CATEGORY_ID)
            .await
        else {
            return BalanceReply::text(format!(
                "❌ Konnte Channels nicht erstellen (Kategorie `{MATCH_CATEGORY_ID}` fehlt oder fehlende Berechtigung)."
            ));
        };
        tokio::time::sleep(CREATE_SLEEP).await;
        let Some(ch2) = self
            .port
            .create_match_channel(guild_id, &format!("🔵 Team Sapphire • {match_id}"), MATCH_CATEGORY_ID)
            .await
        else {
            return BalanceReply::text("❌ Konnte den zweiten Channel nicht erstellen.");
        };

        // Spieler verschieben: Team A → ch1, Team B → ch2.
        let original = self.port.caller_voice_channel(guild_id, user_id).await;
        let (mut moved_a, mut moved_b) = (0u32, 0u32);
        let mut failed: Vec<String> = Vec::new();
        let move_list: Vec<(usize, u64)> = idx_a
            .iter()
            .map(|&i| (i, ch1))
            .chain(idx_b.iter().map(|&i| (i, ch2)))
            .collect();
        for (i, target) in move_list {
            let (uid, name, _) = &players[i];
            match self.port.move_member(guild_id, *uid, target).await {
                MoveOutcome::Moved => {
                    if target == ch1 {
                        moved_a += 1;
                    } else {
                        moved_b += 1;
                    }
                }
                MoveOutcome::NotInVoice => failed.push(format!("{name} (nicht in Voice)")),
                MoveOutcome::Failed(err) => failed.push(format!("{name} ({err})")),
            }
            tokio::time::sleep(MOVE_SLEEP).await;
        }

        // Match merken (in-memory).
        let player_ids: Vec<u64> = idx_a.iter().chain(idx_b.iter()).map(|&i| players[i].0).collect();
        self.matches.lock().expect("matches").insert(
            match_id.clone(),
            MatchInfo {
                guild_id,
                team1_channel_id: ch1,
                team2_channel_id: ch2,
                players: player_ids,
                started_at: chrono::Utc::now().timestamp(),
                original_channel_id: original,
            },
        );

        // Ergebnis-Embed (Balance + Move-Resultat).
        let team_a: Vec<(&str, i64)> = idx_a.iter().map(|&i| (players[i].1.as_str(), players[i].2)).collect();
        let team_b: Vec<(&str, i64)> = idx_b.iter().map(|&i| (players[i].1.as_str(), players[i].2)).collect();
        let mut embed = team_embed(
            &team_a,
            &team_b,
            &format!("🎮 Match {match_id} – Teams erstellt & Spieler verschoben"),
        );
        if let Some(fields) = embed["fields"].as_array_mut() {
            fields.push(json!({
                "name": "Move",
                "value": format!("<#{ch1}>: {moved_a}/{}\n<#{ch2}>: {moved_b}/{}", team_a.len(), team_b.len()),
                "inline": false
            }));
            if !failed.is_empty() {
                let shown = failed.iter().take(6).cloned().collect::<Vec<_>>().join("\n");
                let extra = if failed.len() > 6 {
                    format!("\n… und {} weitere", failed.len() - 6)
                } else {
                    String::new()
                };
                fields.push(json!({
                    "name": "⚠️ Nicht bewegt",
                    "value": format!("{shown}{extra}"),
                    "inline": false
                }));
            }
        }
        embed["footer"] = json!({
            "text": format!("Match-ID: {match_id} • Beenden: !balance end {match_id}")
        });
        BalanceReply::embed(embed)
    }

    /// `matches`: listet alle aktiven Matches mit Channel-Belegung und Laufzeit.
    async fn matches_list(&self) -> BalanceReply {
        // Metadaten unter dem Lock kopieren (kein await im std-Mutex).
        let mut snapshot: Vec<(String, u64, u64, u64, i64)> = {
            let m = self.matches.lock().expect("matches");
            m.iter()
                .map(|(id, i)| (id.clone(), i.guild_id, i.team1_channel_id, i.team2_channel_id, i.started_at))
                .collect()
        };
        if snapshot.is_empty() {
            return BalanceReply::text("📭 Keine aktiven Matches");
        }
        snapshot.sort_by(|a, b| a.0.cmp(&b.0)); // stabile Reihenfolge nach Match-ID
        let now = chrono::Utc::now().timestamp();
        let mut fields = Vec::new();
        for (id, gid, ch1, ch2, started) in snapshot {
            let mut lines = Vec::new();
            for ch_id in [ch1, ch2] {
                match self.port.channel_member_count(gid, ch_id).await {
                    Some((name, count)) => lines.push(format!("{name}: {count} Spieler")),
                    None => lines.push(format!("{ch_id}: gelöscht")),
                }
            }
            let (min, sec) = dur_min_sec(now - started);
            fields.push(json!({
                "name": format!("Match {id}"),
                "value": format!("Dauer: {min}min {sec}s\n{}", lines.join("\n")),
                "inline": false
            }));
        }
        BalanceReply::embed(json!({
            "title": "🎮 Aktive Deadlock Matches",
            "color": 0x2ECC71,
            "fields": fields
        }))
    }

    /// `cleanup <hours>`: löscht Team-Channels von Matches, die älter als `hours`
    /// sind (Default 2, 1–24). `manage_channels`-gated.
    async fn cleanup(&self, guild_id: u64, user_id: u64, hours_arg: Option<&str>) -> BalanceReply {
        if !self.port.can_manage_channels(guild_id, user_id).await {
            return BalanceReply::text("❌ Dafür brauchst du die Berechtigung „Kanäle verwalten“.");
        }
        let hours: i64 = hours_arg.and_then(|s| s.parse().ok()).unwrap_or(2);
        if !(1..=24).contains(&hours) {
            return BalanceReply::text("❌ Stunden müssen zwischen 1–24 liegen");
        }
        let cutoff = chrono::Utc::now().timestamp() - hours * 3600;
        let targets: Vec<(String, u64, u64)> = {
            let m = self.matches.lock().expect("matches");
            m.iter()
                .filter(|(_, i)| i.started_at < cutoff)
                .map(|(id, i)| (id.clone(), i.team1_channel_id, i.team2_channel_id))
                .collect()
        };
        if targets.is_empty() {
            return BalanceReply::text(format!("🧹 Keine Matches älter als {hours}h gefunden"));
        }
        let mut deleted_ch = 0u32;
        for (id, ch1, ch2) in &targets {
            for ch_id in [*ch1, *ch2] {
                if self.port.delete_channel(ch_id).await {
                    deleted_ch += 1;
                    tokio::time::sleep(CREATE_SLEEP).await; // 0.4s Rate-Limit-Schoner
                }
            }
            self.matches.lock().expect("matches").remove(id);
        }
        BalanceReply::text(format!(
            "🧹 {} Matches bereinigt ({deleted_ch} Channels gelöscht)",
            targets.len()
        ))
    }

    /// `end <id> [skip]`: optionale Debrief-Lane + Move, löscht die Team-Channels
    /// und entfernt das Match (Port von `balance_end`).
    async fn end(&self, guild_id: u64, match_id: Option<&str>, skip_arg: Option<&str>) -> BalanceReply {
        let _ = guild_id; // Gilde steckt im MatchInfo, nicht im Aufruf.
        let Some(match_id) = match_id else {
            return BalanceReply::text("❌ Bitte Match-ID angeben: `!balance end <id>`");
        };
        let info = {
            let m = self.matches.lock().expect("matches");
            m.get(match_id).map(|i| {
                (i.guild_id, i.team1_channel_id, i.team2_channel_id, i.players.clone(), i.started_at)
            })
        };
        let Some((gid, ch1, ch2, players, started)) = info else {
            return BalanceReply::text(format!("❌ Match `{match_id}` nicht gefunden."));
        };
        let skip_debrief = matches!(
            skip_arg.map(|s| s.to_lowercase()).as_deref(),
            Some("true" | "1" | "yes" | "y" | "ja" | "skip")
        );

        // 1) Optionale Nachbesprechungs-Lane + Move der noch in Team-Channels Sitzenden.
        let mut debrief_ch: Option<u64> = None;
        let mut moved = 0u32;
        let mut move_fail: Vec<String> = Vec::new();
        if !skip_debrief {
            debrief_ch = self
                .port
                .create_match_channel(gid, &format!("💬 Nachbesprechung • {match_id}"), MATCH_CATEGORY_ID)
                .await;
            if let Some(debrief) = debrief_ch {
                for uid in &players {
                    let cur = self.port.caller_voice_channel(gid, *uid).await;
                    if cur != Some(ch1) && cur != Some(ch2) {
                        continue; // nur bewegen, wer noch im Match-Channel sitzt
                    }
                    match self.port.move_member(gid, *uid, debrief).await {
                        MoveOutcome::Moved => {
                            moved += 1;
                            tokio::time::sleep(MOVE_SLEEP).await;
                        }
                        MoveOutcome::NotInVoice => move_fail.push(format!("<@{uid}> (nicht in Voice)")),
                        MoveOutcome::Failed(err) => move_fail.push(format!("<@{uid}> ({err})")),
                    }
                }
            }
        }

        // 2) Team-Channels löschen (Namen vorher ziehen für die Anzeige).
        let mut deleted: Vec<String> = Vec::new();
        let mut failed: Vec<String> = Vec::new();
        for ch_id in [ch1, ch2] {
            let name = self.port.channel_member_count(gid, ch_id).await.map(|(n, _)| n);
            if self.port.delete_channel(ch_id).await {
                deleted.push(name.unwrap_or_else(|| ch_id.to_string()));
                tokio::time::sleep(CREATE_SLEEP).await;
            } else {
                failed.push(format!("{} (nicht gefunden)", name.unwrap_or_else(|| ch_id.to_string())));
            }
        }

        // 3) Match entfernen.
        self.matches.lock().expect("matches").remove(match_id);

        // 4) Ergebnis-Embed (grün mit Debrief, sonst orange).
        let mut fields = Vec::new();
        if let Some(debrief) = debrief_ch {
            fields.push(json!({
                "name": "💬 Nachbesprechungs-Lane",
                "value": format!("<#{debrief}>\nSpieler bewegt: {moved}/{}", players.len()),
                "inline": false
            }));
            if !move_fail.is_empty() {
                let shown = move_fail.iter().take(6).cloned().collect::<Vec<_>>().join("\n");
                let extra = if move_fail.len() > 6 {
                    format!("\n… und {} weitere", move_fail.len() - 6)
                } else {
                    String::new()
                };
                fields.push(json!({ "name": "⚠️ Nicht bewegt", "value": format!("{shown}{extra}"), "inline": false }));
            }
        }
        if !deleted.is_empty() {
            fields.push(json!({ "name": "🗑️ Gelöschte Team-Channels", "value": deleted.join("\n"), "inline": false }));
        }
        if !failed.is_empty() {
            fields.push(json!({ "name": "⚠️ Nicht gelöscht", "value": failed.join("\n"), "inline": false }));
        }
        let (min, sec) = dur_min_sec(chrono::Utc::now().timestamp() - started);
        fields.push(json!({ "name": "📊 Dauer", "value": format!("{min}min {sec}s"), "inline": true }));
        fields.push(json!({ "name": "👥 Spieler", "value": players.len().to_string(), "inline": true }));
        BalanceReply::embed(json!({
            "title": format!("🏁 Match {match_id} beendet"),
            "color": if debrief_ch.is_some() { 0x2ECC71 } else { 0xE67E22 },
            "fields": fields
        }))
    }
}

/// Laufzeit wie Pythons `timedelta.seconds`: Rest innerhalb eines Tages → min/sek.
fn dur_min_sec(elapsed_secs: i64) -> (i64, i64) {
    let within_day = elapsed_secs.rem_euclid(86_400);
    (within_day / 60, within_day % 60)
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
                          `!balance start` – Channels erstellen & Spieler moven\n\
                          `!balance matches` – aktive Matches\n\
                          `!balance end <id> [skip]` – Match beenden (+ Debrief)\n\
                          `!balance cleanup <hours>` – alte Matches löschen\n\
                          `!balance manual @u1 …` – manuelle Auswahl (folgt)\n\
                          `!balance status [@user]` – Rank-Status (folgt)",
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
        assert_eq!(rank_name(99), "Obscurus"); // außerhalb → Obscurus
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

    struct MockPort {
        members: Vec<(u64, i64)>,
        admin: bool,
    }

    impl MockPort {
        fn new(members: Vec<(u64, i64)>) -> Arc<Self> {
            Arc::new(Self { members, admin: false })
        }
        fn admin(members: Vec<(u64, i64)>) -> Arc<Self> {
            Arc::new(Self { members, admin: true })
        }
    }

    #[async_trait::async_trait]
    impl BalancePort for MockPort {
        async fn caller_voice_members(&self, _g: u64, _u: u64) -> Vec<VoiceMember> {
            self.members
                .iter()
                .map(|&(id, val)| VoiceMember {
                    user_id: id,
                    display_name: format!("U{id}"),
                    // Den Wert auf die passende Rang-Rollen-ID abbilden.
                    role_ids: DISCORD_RANK_ROLES
                        .iter()
                        .find(|(_, v)| *v == val)
                        .map(|(rid, _)| vec![*rid])
                        .unwrap_or_default(),
                })
                .collect()
        }
        async fn is_admin(&self, _g: u64, _u: u64) -> bool {
            self.admin
        }
        async fn caller_voice_channel(&self, _g: u64, _u: u64) -> Option<u64> {
            Some(999)
        }
        async fn create_match_channel(&self, _g: u64, name: &str, _cat: u64) -> Option<u64> {
            // Distinkte IDs je Team, damit ch1 != ch2.
            Some(if name.contains("Amber") { 7001 } else { 7002 })
        }
        async fn move_member(&self, _g: u64, _u: u64, _ch: u64) -> MoveOutcome {
            MoveOutcome::Moved
        }
        async fn channel_member_count(&self, _g: u64, _ch: u64) -> Option<(String, usize)> {
            Some(("Team-Channel".to_string(), 0))
        }
        async fn delete_channel(&self, _ch: u64) -> bool {
            true
        }
        async fn can_manage_channels(&self, _g: u64, _u: u64) -> bool {
            self.admin
        }
    }

    #[tokio::test]
    async fn auto_zu_wenige_spieler() {
        let cmds = BalanceCommands::new(MockPort::new(vec![(1, 5), (2, 5), (3, 5)]));
        let reply = cmds.reply_for("!balance auto", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("Mindestens 4"));
    }

    #[tokio::test]
    async fn auto_baut_zwei_teams() {
        let cmds = BalanceCommands::new(MockPort::new(vec![(1, 11), (2, 1), (3, 11), (4, 1)]));
        let reply = cmds.reply_for("!balance auto", 1, 1).await.unwrap();
        assert_eq!(reply.embeds.len(), 1);
        let balance = reply.embeds[0]["fields"][2]["value"].as_str().unwrap();
        // 11+1 vs 11+1 → perfekt ausgeglichen.
        assert!(balance.contains("✅"), "balance feld: {balance}");
    }

    #[tokio::test]
    async fn nicht_balance_befehl_ist_none() {
        let cmds = BalanceCommands::new(MockPort::new(vec![]));
        assert!(cmds.reply_for("hallo welt", 1, 1).await.is_none());
    }

    #[tokio::test]
    async fn unbekannter_subcommand_meldet_folgt() {
        let cmds = BalanceCommands::new(MockPort::new(vec![]));
        let reply = cmds.reply_for("!balance gibtsnicht", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("noch nicht verfügbar"));
    }

    #[tokio::test]
    async fn start_ohne_admin_blockt() {
        let cmds = BalanceCommands::new(MockPort::new(vec![(1, 11), (2, 1), (3, 11), (4, 1)]));
        let reply = cmds.reply_for("!balance start", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("Server verwalten"));
    }

    // start_paused: die Rate-Limit-Sleeps laufen instant durch.
    #[tokio::test(start_paused = true)]
    async fn start_erstellt_match_und_bewegt() {
        let cmds = BalanceCommands::new(MockPort::admin(vec![(1, 11), (2, 1), (3, 11), (4, 1)]));
        let reply = cmds.reply_for("!balance start", 1, 1).await.unwrap();
        assert_eq!(reply.embeds.len(), 1);
        assert!(reply.embeds[0]["title"].as_str().unwrap().contains("Match 001"));
        // Match wurde in-memory gemerkt.
        assert_eq!(cmds.matches.lock().unwrap().len(), 1);
        // Move-Feld: alle 4 bewegt (2/2 + 2/2).
        let fields = reply.embeds[0]["fields"].as_array().unwrap();
        let move_field = fields.iter().find(|f| f["name"] == "Move").unwrap();
        assert!(move_field["value"].as_str().unwrap().contains("2/2"));
    }

    #[tokio::test(start_paused = true)]
    async fn matches_und_end_lifecycle() {
        let cmds = BalanceCommands::new(MockPort::admin(vec![(1, 11), (2, 1), (3, 11), (4, 1)]));
        cmds.reply_for("!balance start", 1, 1).await.unwrap();
        assert_eq!(cmds.matches.lock().unwrap().len(), 1);

        // matches listet das laufende Match.
        let list = cmds.reply_for("!balance matches", 1, 1).await.unwrap();
        assert_eq!(list.embeds.len(), 1);
        let fields = list.embeds[0]["fields"].as_array().unwrap();
        assert!(fields.iter().any(|f| f["name"] == "Match 001"));

        // end 001 beendet es und entfernt es aus dem Speicher.
        let ended = cmds.reply_for("!balance end 001", 1, 1).await.unwrap();
        assert!(ended.embeds[0]["title"].as_str().unwrap().contains("Match 001 beendet"));
        assert!(cmds.matches.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn end_meldet_fehlende_und_unbekannte_id() {
        let cmds = BalanceCommands::new(MockPort::new(vec![]));
        let no_id = cmds.reply_for("!balance end", 1, 1).await.unwrap();
        assert!(no_id.content.unwrap().contains("Match-ID angeben"));
        let unknown = cmds.reply_for("!balance end 042", 1, 1).await.unwrap();
        assert!(unknown.content.unwrap().contains("nicht gefunden"));
    }

    #[tokio::test]
    async fn cleanup_ohne_recht_blockt() {
        let cmds = BalanceCommands::new(MockPort::new(vec![]));
        let reply = cmds.reply_for("!balance cleanup 2", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("Kanäle verwalten"));
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_loescht_alte_matches() {
        let cmds = BalanceCommands::new(MockPort::admin(vec![]));
        // Künstlich „altes“ Match (3h) direkt einsetzen.
        let old_start = chrono::Utc::now().timestamp() - 3 * 3600;
        cmds.matches.lock().unwrap().insert(
            "009".to_string(),
            MatchInfo {
                guild_id: 1,
                team1_channel_id: 7001,
                team2_channel_id: 7002,
                players: vec![1, 2],
                started_at: old_start,
                original_channel_id: None,
            },
        );
        // cleanup 1 → älter als 1h → gelöscht.
        let reply = cmds.reply_for("!balance cleanup 1", 1, 1).await.unwrap();
        let text = reply.content.unwrap();
        assert!(text.contains("1 Matches bereinigt"), "text: {text}");
        assert!(text.contains("2 Channels gelöscht"), "text: {text}");
        assert!(cmds.matches.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cleanup_ohne_alte_matches_meldet_leer() {
        let cmds = BalanceCommands::new(MockPort::admin(vec![]));
        let reply = cmds.reply_for("!balance cleanup 2", 1, 1).await.unwrap();
        assert!(reply.content.unwrap().contains("Keine Matches älter als 2h"));
    }
}
