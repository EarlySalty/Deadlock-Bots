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

use dl_discord::interactions::{ChannelMessage, ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, CommandSpec, InteractionHandler, InteractionRouter,
};
use serde_json::{json, Value};

use crate::store::{self, TournamentStore};

pub const TURNIER_ROLE_ID: u64 = 1474210107255554331;
pub const TEAM_MAX_SIZE: i64 = store::TEAM_MAX_SIZE;

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

/// `_fmt_dt`: ISO-Datum → „TT.MM.JJJJ HH:MM Uhr", sonst „—".
fn fmt_dt(raw: Option<&str>) -> String {
    match raw.and_then(crate::web::parse_period_dt) {
        Some(dt) => dt.format("%d.%m.%Y %H:%M Uhr").to_string(),
        None => "—".to_string(),
    }
}

/// `_period_status_str`: Kein Zeitraum / Geschlossen / Startet … / Abgelaufen / Offen.
fn period_status_str(period: Option<&Value>, now: chrono::NaiveDateTime) -> String {
    let Some(p) = period else {
        return "Kein Zeitraum".to_string();
    };
    if p.get("is_active").and_then(Value::as_i64).unwrap_or(0) == 0 {
        return "⛔ Geschlossen".to_string();
    }
    let parse = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .and_then(crate::web::parse_period_dt)
    };
    match (parse("registration_start"), parse("registration_end")) {
        (Some(start), Some(end)) => {
            if now < start {
                format!(
                    "⏳ Startet {}",
                    fmt_dt(p.get("registration_start").and_then(Value::as_str))
                )
            } else if now > end {
                "⛔ Abgelaufen".to_string()
            } else {
                "🟢 Offen".to_string()
            }
        }
        _ => "Unbekannt".to_string(),
    }
}

/// Persistentes Anmelde-Panel (Port von `_build_panel_embed`, turnier.py:115-149).
fn panel_embed(
    period: Option<&Value>,
    summary: Option<&Value>,
    now: chrono::NaiveDateTime,
) -> Value {
    let active = period.filter(|p| p.get("is_active").and_then(Value::as_i64).unwrap_or(0) != 0);
    let mut fields: Vec<Value> = Vec::new();
    let mut description: Option<String> = None;

    if let Some(p) = active {
        fields.push(json!({
            "name": "📅 Zeitraum",
            "value": p.get("name").and_then(Value::as_str).unwrap_or("—"),
            "inline": false,
        }));
        fields.push(
            json!({ "name": "Status", "value": period_status_str(period, now), "inline": true }),
        );
        fields.push(json!({
            "name": "🕐 Start",
            "value": fmt_dt(p.get("registration_start").and_then(Value::as_str)),
            "inline": true,
        }));
        fields.push(json!({
            "name": "🕐 Ende",
            "value": fmt_dt(p.get("registration_end").and_then(Value::as_str)),
            "inline": true,
        }));
    } else {
        description = Some("Aktuell ist **kein Anmeldezeitraum** aktiv.".to_string());
    }

    if let Some(s) = summary {
        let g = |k: &str| s.get(k).and_then(Value::as_i64).unwrap_or(0);
        fields.push(json!({
            "name": "👥 Anmeldungen",
            "value": format!(
                "Gesamt: **{}** | Solo: **{}** | Team: **{}**",
                g("signups_total"), g("solo_count"), g("team_count")
            ),
            "inline": false,
        }));
    }

    fields.push(json!({
        "name": "ℹ️ Voraussetzungen",
        "value": format!(
            "• Du benötigst die <@&{TURNIER_ROLE_ID}> Rolle\n\
             • Steam-Konto verknüpfen: `/account_verknüpfen`"
        ),
        "inline": false,
    }));

    let mut embed = json!({
        "title": "🏆 Deadlock Turnier-Anmeldung",
        "color": 0xF1C40F,
        "fields": fields,
    });
    if let Some(desc) = description {
        embed["description"] = json!(desc);
    }
    embed
}

/// Persistente Panel-Buttons (Original-custom_ids `turnier_panel_*`).
fn panel_components() -> Value {
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 3, "label": "Jetzt anmelden", "emoji": { "name": "✅" },
          "custom_id": "turnier_panel_anmelden" },
        { "type": 2, "style": 4, "label": "Abmelden", "emoji": { "name": "🚪" },
          "custom_id": "turnier_panel_abmelden" },
        { "type": 2, "style": 2, "label": "Mein Status", "emoji": { "name": "📊" },
          "custom_id": "turnier_panel_status" },
    ]}])
}

fn period_team_size(period: Option<&Value>) -> i64 {
    period
        .and_then(|p| p.get("team_size"))
        .and_then(Value::as_i64)
        .filter(|size| *size > 0)
        .unwrap_or(TEAM_MAX_SIZE)
}

fn mode_components(user_id: u64, rank_name: &str, rank_sub: i64, team_size: i64) -> Value {
    // Rang in die ID einbetten — das Menü ist zustandslos
    let rank = rank_name.to_lowercase();
    json!([{ "type": 1, "components": [
        { "type": 2, "style": 1, "label": "Solo anmelden", "emoji": { "name": "🎯" },
          "custom_id": format!("tn:solo:{rank}:{rank_sub}:{team_size}:{user_id}") },
        { "type": 2, "style": 2, "label": "Mit Team anmelden", "emoji": { "name": "🛡️" },
          "custom_id": format!("tn:team:{rank}:{rank_sub}:{team_size}:{user_id}") },
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
        // /turnier-Slash → Dashboard-Embed.
        if interaction.command == "turnier" {
            return self.dashboard(interaction).await;
        }
        // /turnierpanel-Slash → Panel öffentlich in den Kanal posten (Admin).
        if interaction.command == "turnierpanel" {
            return self.post_panel(interaction).await;
        }
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
                let team_size = period_team_size(period.as_ref());
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
                    components: Some(mode_components(
                        interaction.user_id,
                        &rank_name,
                        rank_sub,
                        team_size,
                    )),
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
    /// `/turnier`-Dashboard (Port von `turnier_cmd`): Zeitraum- + Nutzer-Status
    /// plus die schon portierten Panel-Buttons (anmelden/abmelden/status, je
    /// nach Lage). Admin-Tools bleiben die dokumentierte Python-Lücke.
    async fn dashboard(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        let now = chrono::Local::now().naive_local();
        let period = ui.store.active_period_json(interaction.guild_id).await;
        let summary = ui.store.summary(interaction.guild_id).await.ok();
        let signup = ui
            .store
            .get_signup(interaction.guild_id, interaction.user_id)
            .await;
        let period_open = is_period_open(period.as_ref(), now);
        // Aktive Periode (is_active != 0) — None deckt „kein/inaktiv" ab.
        let active_period = period
            .as_ref()
            .filter(|p| p.get("is_active").and_then(Value::as_i64).unwrap_or(0) != 0);

        let mut fields: Vec<Value> = Vec::new();
        let mut description = String::new();

        if let Some(p) = active_period {
            fields.push(json!({
                "name": "📅 Zeitraum",
                "value": p.get("name").and_then(Value::as_str).unwrap_or("—"),
                "inline": false,
            }));
            fields.push(json!({
                "name": "Status",
                "value": period_status_str(period.as_ref(), now),
                "inline": true,
            }));
            fields.push(json!({
                "name": "🕐 Ende",
                "value": fmt_dt(p.get("registration_end").and_then(Value::as_str)),
                "inline": true,
            }));
            let g = |k: &str| {
                summary
                    .as_ref()
                    .and_then(|s| s.get(k))
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            };
            fields.push(json!({
                "name": "👥 Anmeldungen",
                "value": format!(
                    "**{}** gesamt  |  Solo: {}  |  Team: {}",
                    g("signups_total"), g("solo_count"), g("team_count")
                ),
                "inline": false,
            }));
        } else {
            description = "Kein aktiver Anmeldezeitraum.".to_string();
        }

        if let Some(s) = &signup {
            let rank_name = store::rank_label(&s.rank);
            let mode = if s.registration_mode == "team" {
                "Team"
            } else {
                "Solo"
            };
            let team = s.team_name.clone().unwrap_or_else(|| "—".to_string());
            fields.push(json!({
                "name": "✅ Du bist angemeldet",
                "value": format!(
                    "Rang: **{}**  |  Modus: {mode}  |  Team: {team}",
                    rank_display(rank_name, s.rank_subvalue)
                ),
                "inline": false,
            }));
        } else {
            let roles = ui
                .port
                .member_role_ids(interaction.guild_id, interaction.user_id)
                .await;
            if !roles.contains(&TURNIER_ROLE_ID) {
                fields.push(json!({
                    "name": "⚠️ Fehlende Rolle",
                    "value": format!("Du benötigst die <@&{TURNIER_ROLE_ID}> Rolle."),
                    "inline": false,
                }));
            } else if period_open {
                fields.push(json!({
                    "name": "📋 Status",
                    "value": "Noch nicht angemeldet. Klicke **Anmelden**.",
                    "inline": false,
                }));
            }
        }

        let mut embed =
            json!({ "title": "🏆 Deadlock Turnier", "color": 0xF1C40F, "fields": fields });
        if !description.is_empty() {
            embed["description"] = json!(description);
        }

        // Buttons je nach Lage (Original-IDs, schon portierte Handler).
        let mut buttons: Vec<Value> = Vec::new();
        if period_open && signup.is_none() {
            buttons.push(json!({ "type": 2, "style": 1, "label": "Anmelden", "custom_id": "turnier_panel_anmelden" }));
        }
        if signup.is_some() {
            buttons.push(json!({ "type": 2, "style": 4, "label": "Abmelden", "custom_id": "turnier_panel_abmelden" }));
        }
        buttons.push(json!({ "type": 2, "style": 2, "label": "Status", "custom_id": "turnier_panel_status" }));

        BridgeReply {
            embeds: vec![embed],
            components: Some(json!([{ "type": 1, "components": buttons }])),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    /// `/turnierpanel` (Port von `AdminDashboardView.post_panel_btn` /
    /// `balance_tournament_panel`): postet das persistente Anmelde-Panel
    /// (Embed + `turnier_panel_*`-Buttons) öffentlich in den aktuellen Kanal und
    /// bestätigt ephemeral. Admin-gated über `default_member_permissions` (Manage
    /// Guild) bei der Command-Registrierung — Discord blockt serverseitig.
    async fn post_panel(&self, interaction: BridgeInteraction) -> BridgeReply {
        let ui = &self.ui;
        let now = chrono::Local::now().naive_local();
        let period = ui.store.active_period_json(interaction.guild_id).await;
        let summary = ui.store.summary(interaction.guild_id).await.ok();
        let embed = panel_embed(period.as_ref(), summary.as_ref(), now);
        BridgeReply {
            channel_message: Some(ChannelMessage {
                embeds: vec![embed],
                components: Some(panel_components()),
                confirmation: "✅ Panel in diesem Channel gepostet.".to_string(),
            }),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

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
        let team_size = if parts.len() >= 6 {
            parts
                .get(4)
                .and_then(|raw| raw.parse::<i64>().ok())
                .filter(|size| *size > 0)
                .unwrap_or(TEAM_MAX_SIZE)
        } else {
            ui.store
                .active_period(interaction.guild_id)
                .await
                .map(|(_, _, size)| if size > 0 { size } else { TEAM_MAX_SIZE })
                .unwrap_or(TEAM_MAX_SIZE)
        };
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
                        modal: Some(team_create_modal(
                            &rank,
                            rank_sub,
                            team_size,
                            interaction.user_id,
                        )),
                        ..BridgeReply::default()
                    };
                }
                let mut options: Vec<Value> = teams
                    .iter()
                    .take(24)
                    .map(|team| {
                        let full_tag = if team.member_count >= team_size {
                            " ✗ voll".to_string()
                        } else {
                            format!(" ({}/{team_size})", team.member_count)
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
                        "custom_id": format!("tn:pick:{rank}:{rank_sub}:{team_size}:{}", interaction.user_id),
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
                        modal: Some(team_create_modal(
                            &rank,
                            rank_sub,
                            team_size,
                            interaction.user_id,
                        )),
                        ..BridgeReply::default()
                    };
                }
                let team_id: i64 = value.parse().unwrap_or(0);
                if let Some(team) = ui.store.get_team(interaction.guild_id, team_id).await {
                    if team.member_count >= team_size {
                        return BridgeReply::ephemeral_text(format!(
                            "❌ Dieses Team ist bereits voll ({team_size}/{team_size})."
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
                if !team.created && team.member_count >= team_size {
                    return BridgeReply::ephemeral_text(format!(
                        "❌ Team **{}** ist bereits voll ({}/{team_size}).",
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

fn team_create_modal(rank: &str, rank_sub: i64, team_size: i64, user_id: u64) -> ModalSpec {
    ModalSpec {
        custom_id: format!("tn:create:{rank}:{rank_sub}:{team_size}:{user_id}"),
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
    router.on_command(
        "turnier",
        CommandSpec {
            definition: json!({
                "name": "turnier",
                "description": "Turnier-Dashboard: Anmelden, Status und Verwaltung",
                "type": 1,
                "dm_permission": false,
            }),
        },
        handler.clone(),
    );
    // Admin: Anmelde-Panel in den Kanal posten (Manage Guild = 32).
    router.on_command(
        "turnierpanel",
        CommandSpec {
            definition: json!({
                "name": "turnierpanel",
                "description": "Postet das Turnier-Anmelde-Panel in diesen Kanal (Admin).",
                "type": 1,
                "dm_permission": false,
                "default_member_permissions": "32",
            }),
        },
        handler.clone(),
    );
    router.on_custom_id("turnier_panel_anmelden", handler.clone());
    router.on_custom_id("turnier_panel_abmelden", handler.clone());
    router.on_custom_id("turnier_panel_status", handler.clone());
    router.on_prefix("tn:", handler);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockTurnierPort;

    #[async_trait::async_trait]
    impl TurnierPort for MockTurnierPort {
        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            vec![TURNIER_ROLE_ID]
        }
    }

    async fn test_handler() -> (tempfile::TempDir, TurnierHandler) {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dl_db::Db::open_creating(dir.path().join("t.sqlite3")).expect("db");
        let store = TournamentStore::new(db);
        store.ensure_schema().await.expect("schema");
        let ui = Arc::new(TurnierUi {
            store: Arc::new(store),
            port: Arc::new(MockTurnierPort),
        });
        (dir, TurnierHandler { ui })
    }

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
        assert_eq!(TEAM_MAX_SIZE, 6);
        assert_eq!(rank_display("Archon", 3), "Archon 3");
        assert_eq!(rank_display("Archon", 0), "Archon");
        assert_eq!(rank_display("", 2), "Unbekannt");
        let components = mode_components(42, "Phantom", 2, 4);
        assert_eq!(
            components[0]["components"][0]["custom_id"],
            "tn:solo:phantom:2:4:42"
        );
        assert_eq!(
            components[0]["components"][1]["custom_id"],
            "tn:team:phantom:2:4:42"
        );
        let modal = team_create_modal("phantom", 2, 4, 42);
        assert_eq!(modal.custom_id, "tn:create:phantom:2:4:42");
        let embed = signup_success_embed("solo", "inserted", "Phantom", 2, None);
        assert_eq!(embed["fields"][0]["value"], "Eingetragen");
        let embed = signup_success_embed("team", "updated", "Archon", 0, Some("Alpha"));
        assert_eq!(embed["fields"].as_array().expect("fields").len(), 4);
    }

    #[tokio::test]
    async fn legacy_fuenfteilige_tn_id_nutzt_owner_und_team_size_fallback() {
        let (_dir, handler) = test_handler().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tn:team:phantom:2:42".to_string(),
                user_id: 42,
                guild_id: 1,
                ..BridgeInteraction::default()
            })
            .await;

        let modal = reply.modal.expect("modal");
        assert_eq!(modal.custom_id, "tn:create:phantom:2:6:42");
        assert!(reply.content.is_none());
    }

    #[test]
    fn panel_embed_und_buttons() {
        let now = chrono::NaiveDate::from_ymd_opt(2026, 6, 10)
            .expect("datum")
            .and_hms_opt(12, 0, 0)
            .expect("uhrzeit");
        // Aktive Periode → Zeitraum-Felder + Anmeldungen + Voraussetzungen.
        let period = json!({
            "is_active": 1,
            "name": "Cup #1",
            "registration_start": "2026-06-09T00:00:00",
            "registration_end": "2026-06-11T23:59:59",
        });
        let summary = json!({ "signups_total": 5, "solo_count": 2, "team_count": 3 });
        let embed = panel_embed(Some(&period), Some(&summary), now);
        assert_eq!(embed["title"], "🏆 Deadlock Turnier-Anmeldung");
        let fields = embed["fields"].as_array().expect("fields");
        // Zeitraum, Status, Start, Ende, Anmeldungen, Voraussetzungen = 6.
        assert_eq!(fields.len(), 6);
        assert!(embed.get("description").is_none());

        // Ohne aktive Periode → Beschreibung statt Zeitraum-Feldern.
        let embed = panel_embed(None, None, now);
        assert_eq!(
            embed["description"].as_str().expect("desc"),
            "Aktuell ist **kein Anmeldezeitraum** aktiv."
        );

        // Buttons tragen die Original-custom_ids.
        let comps = panel_components();
        let ids: Vec<&str> = comps[0]["components"]
            .as_array()
            .expect("buttons")
            .iter()
            .map(|b| b["custom_id"].as_str().expect("id"))
            .collect();
        assert_eq!(
            ids,
            vec![
                "turnier_panel_anmelden",
                "turnier_panel_abmelden",
                "turnier_panel_status"
            ]
        );
    }
}
