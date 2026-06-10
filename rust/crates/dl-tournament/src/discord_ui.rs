//! Turnier-Discord-UI (User-Flow) — Port von `cogs/customgames/turnier.py`.
//!
//! Panel-Buttons mit Original-IDs (`turnier_panel_anmelden/abmelden/status`):
//! Anmelden prüft Turnier-Rolle, offenen Zeitraum und Steam-Verknüpfung,
//! zeigt den verifizierten Rang und bietet Solo- oder Team-Anmeldung
//! (Team-Auswahl mit Voll-Markierung bzw. Team-Neuerstellung per Formular).
//! Der Admin-Flow (Zeiträume/Teams/Clear/Panel-Posten) ist eine
//! dokumentierte Lücke und läuft in Python weiter — die Datenbasis ist
//! identisch (dl-tournament::store).
//!
//! Die Auswahl-Menüs des Originals sind RAM-Views (120-s-Timeout) — Rust
//! vergibt stabile `tn:*`-IDs mit eingebettetem Besitzer; sie überleben
//! damit auch Neustarts.

use std::sync::Arc;

use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::{json, Value};

use crate::store::{self, TournamentStore};

pub const TURNIER_ROLE_ID: u64 = 1474210107255554331;
pub const TEAM_MAX_SIZE: i64 = 5;

pub fn rank_display(rank_name: &str, subrank: i64) -> String {
    if rank_name.is_empty() {
        return "Unbekannt".to_string();
    }
    if subrank > 0 {
        format!("{rank_name} {subrank}")
    } else {
        rank_name.to_string()
    }
}

/// Anmeldung offen? (wie `_is_period_open`: aktiv + jetzt im Fenster,
/// lokale Zeit wie das Original).
pub fn is_period_open(period: Option<&Value>, now_local: chrono::NaiveDateTime) -> bool {
    let Some(period) = period else { return false };
    if period.get("is_active").and_then(Value::as_i64).unwrap_or(0) == 0 {
        return false;
    }
    let parse = |key: &str| {
        period
            .get(key)
            .and_then(Value::as_str)
            .and_then(crate::web::parse_period_dt)
    };
    match (parse("registration_start"), parse("registration_end")) {
        (Some(start), Some(end)) => start <= now_local && now_local <= end,
        _ => false,
    }
}

fn mode_components(user_id: u64, rank_name: &str, rank_sub: i64) -> Value {
    // Rang in die ID einbetten — das Menü ist zustandslos
    let rank = rank_name.to_lowercase();
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": "Solo anmelden", "emoji": { "name": "🎯" },
          "custom_id": format!("tn:solo:{rank}:{rank_sub}:{user_id}") },
        { "type": 2, "style": 2, "label": "Mit Team anmelden", "emoji": { "name": "🛡️" },
          "custom_id": format!("tn:team:{rank}:{rank_sub}:{user_id}") },
    ]}])
}

fn signup_success_embed(
    mode: &str,
    status: &str,
    rank_name: &str,
    rank_sub: i64,
    team: Option<&str>,
) -> Value {
    let status_txt = match status {
        "inserted" => "Eingetragen",
        "updated" => "Aktualisiert",
        _ => "Unverändert",
    };
    let title = if mode == "solo" {
        "✅ Solo-Anmeldung erfolgreich"
    } else {
        "✅ Team-Anmeldung erfolgreich"
    };
    let mut fields = vec![
        json!({ "name": "Status", "value": status_txt, "inline": true }),
        json!({ "name": "Modus", "value": if mode == "solo" { "Solo" } else { "Team" }, "inline": true }),
        json!({ "name": "Rang", "value": rank_display(rank_name, rank_sub), "inline": true }),
    ];
    if let Some(team) = team {
        fields.push(json!({ "name": "Team", "value": team, "inline": true }));
    }
    json!({ "title": title, "color": 0x2ECC71, "fields": fields })
}

// ── Discord-Seite ──────────────────────────────────────────────────────────

#[async_trait::async_trait]
pub trait TurnierPort: Send + Sync {
    async fn member_role_ids(&self, guild_id: u64, user_id: u64) -> Vec<u64>;
}

pub struct TurnierUi {
    pub store: Arc<TournamentStore>,
    pub port: Arc<dyn TurnierPort>,
}

struct TurnierHandler {
    ui: Arc<TurnierUi>,
}

#[async_trait::async_trait]
impl InteractionHandler for TurnierHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        match interaction.custom_id.as_str() {
            "turnier_panel_anmelden" => {
                let roles = ui
                    .port
                    .member_role_ids(interaction.guild_id, interaction.user_id)
                    .await;
                if !roles.contains(&TURNIER_ROLE_ID) {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Du benötigst die <@&{TURNIER_ROLE_ID}> Rolle, um dich anzumelden."
                    ));
                }
                let period = ui.store.active_period_json(interaction.guild_id).await;
                if !is_period_open(period.as_ref(), chrono::Local::now().naive_local()) {
                    return BridgeReply::ephemeral_text(
                        "❌ Die Anmeldung ist aktuell **nicht geöffnet**.",
                    );
                }
                let Some((rank_name, rank_sub)) =
                    ui.store.verified_steam_rank(interaction.user_id).await
                else {
                    return BridgeReply::ephemeral_text(
                        "❌ Dein Steam-Konto ist noch nicht mit Discord verknüpft.\n\
Nutze `/account_verknüpfen` auf dem Server, um dein Konto zu verbinden.",
                    );
                };
                let team_size = period
                    .as_ref()
                    .and_then(|p| p.get("team_size"))
                    .and_then(Value::as_i64)
                    .unwrap_or(TEAM_MAX_SIZE);
                BridgeReply {
                    embeds: vec![json!({
                        "title": "🏆 Turnier-Anmeldung",
                        "description": format!(
                            "Dein aktueller Rang: **{}**\nMax. Teamgröße: **{team_size}** Spieler\n\n\
                    Möchtest du dich **solo** oder **mit einem Team** anmelden?",
                            rank_display(&rank_name, rank_sub)
                        ),
                        "color": 0xF1C40F,
                    })],
                    components: Some(mode_components(interaction.user_id, &rank_name, rank_sub)),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "turnier_panel_abmelden" => {
                let removed = ui
                    .store
                    .remove_signup(interaction.guild_id, interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text(if removed {
                    "✅ Du wurdest aus dem Turnier abgemeldet."
                } else {
                    "ℹ️ Du warst nicht angemeldet."
                })
            }
            "turnier_panel_status" => {
                let Some(signup) = ui
                    .store
                    .get_signup(interaction.guild_id, interaction.user_id)
                    .await
                else {
                    return BridgeReply::ephemeral_text(
                        "ℹ️ Du bist aktuell **nicht** für das Turnier angemeldet.",
                    );
                };
                let rank_name = store::rank_label(&signup.rank);
                let mode = if signup.registration_mode == "team" {
                    "Team"
                } else {
                    "Solo"
                };
                BridgeReply {
                    embeds: vec![json!({
                        "title": "📊 Mein Turnierstatus",
                        "color": 0x3498DB,
                        "fields": [
                            { "name": "Rang", "value": rank_display(rank_name, signup.rank_subvalue), "inline": true },
                            { "name": "Modus", "value": mode, "inline": true },
                            { "name": "Team", "value": signup.team_name.unwrap_or_else(|| "—".to_string()), "inline": true },
                        ],
                    })],
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            _ => self.handle_flow(interaction).await,
        }
    }
}

impl TurnierHandler {
    /// tn:solo/team/pick/create-Flow (Besitzer steckt am ID-Ende).
    async fn handle_flow(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        let parts: Vec<&str> = interaction.custom_id.split(':').collect();
        let owner: u64 = parts
            .last()
            .and_then(|raw| raw.parse().ok())
            .unwrap_or_default();
        if owner != interaction.user_id {
            return BridgeReply::ephemeral_text("Dieses Menü gehört dir nicht.");
        }
        let action = parts.get(1).copied().unwrap_or_default();
        let rank = parts.get(2).copied().unwrap_or("initiate").to_string();
        let rank_sub: i64 = parts.get(3).and_then(|r| r.parse().ok()).unwrap_or(0);
        let rank_name = store::rank_label(&rank);

        match action {
            "solo" => {
                let result = ui
                    .store
                    .upsert_signup(
                        interaction.guild_id,
                        interaction.user_id,
                        "solo",
                        &rank,
                        rank_sub,
                        None,
                        false,
                        None,
                    )
                    .await;
                match result {
                    Ok(signup) => BridgeReply {
                        embeds: vec![signup_success_embed(
                            "solo",
                            &signup.status,
                            rank_name,
                            rank_sub,
                            None,
                        )],
                        ephemeral: true,
                        ..BridgeReply::default()
                    },
                    Err(err) => BridgeReply::ephemeral_text(format!("❌ {err}")),
                }
            }
            "team" => {
                let teams = ui.store.list_teams(interaction.guild_id).await;
                if teams.is_empty() {
                    return BridgeReply {
                        modal: Some(team_create_modal(&rank, rank_sub, interaction.user_id)),
                        ..BridgeReply::default()
                    };
                }
                let mut options: Vec<Value> = teams
                    .iter()
                    .take(24)
                    .map(|team| {
                        let full_tag = if team.member_count >= TEAM_MAX_SIZE {
                            " ✗ voll".to_string()
                        } else {
                            format!(" ({}/{TEAM_MAX_SIZE})", team.member_count)
                        };
                        json!({
                            "label": team.name.chars().take(100).collect::<String>(),
                            "value": team.id.to_string(),
                            "description": format!("Mitglieder: {}/{TEAM_MAX_SIZE}{full_tag}", team.member_count),
                        })
                    })
                    .collect();
                options.push(json!({
                    "label": "Neues Team erstellen",
                    "value": "__create__",
                    "emoji": { "name": "➕" },
                    "description": "Erstelle ein neues Team für dich",
                }));
                BridgeReply {
                    embeds: vec![json!({
                        "title": "🛡️ Team auswählen",
                        "description": "Wähle dein Team oder erstelle ein neues.",
                        "color": 0x3498DB,
                    })],
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 3,
                        "custom_id": format!("tn:pick:{rank}:{rank_sub}:{}", interaction.user_id),
                        "placeholder": "Team auswählen…",
                        "options": options,
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "pick" => {
                let Some(value) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                if value == "__create__" {
                    return BridgeReply {
                        modal: Some(team_create_modal(&rank, rank_sub, interaction.user_id)),
                        ..BridgeReply::default()
                    };
                }
                let team_id: i64 = value.parse().unwrap_or(0);
                if let Some(team) = ui.store.get_team(interaction.guild_id, team_id).await {
                    if team.member_count >= TEAM_MAX_SIZE {
                        return BridgeReply::ephemeral_text(format!(
                            "❌ Dieses Team ist bereits voll ({TEAM_MAX_SIZE}/{TEAM_MAX_SIZE})."
                        ));
                    }
                }
                self.join_team(&interaction, &rank, rank_sub, team_id).await
            }
            "create" => {
                let name = interaction
                    .options
                    .get("team_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let team = match ui
                    .store
                    .get_or_create_team(interaction.guild_id, &name, Some(interaction.user_id))
                    .await
                {
                    Ok(team) => team,
                    Err(err) => return BridgeReply::ephemeral_text(format!("❌ {err}")),
                };
                if !team.created && team.member_count >= TEAM_MAX_SIZE {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Team **{}** ist bereits voll ({}/{TEAM_MAX_SIZE}).",
                        team.name, team.member_count
                    ));
                }
                self.join_team(&interaction, &rank, rank_sub, team.id).await
            }
            _ => BridgeReply::ephemeral_text("Unbekannte Aktion."),
        }
    }

    async fn join_team(
        &self,
        interaction: &BridgeInteraction,
        rank: &str,
        rank_sub: i64,
        team_id: i64,
    ) -> BridgeReply {
        let result = self
            .ui
            .store
            .upsert_signup(
                interaction.guild_id,
                interaction.user_id,
                "team",
                rank,
                rank_sub,
                Some(team_id),
                false,
                None,
            )
            .await;
        match result {
            Ok(signup) => BridgeReply {
                embeds: vec![signup_success_embed(
                    "team",
                    &signup.status,
                    store::rank_label(rank),
                    rank_sub,
                    signup.team_name.as_deref(),
                )],
                ephemeral: true,
                ..BridgeReply::default()
            },
            Err(err) => BridgeReply::ephemeral_text(format!("❌ {err}")),
        }
    }
}

fn team_create_modal(rank: &str, rank_sub: i64, user_id: u64) -> ModalSpec {
    ModalSpec {
        custom_id: format!("tn:create:{rank}:{rank_sub}:{user_id}"),
        title: "Neues Team erstellen".to_string(),
        fields: vec![ModalField {
            custom_id: "team_name".to_string(),
            label: "Teamname".to_string(),
            placeholder: "z. B. Team Alpha".to_string(),
            required: true,
            min_length: 2,
            max_length: 32,
            paragraph: false,
        }],
    }
}

pub fn register(router: &mut InteractionRouter, ui: Arc<TurnierUi>) {
    let handler = Arc::new(TurnierHandler { ui });
    router.on_custom_id("turnier_panel_anmelden", handler.clone());
    router.on_custom_id("turnier_panel_abmelden", handler.clone());
    router.on_custom_id("turnier_panel_status", handler.clone());
    router.on_prefix("tn:", handler);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn periode_offen() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 6, 10)
            .expect("datum")
            .and_hms_opt(12, 0, 0)
            .expect("uhrzeit");
        let period = json!({
            "is_active": 1,
            "registration_start": "2026-06-09T00:00:00",
            "registration_end": "2026-06-11T23:59:59",
        });
        assert!(is_period_open(Some(&period), now));
        let closed = json!({
            "is_active": 0,
            "registration_start": "2026-06-09T00:00:00",
            "registration_end": "2026-06-11T23:59:59",
        });
        assert!(!is_period_open(Some(&closed), now));
        let past = json!({
            "is_active": 1,
            "registration_start": "2026-06-01T00:00:00",
            "registration_end": "2026-06-02T00:00:00",
        });
        assert!(!is_period_open(Some(&past), now));
        assert!(!is_period_open(None, now));
    }

    #[test]
    fn anzeige_und_ids() {
        assert_eq!(rank_display("Archon", 3), "Archon 3");
        assert_eq!(rank_display("Archon", 0), "Archon");
        assert_eq!(rank_display("", 2), "Unbekannt");
        let components = mode_components(42, "Phantom", 2);
        assert_eq!(
            components[0]["components"][0]["custom_id"],
            "tn:solo:phantom:2:42"
        );
        let embed = signup_success_embed("solo", "inserted", "Phantom", 2, None);
        assert_eq!(embed["fields"][0]["value"], "Eingetragen");
        let embed = signup_success_embed("team", "updated", "Archon", 0, Some("Alpha"));
        assert_eq!(embed["fields"].as_array().expect("fields").len(), 4);
    }
}
