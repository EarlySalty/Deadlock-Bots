use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub const VOICE_UX_CHANNEL_ID: u64 = dl_voice::router::ROUTER_TEXT_CHANNEL_ID;
pub const VOICE_UX_ROUTER_CHAT_CHANNEL_ID: u64 = dl_voice::router::ROUTER_VC_ID;
pub const VOICE_UX_TARGET_CHANNEL_IDS: [u64; 2] =
    [VOICE_UX_CHANNEL_ID, VOICE_UX_ROUTER_CHAT_CHANNEL_ID];
pub const VOICE_UX_PAYLOAD_FORMAT: &str = "1";
pub const VOICE_UX_MESSAGE_ID_PREFIX_BASE: &str = "voice_ux_message_id_";
pub const VOICE_UX_PAYLOAD_FORMAT_KEY_BASE: &str = "voice_ux_payload_format_";
pub const VOICE_UX_PAYLOAD_HASH_KEY_BASE: &str = "voice_ux_payload_hash_";
pub const VOICE_UX_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const VOICE_UX_ACCENT_GOLD: u64 = 0xC8A86B;

pub const VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID: u64 = 1_522_516_664_279_760_999;
pub const VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID: u64 = 1_522_795_818_803_749_904;
pub const VOICE_UX_LFG_FORUM_CHANNEL_ID: u64 = 1_522_769_149_208_821_881;
pub const VOICE_UX_LFG_FORUM_TITLE: &str = "So findest du Mitspieler";
pub const VOICE_UX_LFG_FORUM_THREAD_ID_KEY: &str = "voice_ux_lfg_forum_thread_id";
pub const VOICE_UX_LFG_FORUM_PAYLOAD_HASH_KEY: &str = "voice_ux_lfg_forum_payload_hash";
pub const VOICE_UX_LFG_FORUM_PAYLOAD_FORMAT_KEY: &str = "voice_ux_lfg_forum_payload_format";

pub const VOICE_UX_COMPONENT_ID_GUIDE_CONTAINER: u64 = 34_001;
pub const VOICE_UX_COMPONENT_ID_GUIDE_TEXT: u64 = 34_002;
pub const VOICE_UX_COMPONENT_ID_GUIDE_ACTION_ROW: u64 = 34_003;
pub const VOICE_UX_COMPONENT_ID_GUIDE_DETAIL_BUTTON: u64 = 34_004;
pub const VOICE_UX_COMPONENT_ID_GUIDE_PREFS_BUTTON: u64 = 34_005;
pub const VOICE_UX_COMPONENT_ID_LFG_CONTAINER: u64 = 34_010;
pub const VOICE_UX_COMPONENT_ID_LFG_TEXT: u64 = 34_011;
pub const VOICE_UX_COMPONENT_ID_LFG_ACTION_ROW: u64 = 34_012;
pub const VOICE_UX_COMPONENT_ID_SPAWN_CONTAINER: u64 = 34_020;
pub const VOICE_UX_COMPONENT_ID_SPAWN_TEXT: u64 = 34_021;
pub const VOICE_UX_COMPONENT_ID_SPAWN_ACTION_ROW: u64 = 34_022;
pub const VOICE_UX_COMPONENT_ID_SPAWN_HINT: u64 = 34_023;
pub const VOICE_UX_COMPONENT_ID_MANAGE_CONTAINER: u64 = 34_030;
pub const VOICE_UX_COMPONENT_ID_MANAGE_TEXT: u64 = 34_031;
pub const VOICE_UX_COMPONENT_ID_MANAGE_LANE_CAPTION: u64 = 34_032;
pub const VOICE_UX_COMPONENT_ID_MANAGE_LANE_ROW: u64 = 34_033;
pub const VOICE_UX_COMPONENT_ID_MANAGE_MOD_CAPTION: u64 = 34_034;
pub const VOICE_UX_COMPONENT_ID_MANAGE_MOD_ROW: u64 = 34_035;
pub const VOICE_UX_COMPONENT_ID_FORUM_CONTAINER: u64 = 34_040;
pub const VOICE_UX_COMPONENT_ID_FORUM_TEXT: u64 = 34_041;
pub const VOICE_UX_COMPONENT_ID_FORUM_ACTION_ROW: u64 = 34_042;

pub const VOICE_UX_MARKER_COMPONENT_IDS: &[u64] = &[
    VOICE_UX_COMPONENT_ID_GUIDE_CONTAINER,
    VOICE_UX_COMPONENT_ID_GUIDE_TEXT,
    VOICE_UX_COMPONENT_ID_LFG_CONTAINER,
    VOICE_UX_COMPONENT_ID_LFG_TEXT,
    VOICE_UX_COMPONENT_ID_SPAWN_CONTAINER,
    VOICE_UX_COMPONENT_ID_SPAWN_TEXT,
    VOICE_UX_COMPONENT_ID_MANAGE_CONTAINER,
    VOICE_UX_COMPONENT_ID_MANAGE_TEXT,
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxPublishOutput {
    pub guild_id: u64,
    pub dry_run: bool,
    pub payload_format: String,
    pub warnings: Vec<String>,
    pub targets: Vec<VoiceUxTargetOutput>,
    pub legacy_cleanup_candidates: Vec<VoiceUxLegacyCleanupCandidate>,
    pub deleted_legacy_message_ids: Vec<u64>,
    pub cleared_legacy_kv_keys: Vec<String>,
    pub forum_post: VoiceUxForumPostOutput,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxTargetOutput {
    pub channel_id: u64,
    pub stored_payload_format: Option<String>,
    pub payload_hash: String,
    pub stored_payload_hash: Option<String>,
    pub repost_required: bool,
    pub messages: Vec<VoiceUxMessageOutput>,
    pub stored_message_ids: Vec<u64>,
    pub posted_message_ids: Vec<u64>,
    pub edited_message_ids: Vec<u64>,
    pub deleted_message_ids: Vec<u64>,
    pub pinned_message_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxMessageOutput {
    pub message_index: usize,
    pub message_key: String,
    pub action: String,
    pub stored_message_id: Option<u64>,
    pub message_id: Option<u64>,
    pub payload: VoiceUxMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxForumPostOutput {
    pub forum_channel_id: u64,
    pub title: String,
    pub stored_thread_id: Option<u64>,
    pub thread_id: Option<u64>,
    pub action: String,
    pub payload_hash: String,
    pub stored_payload_hash: Option<String>,
    pub payload: VoiceUxMessagePayload,
    pub pinned: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxMessagePayload {
    pub flags: u64,
    pub allowed_mentions: VoiceUxAllowedMentions,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxAllowedMentions {
    pub parse: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxLegacyCleanupCandidate {
    pub channel_id: u64,
    pub message_id: u64,
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceUxV2Message {
    pub message_id: u64,
    pub author_id: u64,
    pub flags: u64,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceUxLegacyMessage {
    pub channel_id: u64,
    pub message_id: u64,
    pub author_id: u64,
    pub custom_ids: Vec<String>,
}

pub fn voice_ux_message_id_prefix(channel_id: u64) -> String {
    format!("{VOICE_UX_MESSAGE_ID_PREFIX_BASE}{channel_id}_")
}

pub fn voice_ux_payload_format_key(channel_id: u64) -> String {
    format!("{VOICE_UX_PAYLOAD_FORMAT_KEY_BASE}{channel_id}")
}

pub fn voice_ux_payload_hash_key(channel_id: u64) -> String {
    format!("{VOICE_UX_PAYLOAD_HASH_KEY_BASE}{channel_id}")
}

pub fn build_voice_ux_publish_output(
    stored_message_ids: &BTreeMap<u64, Vec<u64>>,
    stored_payload_formats: &BTreeMap<u64, Option<String>>,
    stored_payload_hashes: &BTreeMap<u64, Option<String>>,
    forum_thread_id: Option<u64>,
    forum_stored_payload_hash: Option<&str>,
    dry_run: bool,
) -> Result<VoiceUxPublishOutput, String> {
    let base_messages = voice_ux_messages();
    validate_voice_ux_messages(&base_messages)?;
    let payload_hash = voice_ux_payload_hash(&base_messages)?;
    let mut targets = Vec::new();

    for channel_id in VOICE_UX_TARGET_CHANNEL_IDS {
        let stored_ids = stored_message_ids
            .get(&channel_id)
            .cloned()
            .unwrap_or_default();
        let stored_payload_format = stored_payload_formats.get(&channel_id).cloned().flatten();
        let stored_payload_hash = stored_payload_hashes.get(&channel_id).cloned().flatten();
        let repost_required = !voice_ux_storage_matches(
            stored_payload_format.as_deref(),
            &stored_ids,
            base_messages.len(),
        );
        let hash_matches = stored_payload_hash.as_deref() == Some(payload_hash.as_str());
        let messages = base_messages
            .iter()
            .cloned()
            .enumerate()
            .map(|(message_index, mut message)| {
                let stored_message_id = stored_ids.get(message_index).copied();
                message.message_index = message_index;
                message.stored_message_id = stored_message_id;
                message.message_id = stored_message_id;
                message.action = if dry_run {
                    planned_voice_ux_action(stored_message_id, repost_required, hash_matches)
                } else {
                    "pending".to_string()
                };
                message
            })
            .collect();
        targets.push(VoiceUxTargetOutput {
            channel_id,
            stored_payload_format,
            payload_hash: payload_hash.clone(),
            stored_payload_hash,
            repost_required,
            messages,
            stored_message_ids: stored_ids,
            posted_message_ids: Vec::new(),
            edited_message_ids: Vec::new(),
            deleted_message_ids: Vec::new(),
            pinned_message_ids: Vec::new(),
        });
    }

    let forum_payload = voice_ux_forum_payload();
    let forum_payload_hash = voice_ux_payload_hash(&[VoiceUxMessageOutput {
        message_index: 0,
        message_key: "lfg_forum".to_string(),
        action: String::new(),
        stored_message_id: None,
        message_id: None,
        payload: forum_payload.clone(),
    }])?;
    let forum_hash_matches = forum_stored_payload_hash == Some(forum_payload_hash.as_str());
    let forum_action = if dry_run {
        match (forum_thread_id, forum_hash_matches) {
            (Some(_), true) => "planned_no_op",
            (Some(_), false) => "planned_edit",
            (None, _) => "planned_post",
        }
    } else {
        "pending"
    };

    Ok(VoiceUxPublishOutput {
        guild_id: dl_server_as_code::DEFAULT_GUILD_ID,
        dry_run,
        payload_format: VOICE_UX_PAYLOAD_FORMAT.to_string(),
        warnings: Vec::new(),
        targets,
        legacy_cleanup_candidates: Vec::new(),
        deleted_legacy_message_ids: Vec::new(),
        cleared_legacy_kv_keys: Vec::new(),
        forum_post: VoiceUxForumPostOutput {
            forum_channel_id: VOICE_UX_LFG_FORUM_CHANNEL_ID,
            title: VOICE_UX_LFG_FORUM_TITLE.to_string(),
            stored_thread_id: forum_thread_id,
            thread_id: forum_thread_id,
            action: forum_action.to_string(),
            payload_hash: forum_payload_hash,
            stored_payload_hash: forum_stored_payload_hash.map(str::to_string),
            payload: forum_payload,
            pinned: false,
            warnings: Vec::new(),
        },
    })
}

pub fn voice_ux_storage_matches(
    stored_payload_format: Option<&str>,
    stored_message_ids: &[u64],
    expected_message_count: usize,
) -> bool {
    stored_payload_format == Some(VOICE_UX_PAYLOAD_FORMAT)
        && stored_message_ids.len() == expected_message_count
}

pub fn voice_ux_target_payload_is_unchanged(target: &VoiceUxTargetOutput) -> bool {
    target.stored_payload_hash.as_deref() == Some(target.payload_hash.as_str())
}

pub fn voice_ux_forum_payload_is_unchanged(forum: &VoiceUxForumPostOutput) -> bool {
    forum.stored_payload_hash.as_deref() == Some(forum.payload_hash.as_str())
}

pub fn adopt_voice_ux_message_ids(
    target: &mut VoiceUxTargetOutput,
    discovered_message_ids: &[u64],
    confirm: bool,
) {
    target.stored_message_ids = discovered_message_ids.to_vec();
    target.repost_required = false;
    for (message_index, message_id) in discovered_message_ids.iter().copied().enumerate() {
        if let Some(message) = target.messages.get_mut(message_index) {
            message.stored_message_id = Some(message_id);
            message.message_id = Some(message_id);
            if !confirm {
                message.action = "planned_adopted_edit".to_string();
            }
        }
    }
}

pub fn voice_ux_payload_hash(messages: &[VoiceUxMessageOutput]) -> Result<String, String> {
    let raw = serde_json::to_vec(messages)
        .map_err(|err| format!("Voice-UX-Payload konnte nicht serialisiert werden: {err}"))?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

pub fn is_voice_ux_v2_message(message: &VoiceUxV2Message, bot_user_id: u64) -> bool {
    message.author_id == bot_user_id
        && message.flags & VOICE_UX_COMPONENTS_V2_FLAG != 0
        && has_voice_ux_v2_marker(&message.components)
}

pub fn is_voice_ux_legacy_cleanup_candidate(
    message: &VoiceUxLegacyMessage,
    bot_user_id: u64,
) -> bool {
    if message.author_id != bot_user_id {
        return false;
    }
    match message.message_id {
        VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID => message
            .custom_ids
            .iter()
            .any(|custom_id| custom_id.starts_with("router_spawn_")),
        VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID => message.custom_ids.iter().any(|custom_id| {
            custom_id == dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID
                || custom_id == dl_voice::lfg_panel::LFG_WATCH_START_CUSTOM_ID
        }),
        _ => false,
    }
}

pub fn has_voice_ux_v2_marker(components: &[Value]) -> bool {
    collect_component_ids(components)
        .iter()
        .any(|component_id| VOICE_UX_MARKER_COMPONENT_IDS.contains(component_id))
}

pub fn collect_component_ids(components: &[Value]) -> Vec<u64> {
    let mut ids = Vec::new();
    for component in components {
        collect_component_ids_from_value(component, &mut ids);
    }
    ids
}

pub fn collect_component_custom_ids(components: &[Value]) -> Vec<String> {
    let mut ids = Vec::new();
    for component in components {
        collect_component_custom_ids_from_value(component, &mut ids);
    }
    ids
}

fn collect_component_ids_from_value(value: &Value, ids: &mut Vec<u64>) {
    if let Some(id) = value.get("id").and_then(Value::as_u64) {
        ids.push(id);
    }
    if let Some(children) = value.get("components").and_then(Value::as_array) {
        for child in children {
            collect_component_ids_from_value(child, ids);
        }
    }
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        for item in items {
            collect_component_ids_from_value(item, ids);
        }
    }
}

fn collect_component_custom_ids_from_value(value: &Value, ids: &mut Vec<String>) {
    if let Some(custom_id) = value.get("custom_id").and_then(Value::as_str) {
        ids.push(custom_id.to_string());
    }
    if let Some(children) = value.get("components").and_then(Value::as_array) {
        for child in children {
            collect_component_custom_ids_from_value(child, ids);
        }
    }
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        for item in items {
            collect_component_custom_ids_from_value(item, ids);
        }
    }
}

fn planned_voice_ux_action(
    stored_message_id: Option<u64>,
    repost_required: bool,
    hash_matches: bool,
) -> String {
    match (stored_message_id, repost_required, hash_matches) {
        (Some(_), false, true) => "planned_no_op",
        (Some(_), false, false) => "planned_edit",
        (Some(_), true, _) => "planned_repost",
        (None, _, _) => "planned_post",
    }
    .to_string()
}

fn validate_voice_ux_messages(messages: &[VoiceUxMessageOutput]) -> Result<(), String> {
    for message in messages {
        let text_chars = text_display_chars(&message.payload.components);
        if text_chars > 3_500 {
            return Err(format!(
                "Voice-UX-Message `{}` ist zu lang ({text_chars} Zeichen)",
                message.message_key
            ));
        }
    }
    Ok(())
}

fn text_display_chars(components: &[Value]) -> usize {
    components.iter().map(text_display_chars_from_value).sum()
}

fn text_display_chars_from_value(value: &Value) -> usize {
    let own = if value.get("type").and_then(Value::as_u64) == Some(10) {
        value
            .get("content")
            .and_then(Value::as_str)
            .map(str::chars)
            .map(Iterator::count)
            .unwrap_or(0)
    } else {
        0
    };
    let children = value
        .get("components")
        .and_then(Value::as_array)
        .map(|children| children.iter().map(text_display_chars_from_value).sum())
        .unwrap_or(0);
    own + children
}

fn voice_ux_messages() -> Vec<VoiceUxMessageOutput> {
    vec![
        message("guide", guide_payload()),
        message(
            "lfg",
            lfg_payload(
                VOICE_UX_COMPONENT_ID_LFG_CONTAINER,
                VOICE_UX_COMPONENT_ID_LFG_TEXT,
                VOICE_UX_COMPONENT_ID_LFG_ACTION_ROW,
            ),
        ),
        message("spawn", spawn_payload()),
        message("manage", manage_payload()),
    ]
}

fn voice_ux_forum_payload() -> VoiceUxMessagePayload {
    lfg_payload(
        VOICE_UX_COMPONENT_ID_FORUM_CONTAINER,
        VOICE_UX_COMPONENT_ID_FORUM_TEXT,
        VOICE_UX_COMPONENT_ID_FORUM_ACTION_ROW,
    )
}

fn message(message_key: &str, payload: VoiceUxMessagePayload) -> VoiceUxMessageOutput {
    VoiceUxMessageOutput {
        message_index: 0,
        message_key: message_key.to_string(),
        action: String::new(),
        stored_message_id: None,
        message_id: None,
        payload,
    }
}

fn guide_payload() -> VoiceUxMessagePayload {
    payload(vec![container(
        VOICE_UX_COMPONENT_ID_GUIDE_CONTAINER,
        vec![
            text_display(
                VOICE_UX_COMPONENT_ID_GUIDE_TEXT,
                format!(
                    "{}\n{}",
                    dl_voice::router::VOICE_GUIDE_TITLE,
                    dl_voice::router::VOICE_GUIDE_BODY
                ),
            ),
            action_row(
                VOICE_UX_COMPONENT_ID_GUIDE_ACTION_ROW,
                vec![
                    button_with_id(
                        VOICE_UX_COMPONENT_ID_GUIDE_DETAIL_BUTTON,
                        dl_voice::router::VOICE_GUIDE_DETAIL_BUTTON,
                        1,
                        "voice:guide:detail",
                    ),
                    button_with_id(
                        VOICE_UX_COMPONENT_ID_GUIDE_PREFS_BUTTON,
                        dl_voice::router::VOICE_PREFS_BUTTON,
                        2,
                        "tv_prefs_open",
                    ),
                ],
            ),
        ],
    )])
}

fn lfg_payload(container_id: u64, text_id: u64, row_id: u64) -> VoiceUxMessagePayload {
    payload(vec![container(
        container_id,
        vec![
            text_display(text_id, dl_voice::lfg_panel::LFG_PANEL_BODY.to_string()),
            action_row(
                row_id,
                vec![
                    emoji_button(
                        dl_voice::lfg_panel::LFG_PANEL_BUTTON,
                        1,
                        dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID,
                        dl_voice::lfg_panel::LFG_EMOJI_SEARCH,
                    ),
                    button(
                        dl_voice::lfg_panel::LFG_WATCH_PANEL_BUTTON,
                        2,
                        dl_voice::lfg_panel::LFG_WATCH_START_CUSTOM_ID,
                    ),
                ],
            ),
        ],
    )])
}

fn spawn_payload() -> VoiceUxMessagePayload {
    payload(vec![container(
        VOICE_UX_COMPONENT_ID_SPAWN_CONTAINER,
        vec![
            text_display(
                VOICE_UX_COMPONENT_ID_SPAWN_TEXT,
                "**Lane erstellen**\nRanked = verifizierter Rang".to_string(),
            ),
            action_row(
                VOICE_UX_COMPONENT_ID_SPAWN_ACTION_ROW,
                dl_voice::router::router_modes()
                    .iter()
                    .map(|mode| {
                        emoji_button(
                            mode.label,
                            mode.style,
                            &format!("router_spawn_{}", mode.id),
                            mode.emoji,
                        )
                    })
                    .collect(),
            ),
            text_display(
                VOICE_UX_COMPONENT_ID_SPAWN_HINT,
                "-# <:dl_ranked:1522518271306366996> Ranked nur mit verifiziertem Rang".to_string(),
            ),
        ],
    )])
}

fn manage_payload() -> VoiceUxMessagePayload {
    payload(vec![container(
        VOICE_UX_COMPONENT_ID_MANAGE_CONTAINER,
        vec![
            text_display(
                VOICE_UX_COMPONENT_ID_MANAGE_TEXT,
                dl_voice::router::ROUTER_PANEL_MANAGE_INTRO.to_string(),
            ),
            text_display(
                VOICE_UX_COMPONENT_ID_MANAGE_LANE_CAPTION,
                dl_voice::router::ROUTER_PANEL_LANE_CAPTION.to_string(),
            ),
            action_row(
                VOICE_UX_COMPONENT_ID_MANAGE_LANE_ROW,
                vec![
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_CLAIM,
                        3,
                        "tv_owner_claim",
                        dl_voice::router::ROUTER_EMOJI_CROWN,
                    ),
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_RENAME,
                        2,
                        "tv_rename_btn",
                        dl_voice::router::ROUTER_EMOJI_RENAME,
                    ),
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_LIMIT,
                        2,
                        "tv_limit_btn",
                        dl_voice::router::ROUTER_EMOJI_LIMIT,
                    ),
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_MODE,
                        2,
                        "tv_mode_switch_btn",
                        dl_voice::router::ROUTER_EMOJI_MODE,
                    ),
                    button(dl_voice::router::VOICE_PREFS_BUTTON, 2, "tv_prefs_open"),
                ],
            ),
            text_display(
                VOICE_UX_COMPONENT_ID_MANAGE_MOD_CAPTION,
                dl_voice::router::ROUTER_PANEL_MOD_CAPTION.to_string(),
            ),
            action_row(
                VOICE_UX_COMPONENT_ID_MANAGE_MOD_ROW,
                vec![
                    button("💾 Presets", 2, "tv_presets"),
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_KICK,
                        4,
                        "tv_kick",
                        dl_voice::router::ROUTER_EMOJI_KICK,
                    ),
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_BAN,
                        4,
                        "tv_ban",
                        dl_voice::router::ROUTER_EMOJI_BAN,
                    ),
                    emoji_button(
                        dl_voice::router::ROUTER_BUTTON_UNBAN,
                        2,
                        "tv_unban",
                        dl_voice::router::ROUTER_EMOJI_UNBAN,
                    ),
                ],
            ),
        ],
    )])
}

fn payload(components: Vec<Value>) -> VoiceUxMessagePayload {
    VoiceUxMessagePayload {
        flags: VOICE_UX_COMPONENTS_V2_FLAG,
        allowed_mentions: VoiceUxAllowedMentions { parse: Vec::new() },
        components,
    }
}

fn container(id: u64, components: Vec<Value>) -> Value {
    json!({
        "type": 17,
        "id": id,
        "accent_color": VOICE_UX_ACCENT_GOLD,
        "components": components,
    })
}

fn text_display(id: u64, content: String) -> Value {
    json!({
        "type": 10,
        "id": id,
        "content": content,
    })
}

fn action_row(id: u64, components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "id": id,
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

fn button_with_id(id: u64, label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "id": id,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
}

fn emoji_button(label: &str, style: u8, custom_id: &str, emoji: (&str, &str)) -> Value {
    json!({
        "type": 2,
        "style": style,
        "label": label,
        "custom_id": custom_id,
        "emoji": { "name": emoji.0, "id": emoji.1 },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_ux_baut_vier_messages_in_spec_reihenfolge() {
        let output = build_voice_ux_publish_output(
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .expect("output");
        assert_eq!(output.targets.len(), 2);
        for target in &output.targets {
            let keys: Vec<&str> = target
                .messages
                .iter()
                .map(|message| message.message_key.as_str())
                .collect();
            assert_eq!(keys, vec!["guide", "lfg", "spawn", "manage"]);
            assert!(target.messages.iter().all(|message| message
                .payload
                .allowed_mentions
                .parse
                .is_empty()));
            assert_eq!(
                target.messages[0].payload.components[0]["components"][0]["content"],
                format!(
                    "{}\n{}",
                    dl_voice::router::VOICE_GUIDE_TITLE,
                    dl_voice::router::VOICE_GUIDE_BODY
                )
            );
            assert_eq!(target.messages[0].action, "planned_post");
        }
    }

    #[test]
    fn voice_ux_doppel_apply_wird_planned_no_op() {
        let ids = BTreeMap::from([
            (VOICE_UX_CHANNEL_ID, vec![1, 2, 3, 4]),
            (VOICE_UX_ROUTER_CHAT_CHANNEL_ID, vec![5, 6, 7, 8]),
        ]);
        let first = build_voice_ux_publish_output(
            &ids,
            &BTreeMap::new(),
            &BTreeMap::new(),
            Some(9),
            None,
            true,
        )
        .expect("first");
        let hash = first.targets[0].payload_hash.clone();
        let formats = BTreeMap::from([
            (
                VOICE_UX_CHANNEL_ID,
                Some(VOICE_UX_PAYLOAD_FORMAT.to_string()),
            ),
            (
                VOICE_UX_ROUTER_CHAT_CHANNEL_ID,
                Some(VOICE_UX_PAYLOAD_FORMAT.to_string()),
            ),
        ]);
        let hashes = BTreeMap::from([
            (VOICE_UX_CHANNEL_ID, Some(hash.clone())),
            (VOICE_UX_ROUTER_CHAT_CHANNEL_ID, Some(hash)),
        ]);
        let same = build_voice_ux_publish_output(
            &ids,
            &formats,
            &hashes,
            Some(9),
            first.forum_post.stored_payload_hash.as_deref(),
            true,
        )
        .expect("same");
        assert!(same.targets.iter().all(|target| target
            .messages
            .iter()
            .all(|message| message.action == "planned_no_op")));
    }

    #[test]
    fn voice_ux_legacy_cleanup_prueft_autor_und_custom_id() {
        let bot_id = 42;
        let router = VoiceUxLegacyMessage {
            channel_id: VOICE_UX_CHANNEL_ID,
            message_id: VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID,
            author_id: bot_id,
            custom_ids: vec!["router_spawn_casual".to_string()],
        };
        assert!(is_voice_ux_legacy_cleanup_candidate(&router, bot_id));
        let foreign = VoiceUxLegacyMessage {
            author_id: 99,
            ..router.clone()
        };
        assert!(!is_voice_ux_legacy_cleanup_candidate(&foreign, bot_id));
        let lfg = VoiceUxLegacyMessage {
            channel_id: VOICE_UX_CHANNEL_ID,
            message_id: VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID,
            author_id: bot_id,
            custom_ids: vec![dl_voice::lfg_panel::LFG_CREATE_START_CUSTOM_ID.to_string()],
        };
        assert!(is_voice_ux_legacy_cleanup_candidate(&lfg, bot_id));
    }
}
