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
use dl_discord::{BridgeInteraction, BridgeReply, InteractionHandler, InteractionRouter};
use serde_json::json;

use super::engine::TempVoiceEngine;
use super::store::PresetRecord;

const NOT_IN_LANE: &str = "Du musst dafür in einer TempVoice-Lane sein.";
const NOT_OWNER: &str = "Nur der Lane-Owner kann das.";

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
        if self.engine.lane_owner(lane).await != Some(interaction.user_id) {
            return Err(BridgeReply::ephemeral_text(NOT_OWNER));
        }
        Ok(lane)
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
                engine.set_region(lane, interaction.user_id, region).await;
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
                    .values
                    .first()
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
                let _ = engine.store.add_ban(interaction.user_id, target).await;
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
                let bans = engine
                    .store
                    .list_bans(interaction.user_id)
                    .await
                    .unwrap_or_default();
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
                let Some(target) = Self::selected_user(&interaction) else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let _ = engine.store.remove_ban(interaction.user_id, target).await;
                if let Some(lane) = self.lane_of(&interaction).await {
                    let _ = engine.port.set_member_connect(lane, target, None).await;
                }
                BridgeReply::ephemeral_text(format!("<@{target}> entbannt."))
            }

            // ── Quick-Templates ───────────────────────────────────────
            "tv_tpl_duo" | "tv_tpl_trio" | "tv_tpl_reset" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let limit = match interaction.custom_id.as_str() {
                    "tv_tpl_duo" => 2,
                    "tv_tpl_trio" => 3,
                    _ => 6,
                };
                match engine.set_limit(lane, limit).await {
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
                    .values
                    .first()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                else {
                    return BridgeReply::ephemeral_text("Bitte einen Namen eingeben.");
                };
                let info = engine.lane_snapshot(lane).await;
                let Some((base_name, category_id)) = info else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
                };
                let region = engine
                    .store
                    .region_pref(interaction.user_id)
                    .await
                    .unwrap_or_else(|_| "EU".to_string());
                let result = engine
                    .store
                    .save_preset(PresetRecord {
                        user_id: interaction.user_id,
                        category_id,
                        name: name.clone(),
                        base_name,
                        limit: 6,
                        min_rank: "unknown".to_string(),
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
                let presets = engine
                    .store
                    .list_presets(interaction.user_id, category_id)
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
                let Some((_base, limit, _min_rank, region)) = engine
                    .store
                    .get_preset(interaction.user_id, category_id, name)
                    .await
                    .ok()
                    .flatten()
                else {
                    return BridgeReply::ephemeral_text("Preset nicht gefunden.");
                };
                let _ = engine.set_limit(lane, limit).await;
                engine.set_region(lane, interaction.user_id, &region).await;
                BridgeReply::ephemeral_text(format!(
                    "Preset **{name}** angewendet (Limit {limit}, Region {region})."
                ))
            }

            // ── Rang-Präferenz ────────────────────────────────────────
            "tv_rank_pref" => BridgeReply {
                content: Some("Welchen Rang soll deine Chill-Lane tragen?".to_string()),
                components: Some(json!([{ "type": 1, "components": [{
                    "type": 3, "custom_id": "tv_rank_pref_sel",
                    "options": super::logic::RANK_ORDER.iter().skip(1)
                        .map(|rank| json!({
                            "label": super::logic::capitalize(rank),
                            "value": rank,
                        }))
                        .collect::<Vec<_>>(),
                    "min_values": 1, "max_values": 1,
                }]}])),
                ephemeral: true,
                ..BridgeReply::default()
            },
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
                    .values
                    .first()
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
                    let own = super::logic::RANK_ORDER.get(member_idx).copied().unwrap_or("unknown");
                    return BridgeReply::ephemeral_text(format!(
                        "Du kannst keinen Mindest-Rang über deinem eigenen setzen. Dein Rang: {}.",
                        super::logic::capitalize(own)
                    ));
                }
                self.pending_main_rank.lock().await.insert(lane, choice.clone());
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
