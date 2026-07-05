use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

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
pub const VOICE_UX_BANNER_DIR: &str = "assets/welcome-banners";
pub const VOICE_UX_GUIDE_BANNER_FILENAME: &str = "router-hero.png";
pub const VOICE_UX_LFG_BANNER_FILENAME: &str = "divider-mitspieler-finden.png";
pub const VOICE_UX_SPAWN_BANNER_FILENAME: &str = "divider-lane-erstellen.png";
pub const VOICE_UX_MANAGE_BANNER_FILENAME: &str = "divider-lane-verwalten.png";

pub const VOICE_UX_OLD_ROUTER_PANEL_MESSAGE_ID: u64 = 1_522_516_664_279_760_999;
pub const VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID: u64 = 1_522_795_818_803_789_904;
pub const VOICE_UX_LFG_FORUM_CHANNEL_ID: u64 = 1_522_769_149_208_821_881;
pub const VOICE_UX_LFG_FORUM_TITLE: &str = "So findest du Mitspieler";
pub const VOICE_UX_LFG_FORUM_INFO_TAG_ID: &str = "1523260926797676586";
pub const VOICE_UX_LFG_FORUM_THREAD_ID_KEY: &str = "voice_ux_lfg_forum_thread_id";
pub const VOICE_UX_LFG_FORUM_PAYLOAD_HASH_KEY: &str = "voice_ux_lfg_forum_payload_hash";
pub const VOICE_UX_LFG_FORUM_PAYLOAD_FORMAT_KEY: &str = "voice_ux_lfg_forum_payload_format";

pub const VOICE_UX_COMPONENT_ID_GUIDE_MEDIA: u64 = 34_000;
pub const VOICE_UX_COMPONENT_ID_GUIDE_CONTAINER: u64 = 34_001;
pub const VOICE_UX_COMPONENT_ID_GUIDE_TEXT: u64 = 34_002;
pub const VOICE_UX_COMPONENT_ID_GUIDE_ACTION_ROW: u64 = 34_003;
pub const VOICE_UX_COMPONENT_ID_GUIDE_DETAIL_BUTTON: u64 = 34_004;
pub const VOICE_UX_COMPONENT_ID_GUIDE_PREFS_BUTTON: u64 = 34_005;
pub const VOICE_UX_COMPONENT_ID_LFG_MEDIA: u64 = 34_009;
pub const VOICE_UX_COMPONENT_ID_LFG_CONTAINER: u64 = 34_010;
pub const VOICE_UX_COMPONENT_ID_LFG_TEXT: u64 = 34_011;
pub const VOICE_UX_COMPONENT_ID_LFG_ACTION_ROW: u64 = 34_012;
pub const VOICE_UX_COMPONENT_ID_SPAWN_MEDIA: u64 = 34_019;
pub const VOICE_UX_COMPONENT_ID_SPAWN_CONTAINER: u64 = 34_020;
pub const VOICE_UX_COMPONENT_ID_SPAWN_TEXT: u64 = 34_021;
pub const VOICE_UX_COMPONENT_ID_SPAWN_ACTION_ROW: u64 = 34_022;
pub const VOICE_UX_COMPONENT_ID_SPAWN_HINT: u64 = 34_023;
pub const VOICE_UX_COMPONENT_ID_MANAGE_MEDIA: u64 = 34_029;
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
    pub banner: Option<VoiceUxBannerOutput>,
    pub payload: VoiceUxMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxBannerOutput {
    pub filename: String,
    pub relative_path: String,
    pub present: bool,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<VoiceUxPayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxAllowedMentions {
    pub parse: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VoiceUxPayloadAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
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
    repo_root: &Path,
    stored_message_ids: &BTreeMap<u64, Vec<u64>>,
    stored_payload_formats: &BTreeMap<u64, Option<String>>,
    stored_payload_hashes: &BTreeMap<u64, Option<String>>,
    forum_thread_id: Option<u64>,
    forum_stored_payload_hash: Option<&str>,
    dry_run: bool,
) -> Result<VoiceUxPublishOutput, String> {
    let mut warnings = Vec::new();
    let base_messages = voice_ux_messages(repo_root, &mut warnings);
    validate_voice_ux_messages(&base_messages)?;
    let payload_hash = voice_ux_payload_hash(repo_root, &base_messages)?;
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
    let forum_payload_hash = voice_ux_payload_hash(
        repo_root,
        &[VoiceUxMessageOutput {
            message_index: 0,
            message_key: "lfg_forum".to_string(),
            action: String::new(),
            stored_message_id: None,
            message_id: None,
            payload: forum_payload.clone(),
            banner: None,
        }],
    )?;
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
        warnings,
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

pub fn voice_ux_payload_hash(
    repo_root: &Path,
    messages: &[VoiceUxMessageOutput],
) -> Result<String, String> {
    let payloads = messages
        .iter()
        .map(|message| {
            let attachment_hashes = message
                .payload
                .attachments
                .iter()
                .map(|attachment| {
                    let path = repo_root.join(&attachment.relative_path);
                    let bytes = fs::read(&path).map_err(|err| {
                        format!(
                            "Voice-UX-Attachment `{}` konnte fuer Hash nicht gelesen werden: {err}",
                            path.display()
                        )
                    })?;
                    Ok(VoiceUxPayloadHashAttachment {
                        id: attachment.id,
                        filename: attachment.filename.as_str(),
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(VoiceUxPayloadHashMessage {
                message,
                attachment_hashes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let raw = serde_json::to_vec(&payloads)
        .map_err(|err| format!("Voice-UX-Payload konnte nicht serialisiert werden: {err}"))?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

#[derive(Serialize)]
struct VoiceUxPayloadHashMessage<'a> {
    message: &'a VoiceUxMessageOutput,
    attachment_hashes: Vec<VoiceUxPayloadHashAttachment<'a>>,
}

#[derive(Serialize)]
struct VoiceUxPayloadHashAttachment<'a> {
    id: u8,
    filename: &'a str,
    sha256: String,
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

fn voice_ux_messages(repo_root: &Path, warnings: &mut Vec<String>) -> Vec<VoiceUxMessageOutput> {
    let guide_banner =
        optional_voice_ux_banner(repo_root, VOICE_UX_GUIDE_BANNER_FILENAME, warnings);
    let lfg_banner = optional_voice_ux_banner(repo_root, VOICE_UX_LFG_BANNER_FILENAME, warnings);
    let spawn_banner =
        optional_voice_ux_banner(repo_root, VOICE_UX_SPAWN_BANNER_FILENAME, warnings);
    let manage_banner =
        optional_voice_ux_banner(repo_root, VOICE_UX_MANAGE_BANNER_FILENAME, warnings);
    vec![
        message(
            "guide",
            guide_banner.clone(),
            guide_payload(guide_banner.as_ref()),
        ),
        message(
            "lfg",
            lfg_banner.clone(),
            lfg_payload(
                VOICE_UX_COMPONENT_ID_LFG_MEDIA,
                VOICE_UX_COMPONENT_ID_LFG_CONTAINER,
                VOICE_UX_COMPONENT_ID_LFG_TEXT,
                VOICE_UX_COMPONENT_ID_LFG_ACTION_ROW,
                lfg_banner.as_ref(),
            ),
        ),
        message(
            "spawn",
            spawn_banner.clone(),
            spawn_payload(spawn_banner.as_ref()),
        ),
        message(
            "manage",
            manage_banner.clone(),
            manage_payload(manage_banner.as_ref()),
        ),
    ]
}

fn voice_ux_forum_payload() -> VoiceUxMessagePayload {
    lfg_payload(
        0,
        VOICE_UX_COMPONENT_ID_FORUM_CONTAINER,
        VOICE_UX_COMPONENT_ID_FORUM_TEXT,
        VOICE_UX_COMPONENT_ID_FORUM_ACTION_ROW,
        None,
    )
}

fn message(
    message_key: &str,
    banner: Option<VoiceUxBannerOutput>,
    payload: VoiceUxMessagePayload,
) -> VoiceUxMessageOutput {
    VoiceUxMessageOutput {
        message_index: 0,
        message_key: message_key.to_string(),
        action: String::new(),
        stored_message_id: None,
        message_id: None,
        banner,
        payload,
    }
}

fn guide_payload(banner: Option<&VoiceUxBannerOutput>) -> VoiceUxMessagePayload {
    payload_with_optional_banner(
        VOICE_UX_COMPONENT_ID_GUIDE_MEDIA,
        banner,
        container(
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
        ),
    )
}

fn lfg_payload(
    media_id: u64,
    container_id: u64,
    text_id: u64,
    row_id: u64,
    banner: Option<&VoiceUxBannerOutput>,
) -> VoiceUxMessagePayload {
    payload_with_optional_banner(
        media_id,
        banner,
        container(
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
        ),
    )
}

fn spawn_payload(banner: Option<&VoiceUxBannerOutput>) -> VoiceUxMessagePayload {
    payload_with_optional_banner(
        VOICE_UX_COMPONENT_ID_SPAWN_MEDIA,
        banner,
        container(
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
                    "-# <:dl_ranked:1522518271306366996> Ranked nur mit verifiziertem Rang"
                        .to_string(),
                ),
            ],
        ),
    )
}

fn manage_payload(banner: Option<&VoiceUxBannerOutput>) -> VoiceUxMessagePayload {
    payload_with_optional_banner(
        VOICE_UX_COMPONENT_ID_MANAGE_MEDIA,
        banner,
        container(
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
        ),
    )
}

fn payload_with_optional_banner(
    media_id: u64,
    banner: Option<&VoiceUxBannerOutput>,
    container: Value,
) -> VoiceUxMessagePayload {
    let mut components = Vec::new();
    let mut attachments = Vec::new();
    if let Some(banner) = banner {
        components.push(media_gallery(media_id, &banner.filename));
        attachments.push(VoiceUxPayloadAttachment {
            id: 0,
            filename: banner.filename.clone(),
            relative_path: banner.relative_path.clone(),
        });
    }
    components.push(container);
    payload(components, attachments)
}

fn payload(
    components: Vec<Value>,
    attachments: Vec<VoiceUxPayloadAttachment>,
) -> VoiceUxMessagePayload {
    VoiceUxMessagePayload {
        flags: VOICE_UX_COMPONENTS_V2_FLAG,
        allowed_mentions: VoiceUxAllowedMentions { parse: Vec::new() },
        components,
        attachments,
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

fn media_gallery(id: u64, filename: &str) -> Value {
    json!({
        "type": 12,
        "id": id,
        "items": [{
            "media": {
                "url": format!("attachment://{filename}"),
            },
        }],
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

fn optional_voice_ux_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> Option<VoiceUxBannerOutput> {
    let relative_path = format!("{VOICE_UX_BANNER_DIR}/{filename}");
    let path = repo_root.join(&relative_path);
    if !path.is_file() {
        warnings.push(format!(
            "Voice-UX-Banner `{relative_path}` fehlt; Publish laeuft ohne diesen Banner"
        ));
        return None;
    }
    Some(VoiceUxBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_voice_ux_banner(repo_root: &Path, filename: &str, bytes: &[u8]) {
        let path = repo_root.join(format!("{VOICE_UX_BANNER_DIR}/{filename}"));
        fs::create_dir_all(path.parent().expect("banner parent")).expect("mkdir banner parent");
        fs::write(path, bytes).expect("write banner");
    }

    fn write_all_voice_ux_banners(repo_root: &Path, bytes: &[u8]) {
        for filename in [
            VOICE_UX_GUIDE_BANNER_FILENAME,
            VOICE_UX_LFG_BANNER_FILENAME,
            VOICE_UX_SPAWN_BANNER_FILENAME,
            VOICE_UX_MANAGE_BANNER_FILENAME,
        ] {
            write_voice_ux_banner(repo_root, filename, bytes);
        }
    }

    #[test]
    fn voice_ux_baut_vier_messages_in_spec_reihenfolge() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_all_voice_ux_banners(temp.path(), b"banner");
        let output = build_voice_ux_publish_output(
            temp.path(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .expect("output");
        assert!(output.warnings.is_empty());
        assert_eq!(output.targets.len(), 2);
        for target in &output.targets {
            let keys: Vec<&str> = target
                .messages
                .iter()
                .map(|message| message.message_key.as_str())
                .collect();
            assert_eq!(keys, vec!["guide", "lfg", "spawn", "manage"]);
            let banners = [
                VOICE_UX_GUIDE_BANNER_FILENAME,
                VOICE_UX_LFG_BANNER_FILENAME,
                VOICE_UX_SPAWN_BANNER_FILENAME,
                VOICE_UX_MANAGE_BANNER_FILENAME,
            ];
            for (message, filename) in target.messages.iter().zip(banners) {
                assert_eq!(
                    message
                        .banner
                        .as_ref()
                        .map(|banner| banner.filename.as_str()),
                    Some(filename)
                );
                assert_eq!(message.payload.attachments.len(), 1);
                assert_eq!(message.payload.attachments[0].filename, filename);
                assert_eq!(message.payload.attachments[0].id, 0);
                assert_eq!(message.payload.components[0]["type"], json!(12));
                assert_eq!(
                    message.payload.components[0]["items"][0]["media"]["url"],
                    format!("attachment://{filename}")
                );
                assert_eq!(
                    message.payload.components[1]["accent_color"],
                    json!(VOICE_UX_ACCENT_GOLD)
                );
            }
            assert!(target.messages.iter().all(|message| message
                .payload
                .allowed_mentions
                .parse
                .is_empty()));
            assert_eq!(
                target.messages[0].payload.components[1]["components"][0]["content"],
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
        let temp = tempfile::tempdir().expect("tempdir");
        write_all_voice_ux_banners(temp.path(), b"banner");
        let ids = BTreeMap::from([
            (VOICE_UX_CHANNEL_ID, vec![1, 2, 3, 4]),
            (VOICE_UX_ROUTER_CHAT_CHANNEL_ID, vec![5, 6, 7, 8]),
        ]);
        let first = build_voice_ux_publish_output(
            temp.path(),
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
            temp.path(),
            &ids,
            &formats,
            &hashes,
            Some(9),
            Some(&first.forum_post.payload_hash),
            true,
        )
        .expect("same");
        assert!(same.targets.iter().all(|target| target
            .messages
            .iter()
            .all(|message| message.action == "planned_no_op")));
    }

    #[test]
    fn voice_ux_payload_hash_noop_und_banner_bytes_aendern_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_all_voice_ux_banners(temp.path(), b"banner-one");
        let ids = BTreeMap::from([(VOICE_UX_CHANNEL_ID, vec![1, 2, 3, 4])]);
        let formats = BTreeMap::from([(
            VOICE_UX_CHANNEL_ID,
            Some(VOICE_UX_PAYLOAD_FORMAT.to_string()),
        )]);
        let first = build_voice_ux_publish_output(
            temp.path(),
            &ids,
            &formats,
            &BTreeMap::new(),
            None,
            None,
            true,
        )
        .expect("first");
        let first_hash = first.targets[0].payload_hash.clone();
        let hashes = BTreeMap::from([(VOICE_UX_CHANNEL_ID, Some(first_hash.clone()))]);
        let same =
            build_voice_ux_publish_output(temp.path(), &ids, &formats, &hashes, None, None, true)
                .expect("same");
        assert_eq!(same.targets[0].messages[0].action, "planned_no_op");
        assert!(voice_ux_target_payload_is_unchanged(&same.targets[0]));

        write_voice_ux_banner(temp.path(), VOICE_UX_LFG_BANNER_FILENAME, b"banner-two");
        let changed =
            build_voice_ux_publish_output(temp.path(), &ids, &formats, &hashes, None, None, true)
                .expect("changed");
        assert_eq!(changed.targets[0].messages[0].action, "planned_edit");
        assert_ne!(changed.targets[0].payload_hash, first_hash);
    }

    #[test]
    fn voice_ux_legacy_cleanup_prueft_autor_und_custom_id() {
        let bot_id = 42;
        assert_eq!(VOICE_UX_OLD_LFG_PANEL_MESSAGE_ID, 1_522_795_818_803_789_904);
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
        let lfg_watch_only = VoiceUxLegacyMessage {
            custom_ids: vec![dl_voice::lfg_panel::LFG_WATCH_START_CUSTOM_ID.to_string()],
            ..lfg
        };
        assert!(is_voice_ux_legacy_cleanup_candidate(
            &lfg_watch_only,
            bot_id
        ));
    }
}
