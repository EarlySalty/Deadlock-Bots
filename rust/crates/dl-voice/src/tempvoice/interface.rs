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
use super::store::{DefaultPresetRecord, InterfaceRecord, PresetRecord};

const NOT_IN_LANE: &str = "Du musst dafür in einem Sprachkanal sein.";
const NOT_OWNER: &str = "Nur der Lane-Owner kann das.";
pub const MIN_RANK_VERIFY_REQUIRED: &str =
    "Du kannst den Mindest-Rang nur setzen, wenn du verifiziert bist.";
pub const MIN_RANK_BLOCKED_REPLY: &str =
    "Du kannst keinen Mindest-Rang über deinem eigenen setzen.";
pub const RANK_PREF_UNKNOWN_LABEL: &str = "Kein Rang";
const GLOBAL_PANEL_TITLE: &str = "🚧 Sprachkanal verwalten";
const TV_COMPONENTS_V2_FLAG: u64 = 1 << 15;
const TV_EPHEMERAL_FLAG: u64 = 1 << 6;
const TV_ACCENT_GOLD: u64 = 0xC8A86B;
const TV_RANK_GATE_LABEL: &str = "🔓 Rang-Gate";
const TV_RANK_GATE_EXPLANATION: &str = "## 🔓 Rang-Gate\n\
Damit machst du deine Lane exklusiv für verifizierte Ränge in einem Fenster deiner Wahl.\n\
- Nur wer eine **verifizierte Rang-Rolle** im Fenster hat, kann noch joinen — alle anderen sehen ab dann ein 🔒 an deiner Lane.\n\
- **Niemand fliegt raus:** Wer schon drin ist, bleibt drin. Das Gate wirkt nur auf neue Joins.\n\
- Unverifizierte können nicht mehr joinen, solange das Gate an ist.\n\n\
Standard ist dein Rang **±1,5 Ränge**. Unten kannst du Mindestrang und Toleranz anpassen, dann bestätigen. Zum Ausschalten drückst du den 🔓-Knopf im Panel einfach nochmal.";
const TV_RANK_GATE_CONFIRM_LABEL: &str = "✅ Gate aktivieren";
const TV_RANK_GATE_ON: &str = "🔒 Rang-Gate ist an — nur verifizierte Ränge im gewählten Fenster können jetzt joinen. Wer schon drin ist, bleibt drin. Nochmal drücken schaltet es wieder aus.";
const TV_RANK_GATE_OFF: &str = "🔓 Rang-Gate ist aus — deine Lane ist wieder für alle offen.";
const TV_RANK_GATE_ONLY_RANKED: &str =
    "Das Rang-Gate gibt es nur in Ranked-Lanes, und nur der Owner kann es schalten.";
const TV_RANK_GATE_INVALID: &str = "Rang-Gate hat nicht geklappt";
const TV_RANK_GATE_SELECT_RANK: &str = "Mindestrang wählen";
const TV_RANK_GATE_SELECT_TOLERANCE: &str = "Toleranz wählen (± Subränge)";
const TV_GUIDE_TEXT: &str = "## 📖 So funktionieren die Sprachkanäle\n\
**Lane erstellen:** Geh in einen der (+)-Einstiegskanäle (Casual, Ranked oder Street Brawl) oder in den Router-Kanal — der Bot erstellt dir sofort eine eigene Lane und zieht dich rein. Du bist automatisch der Owner und steuerst alles über das Panel.\n\n\
**Alles in einer Kategorie:** Alle Lanes stehen jetzt zusammen — oben Ranked (nach Rang sortiert, klein → groß), darunter Casual, unten Street Brawl. Der Name sagt dir, was drin läuft: *Ranked Phantom 3*, *Chill Lane 2 · Oracle*, *Street Brawl 1*.\n\n\
**🎯 Mein Rang:** Damit stellst du im Panel ein, mit welchem Rang deine Lanes benannt werden. Bei Ranked ist dein Rang zusätzlich die Vorgabe fürs Rang-Gate.\n\n\
**Ranked ist offen:** Zum Erstellen und Joinen von Ranked-Lanes brauchst du keine Verifizierung mehr.\n\n\
**🔓 Rang-Gate (nur Ranked, nur Owner):** Ein Klick zeigt dir die Erklärung mit zwei Auswahlfeldern — Mindestrang und Toleranz, Standard: dein Rang ±1,5 Ränge. Nach dem Bestätigen können nur noch verifizierte Ränge im Fenster joinen, alle anderen sehen ein 🔒. Niemand wird gekickt. Nochmal klicken schaltet das Gate wieder aus.";
const TV_GUIDE_BUTTON_LABEL: &str = "📖 Anleitung";
const TV_ANNOUNCEMENT_TEXT: &str = "## 🔊 Voice-Umbau: Alle Lanes in einer Kategorie\n\
Wir haben die Sprachkanäle umgebaut, damit Ranked nicht mehr abschreckt und ihr schneller zusammenfindet:\n\
- **Eine Kategorie für alles:** Ranked, Casual und Street Brawl stehen jetzt zusammen — oben Ranked (nach Rang sortiert, klein → groß), darunter Casual, unten Street Brawl. Der Name sagt, was drin läuft: *Ranked Phantom 3*, *Chill Lane 2 · Oracle*, *Street Brawl 1*.\n\
- **Ranked ist jetzt offen:** Kein Verifizierungs-Zwang und kein Dauer-Schloss mehr — jeder kann Ranked-Lanes aufmachen und joinen.\n\
- **Ihr entscheidet selbst:** Der Lane-Owner kann übers Panel ein 🔓 **Rang-Gate** schalten. Dann können nur noch verifizierte Ränge in einem Fenster joinen (Standard: eigener Rang ±1,5 Ränge) und erst dann gibt es ein 🔒 — nur an dieser einen Lane. Wer schon drin ist, fliegt nie raus.\n\
- **🎯 Mein Rang** im Panel bestimmt, mit welchem Rang deine Lanes benannt werden — bei Ranked ist er auch die Gate-Vorgabe.\n\n\
Die Schritt-für-Schritt-Anleitung gibt es hier:";
const TV_ANNOUNCEMENT_CONFIRM: &str = "Sprachkanal-Ankündigung gepostet.";
const TV_ANNOUNCEMENT_FAILED: &str = "Sprachkanal-Ankündigung konnte nicht gepostet werden.";

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
    lfg_cutover_active: bool,
}

impl TempVoiceInterface {
    pub fn new(
        engine: Arc<TempVoiceEngine>,
        port: Arc<dyn TempVoiceInterfacePort>,
        lfg_cutover_active: bool,
    ) -> Arc<Self> {
        Arc::new(Self {
            engine,
            port,
            lfg_cutover_active,
        })
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
        let body = global_panel_body_for_cutover(category_id, self.lfg_cutover_active);
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
            let body = global_panel_body_for_cutover(row.category_id, self.lfg_cutover_active);
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
            let include_ranked = self.engine.lane_is_ranked(lane_id).await;
            let body = lane_panel_body_for_cutover(
                &lane_name,
                owner_id,
                category_id,
                include_ranked,
                self.lfg_cutover_active,
            );
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

#[cfg(test)]
fn global_panel_body(category_id: Option<u64>) -> Map<String, Value> {
    global_panel_body_for_cutover(category_id, false)
}

fn global_panel_body_for_cutover(
    category_id: Option<u64>,
    lfg_cutover_active: bool,
) -> Map<String, Value> {
    let _ = category_id;
    panel_body(
        global_panel_embed(),
        main_view_components(false, lfg_cutover_active),
    )
}

#[cfg(test)]
fn lane_panel_body(
    lane_name: &str,
    owner_id: Option<u64>,
    category_id: Option<u64>,
) -> Map<String, Value> {
    lane_panel_body_for_cutover(
        lane_name,
        owner_id,
        category_id,
        is_ranked_lane_name(lane_name),
        false,
    )
}

fn lane_panel_body_for_cutover(
    lane_name: &str,
    owner_id: Option<u64>,
    category_id: Option<u64>,
    include_ranked: bool,
    lfg_cutover_active: bool,
) -> Map<String, Value> {
    let _ = category_id;
    panel_body(
        lane_panel_embed(lane_name, owner_id),
        main_view_components(include_ranked, lfg_cutover_active),
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
            "So funktionieren Sprachkanäle:\n",
            "• Betritt einen **(+) Sprachkanal**, dein eigener Sprachkanal wird automatisch erstellt.\n",
            "• Passe deine Lane hier an; die Buttons wirken sofort, wenn du Owner bist.\n\n",
            "Was ihr hier machen könnt:\n",
            "• **Kick:** Jemand AFK oder stört? Entferne die Person, wenn Reden nicht reicht.\n",
            "• **Ban:** Sperre jemanden dauerhaft aus deinem Kanal, solange du Owner bist.\n",
            "• **Unban:** Hebe die Sperre wieder auf.\n",
            "• **Duo/Trio Call:** Stelle 2er/3er-Runden ein; andere können (fast) nicht beitreten.\n",
            "• **Normale Lane:** Setzt die Berechtigungen wieder auf offen.\n",
            "• **Lurker-Rolle:** Für Zuhörer; schafft einen zusätzlichen Platz für Mitspieler.\n",
            "• **Limit & Sprache:** Setze Teilnehmerlimit (0–99) und Deutsch/Offen-Filter.\n",
            "• **Owner Claim:** Übernimm die Lane, wenn der Owner weg ist.\n",
            "• **🔓 Rang-Gate** *(nur Ranked)*: Mach deine Lane exklusiv für verifizierte Ränge in einem Fenster (Standard: dein Rang ±1,5). Niemand fliegt raus — wirkt nur auf neue Joins."
        ),
        "color": 0x2ECC71,
        "footer": {"text": "Deutsche Deadlock Community • Sprachkanäle"},
    })
}

fn lane_panel_embed(lane_name: &str, owner_id: Option<u64>) -> Value {
    let owner_display = owner_id
        .map(|id| format!("<@{id}>"))
        .unwrap_or_else(|| "Unbekannt".to_string());
    json!({
        "title": format!("🎙️ Sprachkanal – {lane_name}"),
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
                "**🔓 Rang-Gate** *(nur Ranked, nur Owner)*\n",
                "Macht die Lane exklusiv für verifizierte Ränge in einem Fenster\n",
                "(Standard: dein Rang ±1,5). Wer drin ist, bleibt drin —\n",
                "wirkt nur auf neue Joins. Nochmal drücken = wieder offen."
            ),
            owner_display
        ),
        "color": 0x2ECC71,
        "footer": {"text": "Deutsche Deadlock Community • Sprachkanäle"},
    })
}

#[cfg(test)]
fn is_ranked_lane_name(lane_name: &str) -> bool {
    let lower = lane_name.trim().to_lowercase();
    lower == "ranked" || lower.starts_with("ranked ")
}

fn main_view_components(include_ranked: bool, lfg_cutover_active: bool) -> Value {
    if include_ranked {
        let preset_button = if lfg_cutover_active {
            button("💾 Presets", 2, "tv_presets")
        } else {
            button("💾 Preset speichern", 3, "tv_preset_save")
        };
        let final_button = if lfg_cutover_active {
            button(
                crate::lfg_panel::LFG_BTN_PUBLISH_LANE,
                1,
                crate::lfg_panel::LFG_PUBLISH_LANE_CUSTOM_ID,
            )
        } else {
            button("📂 Preset laden", 1, "tv_preset_load")
        };
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
            action_row(vec![button(TV_RANK_GATE_LABEL, 2, "tv_rank_gate")]),
            action_row(vec![
                button("Normale Lane", 2, "tv_tpl_reset"),
                button("Duo Call (2)", 1, "tv_tpl_duo"),
                button("Trio Call (3)", 1, "tv_tpl_trio"),
                preset_button,
                final_button,
            ]),
        ])
    } else {
        let mut final_row = vec![
            button("✏️ Umbenennen", 2, "tv_rename_btn"),
            button("🔄 Modus wechseln", 2, "tv_mode_switch_btn"),
        ];
        if lfg_cutover_active {
            final_row.push(button(
                crate::lfg_panel::LFG_BTN_PUBLISH_LANE,
                1,
                crate::lfg_panel::LFG_PUBLISH_LANE_CUSTOM_ID,
            ));
        }
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
            action_row(final_row),
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

fn emoji_button(
    label: &str,
    style: u8,
    custom_id: &str,
    emoji_name: &str,
    emoji_id: &str,
) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
        "emoji": { "name": emoji_name, "id": emoji_id },
    })
}

fn tv_container(id: u64, components: Vec<Value>) -> Value {
    json!({
        "type": 17,
        "id": id,
        "accent_color": TV_ACCENT_GOLD,
        "components": components,
    })
}

fn tv_text_display(id: u64, content: &str) -> Value {
    json!({
        "type": 10,
        "id": id,
        "content": content,
    })
}

fn empty_allowed_mentions() -> Value {
    json!({ "parse": Vec::<String>::new() })
}

fn tv_v2_reply(components: Value, fallback_text: &str) -> BridgeReply {
    BridgeReply {
        components: Some(components),
        ephemeral: true,
        message_flags: Some(TV_EPHEMERAL_FLAG | TV_COMPONENTS_V2_FLAG),
        allowed_mentions: Some(empty_allowed_mentions()),
        fallback: Some(Box::new(BridgeReply::ephemeral_text(fallback_text))),
        ..BridgeReply::default()
    }
}

fn rank_gate_rank_options(current: &str) -> Vec<Value> {
    super::logic::RANK_ORDER
        .iter()
        .skip(1)
        .map(|rank| {
            json!({
                "label": super::logic::capitalize(rank),
                "value": rank,
                "default": current == *rank,
            })
        })
        .collect()
}

fn rank_gate_tolerance_options(current: i64) -> Vec<Value> {
    [3_i64, 6, 9, 12]
        .into_iter()
        .map(|value| {
            json!({
                "label": match value {
                    3 => "±3 Subränge (± halber Rang)".to_string(),
                    6 => "±6 Subränge (±1 Rang)".to_string(),
                    9 => "±9 Subränge (±1,5 Ränge) — Standard".to_string(),
                    12 => "±12 Subränge (±2 Ränge)".to_string(),
                    _ => format!("±{value} Subränge"),
                },
                "value": value.to_string(),
                "default": current == value,
            })
        })
        .collect()
}

fn rank_gate_dialog(pending: &PendingRankGate) -> BridgeReply {
    tv_v2_reply(
        json!([tv_container(
            1,
            vec![
                tv_text_display(2, TV_RANK_GATE_EXPLANATION),
                action_row(vec![json!({
                    "type": 3,
                    "custom_id": "tv_rank_gate_rank",
                    "placeholder": TV_RANK_GATE_SELECT_RANK,
                    "min_values": 1,
                    "max_values": 1,
                    "options": rank_gate_rank_options(&pending.rank),
                })]),
                action_row(vec![json!({
                    "type": 3,
                    "custom_id": "tv_rank_gate_tolerance",
                    "placeholder": TV_RANK_GATE_SELECT_TOLERANCE,
                    "min_values": 1,
                    "max_values": 1,
                    "options": rank_gate_tolerance_options(pending.tolerance),
                })]),
                action_row(vec![button(
                    TV_RANK_GATE_CONFIRM_LABEL,
                    3,
                    "tv_rank_gate_confirm"
                )]),
            ],
        )]),
        TV_RANK_GATE_EXPLANATION,
    )
}

fn tv_guide_reply() -> BridgeReply {
    tv_v2_reply(
        json!([tv_container(10, vec![tv_text_display(11, TV_GUIDE_TEXT)])]),
        TV_GUIDE_TEXT,
    )
}

pub fn tempvoice_announcement_body() -> Map<String, Value> {
    let mut body = Map::new();
    body.insert("flags".to_string(), json!(TV_COMPONENTS_V2_FLAG));
    body.insert("allowed_mentions".to_string(), empty_allowed_mentions());
    body.insert(
        "components".to_string(),
        json!([tv_container(
            20,
            vec![
                tv_text_display(21, TV_ANNOUNCEMENT_TEXT),
                action_row(vec![button(TV_GUIDE_BUTTON_LABEL, 1, "tv_guide")]),
            ],
        )]),
    );
    body
}

fn find_existing_tempvoice_global_panel(messages: &[TempVoicePanelMessage]) -> Option<u64> {
    messages
        .iter()
        .rev()
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

struct PanelHandler {
    engine: Arc<TempVoiceEngine>,
    lfg: Option<Arc<crate::lfg_panel::LfgPanelInterface>>,
    /// Zwischenspeicher für den ① gewählten Haupt-Rang, bis ② der Sub-Rang
    /// kommt (Port von Pythons modul-globalem `_pending_main_rank`). Per-Prozess-
    /// RAM, geht — wie im Original — bei Neustart verloren.
    pending_main_rank: tokio::sync::Mutex<std::collections::HashMap<u64, String>>,
    pending_default_rank: tokio::sync::Mutex<std::collections::HashMap<u64, String>>,
    pending_rank_gate: tokio::sync::Mutex<std::collections::HashMap<u64, PendingRankGate>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingRankGate {
    rank: String,
    subrank: i64,
    tolerance: i64,
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

    fn rank_parts_from_label(label: &str) -> Option<(String, i64)> {
        let lower = label.trim().to_lowercase();
        for rank in super::logic::RANK_ORDER.iter().skip(1) {
            if let Some(rest) = lower.strip_prefix(rank) {
                if rest
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphanumeric())
                {
                    continue;
                }
                let subrank = rest
                    .split_whitespace()
                    .next()
                    .and_then(|part| part.parse::<i64>().ok())
                    .filter(|value| (1..=6).contains(value))
                    .unwrap_or(3);
                return Some((rank.to_string(), subrank));
            }
        }
        None
    }

    async fn rank_gate_default(
        &self,
        interaction: &BridgeInteraction,
        lane: u64,
    ) -> PendingRankGate {
        let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
        let (rank, subrank) = self
            .engine
            .owner_rank_anchor(interaction.guild_id, owner_id)
            .await
            .as_deref()
            .and_then(Self::rank_parts_from_label)
            .unwrap_or_else(|| ("initiate".to_string(), 1));
        PendingRankGate {
            rank,
            subrank,
            tolerance: crate::rank::RANKED_SUBRANK_TOLERANCE,
        }
    }

    async fn ranked_owned_lane_of(
        &self,
        interaction: &BridgeInteraction,
    ) -> Result<u64, BridgeReply> {
        let lane = self.owned_lane_of(interaction).await?;
        if self.engine.lane_snapshot(lane).await.is_none() {
            return Err(BridgeReply::ephemeral_text(NOT_IN_LANE));
        }
        if self.engine.lane_is_ranked(lane).await {
            Ok(lane)
        } else {
            Err(BridgeReply::ephemeral_text(TV_RANK_GATE_ONLY_RANKED))
        }
    }

    /// Lane-Kontext für Ban/Unban: Nur wenn der Klickende die Lane besitzt
    /// (oder Mod-Rechte hat), zählt die Banliste des Lane-Owners und wird
    /// sofort im Kanal durchgesetzt. Sonst — auch ganz ohne Voice — läuft
    /// der Ban rein über die persönliche Liste des Klickenden.
    async fn ban_context_lane(&self, interaction: &BridgeInteraction) -> Option<u64> {
        let lane = self.lane_of(interaction).await?;
        let is_owner = self.engine.lane_owner(lane).await == Some(interaction.user_id);
        if is_owner || interaction.author_can_manage_channels {
            Some(lane)
        } else {
            None
        }
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

    fn prefs_reply(content: String, components: Value, is_dm: bool) -> BridgeReply {
        let mut container_components = vec![tv_text_display(30, &content), json!({ "type": 14 })];
        container_components.extend(components.as_array().cloned().unwrap_or_default());
        let body = json!([tv_container(29, container_components)]);
        if is_dm {
            // In einer DM gibt es kein Ephemeral — das Panel wird in-place editiert.
            BridgeReply {
                components: Some(body),
                update_message: true,
                message_flags: Some(TV_COMPONENTS_V2_FLAG),
                allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
                ..BridgeReply::default()
            }
        } else {
            BridgeReply {
                components: Some(body),
                ephemeral: true,
                message_flags: Some(TV_EPHEMERAL_FLAG | TV_COMPONENTS_V2_FLAG),
                allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
                fallback: Some(Box::new(BridgeReply::ephemeral_text(content))),
                ..BridgeReply::default()
            }
        }
    }

    fn prefs_text(default: Option<&DefaultPresetRecord>) -> String {
        match default {
            Some(default) => format!(
                "## ⚙️ Voreinstellungen\n**Modus:** {}\n**Name:** {}\n**Limit:** {}\n**Rang:** {}",
                prefs_mode_label(&default.mode),
                default.base_name,
                default.limit,
                if default.min_rank == "unknown" {
                    "noch keiner gesetzt".to_string()
                } else {
                    default.min_rank.clone()
                }
            ),
            None => {
                "## ⚙️ Voreinstellungen\n_Noch kein Standard gesetzt — wähle unten einen Modus._"
                    .to_string()
            }
        }
    }

    fn prefs_components(has_default: bool, can_apply_lane: bool, is_dm: bool) -> Value {
        // In der DM ersetzt „Fertig" (router_dm_done) das „Default löschen", damit
        // der User nach der Moduswahl direkt seine Lane bauen kann und der Button
        // bei jedem In-place-Update des Panels erhalten bleibt.
        let (mode_row, second_row) = if is_dm {
            (
                action_row(vec![
                    emoji_button(
                        "Casual",
                        2,
                        "tv_prefs_mode_casual",
                        "dl_casual",
                        "1522518264088100995",
                    ),
                    emoji_button(
                        "Ranked",
                        2,
                        "tv_prefs_mode_ranked",
                        "dl_ranked",
                        "1522518271306366996",
                    ),
                    emoji_button(
                        "Street Brawl",
                        2,
                        "tv_prefs_mode_street_brawl",
                        "dl_brawl",
                        "1522518262708174928",
                    ),
                ]),
                action_row(vec![
                    button("Name+Limit ändern", 2, "tv_prefs_name_limit"),
                    button("Rang ändern", 2, "tv_prefs_rank"),
                    emoji_button(
                        "Fertig",
                        3,
                        "router_dm_done",
                        "dl_crown",
                        "1522518265421631538",
                    ),
                ]),
            )
        } else {
            (
                action_row(vec![
                    button("Casual", 1, "tv_prefs_mode_casual"),
                    button("Ranked", 3, "tv_prefs_mode_ranked"),
                    button("Street Brawl", 2, "tv_prefs_mode_street_brawl"),
                ]),
                action_row(vec![
                    button("Name+Limit ändern", 2, "tv_prefs_name_limit"),
                    button("Rang ändern", 2, "tv_prefs_rank"),
                    button("Default löschen", 4, "tv_prefs_delete"),
                ]),
            )
        };
        let mut rows = vec![mode_row, second_row];
        if has_default && can_apply_lane {
            rows.push(action_row(vec![button(
                "Auf aktuelle Lane anwenden",
                3,
                "tv_prefs_apply_lane",
            )]));
        }
        json!(rows)
    }

    async fn prefs_open_reply(&self, interaction: &BridgeInteraction) -> BridgeReply {
        let default = self
            .engine
            .store
            .get_default_preset(interaction.user_id)
            .await
            .ok()
            .flatten();
        let can_apply_lane = self.owned_lane_of(interaction).await.is_ok();
        Self::prefs_reply(
            Self::prefs_text(default.as_ref()),
            Self::prefs_components(default.is_some(), can_apply_lane, interaction.guild_id == 0),
            interaction.guild_id == 0,
        )
    }

    async fn current_or_new_default(
        &self,
        user_id: u64,
        mode: Option<&str>,
    ) -> DefaultPresetRecord {
        let mut record = self
            .engine
            .store
            .get_default_preset(user_id)
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| crate::router::default_preset_for_mode(user_id, "casual"));
        if let Some(mode) = mode {
            let fallback = crate::router::default_preset_for_mode(user_id, mode);
            record.mode = fallback.mode;
            if record.base_name.trim().is_empty() {
                record.base_name = fallback.base_name;
            }
            if record.limit < 0 {
                record.limit = fallback.limit;
            }
        }
        record
    }

    async fn save_default_and_show(
        &self,
        interaction: &BridgeInteraction,
        record: DefaultPresetRecord,
    ) -> BridgeReply {
        match self.engine.store.save_default_preset(record).await {
            Ok(()) => self.prefs_open_reply(interaction).await,
            Err(err) => BridgeReply {
                content: Some(format!("Speichern fehlgeschlagen: {err}")),
                ephemeral: true,
                allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
                ..BridgeReply::default()
            },
        }
    }
}

fn prefs_mode_label(mode: &str) -> &'static str {
    match mode {
        "ranked" => "Ranked",
        "street_brawl" => "Street Brawl",
        _ => "Casual",
    }
}

#[async_trait::async_trait]
impl InteractionHandler for PanelHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let engine = &self.engine;
        match interaction.custom_id.as_str() {
            "tv_guide" => tv_guide_reply(),
            "tv_rank_gate" => {
                let lane = match self.ranked_owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                if engine.rank_gate_active(lane).await {
                    return match engine.clear_rank_gate(interaction.guild_id, lane).await {
                        Ok(()) => BridgeReply::ephemeral_text(TV_RANK_GATE_OFF),
                        Err(err) => {
                            BridgeReply::ephemeral_text(format!("{TV_RANK_GATE_INVALID}: {err}"))
                        }
                    };
                }
                let pending = self.rank_gate_default(&interaction, lane).await;
                self.pending_rank_gate
                    .lock()
                    .await
                    .insert(lane, pending.clone());
                rank_gate_dialog(&pending)
            }
            "tv_rank_gate_rank" => {
                let lane = match self.ranked_owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(rank) = interaction.values.first().cloned() else {
                    return BridgeReply::ephemeral_text(TV_RANK_GATE_INVALID);
                };
                if super::logic::rank_index(&rank) == 0 {
                    return BridgeReply::ephemeral_text(TV_RANK_GATE_INVALID);
                }
                let mut pending = self
                    .pending_rank_gate
                    .lock()
                    .await
                    .get(&lane)
                    .cloned()
                    .unwrap_or_else(|| PendingRankGate {
                        rank: rank.clone(),
                        subrank: 3,
                        tolerance: crate::rank::RANKED_SUBRANK_TOLERANCE,
                    });
                pending.rank = rank;
                self.pending_rank_gate
                    .lock()
                    .await
                    .insert(lane, pending.clone());
                rank_gate_dialog(&pending)
            }
            "tv_rank_gate_tolerance" => {
                let lane = match self.ranked_owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(tolerance) = interaction
                    .values
                    .first()
                    .and_then(|value| value.parse::<i64>().ok())
                else {
                    return BridgeReply::ephemeral_text(TV_RANK_GATE_INVALID);
                };
                let mut pending = self
                    .pending_rank_gate
                    .lock()
                    .await
                    .get(&lane)
                    .cloned()
                    .unwrap_or_else(|| PendingRankGate {
                        rank: "initiate".to_string(),
                        subrank: 1,
                        tolerance,
                    });
                pending.tolerance = tolerance;
                self.pending_rank_gate
                    .lock()
                    .await
                    .insert(lane, pending.clone());
                rank_gate_dialog(&pending)
            }
            "tv_rank_gate_confirm" => {
                let lane = match self.ranked_owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let pending = self
                    .pending_rank_gate
                    .lock()
                    .await
                    .remove(&lane)
                    .unwrap_or_else(|| PendingRankGate {
                        rank: "initiate".to_string(),
                        subrank: 1,
                        tolerance: crate::rank::RANKED_SUBRANK_TOLERANCE,
                    });
                let owner_id = self.lane_owner_id(lane, interaction.user_id).await;
                match engine
                    .apply_rank_gate(
                        interaction.guild_id,
                        lane,
                        owner_id,
                        &pending.rank,
                        pending.subrank,
                        pending.tolerance,
                    )
                    .await
                {
                    Ok(()) => BridgeReply::ephemeral_text(TV_RANK_GATE_ON),
                    Err(err) => {
                        BridgeReply::ephemeral_text(format!("{TV_RANK_GATE_INVALID}: {err}"))
                    }
                }
            }
            "tv_prefs_open" => self.prefs_open_reply(&interaction).await,
            "tv_prefs_mode_casual" | "tv_prefs_mode_ranked" | "tv_prefs_mode_street_brawl" => {
                let mode = interaction
                    .custom_id
                    .trim_start_matches("tv_prefs_mode_")
                    .to_string();
                let record = self
                    .current_or_new_default(interaction.user_id, Some(&mode))
                    .await;
                self.save_default_and_show(&interaction, record).await
            }
            "tv_prefs_name_limit" => BridgeReply {
                modal: Some(ModalSpec {
                    custom_id: "tv_prefs_name_limit_modal".to_string(),
                    title: "Name+Limit ändern".to_string(),
                    fields: vec![
                        ModalField {
                            custom_id: "name".to_string(),
                            label: "Name".to_string(),
                            placeholder: "z. B. Chill Lane".to_string(),
                            required: true,
                            min_length: 1,
                            max_length: 90,
                            paragraph: false,
                        },
                        ModalField {
                            custom_id: "limit".to_string(),
                            label: "Limit".to_string(),
                            placeholder: "z. B. 6".to_string(),
                            required: true,
                            min_length: 1,
                            max_length: 2,
                            paragraph: false,
                        },
                    ],
                }),
                ..BridgeReply::default()
            },
            "tv_prefs_name_limit_modal" => {
                let Some(name) = interaction
                    .options
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                else {
                    return BridgeReply::ephemeral_text("Bitte einen Namen eingeben.");
                };
                let Some(limit) = interaction
                    .options
                    .get("limit")
                    .and_then(|v| v.as_str())
                    .and_then(|v| v.trim().parse::<i64>().ok())
                else {
                    return BridgeReply::ephemeral_text("Bitte eine Zahl eingeben.");
                };
                let mut record = self.current_or_new_default(interaction.user_id, None).await;
                record.base_name = name;
                record.limit = if record.mode == "street_brawl" {
                    limit.clamp(1, 4)
                } else {
                    limit.clamp(0, 99)
                };
                self.save_default_and_show(&interaction, record).await
            }
            "tv_prefs_rank" => {
                let current = self
                    .engine
                    .store
                    .get_default_preset(interaction.user_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|default| default.min_rank)
                    .unwrap_or_else(|| "unknown".to_string());
                let options = std::iter::once(json!({
                    "label": RANK_PREF_UNKNOWN_LABEL,
                    "value": "unknown",
                    "default": current == "unknown",
                }))
                .chain(super::logic::RANK_ORDER.iter().skip(1).map(|rank| {
                    json!({
                        "label": super::logic::capitalize(rank),
                        "value": rank,
                        "default": current.starts_with(rank),
                    })
                }))
                .collect::<Vec<_>>();
                Self::prefs_reply(
                    "Rang ändern".to_string(),
                    json!([{ "type": 1, "components": [{
                        "type": 3, "custom_id": "tv_prefs_rank_main",
                        "options": options,
                        "min_values": 1, "max_values": 1,
                    }]}]),
                    interaction.guild_id == 0,
                )
            }
            "tv_prefs_rank_main" => {
                let Some(rank) = interaction.values.first().cloned() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                if rank == "unknown" {
                    let mut record = self.current_or_new_default(interaction.user_id, None).await;
                    record.min_rank = "unknown".to_string();
                    return self.save_default_and_show(&interaction, record).await;
                }
                self.pending_default_rank
                    .lock()
                    .await
                    .insert(interaction.user_id, rank);
                Self::prefs_reply(
                    "Rang ändern".to_string(),
                    json!([{ "type": 1, "components": [{
                        "type": 3,
                        "custom_id": "tv_prefs_rank_sub",
                        "placeholder": "Sub-Rang wählen",
                        "min_values": 1,
                        "max_values": 1,
                        "options": std::iter::once(json!({"label": "Ohne Sub-Rang", "value": "0"}))
                            .chain((1..=6).map(|n| json!({"label": format!("Sub-Rang {n}"), "value": n.to_string()})))
                            .collect::<Vec<_>>(),
                    }]}]),
                    interaction.guild_id == 0,
                )
            }
            "tv_prefs_rank_sub" => {
                let Some(sub_raw) = interaction.values.first() else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let sub = sub_raw.trim().parse::<i64>().unwrap_or(-1);
                let Some(main_rank) = self
                    .pending_default_rank
                    .lock()
                    .await
                    .remove(&interaction.user_id)
                else {
                    return BridgeReply::ephemeral_text(
                        "Bitte zuerst den **① Haupt-Rang** auswählen.",
                    );
                };
                let min_rank = if sub == 0 {
                    main_rank
                } else if (1..=6).contains(&sub) {
                    format!("{main_rank} {sub}")
                } else {
                    return BridgeReply::ephemeral_text("Ungültiger Sub-Rang.");
                };
                let mut record = self.current_or_new_default(interaction.user_id, None).await;
                record.min_rank = min_rank;
                self.save_default_and_show(&interaction, record).await
            }
            "tv_prefs_delete" => match self
                .engine
                .store
                .delete_default_preset(interaction.user_id)
                .await
            {
                Ok(()) => self.prefs_open_reply(&interaction).await,
                Err(err) => BridgeReply {
                    content: Some(format!("Speichern fehlgeschlagen: {err}")),
                    ephemeral: true,
                    allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
                    ..BridgeReply::default()
                },
            },
            "tv_prefs_apply_lane" => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(default) = self
                    .engine
                    .store
                    .get_default_preset(interaction.user_id)
                    .await
                    .ok()
                    .flatten()
                else {
                    return Self::prefs_reply(
                        Self::prefs_text(None),
                        Self::prefs_components(false, false, interaction.guild_id == 0),
                        interaction.guild_id == 0,
                    );
                };
                if let Some(err) = self
                    .engine
                    .switch_lane_mode(
                        interaction.guild_id,
                        lane,
                        interaction.user_id,
                        &default.mode,
                    )
                    .await
                {
                    return BridgeReply {
                        content: Some(err),
                        ephemeral: true,
                        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
                        ..BridgeReply::default()
                    };
                }
                if let Err(err) = self
                    .engine
                    .set_lane_template(
                        interaction.guild_id,
                        lane,
                        &default.base_name,
                        default.limit,
                    )
                    .await
                {
                    return BridgeReply {
                        content: Some(format!("Speichern fehlgeschlagen: {err}")),
                        ephemeral: true,
                        allowed_mentions: Some(json!({ "parse": Vec::<String>::new() })),
                        ..BridgeReply::default()
                    };
                }
                if default.min_rank != "unknown" {
                    let _ = self
                        .engine
                        .set_min_rank(interaction.guild_id, lane, &default.min_rank)
                        .await;
                }
                self.prefs_open_reply(&interaction).await
            }
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
                // Bannen braucht keine Lane: Die Banliste hängt am User und wird
                // beim Join jeder seiner künftigen Lanes durchgesetzt. Der
                // User-Select (Typ 5) ist serverweit durchsuchbar — der Störer
                // muss nicht (mehr) in der Lane sitzen.
                BridgeReply {
                    content: Some(
                        "Wen bannen? Name eintippen und auswählen — gilt für alle deine Lanes."
                            .to_string(),
                    ),
                    components: Some(json!([{ "type": 1, "components": [{
                        "type": 5, "custom_id": "tv_ban_sel",
                        "placeholder": "Mitglied suchen…",
                        "min_values": 1, "max_values": 1,
                    }]}])),
                    ephemeral: true,
                    ..BridgeReply::default()
                }
            }
            "tv_ban_sel" => {
                let Some(target) = Self::selected_user(&interaction) else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                if target == interaction.user_id {
                    return BridgeReply::ephemeral_text("Dich selbst bannen geht nicht.");
                }
                let owned_lane = self.ban_context_lane(&interaction).await;
                let owner_id = match owned_lane {
                    Some(lane) => self.lane_owner_id(lane, interaction.user_id).await,
                    None => interaction.user_id,
                };
                let _ = engine.store.add_ban(owner_id, target).await;
                if let Some(lane) = owned_lane {
                    let _ = engine
                        .port
                        .set_member_connect(lane, target, Some(false))
                        .await;
                    let _ = engine
                        .port
                        .disconnect_member(interaction.guild_id, target, "TempVoice: Owner-Bann")
                        .await;
                }
                BridgeReply::ephemeral_text(format!(
                    "<@{target}> gebannt — gilt für alle deine Lanes, bis du den Bann aufhebst."
                ))
            }
            "tv_unban" => {
                let owner_id = match self.ban_context_lane(&interaction).await {
                    Some(lane) => self.lane_owner_id(lane, interaction.user_id).await,
                    None => interaction.user_id,
                };
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
                let Some(target) = Self::selected_user(&interaction) else {
                    return BridgeReply::ephemeral_text("Keine Auswahl.");
                };
                let owned_lane = self.ban_context_lane(&interaction).await;
                let owner_id = match owned_lane {
                    Some(lane) => self.lane_owner_id(lane, interaction.user_id).await,
                    None => interaction.user_id,
                };
                let _ = engine.store.remove_ban(owner_id, target).await;
                if let Some(lane) = owned_lane {
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
                let (base_name, limit) = match interaction.custom_id.as_str() {
                    "tv_tpl_duo" => ("Duo Call".to_string(), 2),
                    "tv_tpl_trio" => ("Trio Call".to_string(), 3),
                    _ => {
                        let base = engine
                            .reset_lane_template_base(interaction.guild_id, lane)
                            .await;
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
            "tv_presets" => BridgeReply {
                content: Some(crate::lfg_panel::LFG_PRESETS_SUBMENU_TEXT.to_string()),
                components: Some(json!([{ "type": 1, "components": [
                    button(crate::lfg_panel::LFG_PRESETS_BTN_SAVE, 3, "tv_preset_save"),
                    button(crate::lfg_panel::LFG_PRESETS_BTN_LOAD, 1, "tv_preset_load"),
                ]}])),
                ephemeral: true,
                ..BridgeReply::default()
            },
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
                if !engine.is_min_rank_blocked(lane).await {
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
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
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

            crate::lfg_panel::LFG_PUBLISH_LANE_CUSTOM_ID => {
                let lane = match self.owned_lane_of(&interaction).await {
                    Ok(lane) => lane,
                    Err(reply) => return reply,
                };
                let Some(lfg) = &self.lfg else {
                    return BridgeReply::ephemeral_text(
                        crate::lfg_panel::LFG_ERR_ERSTELLUNG_FEHLGESCHLAGEN,
                    );
                };
                lfg.handle_publish_lane_start(interaction, lane).await
            }

            // ── Mindest-Rang (① Haupt-Rang → ② Sub-Rang) ──────────────
            // Wie MinRankSelect/SubRankSelectPermanent: kein Owner-Check
            // (jedes Lane-Mitglied), aber nur in Comp/Ranked-Kategorien und
            // nicht über dem eigenen Rang. Der Haupt-Rang wird zwischen den
            // beiden Selects in `pending_main_rank` gehalten (Port von Pythons
            // modul-globalem `_pending_main_rank`).
            "tv_minrank" => {
                let Some(lane) = self.lane_of(&interaction).await else {
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
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
                    return BridgeReply::ephemeral_text(NOT_IN_LANE);
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
pub fn register(
    router: &mut InteractionRouter,
    engine: Arc<TempVoiceEngine>,
    lfg: Option<Arc<crate::lfg_panel::LfgPanelInterface>>,
) {
    let lfg_active = lfg.is_some();
    let handler = Arc::new(PanelHandler {
        engine,
        lfg,
        pending_main_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        pending_default_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        pending_rank_gate: tokio::sync::Mutex::new(std::collections::HashMap::new()),
    });
    for custom_id in [
        "tv_prefs_open",
        "tv_prefs_mode_casual",
        "tv_prefs_mode_ranked",
        "tv_prefs_mode_street_brawl",
        "tv_prefs_name_limit",
        "tv_prefs_name_limit_modal",
        "tv_prefs_rank",
        "tv_prefs_rank_main",
        "tv_prefs_rank_sub",
        "tv_prefs_delete",
        "tv_prefs_apply_lane",
        "tv_guide",
        "tv_rank_gate",
        "tv_rank_gate_rank",
        "tv_rank_gate_tolerance",
        "tv_rank_gate_confirm",
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
        "tv_presets",
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
    if lfg_active {
        router.on_custom_id(crate::lfg_panel::LFG_PUBLISH_LANE_CUSTOM_ID, handler);
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

fn tempvoice_announcement_target(content: &str, fallback_channel_id: u64) -> Option<u64> {
    let mut parts = content.split_whitespace();
    let root = parts.next()?.to_ascii_lowercase();
    if !matches!(root.as_str(), "!tvumbau" | "!tempvoiceumbau") {
        return None;
    }
    let Some(raw) = parts.next() else {
        return Some(fallback_channel_id);
    };
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    digits.parse().ok().or(Some(fallback_channel_id))
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
                    let content = event.content.trim();
                    let is_panel_command = is_tvpanel_command(content);
                    let announcement_target =
                        tempvoice_announcement_target(content, event.channel_id);
                    if !is_panel_command && announcement_target.is_none() {
                        continue;
                    }
                    if !interface
                        .port
                        .member_can_manage_guild(guild_id, event.author_id)
                        .await
                    {
                        continue;
                    }
                    let reply = if let Some(target_channel_id) = announcement_target {
                        match interface
                            .port
                            .post_rich(target_channel_id, tempvoice_announcement_body())
                            .await
                        {
                            Ok(_) => TV_ANNOUNCEMENT_CONFIRM.to_string(),
                            Err(err) => {
                                tracing::warn!(%err, target_channel_id, "TempVoice-Ankündigung fehlgeschlagen");
                                TV_ANNOUNCEMENT_FAILED.to_string()
                            }
                        }
                    } else {
                        match interface
                            .ensure_interface_message(guild_id, event.channel_id, None)
                            .await
                        {
                            Ok(_) => format!(
                                "✅ Sprachkanal-Panel erstellt/aktualisiert in <#{}>.",
                                event.channel_id
                            ),
                            Err(err) => {
                                tracing::warn!(%err, "tvpanel command failed");
                                "❌ Konnte das Interface nicht erstellen (Fehler im Log)."
                                    .to_string()
                            }
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
    use crate::tempvoice::store::LaneRecord;
    use crate::tempvoice::{TempVoiceConfig, TempVoiceEngine, TempVoiceStore};
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    #[test]
    fn prefs_reply_dm_updatet_in_place_ohne_ephemeral() {
        // DM (guild_id 0): in-place Update, kein Ephemeral (in DMs unmöglich).
        let dm = PanelHandler::prefs_reply("x".to_string(), json!([]), true);
        assert!(dm.update_message);
        assert!(!dm.ephemeral);
        assert_eq!(dm.message_flags, Some(TV_COMPONENTS_V2_FLAG));

        // Guild-Kontext bleibt unverändert ephemeral.
        let guild = PanelHandler::prefs_reply("x".to_string(), json!([]), false);
        assert!(!guild.update_message);
        assert!(guild.ephemeral);
        assert_eq!(
            guild.message_flags,
            Some(TV_EPHEMERAL_FLAG | TV_COMPONENTS_V2_FLAG)
        );
    }

    #[test]
    fn prefs_components_dm_behaelt_fertig_statt_loeschen() {
        // Re-Render nach Modus-Klick in der DM MUSS Fertig behalten (Regression).
        let dm = serde_json::to_string(&PanelHandler::prefs_components(true, false, true))
            .expect("json");
        assert!(
            dm.contains("router_dm_done"),
            "DM-Panel muss Fertig behalten"
        );
        assert!(
            !dm.contains("tv_prefs_delete"),
            "DM-Panel zeigt kein Default löschen"
        );
        // Guild-Panel unverändert: Default löschen, kein Fertig.
        let guild = serde_json::to_string(&PanelHandler::prefs_components(true, false, false))
            .expect("json");
        assert!(guild.contains("tv_prefs_delete"));
        assert!(!guild.contains("router_dm_done"));
    }

    #[test]
    fn global_panel_body_enthaelt_python_embed_und_buttons() {
        let body = global_panel_body(None);
        let embeds = body
            .get("embeds")
            .and_then(serde_json::Value::as_array)
            .expect("embeds");
        assert_eq!(embeds[0]["title"], "🚧 Sprachkanal verwalten");
        let description = embeds[0]["description"].as_str().expect("description");
        assert!(description.contains("So funktionieren Sprachkanäle:\n"));
        assert!(description.contains(
            "• Betritt einen **(+) Sprachkanal**, dein eigener Sprachkanal wird automatisch erstellt."
        ));
        assert!(description.contains("• **🔓 Rang-Gate** *(nur Ranked)*"));
        assert_eq!(
            embeds[0]["footer"]["text"],
            "Deutsche Deadlock Community • Sprachkanäle"
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

        let ranked = lane_panel_body("Ranked Phantom 3 1", Some(42), Some(1289721245281292290));
        let ranked_components = ranked
            .get("components")
            .and_then(serde_json::Value::as_array)
            .expect("ranked components");
        let ranked_ids: Vec<&str> = ranked_components
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert!(!ranked_ids.contains(&"tv_minrank"));
        assert!(!ranked_ids.contains(&"tv_subrank_perm"));
        assert!(ranked_ids.contains(&"tv_rank_gate"));
        assert!(ranked_ids.contains(&"tv_preset_save"));
    }

    #[test]
    fn flag_aus_ranked_panel_behaelt_preset_load_und_ohne_lfg_publish() {
        let components = main_view_components(true, false);
        let custom_ids: Vec<&str> = components
            .as_array()
            .expect("components")
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| component["custom_id"].as_str())
            .collect();

        assert!(custom_ids.contains(&"tv_preset_save"));
        assert!(custom_ids.contains(&"tv_preset_load"));
        assert!(!custom_ids.contains(&crate::lfg_panel::LFG_PUBLISH_LANE_CUSTOM_ID));
    }

    #[test]
    fn cutover_ranked_panel_buendelt_presets_und_zeigt_lfg_publish() {
        let components = main_view_components(true, true);
        let buttons: Vec<(&str, u64, &str)> = components
            .as_array()
            .expect("components")
            .iter()
            .flat_map(|row| row["components"].as_array().into_iter().flatten())
            .filter_map(|component| {
                Some((
                    component["label"].as_str()?,
                    component["style"].as_u64()?,
                    component["custom_id"].as_str()?,
                ))
            })
            .collect();

        assert!(buttons
            .iter()
            .any(|(label, style, custom_id)| *label == "💾 Presets"
                && *style == 2
                && *custom_id == "tv_presets"));
        assert!(buttons
            .iter()
            .any(|(_, _, custom_id)| *custom_id == crate::lfg_panel::LFG_PUBLISH_LANE_CUSTOM_ID));
        assert!(!buttons
            .iter()
            .any(|(_, _, custom_id)| *custom_id == "tv_preset_load"));
    }

    #[tokio::test]
    async fn presets_sammelbutton_oeffnet_save_und_load_untermenue() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let engine = TempVoiceEngine::new(
            TempVoiceConfig {
                guild_id_hint: 1,
                staging_channels: HashSet::new(),
                fixed_lane_ids: HashSet::new(),
                tempvoice_categories: HashSet::new(),
                minrank_categories: HashSet::new(),
                ranked_category_id: 0,
                staging_rules: HashMap::new(),
            },
            TempVoiceStore::new(db.pool().clone()),
            Arc::new(ForeignLanePort),
        );
        let handler = PanelHandler {
            engine,
            lfg: None,
            pending_main_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_default_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_rank_gate: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        };

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_presets".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            reply.content.as_deref(),
            Some(crate::lfg_panel::LFG_PRESETS_SUBMENU_TEXT)
        );
        let custom_ids: Vec<&str> = reply.components.as_ref().expect("components")[0]["components"]
            .as_array()
            .expect("buttons")
            .iter()
            .filter_map(|component| component["custom_id"].as_str())
            .collect();
        assert_eq!(custom_ids, vec!["tv_preset_save", "tv_preset_load"]);
    }

    #[test]
    fn sichtbare_texte_sind_final() {
        for text in [
            MIN_RANK_VERIFY_REQUIRED,
            MIN_RANK_BLOCKED_REPLY,
            RANK_PREF_UNKNOWN_LABEL,
            crate::tempvoice::engine::MIN_RANK_DISABLED_REPLY,
        ] {
            assert_ne!(text, "Platzhalter");
            assert!(!text.is_empty());
        }
    }

    #[test]
    fn lane_panel_body_enthaelt_python_lane_embed() {
        let body = lane_panel_body("Lane 1", Some(42), Some(1412804540994162789));
        let embed = &body
            .get("embeds")
            .and_then(serde_json::Value::as_array)
            .expect("embeds")[0];
        assert_eq!(embed["title"], "🎙️ Sprachkanal – Lane 1");
        let description = embed["description"].as_str().expect("description");
        assert!(description.contains("**Owner:** <@42>"));
        assert!(description.contains("👻 Lurker – stumm beitreten ohne Limit-Slot zu belegen"));
        assert!(description.contains("**🔓 Rang-Gate** *(nur Ranked, nur Owner)*"));
    }

    #[test]
    fn not_in_lane_reply_nennt_sprachkanal() {
        assert_eq!(NOT_IN_LANE, "Du musst dafür in einem Sprachkanal sein.");
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
            TempVoicePanelMessage {
                message_id: 13,
                has_embeds: true,
                has_components: true,
                embed_titles: vec![GLOBAL_PANEL_TITLE.to_string()],
            },
        ];

        assert_eq!(find_existing_tempvoice_global_panel(&messages), Some(13));
    }

    struct ForeignLanePort;

    #[async_trait::async_trait]
    impl crate::tempvoice::LanePort for ForeignLanePort {
        async fn create_voice_channel(
            &self,
            _guild_id: u64,
            _category_id: Option<u64>,
            _name: &str,
            _user_limit: i64,
        ) -> Result<u64, String> {
            Ok(1)
        }

        async fn delete_channel(&self, _channel_id: u64, _reason: &str) -> Result<(), String> {
            Ok(())
        }

        async fn move_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _channel_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn rename_channel(
            &self,
            _channel_id: u64,
            _name: &str,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn set_member_connect(
            &self,
            _channel_id: u64,
            _user_id: u64,
            _connect: Option<bool>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn apply_member_connect_batch(
            &self,
            _channel_id: u64,
            _denied_user_ids: &HashSet<u64>,
            _clear_user_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn set_role_connect(
            &self,
            _channel_id: u64,
            _role_id: u64,
            _connect: Option<bool>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn apply_role_connect_batch(
            &self,
            _guild_id: u64,
            _channel_id: u64,
            _allowed_role_ids: &HashSet<u64>,
            _clear_role_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn apply_role_connect_overwrites(
            &self,
            _guild_id: u64,
            _channel_id: u64,
            _allowed_role_ids: &HashSet<u64>,
            _denied_role_ids: &HashSet<u64>,
            _clear_role_ids: &HashSet<u64>,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn set_user_limit(
            &self,
            _channel_id: u64,
            _limit: i64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn disconnect_member(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn member_display_name(&self, _guild_id: u64, _user_id: u64) -> Option<String> {
            None
        }

        async fn add_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn remove_role(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _role_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn set_nick(
            &self,
            _guild_id: u64,
            _user_id: u64,
            _nick: Option<&str>,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn member_nick(&self, _guild_id: u64, _user_id: u64) -> Option<String> {
            None
        }

        async fn channel_user_limit(&self, _guild_id: u64, _channel_id: u64) -> Option<i64> {
            None
        }

        async fn guild_role_names(&self, _guild_id: u64) -> Vec<(u64, String)> {
            Vec::new()
        }

        async fn set_channel_category(
            &self,
            _channel_id: u64,
            _category_id: u64,
            _reason: &str,
        ) -> Result<(), String> {
            Ok(())
        }

        async fn member_voice_channel(&self, _guild_id: u64, _user_id: u64) -> Option<u64> {
            Some(4242)
        }

        async fn member_role_names(&self, _guild_id: u64, _user_id: u64) -> Vec<String> {
            Vec::new()
        }

        async fn member_role_ids(&self, _guild_id: u64, _user_id: u64) -> Vec<u64> {
            Vec::new()
        }

        async fn channel_members(&self, _guild_id: u64, _channel_id: u64) -> Vec<u64> {
            Vec::new()
        }

        async fn channel_name(&self, _guild_id: u64, _channel_id: u64) -> Option<String> {
            Some("Lane 1".to_string())
        }

        async fn channel_category(&self, _guild_id: u64, _channel_id: u64) -> Option<u64> {
            Some(1289721245281292290)
        }

        async fn category_voice_channel_names(
            &self,
            _guild_id: u64,
            _category_id: u64,
        ) -> Vec<String> {
            Vec::new()
        }

        async fn category_voice_channels(
            &self,
            _guild_id: u64,
            _category_id: u64,
        ) -> Vec<(u64, String)> {
            Vec::new()
        }

        async fn channel_created_at(&self, _channel_id: u64) -> Option<i64> {
            None
        }
    }

    async fn panel_handler_for_test() -> (dl_central_db::TestDb, PanelHandler) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let engine = TempVoiceEngine::new(
            TempVoiceConfig {
                guild_id_hint: 1,
                staging_channels: HashSet::new(),
                fixed_lane_ids: HashSet::new(),
                tempvoice_categories: HashSet::new(),
                minrank_categories: HashSet::new(),
                ranked_category_id: 0,
                staging_rules: HashMap::new(),
            },
            TempVoiceStore::new(db.pool().clone()),
            Arc::new(ForeignLanePort),
        );
        let handler = PanelHandler {
            engine,
            lfg: None,
            pending_main_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_default_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_rank_gate: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        };
        (db, handler)
    }

    /// Wie panel_handler_for_test, aber der Port-Kanal 4242 zählt als Staging —
    /// lane_of liefert dann None, d. h. der User sitzt in keiner Lane.
    async fn panel_handler_ohne_lane_for_test() -> (dl_central_db::TestDb, PanelHandler) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let engine = TempVoiceEngine::new(
            TempVoiceConfig {
                guild_id_hint: 1,
                staging_channels: HashSet::from([4242]),
                fixed_lane_ids: HashSet::new(),
                tempvoice_categories: HashSet::new(),
                minrank_categories: HashSet::new(),
                ranked_category_id: 0,
                staging_rules: HashMap::new(),
            },
            TempVoiceStore::new(db.pool().clone()),
            Arc::new(ForeignLanePort),
        );
        let handler = PanelHandler {
            engine,
            lfg: None,
            pending_main_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_default_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_rank_gate: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        };
        (db, handler)
    }

    fn reply_custom_ids(reply: &BridgeReply) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(components) = &reply.components {
            collect_reply_custom_ids(components, &mut out);
        }
        out
    }

    fn reply_prefs_container(reply: &BridgeReply) -> &Value {
        let components = reply
            .components
            .as_ref()
            .and_then(Value::as_array)
            .expect("components");
        assert_eq!(components.len(), 1);
        let container = &components[0];
        assert_eq!(container["type"], 17);
        assert_eq!(container["accent_color"], TV_ACCENT_GOLD);
        container
    }

    fn reply_prefs_text(reply: &BridgeReply) -> &str {
        reply_prefs_container(reply)["components"]
            .as_array()
            .expect("container components")
            .iter()
            .find(|component| component["type"] == 10)
            .and_then(|component| component["content"].as_str())
            .expect("text display")
    }

    fn collect_reply_custom_ids(value: &Value, out: &mut Vec<String>) {
        if let Some(custom_id) = value.get("custom_id").and_then(Value::as_str) {
            out.push(custom_id.to_string());
        }
        if let Some(children) = value.get("components").and_then(Value::as_array) {
            for child in children {
                collect_reply_custom_ids(child, out);
            }
        }
        if let Some(children) = value.as_array() {
            for child in children {
                collect_reply_custom_ids(child, out);
            }
        }
    }

    #[tokio::test]
    async fn prefs_editor_zeigt_keinen_default() {
        let (_db, handler) = panel_handler_for_test().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_prefs_open".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(
            reply.message_flags,
            Some(TV_EPHEMERAL_FLAG | TV_COMPONENTS_V2_FLAG)
        );
        assert!(reply.content.is_none());
        assert_eq!(reply.allowed_mentions, Some(json!({ "parse": [] })));
        assert_eq!(
            reply_prefs_text(&reply),
            "## ⚙️ Voreinstellungen\n_Noch kein Standard gesetzt — wähle unten einen Modus._"
        );
        let container = reply_prefs_container(&reply);
        let container_components = container["components"]
            .as_array()
            .expect("container components");
        assert_eq!(container_components[0]["type"], 10);
        assert_eq!(container_components[1]["type"], 14);
        assert!(container_components[2..]
            .iter()
            .all(|component| component["type"] == 1));
        let ids = reply_custom_ids(&reply);
        assert!(ids.contains(&"tv_prefs_mode_casual".to_string()));
        assert!(ids.contains(&"tv_prefs_mode_ranked".to_string()));
        assert!(ids.contains(&"tv_prefs_mode_street_brawl".to_string()));
        assert!(ids.contains(&"tv_prefs_name_limit".to_string()));
        assert!(ids.contains(&"tv_prefs_rank".to_string()));
        assert!(ids.contains(&"tv_prefs_delete".to_string()));
        assert!(!ids.contains(&"tv_prefs_apply_lane".to_string()));
    }

    #[tokio::test]
    async fn prefs_editor_zeigt_vorhandenen_default() {
        let (_db, handler) = panel_handler_for_test().await;
        handler
            .engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 42,
                mode: "ranked".to_string(),
                base_name: "Scrim Lane".to_string(),
                limit: 5,
                min_rank: "archon 2".to_string(),
            })
            .await
            .expect("default");

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_prefs_open".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply_prefs_text(&reply),
            "## ⚙️ Voreinstellungen\n**Modus:** Ranked\n**Name:** Scrim Lane\n**Limit:** 5\n**Rang:** archon 2"
        );
        assert!(!reply_custom_ids(&reply).contains(&"tv_prefs_apply_lane".to_string()));
    }

    #[tokio::test]
    async fn prefs_editor_zeigt_apply_nur_in_eigener_lane() {
        let (_db, handler) = panel_handler_for_test().await;
        handler
            .engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 42,
                mode: "casual".to_string(),
                base_name: "Chill Lane".to_string(),
                limit: 6,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default");
        handler
            .engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: 4242,
                guild_id: 1,
                owner_id: 42,
                initial_owner_id: Some(42),
                base_name: "Lane 1".to_string(),
                category_id: 1289721245281292290,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        handler.engine.rehydrate().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_prefs_open".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply_custom_ids(&reply).contains(&"tv_prefs_apply_lane".to_string()));
    }

    fn reply_component_types(reply: &BridgeReply) -> Vec<u64> {
        let mut out = Vec::new();
        fn walk(value: &Value, out: &mut Vec<u64>) {
            if let Some(kind) = value.get("type").and_then(Value::as_u64) {
                out.push(kind);
            }
            if let Some(children) = value.get("components").and_then(Value::as_array) {
                for child in children {
                    walk(child, out);
                }
            }
            if let Some(children) = value.as_array() {
                for child in children {
                    walk(child, out);
                }
            }
        }
        if let Some(components) = &reply.components {
            walk(components, &mut out);
        }
        out
    }

    #[tokio::test]
    async fn ban_ohne_lane_zeigt_durchsuchbaren_user_select() {
        let (_db, handler) = panel_handler_ohne_lane_for_test().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_ban".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert!(reply_custom_ids(&reply).contains(&"tv_ban_sel".to_string()));
        // Typ 5 = Discord User-Select: tippen + über alle Servermitglieder suchen.
        assert!(reply_component_types(&reply).contains(&5));
    }

    #[tokio::test]
    async fn ban_ohne_lane_traegt_in_banliste_ein() {
        let (_db, handler) = panel_handler_ohne_lane_for_test().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_ban_sel".to_string(),
                guild_id: 1,
                user_id: 42,
                values: vec!["77".to_string()],
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let bans = handler.engine.store.list_bans(42).await.expect("bans");
        assert_eq!(bans, vec![77]);
    }

    #[tokio::test]
    async fn selbst_ban_wird_abgelehnt() {
        let (_db, handler) = panel_handler_ohne_lane_for_test().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_ban_sel".to_string(),
                guild_id: 1,
                user_id: 42,
                values: vec!["42".to_string()],
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply
            .content
            .as_deref()
            .unwrap_or_default()
            .contains("selbst"));
        let bans = handler.engine.store.list_bans(42).await.expect("bans");
        assert!(bans.is_empty());
    }

    #[tokio::test]
    async fn ban_in_eigener_lane_bannt_unter_owner_id() {
        let (_db, handler) = panel_handler_for_test().await;
        handler
            .engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: 4242,
                guild_id: 1,
                owner_id: 42,
                initial_owner_id: Some(42),
                base_name: "Lane 1".to_string(),
                category_id: 1289721245281292290,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        handler.engine.rehydrate().await;

        handler
            .handle(BridgeInteraction {
                custom_id: "tv_ban_sel".to_string(),
                guild_id: 1,
                user_id: 42,
                values: vec!["77".to_string()],
                ..BridgeInteraction::default()
            })
            .await;

        let bans = handler.engine.store.list_bans(42).await.expect("bans");
        assert_eq!(bans, vec![77]);
    }

    #[tokio::test]
    async fn unban_ohne_lane_listet_banliste_und_entfernt() {
        let (_db, handler) = panel_handler_ohne_lane_for_test().await;
        handler.engine.store.add_ban(42, 77).await.expect("ban");

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_unban".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;
        assert!(reply_custom_ids(&reply).contains(&"tv_unban_sel".to_string()));

        handler
            .handle(BridgeInteraction {
                custom_id: "tv_unban_sel".to_string(),
                guild_id: 1,
                user_id: 42,
                values: vec!["77".to_string()],
                ..BridgeInteraction::default()
            })
            .await;
        let bans = handler.engine.store.list_bans(42).await.expect("bans");
        assert!(bans.is_empty());
    }

    #[tokio::test]
    async fn prefs_editor_loescht_default() {
        let (_db, handler) = panel_handler_for_test().await;
        handler
            .engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 42,
                mode: "street_brawl".to_string(),
                base_name: "Brawl".to_string(),
                limit: 4,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default");

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_prefs_delete".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert_eq!(
            reply_prefs_text(&reply),
            "## ⚙️ Voreinstellungen\n_Noch kein Standard gesetzt — wähle unten einen Modus._"
        );
        assert!(handler
            .engine
            .store
            .get_default_preset(42)
            .await
            .expect("load default")
            .is_none());
    }

    #[tokio::test]
    async fn prefs_editor_wendet_default_auf_eigene_lane_an() {
        let (_db, handler) = panel_handler_for_test().await;
        handler
            .engine
            .store
            .save_default_preset(DefaultPresetRecord {
                user_id: 42,
                mode: "casual".to_string(),
                base_name: "Team Lane".to_string(),
                limit: 3,
                min_rank: "unknown".to_string(),
            })
            .await
            .expect("default");
        handler
            .engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: 4242,
                guild_id: 1,
                owner_id: 42,
                initial_owner_id: Some(42),
                base_name: "Lane 1".to_string(),
                category_id: 1289721245281292290,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        handler.engine.rehydrate().await;

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_prefs_apply_lane".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        let content = reply_prefs_text(&reply);
        assert!(content.contains("**Name:** Team Lane"));
        assert!(content.contains("**Limit:** 3"));
        assert_eq!(
            handler.engine.lane_snapshot(4242).await,
            Some(("Team Lane".to_string(), 1289721245281292290))
        );
    }

    #[tokio::test]
    async fn verwaltungs_handler_fremde_lane_liefert_owner_fehler() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let engine = TempVoiceEngine::new(
            TempVoiceConfig {
                guild_id_hint: 1,
                staging_channels: HashSet::new(),
                fixed_lane_ids: HashSet::new(),
                tempvoice_categories: HashSet::new(),
                minrank_categories: HashSet::new(),
                ranked_category_id: 0,
                staging_rules: HashMap::new(),
            },
            TempVoiceStore::new(db.pool().clone()),
            Arc::new(ForeignLanePort),
        );
        engine
            .store
            .upsert_lane(LaneRecord {
                channel_id: 4242,
                guild_id: 1,
                owner_id: 99,
                initial_owner_id: Some(99),
                base_name: "Lane 1".to_string(),
                category_id: 1289721245281292290,
                source_staging_id: None,
            })
            .await
            .expect("lane");
        engine.rehydrate().await;
        let handler = PanelHandler {
            engine,
            lfg: None,
            pending_main_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_default_rank: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            pending_rank_gate: tokio::sync::Mutex::new(std::collections::HashMap::new()),
        };

        let reply = handler
            .handle(BridgeInteraction {
                custom_id: "tv_rename_btn".to_string(),
                guild_id: 1,
                user_id: 42,
                ..BridgeInteraction::default()
            })
            .await;

        assert!(reply.ephemeral);
        assert_eq!(reply.content.as_deref(), Some(NOT_OWNER));
    }
}
