use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const REGELWERK_CHANNEL_ID: u64 = 1_315_684_135_175_716_975;
pub const REGELWERK_CHANNEL_NAME: &str = "regelwerk";
pub const REGELWERK_TEXTS_FILE: &str = "assets/regelwerk_texts.toml";
pub const REGELWERK_BANNER_DIR: &str = "assets/welcome-banners";
pub const REGELWERK_HERO_FILENAME: &str = "regelwerk-hero.png";
pub const REGELWERK_PAYLOAD_FORMAT: &str = "2";
pub const REGELWERK_PAYLOAD_FORMAT_KEY: &str = "regelwerk_v2_payload_format";
pub const REGELWERK_PAYLOAD_HASH_KEY: &str = "regelwerk_v2_payload_hash";
pub const REGELWERK_MESSAGE_ID_PREFIX: &str = "regelwerk_v2_message_id_";
pub const REGELWERK_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const REGELWERK_EPHEMERAL_FLAG: u64 = 1 << 6;
pub const REGELWERK_ACCENT_GOLD: u64 = 0xC8A86B;
pub const REGELWERK_OLD_MESSAGE_ID: u64 = 1_522_405_857_042_759_690;
pub const REGELWERK_LEGACY_CONTENT_PREFIX: &str = "**📜 Regelwerk";

pub const REGELWERK_COMPONENT_ID_HERO_MEDIA: u64 = 32_001;
pub const REGELWERK_COMPONENT_ID_MAIN_CONTAINER: u64 = 32_002;
pub const REGELWERK_COMPONENT_ID_MAIN_TEXT: u64 = 32_003;
pub const REGELWERK_COMPONENT_ID_ACTION_ROW: u64 = 32_004;
pub const REGELWERK_COMPONENT_ID_BUTTON_VERHALTEN: u64 = 32_005;
pub const REGELWERK_COMPONENT_ID_BUTTON_SPIELKONTEXT: u64 = 32_006;
pub const REGELWERK_COMPONENT_ID_BUTTON_MODERATION: u64 = 32_007;
pub const REGELWERK_COMPONENT_ID_BUTTON_WEGWEISER: u64 = 32_008;
pub const REGELWERK_COMPONENT_ID_EPHEMERAL_CONTAINER: u64 = 32_009;
pub const REGELWERK_COMPONENT_ID_EPHEMERAL_TEXT: u64 = 32_010;

pub const REGELWERK_MARKER_COMPONENT_IDS: &[u64] = &[
    REGELWERK_COMPONENT_ID_HERO_MEDIA,
    REGELWERK_COMPONENT_ID_MAIN_CONTAINER,
    REGELWERK_COMPONENT_ID_MAIN_TEXT,
    REGELWERK_COMPONENT_ID_ACTION_ROW,
];

pub const REGELWERK_TEXT_CHAR_BUDGET: usize = 3500;
pub const REGELWERK_COMPONENT_BUDGET: usize = 35;
pub const REGELWERK_ATTACHMENT_BUDGET: usize = 9;
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;

pub const REGELWERK_VERHALTEN_CUSTOM_ID: &str = "regelwerk:show:verhalten";
pub const REGELWERK_SPIELKONTEXT_CUSTOM_ID: &str = "regelwerk:show:spielkontext";
pub const REGELWERK_MODERATION_CUSTOM_ID: &str = "regelwerk:show:moderation";
pub const REGELWERK_WEGWEISER_CUSTOM_ID: &str = "regelwerk:show:wegweiser";
pub const REGELWERK_SHOW_PREFIX: &str = "regelwerk:show:";

pub const REGELWERK_MAIN_TITLE: &str = "**📜 Regelwerk · Deutsche Deadlock Community**";
pub const REGELWERK_MAIN_BODY: &str = "Die Kurzfassung: **Sei kein Arschloch** 😄 — Respekt gegenüber allen, keine Hassrede, kein Spam, keine Fremdwerbung.\n\nFür die Details tipp unten auf einen Button — die Antwort siehst nur du.";
pub const REGELWERK_BUTTON_VERHALTEN_LABEL: &str = "🤝 Verhalten";
pub const REGELWERK_BUTTON_SPIELKONTEXT_LABEL: &str = "🎮 Ton & Trash-Talk";
pub const REGELWERK_BUTTON_MODERATION_LABEL: &str = "🛡️ Moderation";
pub const REGELWERK_BUTTON_WEGWEISER_LABEL: &str = "🧭 Wichtige Kanäle";
pub const REGELWERK_SECTION_VERHALTEN: &str = "**🤝 Verhalten**\n- Respekt gegenüber allen — keine Beleidigungen, Diskriminierung oder persönlichen Angriffe\n- Keine Hassrede. Kein NSFW außerhalb der dafür markierten Kanäle. Kein Spam, keine Fremdwerbung\n- Privatsphäre respektieren — keine fremden Daten posten\n- Schädliche Inhalte (Viren, IP-Grabber, Scam-Links) = sofortiger permanenter Bann";
pub const REGELWERK_SECTION_SPIELKONTEXT: &str = r#"**🎮 Im Spielkontext erlaubt**
Situatives Trash-Talking, Sarkasmus, Wortspiele sind okay — solange es nicht persönlich wird. Ohne Mimik und Tonfall kommt Geschriebenes schnell falsch an: check vorher ab, ob alle damit fein sind. Und wenn jemand sagt „lass gut sein" — dann ist gut."#;
pub const REGELWERK_SECTION_MODERATION: &str = "**🛡️ Moderation & Konsequenzen**\nProbleme oder Streit? Ping @Moderator oder @Owner — oder mach ein Ticket in <#1483136301271355532> auf, wenn's diskreter sein soll.\nKonsequenzen je nach Schwere: Verwarnung → Timeout → Ban. Schädliche Inhalte führen direkt zum permanenten Bann.";
pub const REGELWERK_SECTION_WEGWEISER: &str = "**🧭 Schnell zurechtfinden**\n- <#1398021105339334666> — Steam verknüpfen, Rang eintragen\n- <#1483136301271355532> — wenn irgendwas nicht funktioniert (Ticket aufmachen)\n- <#1426220702054355077> — jede Frage ist okay; auch dein Deadlock-Invite bekommst du hier\n- <#1522769149208821881> — Mitspieler finden";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkPublishOutput {
    pub guild_id: u64,
    pub channel_id: u64,
    pub channel_name: String,
    pub dry_run: bool,
    pub payload_format: String,
    pub stored_payload_format: Option<String>,
    pub payload_hash: String,
    pub stored_payload_hash: Option<String>,
    pub repost_required: bool,
    pub warnings: Vec<String>,
    pub messages: Vec<RegelwerkMessageOutput>,
    pub stored_message_ids: Vec<u64>,
    pub posted_message_ids: Vec<u64>,
    pub edited_message_ids: Vec<u64>,
    pub deleted_message_ids: Vec<u64>,
    pub legacy_cleanup_candidate: Option<RegelwerkLegacyCleanupCandidate>,
    pub deleted_legacy_message_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkMessageOutput {
    pub message_index: usize,
    pub message_key: String,
    pub action: String,
    pub stored_message_id: Option<u64>,
    pub message_id: Option<u64>,
    pub banner: Option<RegelwerkBannerOutput>,
    pub payload: RegelwerkMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkBannerOutput {
    pub filename: String,
    pub relative_path: String,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkMessagePayload {
    pub flags: u64,
    pub allowed_mentions: RegelwerkAllowedMentions,
    pub components: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<RegelwerkPayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkAllowedMentions {
    pub parse: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkPayloadAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegelwerkLegacyCleanupCandidate {
    pub message_id: u64,
    pub content_prefix: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegelwerkLegacyMessage {
    pub message_id: u64,
    pub author_id: u64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegelwerkV2Message {
    pub message_id: u64,
    pub author_id: u64,
    pub flags: u64,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRegelwerkConfig {
    texts: ResolvedRegelwerkTexts,
    buttons: ResolvedRegelwerkButtons,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRegelwerkTexts {
    title: String,
    body: String,
    verhalten: String,
    spielkontext: String,
    moderation: String,
    wegweiser: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRegelwerkButtons {
    verhalten: String,
    spielkontext: String,
    moderation: String,
    wegweiser: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegelwerkTextsToml {
    #[serde(default)]
    texts: RegelwerkBodyTextsToml,
    #[serde(default)]
    buttons: RegelwerkButtonsToml,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegelwerkBodyTextsToml {
    title: Option<String>,
    body: Option<String>,
    verhalten: Option<String>,
    spielkontext: Option<String>,
    moderation: Option<String>,
    wegweiser: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegelwerkButtonsToml {
    verhalten: Option<String>,
    spielkontext: Option<String>,
    moderation: Option<String>,
    wegweiser: Option<String>,
}

#[derive(Debug, Clone)]
struct RegelwerkBuiltMessage {
    banner: Option<RegelwerkBannerOutput>,
    payload: RegelwerkMessagePayload,
}

#[cfg(test)]
pub fn regelwerk_message_id_key(message_index: usize) -> String {
    format!("{REGELWERK_MESSAGE_ID_PREFIX}{message_index}")
}

pub fn build_regelwerk_publish_output(
    repo_root: &Path,
    stored_message_ids: &[u64],
    stored_payload_format: Option<&str>,
    stored_payload_hash: Option<&str>,
    dry_run: bool,
) -> Result<RegelwerkPublishOutput, String> {
    let mut warnings = Vec::new();
    let config = load_regelwerk_runtime_config(repo_root, &mut warnings)?;
    let built_messages = regelwerk_messages(repo_root, &config, &mut warnings);

    let mut messages = Vec::new();
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored_message_ids.get(message_index).copied();
        messages.push(RegelwerkMessageOutput {
            message_index,
            message_key: regelwerk_message_key(message_index),
            action: if dry_run {
                planned_regelwerk_action(stored_message_id, false, false)
            } else {
                "pending".to_string()
            },
            stored_message_id,
            message_id: stored_message_id,
            banner: built.banner,
            payload: built.payload,
        });
    }

    for message in &messages {
        validate_regelwerk_message_budget(&message.payload)?;
    }

    let payload_hash = regelwerk_payload_hash(repo_root, &messages)?;
    let repost_required =
        !regelwerk_storage_matches(stored_payload_format, stored_message_ids, messages.len());
    let hash_matches = stored_payload_hash == Some(payload_hash.as_str());
    if dry_run {
        for message in &mut messages {
            message.action =
                planned_regelwerk_action(message.stored_message_id, repost_required, hash_matches);
        }
    }

    Ok(RegelwerkPublishOutput {
        guild_id: dl_server_as_code::DEFAULT_GUILD_ID,
        channel_id: REGELWERK_CHANNEL_ID,
        channel_name: REGELWERK_CHANNEL_NAME.to_string(),
        dry_run,
        payload_format: REGELWERK_PAYLOAD_FORMAT.to_string(),
        stored_payload_format: stored_payload_format.map(str::to_string),
        payload_hash,
        stored_payload_hash: stored_payload_hash.map(str::to_string),
        repost_required,
        warnings,
        messages,
        stored_message_ids: stored_message_ids.to_vec(),
        posted_message_ids: Vec::new(),
        edited_message_ids: Vec::new(),
        deleted_message_ids: Vec::new(),
        legacy_cleanup_candidate: None,
        deleted_legacy_message_ids: Vec::new(),
    })
}

pub fn regelwerk_storage_matches(
    stored_payload_format: Option<&str>,
    stored_message_ids: &[u64],
    expected_message_count: usize,
) -> bool {
    stored_payload_format == Some(REGELWERK_PAYLOAD_FORMAT)
        && stored_message_ids.len() == expected_message_count
}

pub fn regelwerk_payload_is_unchanged(output: &RegelwerkPublishOutput) -> bool {
    output.stored_payload_hash.as_deref() == Some(output.payload_hash.as_str())
}

pub fn regelwerk_payload_hash(
    repo_root: &Path,
    messages: &[RegelwerkMessageOutput],
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
                            "Regelwerk-Attachment `{}` konnte fuer Hash nicht gelesen werden: {err}",
                            path.display()
                        )
                    })?;
                    Ok(RegelwerkPayloadHashAttachment {
                        id: attachment.id,
                        filename: attachment.filename.as_str(),
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(RegelwerkPayloadHashMessage {
                payload: &message.payload,
                attachment_hashes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let raw = serde_json::to_vec(&payloads)
        .map_err(|err| format!("Regelwerk-Payload konnte nicht serialisiert werden: {err}"))?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

#[derive(Serialize)]
struct RegelwerkPayloadHashMessage<'a> {
    payload: &'a RegelwerkMessagePayload,
    attachment_hashes: Vec<RegelwerkPayloadHashAttachment<'a>>,
}

#[derive(Serialize)]
struct RegelwerkPayloadHashAttachment<'a> {
    id: u8,
    filename: &'a str,
    sha256: String,
}

pub fn is_legacy_regelwerk_cleanup_candidate(
    message: &RegelwerkLegacyMessage,
    bot_user_id: u64,
) -> bool {
    message.message_id == REGELWERK_OLD_MESSAGE_ID
        && message.author_id == bot_user_id
        && message.content.starts_with(REGELWERK_LEGACY_CONTENT_PREFIX)
}

pub fn is_regelwerk_v2_message(message: &RegelwerkV2Message, bot_user_id: u64) -> bool {
    message.author_id == bot_user_id
        && message.flags & REGELWERK_COMPONENTS_V2_FLAG != 0
        && has_regelwerk_v2_marker(&message.components)
}

pub fn has_regelwerk_v2_marker(components: &[Value]) -> bool {
    collect_component_ids(components)
        .iter()
        .any(|component_id| REGELWERK_MARKER_COMPONENT_IDS.contains(component_id))
}

pub fn collect_component_ids(components: &[Value]) -> Vec<u64> {
    let mut ids = Vec::new();
    for component in components {
        collect_component_ids_from_value(component, &mut ids);
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

#[cfg(test)]
pub fn collect_component_custom_ids(components: &[Value]) -> Vec<String> {
    let mut custom_ids = Vec::new();
    for component in components {
        collect_component_custom_ids_from_value(component, &mut custom_ids);
    }
    custom_ids
}

#[cfg(test)]
fn collect_component_custom_ids_from_value(value: &Value, custom_ids: &mut Vec<String>) {
    if let Some(custom_id) = value.get("custom_id").and_then(Value::as_str) {
        custom_ids.push(custom_id.to_string());
    }
    if let Some(children) = value.get("components").and_then(Value::as_array) {
        for child in children {
            collect_component_custom_ids_from_value(child, custom_ids);
        }
    }
    if let Some(items) = value.get("items").and_then(Value::as_array) {
        for item in items {
            collect_component_custom_ids_from_value(item, custom_ids);
        }
    }
}

pub fn regelwerk_section_text_for_custom_id(custom_id: &str) -> Option<&'static str> {
    match custom_id {
        REGELWERK_VERHALTEN_CUSTOM_ID => Some(REGELWERK_SECTION_VERHALTEN),
        REGELWERK_SPIELKONTEXT_CUSTOM_ID => Some(REGELWERK_SECTION_SPIELKONTEXT),
        REGELWERK_MODERATION_CUSTOM_ID => Some(REGELWERK_SECTION_MODERATION),
        REGELWERK_WEGWEISER_CUSTOM_ID => Some(REGELWERK_SECTION_WEGWEISER),
        _ => None,
    }
}

pub fn register_components(router: &mut dl_discord::InteractionRouter) {
    router.on_prefix(REGELWERK_SHOW_PREFIX, Arc::new(RegelwerkSectionHandler));
}

struct RegelwerkSectionHandler;

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for RegelwerkSectionHandler {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        regelwerk_section_reply(&interaction.custom_id)
    }
}

pub fn regelwerk_section_reply(custom_id: &str) -> dl_discord::BridgeReply {
    let Some(text) = regelwerk_section_text_for_custom_id(custom_id) else {
        return dl_discord::BridgeReply {
            content: Some("Unbekannter Regelwerk-Abschnitt.".to_string()),
            ephemeral: true,
            allowed_mentions: Some(empty_allowed_mentions_value()),
            ..dl_discord::BridgeReply::default()
        };
    };
    dl_discord::BridgeReply {
        components: Some(json!([container(
            REGELWERK_COMPONENT_ID_EPHEMERAL_CONTAINER,
            vec![text_display(
                REGELWERK_COMPONENT_ID_EPHEMERAL_TEXT,
                text.to_string(),
            )],
        )])),
        ephemeral: true,
        message_flags: Some(REGELWERK_EPHEMERAL_FLAG | REGELWERK_COMPONENTS_V2_FLAG),
        allowed_mentions: Some(empty_allowed_mentions_value()),
        fallback: Some(Box::new(regelwerk_section_fallback_reply(text))),
        ..dl_discord::BridgeReply::default()
    }
}

pub fn regelwerk_section_fallback_reply(text: &str) -> dl_discord::BridgeReply {
    dl_discord::BridgeReply {
        embeds: vec![json!({
            "description": text,
            "color": REGELWERK_ACCENT_GOLD,
        })],
        ephemeral: true,
        allowed_mentions: Some(empty_allowed_mentions_value()),
        ..dl_discord::BridgeReply::default()
    }
}

fn empty_allowed_mentions_value() -> Value {
    json!({ "parse": [] })
}

fn load_regelwerk_runtime_config(
    repo_root: &Path,
    warnings: &mut Vec<String>,
) -> Result<ResolvedRegelwerkConfig, String> {
    let path = repo_root.join(REGELWERK_TEXTS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            warnings.push(format!(
                "Regelwerk-Textdatei `{REGELWERK_TEXTS_FILE}` fehlt; Compile-Defaults werden verwendet"
            ));
            return Ok(ResolvedRegelwerkConfig::from_defaults());
        }
        Err(err) => {
            return Err(format!(
                "Regelwerk-Textdatei `{REGELWERK_TEXTS_FILE}` konnte nicht gelesen werden: {err}"
            ));
        }
    };

    let file = toml::from_str::<RegelwerkTextsToml>(&raw).map_err(|err| {
        format!("Regelwerk-Textdatei `{REGELWERK_TEXTS_FILE}` konnte nicht geparst werden: {err}")
    })?;
    Ok(ResolvedRegelwerkConfig::from_defaults().merge(file))
}

impl ResolvedRegelwerkConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedRegelwerkTexts {
                title: REGELWERK_MAIN_TITLE.to_string(),
                body: REGELWERK_MAIN_BODY.to_string(),
                verhalten: REGELWERK_SECTION_VERHALTEN.to_string(),
                spielkontext: REGELWERK_SECTION_SPIELKONTEXT.to_string(),
                moderation: REGELWERK_SECTION_MODERATION.to_string(),
                wegweiser: REGELWERK_SECTION_WEGWEISER.to_string(),
            },
            buttons: ResolvedRegelwerkButtons {
                verhalten: REGELWERK_BUTTON_VERHALTEN_LABEL.to_string(),
                spielkontext: REGELWERK_BUTTON_SPIELKONTEXT_LABEL.to_string(),
                moderation: REGELWERK_BUTTON_MODERATION_LABEL.to_string(),
                wegweiser: REGELWERK_BUTTON_WEGWEISER_LABEL.to_string(),
            },
        }
    }

    fn merge(mut self, file: RegelwerkTextsToml) -> Self {
        apply_optional(&mut self.texts.title, file.texts.title);
        apply_optional(&mut self.texts.body, file.texts.body);
        apply_optional(&mut self.texts.verhalten, file.texts.verhalten);
        apply_optional(&mut self.texts.spielkontext, file.texts.spielkontext);
        apply_optional(&mut self.texts.moderation, file.texts.moderation);
        apply_optional(&mut self.texts.wegweiser, file.texts.wegweiser);

        apply_optional(&mut self.buttons.verhalten, file.buttons.verhalten);
        apply_optional(&mut self.buttons.spielkontext, file.buttons.spielkontext);
        apply_optional(&mut self.buttons.moderation, file.buttons.moderation);
        apply_optional(&mut self.buttons.wegweiser, file.buttons.wegweiser);
        self
    }
}

fn apply_optional(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn regelwerk_messages(
    repo_root: &Path,
    config: &ResolvedRegelwerkConfig,
    warnings: &mut Vec<String>,
) -> Vec<RegelwerkBuiltMessage> {
    let banner = optional_regelwerk_banner(repo_root, REGELWERK_HERO_FILENAME, warnings);
    let mut components = Vec::new();
    let mut attachments = Vec::new();
    if let Some(banner) = &banner {
        components.push(media_gallery(
            REGELWERK_COMPONENT_ID_HERO_MEDIA,
            &banner.filename,
        ));
        attachments.push(RegelwerkPayloadAttachment {
            id: 0,
            filename: banner.filename.clone(),
            relative_path: banner.relative_path.clone(),
        });
    }

    components.push(container(
        REGELWERK_COMPONENT_ID_MAIN_CONTAINER,
        vec![
            text_display(
                REGELWERK_COMPONENT_ID_MAIN_TEXT,
                section_text(&config.texts.title, &config.texts.body),
            ),
            action_row(
                REGELWERK_COMPONENT_ID_ACTION_ROW,
                vec![
                    button(
                        REGELWERK_COMPONENT_ID_BUTTON_VERHALTEN,
                        &config.buttons.verhalten,
                        2,
                        REGELWERK_VERHALTEN_CUSTOM_ID,
                    ),
                    button(
                        REGELWERK_COMPONENT_ID_BUTTON_SPIELKONTEXT,
                        &config.buttons.spielkontext,
                        2,
                        REGELWERK_SPIELKONTEXT_CUSTOM_ID,
                    ),
                    button(
                        REGELWERK_COMPONENT_ID_BUTTON_MODERATION,
                        &config.buttons.moderation,
                        2,
                        REGELWERK_MODERATION_CUSTOM_ID,
                    ),
                    button(
                        REGELWERK_COMPONENT_ID_BUTTON_WEGWEISER,
                        &config.buttons.wegweiser,
                        2,
                        REGELWERK_WEGWEISER_CUSTOM_ID,
                    ),
                ],
            ),
        ],
    ));

    vec![RegelwerkBuiltMessage {
        banner,
        payload: RegelwerkMessagePayload {
            flags: REGELWERK_COMPONENTS_V2_FLAG,
            allowed_mentions: RegelwerkAllowedMentions { parse: Vec::new() },
            components,
            attachments,
        },
    }]
}

fn section_text(title: &str, body: &str) -> String {
    if title.trim().is_empty() {
        body.to_string()
    } else {
        format!("{title}\n{body}")
    }
}

fn optional_regelwerk_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> Option<RegelwerkBannerOutput> {
    let relative_path = format!("{REGELWERK_BANNER_DIR}/{filename}");
    let path = repo_root.join(&relative_path);
    if !path.is_file() {
        warnings.push(format!(
            "Regelwerk-Banner `{relative_path}` fehlt; Publish laeuft ohne Banner"
        ));
        return None;
    }
    Some(RegelwerkBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present: true,
    })
}

fn planned_regelwerk_action(
    stored_message_id: Option<u64>,
    repost_required: bool,
    hash_matches: bool,
) -> String {
    if repost_required && stored_message_id.is_some() {
        "planned_repost".to_string()
    } else if stored_message_id.is_some() && hash_matches {
        "planned_no_op".to_string()
    } else if stored_message_id.is_some() {
        "planned_edit".to_string()
    } else {
        "planned_post".to_string()
    }
}

fn regelwerk_message_key(message_index: usize) -> String {
    if message_index == 0 {
        "regelwerk".to_string()
    } else {
        format!("regelwerk:{message_index}")
    }
}

fn validate_regelwerk_message_budget(payload: &RegelwerkMessagePayload) -> Result<(), String> {
    let text_chars = text_display_chars(&payload.components);
    if text_chars > REGELWERK_TEXT_CHAR_BUDGET {
        return Err(format!(
            "Regelwerk-Payload ueberschreitet Textbudget: {text_chars}/{REGELWERK_TEXT_CHAR_BUDGET}"
        ));
    }
    if max_text_display_chars(&payload.components) > MESSAGE_TEXT_DISPLAY_CHAR_LIMIT {
        return Err(format!(
            "Regelwerk-Payload ueberschreitet TextDisplay-Limit {MESSAGE_TEXT_DISPLAY_CHAR_LIMIT}"
        ));
    }
    let component_count = component_count(&payload.components);
    if component_count > REGELWERK_COMPONENT_BUDGET {
        return Err(format!(
            "Regelwerk-Payload ueberschreitet Komponentenbudget: {component_count}/{REGELWERK_COMPONENT_BUDGET}"
        ));
    }
    if payload.attachments.len() > REGELWERK_ATTACHMENT_BUDGET {
        return Err(format!(
            "Regelwerk-Payload ueberschreitet Attachmentbudget: {}/{REGELWERK_ATTACHMENT_BUDGET}",
            payload.attachments.len()
        ));
    }
    Ok(())
}

fn text_display_chars(values: &[Value]) -> usize {
    values.iter().map(text_display_chars_in_value).sum()
}

fn text_display_chars_in_value(value: &Value) -> usize {
    let own = if value.get("type").and_then(Value::as_u64) == Some(10) {
        value
            .get("content")
            .and_then(Value::as_str)
            .map_or(0, |content| content.chars().count())
    } else {
        0
    };
    own + value
        .get("components")
        .and_then(Value::as_array)
        .map(|children| children.iter().map(text_display_chars_in_value).sum())
        .unwrap_or(0)
}

fn max_text_display_chars(values: &[Value]) -> usize {
    values
        .iter()
        .map(max_text_display_chars_in_value)
        .max()
        .unwrap_or(0)
}

fn max_text_display_chars_in_value(value: &Value) -> usize {
    let own = if value.get("type").and_then(Value::as_u64) == Some(10) {
        value
            .get("content")
            .and_then(Value::as_str)
            .map_or(0, |content| content.chars().count())
    } else {
        0
    };
    let child_max = value
        .get("components")
        .and_then(Value::as_array)
        .and_then(|children| children.iter().map(max_text_display_chars_in_value).max())
        .unwrap_or(0);
    own.max(child_max)
}

fn component_count(values: &[Value]) -> usize {
    values.iter().map(component_count_in_value).sum()
}

fn component_count_in_value(value: &Value) -> usize {
    let child_count = value
        .get("components")
        .and_then(Value::as_array)
        .map(|children| component_count(children))
        .unwrap_or(0);
    let item_count = value
        .get("items")
        .and_then(Value::as_array)
        .map(|items| component_count(items))
        .unwrap_or(0);
    1 + child_count + item_count
}

fn container(id: u64, components: Vec<Value>) -> Value {
    json!({
        "type": 17,
        "id": id,
        "accent_color": REGELWERK_ACCENT_GOLD,
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

fn button(id: u64, label: &str, style: u8, custom_id: &str) -> Value {
    json!({
        "type": 2,
        "id": id,
        "style": style,
        "label": label,
        "custom_id": custom_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::InteractionHandler;
    use std::path::Path;

    fn write_regelwerk_texts_file(repo_root: &Path, body: &str) {
        let path = repo_root.join(REGELWERK_TEXTS_FILE);
        fs::create_dir_all(path.parent().expect("texts parent")).expect("mkdir texts parent");
        fs::write(path, body).expect("write texts");
    }

    fn write_banner_bytes(repo_root: &Path, bytes: &[u8]) {
        let path = repo_root.join(format!("{REGELWERK_BANNER_DIR}/{REGELWERK_HERO_FILENAME}"));
        fs::create_dir_all(path.parent().expect("banner parent")).expect("mkdir banner parent");
        fs::write(path, bytes).expect("write banner");
    }

    #[test]
    fn regelwerk_texts_temp_toml_mergt_fehlende_felder_auf_compile_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_regelwerk_texts_file(
            temp.path(),
            r#"
[texts]
body = "Runtime Body"

[buttons]
moderation = "Runtime Moderation"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_regelwerk_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.title, REGELWERK_MAIN_TITLE);
        assert_eq!(config.texts.body, "Runtime Body");
        assert_eq!(config.texts.verhalten, REGELWERK_SECTION_VERHALTEN);
        assert_eq!(config.buttons.moderation, "Runtime Moderation");
        assert_eq!(config.buttons.wegweiser, REGELWERK_BUTTON_WEGWEISER_LABEL);
    }

    #[test]
    fn regelwerk_texts_parse_fehler_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_regelwerk_texts_file(temp.path(), "[texts]\nbody = [\n");
        let mut warnings = Vec::new();
        let err =
            load_regelwerk_runtime_config(temp.path(), &mut warnings).expect_err("parse error");
        assert!(err.contains("konnte nicht geparst werden"));
    }

    #[test]
    fn regelwerk_payload_haelt_v2_mentions_banner_buttons_und_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner");
        let output = build_regelwerk_publish_output(
            temp.path(),
            &[],
            Some(REGELWERK_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("output");
        assert_eq!(output.messages.len(), 1);
        let payload = &output.messages[0].payload;
        assert_eq!(payload.flags, REGELWERK_COMPONENTS_V2_FLAG);
        assert!(payload.allowed_mentions.parse.is_empty());
        assert_eq!(payload.attachments.len(), 1);
        assert_eq!(payload.attachments[0].filename, REGELWERK_HERO_FILENAME);
        assert_eq!(payload.components[0]["type"], json!(12));
        assert_eq!(
            payload.components[0]["items"][0]["media"]["url"],
            format!("attachment://{REGELWERK_HERO_FILENAME}")
        );
        assert_eq!(
            payload.components[1]["accent_color"],
            json!(REGELWERK_ACCENT_GOLD)
        );
        assert_eq!(
            payload.components[1]["components"][0]["content"],
            format!("{REGELWERK_MAIN_TITLE}\n{REGELWERK_MAIN_BODY}")
        );
        let buttons = payload.components[1]["components"][1]["components"]
            .as_array()
            .expect("buttons");
        assert_eq!(buttons.len(), 4);
        assert!(buttons.iter().all(|button| button["style"] == json!(2)));
        assert_eq!(
            collect_component_custom_ids(&payload.components),
            vec![
                REGELWERK_VERHALTEN_CUSTOM_ID.to_string(),
                REGELWERK_SPIELKONTEXT_CUSTOM_ID.to_string(),
                REGELWERK_MODERATION_CUSTOM_ID.to_string(),
                REGELWERK_WEGWEISER_CUSTOM_ID.to_string(),
            ]
        );
        validate_regelwerk_message_budget(payload).expect("budget");
        assert!(has_regelwerk_v2_marker(&payload.components));
    }

    #[test]
    fn regelwerk_payload_hash_noop_und_banner_bytes_aendern_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner-one");
        let first = build_regelwerk_publish_output(
            temp.path(),
            &[7001],
            Some(REGELWERK_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("first output");

        let same = build_regelwerk_publish_output(
            temp.path(),
            &[7001],
            Some(REGELWERK_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("same output");
        assert_eq!(same.messages[0].action, "planned_no_op");
        assert!(regelwerk_payload_is_unchanged(&same));

        write_banner_bytes(temp.path(), b"banner-two");
        let changed = build_regelwerk_publish_output(
            temp.path(),
            &[7001],
            Some(REGELWERK_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("changed output");
        assert_eq!(changed.messages[0].action, "planned_edit");
        assert_ne!(changed.payload_hash, first.payload_hash);
    }

    #[test]
    fn regelwerk_button_custom_id_mappt_auf_sektionstext() {
        assert_eq!(
            regelwerk_section_text_for_custom_id(REGELWERK_VERHALTEN_CUSTOM_ID),
            Some(REGELWERK_SECTION_VERHALTEN)
        );
        assert_eq!(
            regelwerk_section_text_for_custom_id(REGELWERK_SPIELKONTEXT_CUSTOM_ID),
            Some(REGELWERK_SECTION_SPIELKONTEXT)
        );
        assert_eq!(
            regelwerk_section_text_for_custom_id(REGELWERK_MODERATION_CUSTOM_ID),
            Some(REGELWERK_SECTION_MODERATION)
        );
        assert_eq!(
            regelwerk_section_text_for_custom_id(REGELWERK_WEGWEISER_CUSTOM_ID),
            Some(REGELWERK_SECTION_WEGWEISER)
        );
        assert_eq!(
            regelwerk_section_text_for_custom_id("regelwerk:show:nope"),
            None
        );
    }

    #[tokio::test]
    async fn regelwerk_button_reply_ist_ephemeral_v2_mit_allowed_mentions_und_v1_fallback() {
        let handler = RegelwerkSectionHandler;
        let reply = handler
            .handle(dl_discord::BridgeInteraction {
                custom_id: REGELWERK_MODERATION_CUSTOM_ID.to_string(),
                ..dl_discord::BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            reply.message_flags,
            Some(REGELWERK_EPHEMERAL_FLAG | REGELWERK_COMPONENTS_V2_FLAG)
        );
        assert_eq!(reply.allowed_mentions, Some(json!({"parse": []})));
        let components = reply.components.expect("components");
        assert_eq!(components[0]["type"], json!(17));
        assert_eq!(components[0]["accent_color"], json!(REGELWERK_ACCENT_GOLD));
        assert_eq!(
            components[0]["components"][0]["content"],
            REGELWERK_SECTION_MODERATION
        );
        let fallback = reply.fallback.expect("fallback");
        assert!(fallback.ephemeral);
        assert!(fallback.components.is_none());
        assert_eq!(fallback.allowed_mentions, Some(json!({"parse": []})));
        assert_eq!(fallback.embeds[0]["color"], json!(REGELWERK_ACCENT_GOLD));
        assert_eq!(
            fallback.embeds[0]["description"],
            REGELWERK_SECTION_MODERATION
        );
    }

    #[test]
    fn regelwerk_legacy_cleanup_prueft_message_id_author_und_content_prefix() {
        let valid = RegelwerkLegacyMessage {
            message_id: REGELWERK_OLD_MESSAGE_ID,
            author_id: 42,
            content: "**📜 Regelwerk · alt".to_string(),
        };
        assert!(is_legacy_regelwerk_cleanup_candidate(&valid, 42));
        assert!(!is_legacy_regelwerk_cleanup_candidate(
            &RegelwerkLegacyMessage {
                author_id: 99,
                ..valid.clone()
            },
            42
        ));
        assert!(!is_legacy_regelwerk_cleanup_candidate(
            &RegelwerkLegacyMessage {
                content: "anderer Inhalt".to_string(),
                ..valid
            },
            42
        ));
    }
}
