//! TempVoice-Interface-Panel — Port des Kerns von `cogs/tempvoice/interface.py`.
//!
//! Persistente Buttons im Interface-Kanal (custom_ids unverändert — alte
//! Panels funktionieren nach dem Cutover weiter): Region DE/EU, Owner-Claim,
//! Limit, Kick/Ban/Unban, Quick-Templates, Presets, Rang-Präferenz, Rename.
//!
//! Bewusste Lücken (Backend folgt mit den Restmodulen): `tv_tag_filter`,
//! `tv_lurker`, `tv_mode_switch_*` (Router-Lanes), `tv_minrank`/
//! `tv_subrank_perm` (Min-Rang-Caps via Rank-Manager-Kopplung).

use std::sync::Arc;

use dl_discord::interactions::{ModalField, ModalSpec};
use dl_discord::{
    BridgeInteraction, BridgeReply, ChannelSender, Dispatcher, InteractionHandler,
    InteractionRouter,
};
use serde_json::{json, Map, Value};

use super::engine::{TempVoiceEngine, VERIFIED_ROLE_ID};
use super::store::{InterfaceRecord, PresetRecord};

const NOT_IN_LANE: &str = "Du musst dafür in einer TempVoice-Lane sein.";
const NOT_OWNER: &str = "Nur der Lane-Owner kann das.";
pub const MIN_RANK_VERIFY_REQUIRED: &str = "Platzhalter";
pub const MIN_RANK_BLOCKED_REPLY: &str = "Platzhalter";
pub const RANK_PREF_UNKNOWN_LABEL: &str = "Platzhalter";
const RANKED_CATEGORY_ID: u64 = 1412804540994162789;
const GLOBAL_PANEL_TITLE: &str = "🚧 Sprachkanal verwalten";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TempVoicePanelMessage {
    pub message_id: u64,
    pub has_embeds: bool,
    pub has_components: bool,
    pub embed_titles: Vec<String>,
}

#[async_trait::async_trait]
pub trait TempVoiceInterfacePort: Send + Sync {
    async fn post_rich(&self, channel_id: u64, body: Map<String, Value>) -> Result<u64, String>;
    async fn edit_rich(
        &self,
        channel_id: u64,
        message_id: u64,
        body: Map<String, Value>,
    ) -> Result<(), String>;
    async fn recent_bot_messages(
        &self,
        channel_id: u64,
        limit: u8,
    ) -> Result<Vec<TempVoicePanelMessage>, String>;
    async fn member_can_manage_guild(&self, guild_id: u64, user_id: u64) -> bool;
}

pub struct TempVoiceInterface {
    engine: Arc<TempVoiceEngine>,
    port: Arc<dyn TempVoiceInterfacePort>,
}

impl TempVoiceInterface {
    pub fn new(engine: Arc<TempVoiceEngine>, port: Arc<dyn TempVoiceInterfacePort>) -> Arc<Self> {
        Arc::new(Self { engine, port })
    }

    pub async fn ensure_interface_message(
        &self,
        guild_id: u64,
        channel_id: u64,
        category_id: Option<u64>,
    ) -> Result<u64, String> {
        self.engine
            .store
            .ensure_interface_schema()
            .await
            .map_err(|err| err.to_string())?;
        let body = global_panel_body(category_id);
        if let Some(row) = self.existing_global_record(guild_id, channel_id).await {
            match self
                .port
                .edit_rich(channel_id, row.message_id, body.clone())
                .await
            {
                Ok(()) => {
                    self.record_interface_message(InterfaceRecord {
                        guild_id,
                        channel_id,
                        message_id: row.message_id,
                        category_id,
                        lane_id: None,
                    })
                    .await?;
                    return Ok(row.message_id);
                }
                Err(err) if looks_missing_message(&err) => {
                    self.remove_interface_message(guild_id, row.message_id)
                        .await;
                }
                Err(err) => return Err(err),
            }
        }

        if let Some(message_id) = self.find_existing_global_panel(channel_id).await? {
            self.port
                .edit_rich(channel_id, message_id, body.clone())
                .await?;
            self.record_interface_message(InterfaceRecord {
                guild_id,
                channel_id,
                message_id,
                category_id,
                lane_id: None,
            })
            .await?;
            return Ok(message_id);
        }

        let message_id = self.port.post_rich(channel_id, body).await?;
        self.record_interface_message(InterfaceRecord {
            guild_id,
            channel_id,
            message_id,
            category_id,
            lane_id: None,
        })
        .await?;
        Ok(message_id)
    }

    async fn existing_global_record(
        &self,
        guild_id: u64,
        channel_id: u64,
    ) -> Option<InterfaceRecord> {
        self.engine
            .store
            .global_interface_messages()
            .await
            .ok()?
            .into_iter()
            .find(|row| row.guild_id == guild_id && row.channel_id == channel_id)
    }

    async fn find_existing_global_panel(&self, channel_id: u64) -> Result<Option<u64>, String> {
        let messages = self.port.recent_bot_messages(channel_id, 15).await?;
        Ok(find_existing_tempvoice_global_panel(&messages))
    }

    async fn record_interface_message(&self, record: InterfaceRecord) -> Result<(), String> {
        self.engine
            .store
            .record_interface_message(record)
            .await
            .map_err(|err| err.to_string())
    }

    async fn remove_interface_message(&self, guild_id: u64, message_id: u64) {
        if let Err(err) = self
            .engine
            .store
            .remove_interface_record(guild_id, message_id)
            .await
        {
            tracing::debug!(%err, guild_id, message_id, "TempVoice-Interface: verwaister Panel-Eintrag konnte nicht entfernt werden");
        }
    }

    pub async fn refresh_all_interfaces(&self) {
        if let Err(err) = self.engine.store.ensure_interface_schema().await {
            tracing::warn!(%err, "TempVoice-Interface: Schema-Anlage fehlgeschlagen");
            return;
        }
        self.engine.rehydrate().await;
        self.refresh_global_interface_messages().await;
        self.rehydrate_lane_interfaces().await;
    }

    async fn refresh_global_interface_messages(&self) {
        let rows = match self.engine.store.global_interface_messages().await {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "TempVoice-Interface: globale Panels konnten nicht gelesen werden");
                return;
            }
        };
        for row in rows {
            let body = global_panel_body(row.category_id);
            if let Err(err) = self
                .port
                .edit_rich(row.channel_id, row.message_id, body)
                .await
            {
                if looks_missing_message(&err) {
                    self.remove_interface_message(row.guild_id, row.message_id)
                        .await;
                    continue;
                }
                tracing::debug!(
                    %err,
                    message_id = row.message_id,
                    "TempVoice-Interface: globales Panel konnte nicht editiert werden"
                );
            }
        }
    }

    async fn rehydrate_lane_interfaces(&self) {
        let rows = match self.engine.store.lane_interface_messages().await {
            Ok(rows) => rows,
            Err(err) => {
                tracing::warn!(%err, "TempVoice-Interface: Lane-Panels konnten nicht gelesen werden");
                return;
            }
        };
        for row in rows {
            let Some(lane_id) = row.lane_id else {
                continue;
            };
            let Some((lane_name, category_id)) = self.engine.lane_snapshot(lane_id).await else {
                if let Err(err) = self
                    .engine
                    .store
                    .remove_lane_interface_record(lane_id)
                    .await
                {
                    tracing::debug!(%err, lane_id, "TempVoice-Interface: verwaister Lane-Panel-Eintrag konnte nicht entfernt werden");
                }
                continue;
            };
            let owner_id = self.engine.lane_owner(lane_id).await;
            let category_id = (category_id > 0).then_some(category_id).or(row.category_id);
            let body = lane_panel_body(&lane_name, owner_id, category_id);
            if let Err(err) = self
                .port
                .edit_rich(row.channel_id, row.message_id, body)
                .await
            {
                if looks_missing_message(&err) {
                    if let Err(remove_err) = self
                        .engine
                        .store
                        .remove_lane_interface_record(lane_id)
                        .await
                    {
                        tracing::debug!(%remove_err, lane_id, "TempVoice-Interface: verwaister Lane-Panel-Eintrag konnte nicht entfernt werden");
                    }
                    continue;
                }
                tracing::debug!(
                    %err,
                    lane_id,
                    message_id = row.message_id,
                    "TempVoice-Interface: Lane-Panel konnte nicht editiert werden"
                );
            }
        }
    }
}

fn global_panel_body(category_id: Option<u64>) -> Map<String, Value> {
    panel_body(global_panel_embed(), main_view_components(category_id))
}

fn lane_panel_body(
    lane_name: &str,
    owner_id: Option<u64>,
    category_id: Option<u64>,
) -> Map<String, Value> {
    panel_body(
        lane_panel_embed(lane_name, owner_id),
        main_view_components(category_id),
    )
}

fn panel_body(embed: Value, components: Value) -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("embeds".to_string(), json!([embed]));
    body.insert("components".to_string(), components);
    body
}

fn global_panel_embed() -> Value {
    json!({
        "title": "🚧 Sprachkanal verwalten",
        "description": concat!(
            "So funktioniert Temp Voice:\n",
            "• Betritt einen **(+) Sprachkanal**, deine eigene Lane wird automatisch erstellt.\n",
            "• Passe deine Lane hier an; die Buttons wirken sofort, wenn du Owner bist.\n\n",
            "Was ihr hier machen könnt:\n",
            "• **Kick:** Jemand AFK oder stört? Entferne die Person, wenn Reden nicht reicht.\n",
            "• **Ban:** Sperre jemanden dauerhaft aus deinem Kanal, solange du Owner bist.\n",
            "• **Unban:** Hebe die Sperre wieder auf.\n",
            "• **Duo/Trio Call:** Stelle 2er/3er-Runden ein; andere können (fast) nicht beitreten.\n",
            "• **Normale Lane:** Setzt die Berechtigungen wieder auf offen.\n",
            "• **Lurker-Rolle:** Für Zuhörer; schafft einen zusätzlichen Platz für Mitspieler.\n",
            "• **Limit & Sprache:** Setze Teilnehmerlimit (0–99) und Deutsch/Offen-Filter.\n",
            "• **Owner Claim & Mindest-Rang:** Übernimm die Lane und lege optional einen Mindest-Rang fest."
        ),
        "color": 0x2ECC71,
        "footer": {"text": "Deutsche Deadlock Community • TempVoice"},
    })
}

fn lane_panel_embed(lane_name: &str, owner_id: Option<u64>) -> Value {
    let owner_display = owner_id
        .map(|id| format!("<@{id}>"))
        .unwrap_or_else(|| "Unbekannt".to_string());
    json!({
        "title": format!("🎙️ TempVoice – {lane_name}"),
        "description": format!(
            concat!(
                "**Owner:** {}\n\n",
                "**Steuerung** *(nur wenn du in dieser Lane bist)*\n",
                "🇩🇪 / 🌍 – Sprachfilter: nur DE oder alle EU\n",
                "👑 – Owner übernehmen (wenn der Owner die Lane verlassen hat)\n",
                "🔢 – Spielerlimit setzen (0 = kein Limit)\n",
                "🦵 Kick · 🚫 Ban · ✅ Unban – Mitglieder verwalten\n",
                "👥 Duo / Trio · 🔄 Reset – Lane-Größe schnell anpassen\n",
                "👻 Lurker – stumm beitreten ohne Limit-Slot zu belegen\n\n",
                "**Mindest-Rang** *(nur Ranked)*\n",
                "① Wähle den **Haupt-Rang** (z. B. Archon)\n",
                "② Wähle dann den **Sub-Rang** (1–6)\n",
                "→ Der Rang wird erst gesetzt, wenn **beide** gewählt sind."
            ),
            owner_display
        ),
        "color": 0x2ECC71,
        "footer": {"text": "Deutsche Deadlock Community • TempVoice"},
    })
}

fn main_view_components(category_id: Option<u64>) -> Value {
    let include_ranked = category_id == Some(RANKED_CATEGORY_ID);
    if include_ranked {
        json!([
            action_row(vec![
                button("🇩🇪 DE", 1, "tv_region_de"),
                button("🇪🇺 EU", 2, "tv_region_e"),
                button("👑 Owner übernehmen", 3, "tv_owner_claim"),
                button("🎚️ Limit setzen", 2, "tv_limit_btn"),
                button("🎯 Mein Rang", 2, "tv_rank_pref"),
            ]),
            action_row(vec![
                button("👢 Kick", 4, "tv_kick"),
                button("🚫 Ban", 4, "tv_ban"),
                button("♻️ Unban", 1, "tv_unban"),
                button("👻 Lurker", 2, "tv_lurker"),
                button("🛡️ Tag-Filter", 2, "tv_tag_filter"),
            ]),
            action_row(vec![min_rank_select()]),
            action_row(vec![
                button("Normale Lane", 2, "tv_tpl_reset"),
                button("Duo Call (2)", 1, "tv_tpl_duo"),
                button("Trio Call (3)", 1, "tv_tpl_trio"),
                button("💾 Preset speichern", 3, "tv_preset_save"),
                button("📂 Preset laden", 1, "tv_preset_load"),
            ]),
            action_row(vec![subrank_select()]),
        ])
    } else {
        json!([
            action_row(vec![
                button("🇩🇪 DE", 1, "tv_region_de"),
                button("🇪🇺 EU", 2, "tv_region_e"),
                button("👑 Owner übernehmen", 3, "tv_owner_claim"),
                button("🎚️ Limit setzen", 2, "tv_limit_btn"),
                button("🎯 Mein Rang", 2, "tv_rank_pref"),
            ]),
            action_row(vec![
                button("👢 Kick", 4, "tv_kick"),
                button("🚫 Ban", 4, "tv_ban"),
                button("♻️ Unban", 1, "tv_unban"),
            ]),
            action_row(vec![button("🛡️ Tag-Filter", 2, "tv_tag_filter")]),
            action_row(vec![
                button("Normale Lane", 2, "tv_tpl_reset"),
                button("Duo Call (2)", 1, "tv_tpl_duo"),
                button("Trio Call (3)", 1, "tv_tpl_trio"),
                button("👻 Lurker", 2, "tv_lurker"),
            ]),
            action_row(vec![
                button("✏️ Umbenennen", 2, "tv_rename_btn"),
                button("🔄 Modus wechseln", 2, "tv_mode_switch_btn"),
            ]),
        ])
    }
}

fn action_row(components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": components,
    })
}

fn button(label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
}

fn min_rank_select() -> Value {
    let labels = [
        ("Initiate", "initiate"),
        ("Seeker", "seeker"),
        ("Alchemist", "alchemist"),
        ("Arcanist", "arcanist"),
        ("Ritualist", "ritualist"),
        ("Emissary", "emissary"),
        ("Archon", "archon"),
        ("Oracle", "oracle"),
        ("Phantom", "phantom"),
        ("Ascendant", "ascendant"),
        ("Eternus", "eternus"),
    ];
    json!({
        "type": 3,
        "custom_id": "tv_minrank",
        "placeholder": "① Haupt-Rang wählen →",
        "min_values": 1,
        "max_values": 1,
        "options": labels
            .iter()
            .map(|(label, value)| json!({"label": label, "value": value}))
            .collect::<Vec<_>>(),
    })
}

fn find_existing_tempvoice_global_panel(messages: &[TempVoicePanelMessage]) -> Option<u64> {
    messages
        .iter()
        .find(|message| {
            message.has_embeds
                && message.has_components
                && message
                    .embed_titles
                    .iter()
                    .any(|title| title == GLOBAL_PANEL_TITLE)
        })
        .map(|message| message.message_id)
}

fn looks_missing_message(err: &str) -> bool {
    err.contains("10008")
        || err.contains("Unknown Message")
        || err.contains("NotFound")
        || err.contains("404")
}

fn subrank_select() -> Value {
    json!({
        "type": 3,
        "custom_id": "tv_subrank_perm",
        "placeholder": "② Sub-Rang wählen (1–6)",
        "min_values": 1,
        "max_values": 1,
        "options": (1..=6)
            .map(|n| json!({"label": format!("Sub-Rang {n}"), "value": n.to_string()}))
            .collect::<Vec<_>>(),
    })
}

struct PanelHandler {
    engine: Arc<TempVoiceEngine>,
    /// Zwischenspeicher für den ① gewählten Haupt-Rang, bis ② der Sub-Rang
    /// kommt (Port von Pythons modul-globalem `_pending_main_rank`). Per-Prozess-
    /// RAM, geht — wie im Original — bei Neustart verloren.
    pending_main_rank: tokio::sync::Mutex<std::collections::HashMap<u64, String>>,
}

impl PanelHandler {
    /// Lane des Users (None wenn nicht in einer verwalteten Lane).
    async fn lane_of(&self, interaction: &BridgeInteraction) -> Option<u64> {
        let channel_id = self
            .engine
            .port
            .member_voice_channel(interaction.guild_id, interaction.user_id)
            .await?;
        if self.engine.config.fixed_lane_ids.contains(&channel_id)
            || self.engine.config.staging_channels.contains(&channel_id)
        {
            return None;
        }
        Some(channel_id)
    }

    async fn owned_lane_of(&self, interaction: &BridgeInteraction) -> Result<u64, BridgeReply> {
        let Some(lane) = self.lane_of(interaction).await else {
            return Err(BridgeReply::ephemeral_text(NOT_IN_LANE));
        };
        if self.engine.lane_owner(lane).await != Some(interaction.user_id)
            && !interaction.author_can_manage_channels
        {
            return Err(BridgeReply::ephemeral_text(NOT_OWNER));
        }
        Ok(lane)
    }

    async fn lane_owner_id(&self, lane: u64, actor_id: u64) -> u64 {
        self.engine.lane_owner_or_actor(lane, actor_id).await
    }

    /// Select mit den anderen Membern der Lane (für Kick/Ban).
    async fn member_select(
        &self,
        interaction: &BridgeInteraction,
        lane: u64,
        custom_id: &str,
        prompt: &str,
    ) -> BridgeReply {
        let members = self
            .engine
            .port
            .channel_members(interaction.guild_id, lane)
            .await;
        let mut options = Vec::new();
        for user_id in members {
            if user_id == interaction.user_id {
                continue;
            }
            let label = self
                .engine
                .port
                .member_display_name(interaction.guild_id, user_id)
                .await
                .unwrap_or_else(|| format!("User {user_id}"));
            options.push(json!({
                "label": label.chars().take(100).collect::<String>(),
                "value": user_id.to_string(),
            }));
            if options.len() >= 25 {
                break;
            }
        }
        if options.is_empty() {
            return BridgeReply::ephemeral_text("Niemand sonst in der Lane.");
        }
        BridgeReply {
            content: Some(prompt.to_string()),
            components: Some(json!([{ "type": 1, "components": [{
                "type": 3, "custom_id": custom_id, "options": options,
                "min_values": 1, "max_values": 1,
            }]}])),
            ephemeral: true,
            ..BridgeReply::default()
        }
    }

    fn selected_user(interaction: &BridgeInteraction) -> Option<u64> {
        interaction.values.first().and_then(|v| v.parse().ok())
    }
}

#[async_trait::async_trait]
impl InteractionHandler for PanelHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let engine = &self.engine;
        match interaction.custom_id.as_str() {
            // ── Region ────────────────────────────────────────────────
            "tv_region_de" | "tv_region_e" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let region = if interaction.custom_id == "tv_region_de" {
                    "DE"
                } else {
                    "EU"
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                engine.set_region(lane, owner_id, region).await;
                BridgeReply::ephemeral_text(if region == "DE" {
                    "Region gesetzt: **DE** — English-Only-Accounts können nicht mehr verbinden."
                } else {
                    "Region gesetzt: **EU** — Sprachfilter entfernt."
                })
            }

            // ── Owner-Claim ───────────────────────────────────────────
            "tv_owner_claim" => {
                let Some(lane) = self.lane_of(&interaction).await else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                };
                match engine
                    .evaluate_claim(interaction.guild_id, lane, interaction.user_id)
                    .await
                {
                    Ok(()) => {
                        engine
                            .claim_owner(interaction.guild_id, lane, interaction.user_id)
                            .await;
                        BridgeReply::ephemeral_text("Du bist jetzt Owner dieser Lane.")
                    }
                    Err(message) => BridgeReply::ephemeral_text(message),
                }
            }

            // ── Limit ─────────────────────────────────────────────────
            "tv_limit_btn" => {
                if let Err(reply) = self.owned_lane_of(&interaction).await {
                    return reply;
                }
                BridgeReply {
                    modal: Some(ModalSpec {
                        custom_id: "tv_limit_modal".to_string(),
                        title: "Limit setzen".to_string(),
                        fields: vec![ModalField {
                            custom_id: "limit".to_string(),
                            label: "Neues Limit (0 = unbegrenzt)".to_string(),
                            placeholder: "z. B. 6".to_string(),
                            required: true,
                            min_length: 1,
                            max_length: 2,
                            paragraph: false,
                        }],
                    }),
                    ..BridgeReply::default()
                }
            }
            "tv_limit_modal" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(requested) = interaction
                    .options
                    .get("limit")
                    .and_then(|v| v.as_str())
                    .and_then(|v| v.trim().parse().ok())
                else {
                    return BridgeReply::ephemeral_text("Bitte eine Zahl eingeben.");
                };
                match engine.set_limit(lane, requested).await {
                    Ok(effective) => BridgeReply::ephemeral_text(format!(
                        "Limit gesetzt: **{effective}**{}",
                        if effective != requested {
                            " (auf das Lane-Maximum gekappt)"
                        } else {
                            ""
                        }
                    )),
                    Err(err) => BridgeReply::ephemeral_text(format!("Limit fehlgeschlagen: {err}")),
                }
            }

            // ── Kick / Ban / Unban ────────────────────────────────────
            "tv_kick" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                self.member_select(&interaction, lane, "tv_kick_sel", "Wen kicken?")
                    .await
            }
            "tv_kick_sel" => {
                if let Err(reply) = self.owned_lane_of(&interaction).await {
                    return reply;
                }
                let Some(target) = Self::selected_user(&interaction) else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                match engine
                    .port
                    .disconnect_member(interaction.guild_id, target, "TempVoice: Owner-Kick")
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text(format!("<@{target}> gekickt.")),
                    Err(err) => BridgeReply::ephemeral_text(format!("Kick fehlgeschlagen: {err}")),
                }
            }
            "tv_ban" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                self.member_select(&interaction, lane, "tv_ban_sel", "Wen bannen?")
                    .await
            }
            "tv_ban_sel" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(target) = Self::selected_user(&interaction) else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                let _ = engine.store.add_ban(owner_id, target).await;
                let _ = engine
                    .port
                    .set_member_connect(lane, target, Some(false))
                    .await;
                let _ = engine
                    .port
                    .disconnect_member(interaction.guild_id, target, "TempVoice: Owner-Bann")
                    .await;
                BridgeReply::ephemeral_text(format!(
                    "<@{target}> gebannt — gilt für alle deine Lanes, bis du den Bann aufhebst."
                ))
            }
            "tv_unban" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                let bans = engine.store.list_bans(owner_id).await.unwrap_or_default();
                if bans.is_empty() {
                    return BridgeReply::ephemeral_text("Du hast niemanden gebannt.");
                }
                let mut options = Vec::new();
                for user_id in bans.iter().take(25) {
                    let label = self
                        .engine
                        .port
                        .member_display_name(interaction.guild_id, *user_id)
                        .await
                        .unwrap_or_else(|| format!("User {user_id}"));
                    options.push(json!({ "label": label, "value": user_id.to_string() }));
                }
                BridgeReply {
                    content: Some("Wen entbannen?".to_string()),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 3, "custom_id": "tv_unban_sel", "options": options,
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "tv_unban_sel" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(target) = Self::selected_user(&interaction) else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                let _ = engine.store.remove_ban(owner_id, target).await;
                let _ = engine.port.set_member_connect(lane, target, None).await;
                BridgeReply::ephemeral_text(format!("<@{target}> entbannt."))
            }

            // ── Quick-Templates ───────────────────────────────────────
            "tv_tpl_duo" | "tv_tpl_trio" | "tv_tpl_reset" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let (base_name, limit) = match interaction.custom_id.as_str() {
                    "tv_tpl_duo" => ("Duo Call".to_string(), 2),
                    "tv_tpl_trio" => ("Trio Call".to_string(), 3),
                    _ => {
                        let base = engine
                            .lane_snapshot(lane)
                            .await
                            .map(|(base, _)| base)
                            .filter(|base| base.to_lowercase().starts_with("lane "))
                            .unwrap_or_else(|| "Lane".to_string());
                        (base, engine.default_limit_for_lane(lane).await)
                    }
                };
                match engine
                    .set_lane_template(interaction.guild_id, lane, &base_name, limit)
                    .await
                {
                    Ok(effective) => BridgeReply::ephemeral_text(format!(
                        "Lane umgestellt: Limit **{effective}**."
                    )),
                    Err(err) => BridgeReply::ephemeral_text(format!("Fehlgeschlagen: {err}")),
                }
            }

            // ── Presets ───────────────────────────────────────────────
            "tv_preset_save" => {
                if let Err(reply) = self.owned_lane_of(&interaction).await {
                    return reply;
                }
                BridgeReply {
                    modal: Some(ModalSpec {
                        custom_id: "tv_preset_save_modal".to_string(),
                        title: "Preset speichern".to_string(),
                        fields: vec![ModalField {
                            custom_id: "name".to_string(),
                            label: "Preset-Name".to_string(),
                            placeholder: "z. B. tryhard".to_string(),
                            required: true,
                            min_length: 1,
                            max_length: 32,
                            paragraph: false,
                        }],
                    }),
                    ..BridgeReply::default()
                }
            }
            "tv_preset_save_modal" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(name) = interaction
                    .options
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                else {
                    return BridgeReply::ephemeral_text("Bitte einen Namen eingeben.");
                };
                let info = engine.lane_snapshot(lane).await;
                let Some((_, category_id)) = info else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                };
                let Some((base_name, _, min_rank)) = engine.lane_preset_snapshot(lane).await else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                let limit = match engine
                    .port
                    .channel_user_limit(interaction.guild_id, lane)
                    .await
                {
                    Some(limit) => limit,
                    None => engine.default_limit_for_lane(lane).await,
                };
                let region = engine
                    .store
                    .region_pref(owner_id)
                    .await
                    .unwrap_or_else(|_| "EU".to_string());
                let result = engine
                    .store
                    .save_preset(PresetRecord {
                        user_id: owner_id,
                        category_id,
                        name: name.clone(),
                        base_name,
                        limit,
                        min_rank,
                        region,
                    })
                    .await;
                match result {
                    Ok(()) => {
                        BridgeReply::ephemeral_text(format!("Preset **{name}** gespeichert."))
                    }
                    Err(err) => {
                        BridgeReply::ephemeral_text(format!("Speichern fehlgeschlagen: {err}"))
                    }
                }
            }
            "tv_preset_load" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some((_, category_id)) = engine.lane_snapshot(lane).await else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                let presets = engine
                    .store
                    .list_presets(owner_id, category_id)
                    .await
                    .unwrap_or_default();
                if presets.is_empty() {
                    return BridgeReply::ephemeral_text("Keine Presets in dieser Kategorie.");
                }
                let options: Vec<_> = presets
                    .iter()
                    .take(25)
                    .map(|name| json!({ "label": name, "value": name }))
                    .collect();
                BridgeReply {
                    content: Some("Welches Preset laden?".to_string()),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 3, "custom_id": "tv_preset_pick", "options": options,
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "tv_preset_pick" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(name) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let Some((_, category_id)) = engine.lane_snapshot(lane).await else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                };
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                let Some((base, limit, min_rank, region)) = engine
                    .store
                    .get_preset(owner_id, category_id, name)
                    .await
                    .ok()
                    .flatten()
                else {
                    return BridgeReply::ephemeral_text("Preset nicht gefunden.");
                };
                let _ = engine
                    .set_lane_template(interaction.guild_id, lane, &base, limit)
                    .await;
                if !engine.is_min_rank_blocked(lane).await && min_rank != "unknown" {
                    let _ = engine
                        .set_min_rank(interaction.guild_id, lane, &min_rank)
                        .await;
                }
                engine.set_region(lane, owner_id, &region).await;
                BridgeReply::ephemeral_text(format!(
                    "Preset **{name}** angewendet (Limit {limit}, Region {region})."
                ))
            }

            // ── Rang-Präferenz ────────────────────────────────────────
            "tv_rank_pref" => {
                let current = engine
                    .store
                    .rank_pref(interaction.user_id)
                    .await
                    .map(|(rank, _)| rank)
                    .unwrap_or_else(|_| "unknown".to_string());
                let options = std::iter::once(json!({
                    "label": RANK_PREF_UNKNOWN_LABEL,
                    "value": "unknown",
                    "default": current == "unknown",
                }))
                .chain(super::logic::RANK_ORDER.iter().skip(1).map(|rank| {
                    json!({
                        "label": super::logic::capitalize(rank),
                        "value": rank,
                        "default": current == *rank,
                    })
                }))
                .collect::<Vec<_>>();
                BridgeReply {
                    content: Some("Welchen Rang soll deine Chill-Lane tragen?".to_string()),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 3, "custom_id": "tv_rank_pref_sel",
                        "options": options,
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "tv_rank_pref_sel" => {
                let Some(rank) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let _ = engine
                    .store
                    .set_rank_pref(interaction.user_id, rank, 0)
                    .await;
                BridgeReply::ephemeral_text(format!(
                    "Rang-Präferenz gespeichert: **{}** — gilt für deine nächste Chill-Lane.",
                    super::logic::capitalize(rank)
                ))
            }

            // ── Rename ────────────────────────────────────────────────
            "tv_rename_btn" => {
                if let Err(reply) = self.owned_lane_of(&interaction).await {
                    return reply;
                }
                BridgeReply {
                    modal: Some(ModalSpec {
                        custom_id: "tv_rename_modal".to_string(),
                        title: "Lane umbenennen".to_string(),
                        fields: vec![ModalField {
                            custom_id: "name".to_string(),
                            label: "Neuer Name".to_string(),
                            placeholder: "z. B. Lane 1".to_string(),
                            required: true,
                            min_length: 1,
                            max_length: 90,
                            paragraph: false,
                        }],
                    }),
                    ..BridgeReply::default()
                }
            }
            "tv_rename_modal" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(name) = interaction
                    .options
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                else {
                    return BridgeReply::ephemeral_text("Bitte einen Namen eingeben.");
                };
                match engine
                    .port
                    .rename_channel(lane, &name, "TempVoice: Owner-Rename")
                    .await
                {
                    Ok(()) => {
                        engine.set_base_name(lane, &name).await;
                        BridgeReply::ephemeral_text(format!("Lane heißt jetzt **{name}**."))
                    }
                    Err(err) => {
                        BridgeReply::ephemeral_text(format!("Rename fehlgeschlagen: {err}"))
                    }
                }
            }

            // ── Lurker ────────────────────────────────────────────────
            // Wie LurkerButton: jedes Lane-Mitglied toggelt seinen EIGENEN
            // Lurker-Status (kein Owner-Check — Original-Verhalten).
            "tv_lurker" => {
                let Some(lane) = self.lane_of(&interaction).await else {
                    return BridgeReply::ephemeral_text("Du musst in einer Lane sein.");
                };
                let (_, msg) = engine
                    .toggle_lurker(interaction.guild_id, lane, interaction.user_id)
                    .await;
                BridgeReply::ephemeral_text(msg)
            }

            // ── Modus wechseln ────────────────────────────────────────
            "tv_mode_switch_btn" => {
                if self.lane_of(&interaction).await.is_none() {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                }
                BridgeReply {
                    content: Some("Wähle den neuen Modus:".to_string()),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 3, "custom_id": "tv_mode_switch_select",
                        "placeholder": "Neuen Modus wählen…",
                        "options": [
                            { "label": "Casual", "value": "casual" },
                            { "label": "Ranked", "value": "ranked" },
                            { "label": "Street Brawl", "value": "street_brawl" },
                            { "label": "Off Topic", "value": "off_topic" },
                        ],
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "tv_mode_switch_select" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(mode) = interaction.values.first().cloned() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                match self
                    .engine
                    .switch_lane_mode(interaction.guild_id, lane, interaction.user_id, &mode)
                    .await
                {
                    None => {
                        let label = match mode.as_str() {
                            "casual" => "Casual",
                            "ranked" => "Ranked",
                            "street_brawl" => "Street Brawl",
                            "off_topic" => "Off Topic",
                            other => other,
                        };
                        BridgeReply::ephemeral_text(format!("Lane auf **{label}** umgestellt."))
                    }
                    Some(err) => BridgeReply::ephemeral_text(err),
                }
            }

            // ── Mindest-Rang (① Haupt-Rang → ② Sub-Rang) ──────────────
            // Wie MinRankSelect/SubRankSelectPermanent: kein Owner-Check
            // (jedes Lane-Mitglied), aber nur in Comp/Ranked-Kategorien und
            // nicht über dem eigenen Rang. Der Haupt-Rang wird zwischen den
            // beiden Selects in `pending_main_rank` gehalten (Port von Pythons
            // modul-globalem `_pending_main_rank`).
            "tv_minrank" => {
                let Some(lane) = self.lane_of(&interaction).await else {
                    return BridgeReply::ephemeral_text("Tritt zuerst deiner Lane bei.");
                };
                let in_minrank = match engine.lane_snapshot(lane).await {
                    Some((_, category_id)) => {
                        engine.config.minrank_categories.contains(&category_id)
                    }
                    None => false,
                };
                if !in_minrank {
                    return BridgeReply::ephemeral_text("Mindest-Rang ist hier deaktiviert.");
                }
                if engine.is_min_rank_blocked(lane).await {
                    return BridgeReply::ephemeral_text(MIN_RANK_BLOCKED_REPLY);
                }
                let role_ids = engine
                    .port
                    .member_role_ids(interaction.guild_id, interaction.user_id)
                    .await;
                if !role_ids.contains(&VERIFIED_ROLE_ID) {
                    return BridgeReply::ephemeral_text(MIN_RANK_VERIFY_REQUIRED);
                }
                let Some(choice) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let choice = choice.to_lowercase();
                let roles = engine
                    .port
                    .member_role_names(interaction.guild_id, interaction.user_id)
                    .await;
                let member_idx = super::logic::member_rank_index(&roles);
                if super::logic::rank_index(&choice) > member_idx {
                    let own = super::logic::RANK_ORDER
                        .get(member_idx)
                        .copied()
                        .unwrap_or("unknown");
                    return BridgeReply::ephemeral_text(format!(
                        "Du kannst keinen Mindest-Rang über deinem eigenen setzen. Dein Rang: {}.",
                        super::logic::capitalize(own)
                    ));
                }
                self.pending_main_rank
                    .lock()
                    .await
                    .insert(lane, choice.clone());
                BridgeReply::ephemeral_text(format!(
                    "Haupt-Rang **{}** gespeichert – jetzt **② Sub-Rang (1–6)** auswählen.",
                    super::logic::capitalize(&choice)
                ))
            }
            "tv_subrank_perm" | "tv_subrank" => {
                let Some(lane) = self.lane_of(&interaction).await else {
                    return BridgeReply::ephemeral_text("Tritt zuerst deiner Lane bei.");
                };
                let main_rank = self.pending_main_rank.lock().await.get(&lane).cloned();
                let Some(main_rank) = main_rank else {
                    return BridgeReply::ephemeral_text(
                        "Bitte zuerst den **① Haupt-Rang** auswählen.",
                    );
                };
                let Some(sub_raw) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let sub: i64 = sub_raw.trim().parse().unwrap_or(-1);
                let rank_label = if sub == 0 {
                    main_rank.clone()
                } else if (1..=6).contains(&sub) {
                    format!("{main_rank} {sub}")
                } else {
                    return BridgeReply::ephemeral_text("Ungültiger Sub-Rang.");
                };
                self.pending_main_rank.lock().await.remove(&lane);
                match self
                    .engine
                    .set_min_rank(interaction.guild_id, lane, &rank_label)
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text(format!(
                        "Mindest-Rang gesetzt auf: **{}**.",
                        super::logic::capitalize(&rank_label)
                    )),
                    Err(err) => BridgeReply::ephemeral_text(err),
                }
            }

            // ── Tag-Filter (Owner) ────────────────────────────────────
            // Port von TagFilterConfigView. Da der Router zustandslos ist,
            // persistiert JEDE Auswahl sofort (statt sammeln + Speichern-Knopf)
            // — Endzustand identisch.
            "tv_tag_filter" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let f = engine.store.lane_tag_filter(lane).await;
                BridgeReply {
                    content: Some(
                        "Tag-Filter konfigurieren — jede Auswahl wird sofort übernommen:"
                            .to_string(),
                    ),
                    components: Some(json!([
                        { "type": 1, "components": [{
                            "type": 3, "custom_id": "tv_tagf_age", "placeholder": "Mindest-Alter",
                            "options": [
                                { "label": "Aus", "value": "off", "default": f.min_age_tag.is_none() },
                                { "label": "25+", "value": "25+", "default": f.min_age_tag.as_deref() == Some("25+") },
                            ],
                            "min_values": 1, "max_values": 1,
                        }]},
                        { "type": 1, "components": [{
                            "type": 3, "custom_id": "tv_tagf_tone", "placeholder": "Tonfall",
                            "options": [
                                { "label": "Aus", "value": "off", "default": f.required_tone_tag.is_none() },
                                { "label": "Ragebaiter-Free", "value": "ragebaiter_free", "default": f.required_tone_tag.as_deref() == Some("ragebaiter_free") },
                            ],
                            "min_values": 1, "max_values": 1,
                        }]},
                        { "type": 1, "components": [{
                            "type": 3, "custom_id": "tv_tagf_rb", "placeholder": "Ragebaiter blockieren",
                            "options": [
                                { "label": "Aus", "value": "off", "default": !f.deny_ragebaiter },
                                { "label": "An", "value": "on", "default": f.deny_ragebaiter },
                            ],
                            "min_values": 1, "max_values": 1,
                        }]},
                    ])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "tv_tagf_age" | "tv_tagf_tone" | "tv_tagf_rb" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(value) = interaction.values.first().cloned() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let mut filter = engine.store.lane_tag_filter(lane).await;
                let confirm = match interaction.custom_id.as_str() {
                    "tv_tagf_age" => {
                        filter.min_age_tag = (value != "off").then(|| value.clone());
                        "Mindest-Alter aktualisiert."
                    }
                    "tv_tagf_tone" => {
                        filter.required_tone_tag = (value != "off").then(|| value.clone());
                        "Tonfall-Filter aktualisiert."
                    }
                    _ => {
                        filter.deny_ragebaiter = value == "on";
                        "Ragebaiter-Filter aktualisiert."
                    }
                };
                match engine
                    .save_tag_filter(interaction.guild_id, lane, filter)
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text(confirm),
                    Err(err) => {
                        BridgeReply::ephemeral_text(format!("Speichern fehlgeschlagen: {err}"))
                    }
                }
            }

            other => {
                tracing::warn!(custom_id = other, "TempVoice-Panel: unbekannte Komponente");
                BridgeReply::ephemeral_text("Unbekannte Aktion.")
            }
        }
    }
}

/// Alle Panel-custom_ids am Router registrieren.
pub fn register(router: &mut InteractionRouter, engine: Arc<TempVoiceEngine>) {
    let handler = Arc::new(PanelHandler {
        engine,
        pending_main_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    });
    for custom_id in [
        "tv_region_de",
        "tv_region_e",
        "tv_owner_claim",
        "tv_limit_btn",
        "tv_limit_modal",
        "tv_kick",
        "tv_kick_sel",
        "tv_ban",
        "tv_ban_sel",
        "tv_unban",
        "tv_unban_sel",
        "tv_tpl_duo",
        "tv_tpl_trio",
        "tv_tpl_reset",
        "tv_preset_save",
        "tv_preset_save_modal",
        "tv_preset_load",
        "tv_preset_pick",
        "tv_rank_pref",
        "tv_rank_pref_sel",
        "tv_rename_btn",
        "tv_rename_modal",
        "tv_tag_filter",
        "tv_tagf_age",
        "tv_tagf_tone",
        "tv_tagf_rb",
        "tv_lurker",
        "tv_mode_switch_btn",
        "tv_mode_switch_select",
        "tv_minrank",
        "tv_minrank_sel",
        "tv_subrank_perm",
        "tv_subrank",
    ] {
        router.on_custom_id(custom_id, handler.clone());
    }
}

fn is_tvpanel_command(content: &str) -> bool {
    let Some(root) = content.split_whitespace().next() else {
        return false;
    };
    matches!(
        root.to_ascii_lowercase().as_str(),
        "!tvpanel" | "!tempvoicepanel" | "!tvinterface"
    )
}

pub fn spawn_command(
    interface: Arc<TempVoiceInterface>,
    dispatcher: &Dispatcher,
    sender: Arc<dyn ChannelSender>,
) -> tokio::task::JoinHandle<()> {
    let mut messages = dispatcher.subscribe_messages();
    tokio::spawn(async move {
        loop {
            match messages.recv().await {
                Ok(event) => {
                    let Some(guild_id) = event.guild_id else {
                        continue;
                    };
                    if !is_tvpanel_command(event.content.trim()) {
                        continue;
                    }
                    if !interface
                        .port
                        .member_can_manage_guild(guild_id, event.author_id)
                        .await
                    {
                        continue;
                    }
                    let reply = match interface
                        .ensure_interface_message(guild_id, event.channel_id, None)
                        .await
                    {
                        Ok(_) => format!(
                            "✅ TempVoice Interface erstellt/aktualisiert in <#{}>.",
                            event.channel_id
                        ),
                        Err(err) => {
                            tracing::warn!(%err, "tvpanel command failed");
                            "❌ Konnte das Interface nicht erstellen (Fehler im Log).".to_string()
                        }
                    };
                    let _ = sender
                        .send_to_channel(event.channel_id, Some(&reply), &[])
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
    fn global_panel_body_enthaelt_python_embed_und_buttons() {
        let body = global_panel_body(None);
        let embeds = body
            .get("embeds")
            .and_then(serde_json::Value::as_array)
            .expect("embeds");
        assert_eq!(embeds[0]["title"], "🚧 Sprachkanal verwalten");
        let description = embeds[0]["description"].as_str().expect("description");
        assert!(description.contains("So funktioniert Temp Voice:\n"));
        assert!(description.contains(
            "• Betritt einen **(+) Sprachkanal**, deine eigene Lane wird automatisch erstellt."
        ));
        assert!(description.contains("• **Owner Claim & Mindest-Rang:** Übernimm die Lane und lege optional einen Mindest-Rang fest."));
        assert_eq!(
            embeds[0]["footer"]["text"],
            "Deutsche Deadlock Community • TempVoice"
        );

        let components = body
            .get("components")
            .and_then(serde_json::Value::as_array)
            .expect("components");
        let custom_ids: Vec<&str> = components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        let labels: Vec<&str> = components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["label"].as_str())
            .collect();
        assert!(custom_ids.contains(&"tv_region_de"));
        assert!(custom_ids.contains(&"tv_owner_claim"));
        assert!(custom_ids.contains(&"tv_limit_btn"));
        assert!(custom_ids.contains(&"tv_mode_switch_btn"));
        assert!(!custom_ids.contains(&"tv_minrank"));
        assert!(labels.contains(&"👢 Kick"));
        assert!(labels.contains(&"🛡️ Tag-Filter"));

        let ranked = global_panel_body(Some(1412804540994162789));
        let ranked_components = ranked
            .get("components")
            .and_then(serde_json::Value::as_array)
            .expect("ranked components");
        let ranked_ids: Vec<&str> = ranked_components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert!(ranked_ids.contains(&"tv_minrank"));
        assert!(ranked_ids.contains(&"tv_preset_save"));
        let min_rank_options: Vec<&str> = ranked_components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .find(|component| component["custom_id"] == "tv_minrank")
            .and_then(|component| component["options"].as_array())
            .expect("minrank options")
            .iter()
            .filter_map(|option| option["value"].as_str())
            .collect();
        assert_eq!(min_rank_options.first().copied(), Some("initiate"));
    }

    #[test]
    fn neue_sichtbare_texte_bleiben_platzhalter() {
        assert_eq!(MIN_RANK_VERIFY_REQUIRED, "Platzhalter");
        assert_eq!(MIN_RANK_BLOCKED_REPLY, "Platzhalter");
        assert_eq!(RANK_PREF_UNKNOWN_LABEL, "Platzhalter");
        assert_eq!(
            crate::tempvoice::engine::MIN_RANK_DISABLED_REPLY,
            "Platzhalter"
        );
    }

    #[test]
    fn lane_panel_body_enthaelt_python_lane_embed() {
        let body = lane_panel_body("Lane 1", Some(42), Some(1412804540994162789));
        let embed = &body
            .get("embeds")
            .and_then(serde_json::Value::as_array)
            .expect("embeds")[0];
        assert_eq!(embed["title"], "🎙️ TempVoice – Lane 1");
        let description = embed["description"].as_str().expect("description");
        assert!(description.contains("**Owner:** <@42>"));
        assert!(description.contains("👻 Lurker – stumm beitreten ohne Limit-Slot zu belegen"));
        assert!(description.contains("→ Der Rang wird erst gesetzt, wenn **beide** gewählt sind."));
    }

    #[test]
    fn tvpanel_command_akzeptiert_python_aliases() {
        assert!(is_tvpanel_command("!tvpanel"));
        assert!(is_tvpanel_command("!tempvoicepanel"));
        assert!(is_tvpanel_command("!tvinterface"));
        assert!(is_tvpanel_command("!TVPANEL"));
        assert!(!is_tvpanel_command("!rrang"));
    }

    #[test]
    fn tvpanel_history_adoptiert_vorhandenes_global_panel() {
        let messages = vec![
            TempVoicePanelMessage {
                message_id: 10,
                has_embeds: true,
                has_components: false,
                embed_titles: vec![GLOBAL_PANEL_TITLE.to_string()],
            },
            TempVoicePanelMessage {
                message_id: 11,
                has_embeds: true,
                has_components: true,
                embed_titles: vec![GLOBAL_PANEL_TITLE.to_string()],
            },
            TempVoicePanelMessage {
                message_id: 12,
                has_embeds: true,
                has_components: true,
                embed_titles: vec!["Anderes Panel".to_string()],
            },
        ];

        assert_eq!(find_existing_tempvoice_global_panel(&messages), Some(11));
    }
}
