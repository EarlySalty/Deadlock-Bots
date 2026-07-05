use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SUPPORT_CHANNEL_ID: u64 = 1_483_136_301_271_355_532;
pub const SUPPORT_CHANNEL_NAME: &str = "server-support";
pub const SUPPORT_TEXTS_FILE: &str = "assets/support_texts.toml";
pub const SUPPORT_BANNER_DIR: &str = "assets/welcome-banners";
pub const SUPPORT_HERO_FILENAME: &str = "support-hero.png";
pub const SUPPORT_PAYLOAD_FORMAT: &str = "2";
pub const SUPPORT_PAYLOAD_FORMAT_KEY: &str = "support_v2_payload_format";
pub const SUPPORT_PAYLOAD_HASH_KEY: &str = "support_v2_payload_hash";
pub const SUPPORT_MESSAGE_ID_PREFIX: &str = "support_v2_message_id_";
pub const SUPPORT_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const SUPPORT_ACCENT_GOLD: u64 = 0xC8A86B;
#[cfg(test)]
pub const SUPPORT_OWNER_MESSAGE_ID: u64 = 1_483_147_756_192_403_467;

pub const SUPPORT_COMPONENT_ID_HERO_MEDIA: u64 = 33_001;
pub const SUPPORT_COMPONENT_ID_MAIN_CONTAINER: u64 = 33_002;
pub const SUPPORT_COMPONENT_ID_MAIN_TEXT: u64 = 33_003;
pub const SUPPORT_COMPONENT_ID_ACTION_ROW: u64 = 33_004;
pub const SUPPORT_COMPONENT_ID_DONATE_BUTTON: u64 = 33_005;

pub const SUPPORT_MARKER_COMPONENT_IDS: &[u64] = &[
    SUPPORT_COMPONENT_ID_HERO_MEDIA,
    SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
    SUPPORT_COMPONENT_ID_MAIN_TEXT,
    SUPPORT_COMPONENT_ID_ACTION_ROW,
];

pub const SUPPORT_TEXT_CHAR_BUDGET: usize = 3500;
pub const SUPPORT_COMPONENT_BUDGET: usize = 35;
pub const SUPPORT_ATTACHMENT_BUDGET: usize = 9;
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;

pub const SUPPORT_DONATE_URL_DEFAULT: &str = "https://ko-fi.com/deutschedeadlockcommunity";
pub const SUPPORT_TITLE: &str = "**💜 Server unterstützen**";
pub const SUPPORT_BLOCK_BOOST: &str = "**Du hast Nitro?**\nDann kannst du uns ohne Extra-Kosten mit bis zu zwei Server-Boosts unterstützen: Klick oben links auf den Servernamen → **Server Boost** → auf diesen Server boosten. Das war's schon.";
pub const SUPPORT_BLOCK_DONATION: &str = "**Kein Nitro?**\nDu kannst uns auch mit einer Spende unterstützen — sie fließt komplett in Serverkosten, Bots und unsere Website. Als Dankeschön bekommst du innerhalb von 24 Stunden nach Eingang die exklusive Unterstützer-Rolle für 31 Tage (nicht auf andere Accounts übertragbar).";
pub const SUPPORT_BLOCK_BENEFITS: &str = "**Deine Vorteile als Booster & Unterstützer**\n- Privater Text- & Voice-Bereich\n- Zugriff auf den ||geheimen|| Caster-Chat — plane Turniere & Events direkt mit\n- Lese-Zugriff auf den ||noch geheimeren|| Community-Moderatoren-Chat <:pepecrywow:1483145247319003320>\n- Platz in der Aktivitätenliste direkt unterm Team ❤️\n- Zugriff auf <#1326984426906714236>\n- Voice-Moderationsrechte (Leute moven, in volle Kanäle joinen)\n- Du streamst? Dann warten ein paar exklusive Extra-Features für den Twitch-Bot auf dich\n\nDanke — jede Unterstützung, egal wie groß, trägt den Server. 💜";
pub const SUPPORT_DONATE_BUTTON_LABEL: &str = "☕ Auf Ko-fi spenden";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportPublishOutput {
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
    pub messages: Vec<SupportMessageOutput>,
    pub stored_message_ids: Vec<u64>,
    pub posted_message_ids: Vec<u64>,
    pub edited_message_ids: Vec<u64>,
    pub deleted_message_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportMessageOutput {
    pub message_index: usize,
    pub message_key: String,
    pub action: String,
    pub stored_message_id: Option<u64>,
    pub message_id: Option<u64>,
    pub banner: Option<SupportBannerOutput>,
    pub payload: SupportMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportBannerOutput {
    pub filename: String,
    pub relative_path: String,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportMessagePayload {
    pub flags: u64,
    pub allowed_mentions: SupportAllowedMentions,
    pub components: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<SupportPayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportAllowedMentions {
    pub parse: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportPayloadAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportV2Message {
    pub message_id: u64,
    pub author_id: u64,
    pub flags: u64,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportConfig {
    texts: ResolvedSupportTexts,
    buttons: ResolvedSupportButtons,
    urls: ResolvedSupportUrls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportTexts {
    title: String,
    boost: String,
    donation: String,
    benefits: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportButtons {
    donate: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportUrls {
    donate: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportTextsToml {
    #[serde(default)]
    texts: SupportBodyTextsToml,
    #[serde(default)]
    buttons: SupportButtonsToml,
    #[serde(default)]
    urls: SupportUrlsToml,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportBodyTextsToml {
    title: Option<String>,
    boost: Option<String>,
    donation: Option<String>,
    benefits: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportButtonsToml {
    donate: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportUrlsToml {
    donate: Option<String>,
}

#[derive(Debug, Clone)]
struct SupportBuiltMessage {
    banner: Option<SupportBannerOutput>,
    payload: SupportMessagePayload,
}

#[cfg(test)]
pub fn support_message_id_key(message_index: usize) -> String {
    format!("{SUPPORT_MESSAGE_ID_PREFIX}{message_index}")
}

pub fn build_support_publish_output(
    repo_root: &Path,
    stored_message_ids: &[u64],
    stored_payload_format: Option<&str>,
    stored_payload_hash: Option<&str>,
    dry_run: bool,
) -> Result<SupportPublishOutput, String> {
    let mut warnings = Vec::new();
    let config = load_support_runtime_config(repo_root, &mut warnings)?;
    let built_messages = support_messages(repo_root, &config, &mut warnings);

    let mut messages = Vec::new();
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored_message_ids.get(message_index).copied();
        messages.push(SupportMessageOutput {
            message_index,
            message_key: support_message_key(message_index),
            action: if dry_run {
                planned_support_action(stored_message_id, false, false)
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
        validate_support_message_budget(&message.payload)?;
    }

    let payload_hash = support_payload_hash(repo_root, &messages)?;
    let repost_required =
        !support_storage_matches(stored_payload_format, stored_message_ids, messages.len());
    let hash_matches = stored_payload_hash == Some(payload_hash.as_str());
    if dry_run {
        for message in &mut messages {
            message.action =
                planned_support_action(message.stored_message_id, repost_required, hash_matches);
        }
    }

    Ok(SupportPublishOutput {
        guild_id: dl_server_as_code::DEFAULT_GUILD_ID,
        channel_id: SUPPORT_CHANNEL_ID,
        channel_name: SUPPORT_CHANNEL_NAME.to_string(),
        dry_run,
        payload_format: SUPPORT_PAYLOAD_FORMAT.to_string(),
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
    })
}

pub fn support_storage_matches(
    stored_payload_format: Option<&str>,
    stored_message_ids: &[u64],
    expected_message_count: usize,
) -> bool {
    stored_payload_format == Some(SUPPORT_PAYLOAD_FORMAT)
        && stored_message_ids.len() == expected_message_count
}

pub fn support_payload_is_unchanged(output: &SupportPublishOutput) -> bool {
    output.stored_payload_hash.as_deref() == Some(output.payload_hash.as_str())
}

pub fn support_payload_hash(
    repo_root: &Path,
    messages: &[SupportMessageOutput],
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
                            "Support-Attachment `{}` konnte fuer Hash nicht gelesen werden: {err}",
                            path.display()
                        )
                    })?;
                    Ok(SupportPayloadHashAttachment {
                        id: attachment.id,
                        filename: attachment.filename.as_str(),
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(SupportPayloadHashMessage {
                payload: &message.payload,
                attachment_hashes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let raw = serde_json::to_vec(&payloads)
        .map_err(|err| format!("Support-Payload konnte nicht serialisiert werden: {err}"))?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

#[derive(Serialize)]
struct SupportPayloadHashMessage<'a> {
    payload: &'a SupportMessagePayload,
    attachment_hashes: Vec<SupportPayloadHashAttachment<'a>>,
}

#[derive(Serialize)]
struct SupportPayloadHashAttachment<'a> {
    id: u8,
    filename: &'a str,
    sha256: String,
}

pub fn is_support_v2_message(message: &SupportV2Message, bot_user_id: u64) -> bool {
    message.author_id == bot_user_id
        && message.flags & SUPPORT_COMPONENTS_V2_FLAG != 0
        && has_support_v2_marker(&message.components)
}

pub fn has_support_v2_marker(components: &[Value]) -> bool {
    collect_component_ids(components)
        .iter()
        .any(|component_id| SUPPORT_MARKER_COMPONENT_IDS.contains(component_id))
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

fn load_support_runtime_config(
    repo_root: &Path,
    warnings: &mut Vec<String>,
) -> Result<ResolvedSupportConfig, String> {
    let path = repo_root.join(SUPPORT_TEXTS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            warnings.push(format!(
                "Support-Textdatei `{SUPPORT_TEXTS_FILE}` fehlt; Compile-Defaults werden verwendet"
            ));
            return Ok(ResolvedSupportConfig::from_defaults());
        }
        Err(err) => {
            return Err(format!(
                "Support-Textdatei `{SUPPORT_TEXTS_FILE}` konnte nicht gelesen werden: {err}"
            ));
        }
    };

    let file = toml::from_str::<SupportTextsToml>(&raw).map_err(|err| {
        format!("Support-Textdatei `{SUPPORT_TEXTS_FILE}` konnte nicht geparst werden: {err}")
    })?;
    Ok(ResolvedSupportConfig::from_defaults().merge(file))
}

impl ResolvedSupportConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedSupportTexts {
                title: SUPPORT_TITLE.to_string(),
                boost: SUPPORT_BLOCK_BOOST.to_string(),
                donation: SUPPORT_BLOCK_DONATION.to_string(),
                benefits: SUPPORT_BLOCK_BENEFITS.to_string(),
            },
            buttons: ResolvedSupportButtons {
                donate: SUPPORT_DONATE_BUTTON_LABEL.to_string(),
            },
            urls: ResolvedSupportUrls {
                donate: SUPPORT_DONATE_URL_DEFAULT.to_string(),
            },
        }
    }

    fn merge(mut self, file: SupportTextsToml) -> Self {
        apply_optional(&mut self.texts.title, file.texts.title);
        apply_optional(&mut self.texts.boost, file.texts.boost);
        apply_optional(&mut self.texts.donation, file.texts.donation);
        apply_optional(&mut self.texts.benefits, file.texts.benefits);
        apply_optional(&mut self.buttons.donate, file.buttons.donate);
        apply_optional(&mut self.urls.donate, file.urls.donate);
        self
    }
}

fn apply_optional(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn support_messages(
    repo_root: &Path,
    config: &ResolvedSupportConfig,
    warnings: &mut Vec<String>,
) -> Vec<SupportBuiltMessage> {
    let banner = optional_support_banner(repo_root, SUPPORT_HERO_FILENAME, warnings);
    let mut components = Vec::new();
    let mut attachments = Vec::new();
    if let Some(banner) = &banner {
        components.push(media_gallery(
            SUPPORT_COMPONENT_ID_HERO_MEDIA,
            &banner.filename,
        ));
        attachments.push(SupportPayloadAttachment {
            id: 0,
            filename: banner.filename.clone(),
            relative_path: banner.relative_path.clone(),
        });
    }

    components.push(container(
        SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
        vec![
            text_display(SUPPORT_COMPONENT_ID_MAIN_TEXT, support_text(&config.texts)),
            action_row(
                SUPPORT_COMPONENT_ID_ACTION_ROW,
                vec![link_button(
                    SUPPORT_COMPONENT_ID_DONATE_BUTTON,
                    &config.buttons.donate,
                    &config.urls.donate,
                )],
            ),
        ],
    ));

    vec![SupportBuiltMessage {
        banner,
        payload: SupportMessagePayload {
            flags: SUPPORT_COMPONENTS_V2_FLAG,
            allowed_mentions: SupportAllowedMentions { parse: Vec::new() },
            components,
            attachments,
        },
    }]
}

fn support_text(texts: &ResolvedSupportTexts) -> String {
    format!(
        "{}\n{}\n\n{}\n\n{}",
        texts.title, texts.boost, texts.donation, texts.benefits
    )
}

fn optional_support_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> Option<SupportBannerOutput> {
    let relative_path = format!("{SUPPORT_BANNER_DIR}/{filename}");
    let path = repo_root.join(&relative_path);
    if !path.is_file() {
        warnings.push(format!(
            "Support-Banner `{relative_path}` fehlt; Publish laeuft ohne Banner"
        ));
        return None;
    }
    Some(SupportBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present: true,
    })
}

fn planned_support_action(
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

fn support_message_key(message_index: usize) -> String {
    if message_index == 0 {
        "support".to_string()
    } else {
        format!("support:{message_index}")
    }
}

fn validate_support_message_budget(payload: &SupportMessagePayload) -> Result<(), String> {
    let text_chars = text_display_chars(&payload.components);
    if text_chars > SUPPORT_TEXT_CHAR_BUDGET {
        return Err(format!(
            "Support-Payload ueberschreitet Textbudget: {text_chars}/{SUPPORT_TEXT_CHAR_BUDGET}"
        ));
    }
    if max_text_display_chars(&payload.components) > MESSAGE_TEXT_DISPLAY_CHAR_LIMIT {
        return Err(format!(
            "Support-Payload ueberschreitet TextDisplay-Limit {MESSAGE_TEXT_DISPLAY_CHAR_LIMIT}"
        ));
    }
    let component_count = component_count(&payload.components);
    if component_count > SUPPORT_COMPONENT_BUDGET {
        return Err(format!(
            "Support-Payload ueberschreitet Komponentenbudget: {component_count}/{SUPPORT_COMPONENT_BUDGET}"
        ));
    }
    if payload.attachments.len() > SUPPORT_ATTACHMENT_BUDGET {
        return Err(format!(
            "Support-Payload ueberschreitet Attachmentbudget: {}/{SUPPORT_ATTACHMENT_BUDGET}",
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
        "accent_color": SUPPORT_ACCENT_GOLD,
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

fn link_button(id: u64, label: &str, url: &str) -> Value {
    json!({
        "type": 2,
        "id": id,
        "style": 5,
        "label": label,
        "url": url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn write_support_texts_file(repo_root: &Path, body: &str) {
        let path = repo_root.join(SUPPORT_TEXTS_FILE);
        fs::create_dir_all(path.parent().expect("texts parent")).expect("mkdir texts parent");
        fs::write(path, body).expect("write texts");
    }

    fn write_banner_bytes(repo_root: &Path, bytes: &[u8]) {
        let path = repo_root.join(format!("{SUPPORT_BANNER_DIR}/{SUPPORT_HERO_FILENAME}"));
        fs::create_dir_all(path.parent().expect("banner parent")).expect("mkdir banner parent");
        fs::write(path, bytes).expect("write banner");
    }

    #[test]
    fn support_texts_temp_toml_mergt_fehlende_felder_auf_compile_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(
            temp.path(),
            r#"
[texts]
boost = "Runtime Boost"

[buttons]
donate = "Runtime Button"

[urls]
donate = "https://example.invalid/kofi"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_support_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.title, SUPPORT_TITLE);
        assert_eq!(config.texts.boost, "Runtime Boost");
        assert_eq!(config.texts.donation, SUPPORT_BLOCK_DONATION);
        assert_eq!(config.buttons.donate, "Runtime Button");
        assert_eq!(config.urls.donate, "https://example.invalid/kofi");
    }

    #[test]
    fn support_texts_parse_fehler_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(temp.path(), "[texts]\ntitle = [\n");
        let mut warnings = Vec::new();
        let err = load_support_runtime_config(temp.path(), &mut warnings).expect_err("parse error");
        assert!(err.contains("konnte nicht geparst werden"));
    }

    #[test]
    fn support_payload_haelt_v2_mentions_banner_link_button_und_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner");
        let output = build_support_publish_output(
            temp.path(),
            &[],
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("output");
        assert_eq!(output.messages.len(), 1);
        let payload = &output.messages[0].payload;
        assert_eq!(payload.flags, SUPPORT_COMPONENTS_V2_FLAG);
        assert!(payload.allowed_mentions.parse.is_empty());
        assert_eq!(payload.attachments.len(), 1);
        assert_eq!(payload.attachments[0].filename, SUPPORT_HERO_FILENAME);
        assert_eq!(payload.components[0]["type"], json!(12));
        assert_eq!(
            payload.components[0]["items"][0]["media"]["url"],
            format!("attachment://{SUPPORT_HERO_FILENAME}")
        );
        assert_eq!(
            payload.components[1]["accent_color"],
            json!(SUPPORT_ACCENT_GOLD)
        );
        assert_eq!(
            payload.components[1]["components"][0]["content"],
            format!(
                "{SUPPORT_TITLE}\n{SUPPORT_BLOCK_BOOST}\n\n{SUPPORT_BLOCK_DONATION}\n\n{SUPPORT_BLOCK_BENEFITS}"
            )
        );
        let button = &payload.components[1]["components"][1]["components"][0];
        assert_eq!(button["style"], json!(5));
        assert_eq!(button["label"], SUPPORT_DONATE_BUTTON_LABEL);
        assert_eq!(button["url"], SUPPORT_DONATE_URL_DEFAULT);
        assert!(button.get("custom_id").is_none());
        validate_support_message_budget(payload).expect("budget");
        assert!(has_support_v2_marker(&payload.components));
    }

    #[test]
    fn support_payload_hash_noop_und_banner_bytes_aendern_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner-one");
        let first = build_support_publish_output(
            temp.path(),
            &[7001],
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("first output");

        let same = build_support_publish_output(
            temp.path(),
            &[7001],
            Some(SUPPORT_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("same output");
        assert_eq!(same.messages[0].action, "planned_no_op");
        assert!(support_payload_is_unchanged(&same));

        write_banner_bytes(temp.path(), b"banner-two");
        let changed = build_support_publish_output(
            temp.path(),
            &[7001],
            Some(SUPPORT_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("changed output");
        assert_eq!(changed.messages[0].action, "planned_edit");
        assert_ne!(changed.payload_hash, first.payload_hash);
    }
}
