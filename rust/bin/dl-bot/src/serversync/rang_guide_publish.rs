use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const RANG_GUIDE_CHANNEL_ID: u64 = 1_398_021_105_339_334_666;
pub const RANG_GUIDE_CHANNEL_NAME: &str = "deadlock-rang";
pub const RANG_GUIDE_TEXTS_FILE: &str = "assets/rang_guide_texts.toml";
pub const RANG_GUIDE_BANNER_DIR: &str = "assets/welcome-banners";
pub const RANG_GUIDE_HERO_FILENAME: &str = "rang-guide-hero.png";
pub const RANG_GUIDE_PAYLOAD_FORMAT: &str = "2";
pub const RANG_GUIDE_PAYLOAD_FORMAT_KEY: &str = "rang_guide_payload_format";
pub const RANG_GUIDE_PAYLOAD_HASH_KEY: &str = "rang_guide_payload_hash";
pub const RANG_GUIDE_MESSAGE_ID_PREFIX: &str = "rang_guide_message_id_";
pub const RANG_GUIDE_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const RANG_GUIDE_ACCENT_GOLD: u64 = 0xC8A86B;
pub const RANG_GUIDE_COMPONENT_ID_HERO_CONTAINER: u64 = 31_001;
pub const RANG_GUIDE_COMPONENT_ID_HERO_MEDIA: u64 = 31_002;
pub const RANG_GUIDE_COMPONENT_ID_HERO_TEXT: u64 = 31_003;
pub const RANG_GUIDE_COMPONENT_ID_STEP1_CONTAINER: u64 = 31_004;
pub const RANG_GUIDE_COMPONENT_ID_STEP1_TEXT: u64 = 31_005;
pub const RANG_GUIDE_COMPONENT_ID_STEP1_ACTION_ROW: u64 = 31_006;
pub const RANG_GUIDE_COMPONENT_ID_STEAM_OPEN_BUTTON: u64 = 31_007;
pub const RANG_GUIDE_COMPONENT_ID_STEP2_TEXT: u64 = 31_008;
pub const RANG_GUIDE_COMPONENT_ID_STEP3_TEXT: u64 = 31_009;
pub const RANG_GUIDE_COMPONENT_ID_STEP4_CONTAINER: u64 = 31_010;
pub const RANG_GUIDE_COMPONENT_ID_STEP4_TEXT: u64 = 31_011;
pub const RANG_GUIDE_COMPONENT_ID_STEP4_ACTION_ROW: u64 = 31_012;
pub const RANG_GUIDE_COMPONENT_ID_LINKED_ROLE_BUTTON: u64 = 31_013;
pub const RANG_GUIDE_COMPONENT_ID_EXTRAS_CONTAINER: u64 = 31_014;
pub const RANG_GUIDE_COMPONENT_ID_EXTRAS_TEXT: u64 = 31_015;
pub const RANG_GUIDE_COMPONENT_ID_EXTRAS_ACTION_ROW: u64 = 31_016;
pub const RANG_GUIDE_COMPONENT_ID_FRIEND_CODE_BUTTON: u64 = 31_017;
pub const RANG_GUIDE_COMPONENT_ID_RANKCHECK_BUTTON: u64 = 31_018;
pub const RANG_GUIDE_COMPONENT_ID_UNLINK_BUTTON: u64 = 31_026;
pub const RANG_GUIDE_COMPONENT_ID_STEP2_CONTAINER: u64 = 31_019;
pub const RANG_GUIDE_COMPONENT_ID_STEP3_CONTAINER: u64 = 31_020;
pub const RANG_GUIDE_COMPONENT_ID_STEP1_MEDIA: u64 = 31_021;
pub const RANG_GUIDE_COMPONENT_ID_STEP2_MEDIA: u64 = 31_022;
pub const RANG_GUIDE_COMPONENT_ID_STEP3_MEDIA: u64 = 31_023;
pub const RANG_GUIDE_COMPONENT_ID_STEP4_MEDIA: u64 = 31_024;
pub const RANG_GUIDE_COMPONENT_ID_EXTRAS_MEDIA: u64 = 31_025;

/// Schritt-Banner (divider-Stil) je Abschnitt; fehlende Dateien werden mit
/// Warnung uebersprungen, der Publish laeuft dann ohne das jeweilige Banner.
pub const RANG_GUIDE_STEP_BANNER_FILENAMES: [&str; 4] = [
    "rang-guide-schritt-1.png",
    "rang-guide-schritt-2.png",
    "rang-guide-schritt-3.png",
    "rang-guide-schritt-4.png",
];
pub const RANG_GUIDE_HELP_BANNER_FILENAME: &str = "rang-guide-hilfe.png";
pub const RANG_GUIDE_MARKER_COMPONENT_IDS: &[u64] = &[
    RANG_GUIDE_COMPONENT_ID_HERO_CONTAINER,
    RANG_GUIDE_COMPONENT_ID_HERO_TEXT,
    RANG_GUIDE_COMPONENT_ID_STEP1_CONTAINER,
    RANG_GUIDE_COMPONENT_ID_STEP1_TEXT,
    RANG_GUIDE_COMPONENT_ID_STEP2_TEXT,
    RANG_GUIDE_COMPONENT_ID_STEP3_TEXT,
    RANG_GUIDE_COMPONENT_ID_STEP4_CONTAINER,
    RANG_GUIDE_COMPONENT_ID_STEP4_TEXT,
    RANG_GUIDE_COMPONENT_ID_EXTRAS_CONTAINER,
    RANG_GUIDE_COMPONENT_ID_EXTRAS_TEXT,
];

pub const RANG_GUIDE_TEXT_CHAR_BUDGET: usize = 3500;
pub const RANG_GUIDE_COMPONENT_BUDGET: usize = 35;
pub const RANG_GUIDE_ATTACHMENT_BUDGET: usize = 9;
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;

pub const STEAM_LINK_OPEN_CUSTOM_ID: &str = "steam_link_panel:open";
pub const STEAM_LINK_FRIEND_CODE_CUSTOM_ID: &str = "steam_link_panel:friend_code";
pub const STEAM_LINK_RANKCHECK_CUSTOM_ID: &str = "steam_link_panel:rankcheck";
pub const STEAM_LINK_UNLINK_CUSTOM_ID: &str = "steam_link_panel:unlink";
pub const LINKED_ROLE_LOGIN_URL_DEFAULT: &str =
    "https://deutsche-deadlock-community.de/coaching/api/auth/discord/linked-role/login";

pub const RANG_GUIDE_HERO_INTRO: &str = "**Steam verknüpfen — Rang, Lanes & Spielersuche freischalten**\nEinmal verknüpft, erkennt der Server deinen In-Game-Rang automatisch und hält ihn aktuell — ganz ohne manuelles Nachpflegen.\n\n**Was bringt's?**\n- Dein Rang wird erkannt und als Rolle zugeordnet — und bleibt von selbst aktuell\n- Live-Status in den Voice-Lanes funktioniert\n- Ranked-Lanes und die Mitspieler-Suche stehen dir offen";
pub const RANG_GUIDE_STEP1_TITLE: &str = "1️⃣ Verifizierung starten";
pub const RANG_GUIDE_STEP1_BODY: &str = "Drück auf **Steam verknüpfen** — es folgt ein kurzer Steam-Login (OpenID, kein Passwort nötig), danach gibst du deinen Steam-Freundescode ein.\n\n**Was wir NICHT machen:** keine Passwörter oder Zugangsdaten, keine Steam-Freundesliste auslesen, keine Spielstände oder Profile einsehen, keine Daten an Dritte, keine Werbung oder Tracking.\n**Was wir speichern:** Discord-ID, SteamID64 und Rang-Daten — nur für die Server-Zuordnung. Der Code ist offen: https://github.com/NaniDerEchte2/Deadlock-Bots";
pub const RANG_GUIDE_STEP2_BODY: &str = "2️⃣ **Freundschaftsanfrage annehmen.** Wir schicken dir eine Anfrage in Steam — einfach annehmen, die Verifikation läuft dann automatisch. Alternativ schickst du selbst eine an den Freundescode **820142646**, wir nehmen automatisch an.\n⚠️ Bleib mit dem Bot befreundet — nur so können wir deinen Rang aktuell halten.";
pub const RANG_GUIDE_STEP3_BODY: &str = "3️⃣ **Fertig — Rang-Rollen kommen von selbst.** Dein In-Game-Rang wird erkannt, als Rolle zugeordnet und ab da automatisch aktuell gehalten.";
pub const RANG_GUIDE_STEP4_TITLE: &str = "4️⃣ Das offizielle Discord-Siegel: Steam Verifiziert";
pub const RANG_GUIDE_STEP4_BODY: &str = "Hol dir zum Schluss die Rolle **Steam Verifiziert** — die vergibt **Discord selbst**, niemand kann sie von Hand verteilen. Klick auf **Discord-Verknüpfung herstellen**, kurzer Discord-Login, fertig.\nAlternativ geht's auch über den Servernamen oben links → **Verknüpfte Rollen**.";
pub const RANG_GUIDE_EXTRAS_TITLE: &str = "Wenn's mal hakt";
pub const RANG_GUIDE_EXTRAS_BODY: &str = "- Verknüpft, aber nichts passiert? Drück einmal auf **Rang prüfen** — der Abgleich läuft sonst automatisch alle paar Minuten.\n- Rolle da, aber es klappt trotzdem nicht? Prüf die Freundschaft mit dem Bot (Freundescode **820142646**) oder sag den Mods Bescheid.\n- Mit **Freundescode eingeben** kannst du deinen Steam-Freundescode jederzeit nachtragen oder korrigieren.";
pub const RANG_GUIDE_STEAM_OPEN_BUTTON_LABEL: &str = "🔗 Steam verknüpfen";
pub const RANG_GUIDE_LINKED_ROLE_BUTTON_LABEL: &str = "✅ Discord-Verknüpfung herstellen";
pub const RANG_GUIDE_FRIEND_CODE_BUTTON_LABEL: &str = "🔢 Freundescode eingeben";
pub const RANG_GUIDE_RANKCHECK_BUTTON_LABEL: &str = "📊 Rang prüfen";
pub const RANG_GUIDE_UNLINK_BUTTON_LABEL: &str = "🔓 Verknüpfung entfernen";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuidePublishOutput {
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
    pub messages: Vec<RangGuideMessageOutput>,
    pub stored_message_ids: Vec<u64>,
    pub posted_message_ids: Vec<u64>,
    pub edited_message_ids: Vec<u64>,
    pub deleted_message_ids: Vec<u64>,
    pub legacy_cleanup_candidates: Vec<RangGuideLegacyCleanupCandidate>,
    pub deleted_legacy_message_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuideMessageOutput {
    pub message_index: usize,
    pub message_key: String,
    pub action: String,
    pub stored_message_id: Option<u64>,
    pub message_id: Option<u64>,
    pub banner: Option<RangGuideBannerOutput>,
    pub payload: RangGuideMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuideBannerOutput {
    pub filename: String,
    pub relative_path: String,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuideMessagePayload {
    pub flags: u64,
    pub allowed_mentions: RangGuideAllowedMentions,
    pub components: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<RangGuidePayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuideAllowedMentions {
    pub parse: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuidePayloadAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RangGuideLegacyCleanupCandidate {
    pub message_id: u64,
    pub custom_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangGuideLegacyMessage {
    pub message_id: u64,
    pub author_id: u64,
    pub flags: u64,
    pub has_embeds: bool,
    pub custom_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangGuideV2Message {
    pub message_id: u64,
    pub author_id: u64,
    pub flags: u64,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRangGuideConfig {
    texts: ResolvedRangGuideTexts,
    buttons: ResolvedRangGuideButtons,
    urls: ResolvedRangGuideUrls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRangGuideTexts {
    hero_intro: String,
    step1_title: String,
    step1_body: String,
    step2_body: String,
    step3_body: String,
    step4_title: String,
    step4_body: String,
    extras_title: String,
    extras_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRangGuideButtons {
    steam_open: String,
    linked_role: String,
    friend_code: String,
    rankcheck: String,
    unlink: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedRangGuideUrls {
    linked_role_login: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangGuideTextsToml {
    #[serde(default)]
    texts: RangGuideBodyTextsToml,
    #[serde(default)]
    buttons: RangGuideButtonsToml,
    #[serde(default)]
    urls: RangGuideUrlsToml,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangGuideBodyTextsToml {
    hero_intro: Option<String>,
    step1_title: Option<String>,
    step1_body: Option<String>,
    step2_body: Option<String>,
    step3_body: Option<String>,
    step4_title: Option<String>,
    step4_body: Option<String>,
    extras_title: Option<String>,
    extras_body: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangGuideButtonsToml {
    steam_open: Option<String>,
    linked_role: Option<String>,
    friend_code: Option<String>,
    rankcheck: Option<String>,
    unlink: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RangGuideUrlsToml {
    linked_role_login: Option<String>,
}

#[derive(Debug, Clone)]
struct RangGuideBuiltMessage {
    banner: Option<RangGuideBannerOutput>,
    payload: RangGuideMessagePayload,
}

pub fn rang_guide_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn rang_guide_message_id_key(message_index: usize) -> String {
    format!("{RANG_GUIDE_MESSAGE_ID_PREFIX}{message_index}")
}

pub fn build_rang_guide_publish_output(
    repo_root: &Path,
    stored_message_ids: &[u64],
    stored_payload_format: Option<&str>,
    stored_payload_hash: Option<&str>,
    dry_run: bool,
) -> Result<RangGuidePublishOutput, String> {
    let mut warnings = Vec::new();
    let config = load_rang_guide_runtime_config(repo_root, &mut warnings)?;
    let built_messages = rang_guide_messages(repo_root, &config, &mut warnings);

    let mut messages = Vec::new();
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored_message_ids.get(message_index).copied();
        messages.push(RangGuideMessageOutput {
            message_index,
            message_key: rang_guide_message_key(message_index),
            action: if dry_run {
                planned_rang_guide_action(stored_message_id, false, false)
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
        validate_rang_guide_message_budget(&message.payload)?;
    }

    let payload_hash = rang_guide_payload_hash(repo_root, &messages)?;
    let repost_required =
        !rang_guide_storage_matches(stored_payload_format, stored_message_ids, messages.len());
    let hash_matches = stored_payload_hash == Some(payload_hash.as_str());
    if dry_run {
        for message in &mut messages {
            message.action =
                planned_rang_guide_action(message.stored_message_id, repost_required, hash_matches);
        }
    }

    Ok(RangGuidePublishOutput {
        guild_id: dl_server_as_code::DEFAULT_GUILD_ID,
        channel_id: RANG_GUIDE_CHANNEL_ID,
        channel_name: RANG_GUIDE_CHANNEL_NAME.to_string(),
        dry_run,
        payload_format: RANG_GUIDE_PAYLOAD_FORMAT.to_string(),
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
        legacy_cleanup_candidates: Vec::new(),
        deleted_legacy_message_ids: Vec::new(),
    })
}

pub fn rang_guide_storage_matches(
    stored_payload_format: Option<&str>,
    stored_message_ids: &[u64],
    expected_message_count: usize,
) -> bool {
    stored_payload_format == Some(RANG_GUIDE_PAYLOAD_FORMAT)
        && stored_message_ids.len() == expected_message_count
}

pub fn rang_guide_payload_is_unchanged(output: &RangGuidePublishOutput) -> bool {
    output.stored_payload_hash.as_deref() == Some(output.payload_hash.as_str())
}

pub fn rang_guide_payload_hash(
    repo_root: &Path,
    messages: &[RangGuideMessageOutput],
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
                            "Rang-Guide-Attachment `{}` konnte fuer Hash nicht gelesen werden: {err}",
                            path.display()
                        )
                    })?;
                    Ok(RangGuidePayloadHashAttachment {
                        id: attachment.id,
                        filename: attachment.filename.as_str(),
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(RangGuidePayloadHashMessage {
                payload: &message.payload,
                attachment_hashes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let raw = serde_json::to_vec(&payloads)
        .map_err(|err| format!("Rang-Guide-Payload konnte nicht serialisiert werden: {err}"))?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

#[derive(Serialize)]
struct RangGuidePayloadHashMessage<'a> {
    payload: &'a RangGuideMessagePayload,
    attachment_hashes: Vec<RangGuidePayloadHashAttachment<'a>>,
}

#[derive(Serialize)]
struct RangGuidePayloadHashAttachment<'a> {
    id: u8,
    filename: &'a str,
    sha256: String,
}

pub fn is_legacy_rank_guide_cleanup_candidate(
    message: &RangGuideLegacyMessage,
    bot_user_id: u64,
) -> bool {
    message.author_id == bot_user_id
        && message.has_embeds
        && message.flags & RANG_GUIDE_COMPONENTS_V2_FLAG == 0
        && message
            .custom_ids
            .iter()
            .any(|custom_id| custom_id.starts_with("steam_link_panel:"))
}

pub fn is_rang_guide_v2_message(message: &RangGuideV2Message, bot_user_id: u64) -> bool {
    message.author_id == bot_user_id
        && message.flags & RANG_GUIDE_COMPONENTS_V2_FLAG != 0
        && has_rang_guide_v2_marker(&message.components)
}

pub fn has_rang_guide_v2_marker(components: &[Value]) -> bool {
    collect_component_ids(components)
        .iter()
        .any(|component_id| RANG_GUIDE_MARKER_COMPONENT_IDS.contains(component_id))
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

pub fn collect_component_custom_ids(components: &[Value]) -> Vec<String> {
    let mut custom_ids = Vec::new();
    for component in components {
        collect_component_custom_ids_from_value(component, &mut custom_ids);
    }
    custom_ids
}

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

fn load_rang_guide_runtime_config(
    repo_root: &Path,
    warnings: &mut Vec<String>,
) -> Result<ResolvedRangGuideConfig, String> {
    let path = repo_root.join(RANG_GUIDE_TEXTS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            warnings.push(format!(
                "Rang-Guide-Textdatei `{RANG_GUIDE_TEXTS_FILE}` fehlt; Compile-Defaults werden verwendet"
            ));
            return Ok(ResolvedRangGuideConfig::from_defaults());
        }
        Err(err) => {
            return Err(format!(
                "Rang-Guide-Textdatei `{RANG_GUIDE_TEXTS_FILE}` konnte nicht gelesen werden: {err}"
            ));
        }
    };

    let file = toml::from_str::<RangGuideTextsToml>(&raw).map_err(|err| {
        format!("Rang-Guide-Textdatei `{RANG_GUIDE_TEXTS_FILE}` konnte nicht geparst werden: {err}")
    })?;
    Ok(ResolvedRangGuideConfig::from_defaults().merge(file))
}

impl ResolvedRangGuideConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedRangGuideTexts {
                hero_intro: RANG_GUIDE_HERO_INTRO.to_string(),
                step1_title: RANG_GUIDE_STEP1_TITLE.to_string(),
                step1_body: RANG_GUIDE_STEP1_BODY.to_string(),
                step2_body: RANG_GUIDE_STEP2_BODY.to_string(),
                step3_body: RANG_GUIDE_STEP3_BODY.to_string(),
                step4_title: RANG_GUIDE_STEP4_TITLE.to_string(),
                step4_body: RANG_GUIDE_STEP4_BODY.to_string(),
                extras_title: RANG_GUIDE_EXTRAS_TITLE.to_string(),
                extras_body: RANG_GUIDE_EXTRAS_BODY.to_string(),
            },
            buttons: ResolvedRangGuideButtons {
                steam_open: RANG_GUIDE_STEAM_OPEN_BUTTON_LABEL.to_string(),
                linked_role: RANG_GUIDE_LINKED_ROLE_BUTTON_LABEL.to_string(),
                friend_code: RANG_GUIDE_FRIEND_CODE_BUTTON_LABEL.to_string(),
                rankcheck: RANG_GUIDE_RANKCHECK_BUTTON_LABEL.to_string(),
                unlink: RANG_GUIDE_UNLINK_BUTTON_LABEL.to_string(),
            },
            urls: ResolvedRangGuideUrls {
                linked_role_login: LINKED_ROLE_LOGIN_URL_DEFAULT.to_string(),
            },
        }
    }

    fn merge(mut self, file: RangGuideTextsToml) -> Self {
        apply_optional(&mut self.texts.hero_intro, file.texts.hero_intro);
        apply_optional(&mut self.texts.step1_title, file.texts.step1_title);
        apply_optional(&mut self.texts.step1_body, file.texts.step1_body);
        apply_optional(&mut self.texts.step2_body, file.texts.step2_body);
        apply_optional(&mut self.texts.step3_body, file.texts.step3_body);
        apply_optional(&mut self.texts.step4_title, file.texts.step4_title);
        apply_optional(&mut self.texts.step4_body, file.texts.step4_body);
        apply_optional(&mut self.texts.extras_title, file.texts.extras_title);
        apply_optional(&mut self.texts.extras_body, file.texts.extras_body);

        apply_optional(&mut self.buttons.steam_open, file.buttons.steam_open);
        apply_optional(&mut self.buttons.linked_role, file.buttons.linked_role);
        apply_optional(&mut self.buttons.friend_code, file.buttons.friend_code);
        apply_optional(&mut self.buttons.rankcheck, file.buttons.rankcheck);
        apply_optional(&mut self.buttons.unlink, file.buttons.unlink);

        apply_optional(
            &mut self.urls.linked_role_login,
            file.urls.linked_role_login,
        );
        self
    }
}

fn apply_optional(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn rang_guide_messages(
    repo_root: &Path,
    config: &ResolvedRangGuideConfig,
    warnings: &mut Vec<String>,
) -> Vec<RangGuideBuiltMessage> {
    let hero_banner = optional_rang_guide_banner(repo_root, RANG_GUIDE_HERO_FILENAME, warnings);
    let step_banners: Vec<Option<RangGuideBannerOutput>> = RANG_GUIDE_STEP_BANNER_FILENAMES
        .iter()
        .map(|filename| optional_rang_guide_banner(repo_root, filename, warnings))
        .collect();
    let help_banner =
        optional_rang_guide_banner(repo_root, RANG_GUIDE_HELP_BANNER_FILENAME, warnings);

    let mut banner_candidates: Vec<RangGuideBannerOutput> = Vec::new();
    for banner in std::iter::once(&hero_banner)
        .chain(step_banners.iter())
        .chain(std::iter::once(&help_banner))
        .flatten()
    {
        banner_candidates.push(banner.clone());
    }

    let mut hero_components = Vec::new();
    if let Some(banner) = &hero_banner {
        hero_components.push(media_gallery(
            RANG_GUIDE_COMPONENT_ID_HERO_MEDIA,
            &banner.filename,
        ));
    }
    hero_components.push(text_display(
        RANG_GUIDE_COMPONENT_ID_HERO_TEXT,
        config.texts.hero_intro.clone(),
    ));

    let step_media = |index: usize, component_id: u64| -> Option<Value> {
        step_banners[index]
            .as_ref()
            .map(|banner| media_gallery(component_id, &banner.filename))
    };

    let mut step1_children = Vec::new();
    step1_children.extend(step_media(0, RANG_GUIDE_COMPONENT_ID_STEP1_MEDIA));
    step1_children.push(text_display(
        RANG_GUIDE_COMPONENT_ID_STEP1_TEXT,
        section_text(&config.texts.step1_title, &config.texts.step1_body),
    ));
    step1_children.push(action_row(
        RANG_GUIDE_COMPONENT_ID_STEP1_ACTION_ROW,
        vec![button(
            RANG_GUIDE_COMPONENT_ID_STEAM_OPEN_BUTTON,
            &config.buttons.steam_open,
            1,
            STEAM_LINK_OPEN_CUSTOM_ID,
        )],
    ));

    let mut step2_children = Vec::new();
    step2_children.extend(step_media(1, RANG_GUIDE_COMPONENT_ID_STEP2_MEDIA));
    step2_children.push(text_display(
        RANG_GUIDE_COMPONENT_ID_STEP2_TEXT,
        config.texts.step2_body.clone(),
    ));

    let mut step3_children = Vec::new();
    step3_children.extend(step_media(2, RANG_GUIDE_COMPONENT_ID_STEP3_MEDIA));
    step3_children.push(text_display(
        RANG_GUIDE_COMPONENT_ID_STEP3_TEXT,
        config.texts.step3_body.clone(),
    ));

    let mut step4_children = Vec::new();
    step4_children.extend(step_media(3, RANG_GUIDE_COMPONENT_ID_STEP4_MEDIA));
    step4_children.push(text_display(
        RANG_GUIDE_COMPONENT_ID_STEP4_TEXT,
        section_text(&config.texts.step4_title, &config.texts.step4_body),
    ));
    step4_children.push(action_row(
        RANG_GUIDE_COMPONENT_ID_STEP4_ACTION_ROW,
        vec![link_button(
            RANG_GUIDE_COMPONENT_ID_LINKED_ROLE_BUTTON,
            &config.buttons.linked_role,
            &config.urls.linked_role_login,
        )],
    ));

    let mut extras_children = Vec::new();
    if let Some(banner) = &help_banner {
        extras_children.push(media_gallery(
            RANG_GUIDE_COMPONENT_ID_EXTRAS_MEDIA,
            &banner.filename,
        ));
    }
    extras_children.push(text_display(
        RANG_GUIDE_COMPONENT_ID_EXTRAS_TEXT,
        section_text(&config.texts.extras_title, &config.texts.extras_body),
    ));
    extras_children.push(action_row(
        RANG_GUIDE_COMPONENT_ID_EXTRAS_ACTION_ROW,
        vec![
            button(
                RANG_GUIDE_COMPONENT_ID_FRIEND_CODE_BUTTON,
                &config.buttons.friend_code,
                2,
                STEAM_LINK_FRIEND_CODE_CUSTOM_ID,
            ),
            button(
                RANG_GUIDE_COMPONENT_ID_RANKCHECK_BUTTON,
                &config.buttons.rankcheck,
                2,
                STEAM_LINK_RANKCHECK_CUSTOM_ID,
            ),
            button(
                RANG_GUIDE_COMPONENT_ID_UNLINK_BUTTON,
                &config.buttons.unlink,
                4,
                STEAM_LINK_UNLINK_CUSTOM_ID,
            ),
        ],
    ));

    let mut components = vec![
        container(RANG_GUIDE_COMPONENT_ID_HERO_CONTAINER, hero_components),
        container(RANG_GUIDE_COMPONENT_ID_STEP1_CONTAINER, step1_children),
        container(RANG_GUIDE_COMPONENT_ID_STEP2_CONTAINER, step2_children),
        container(RANG_GUIDE_COMPONENT_ID_STEP3_CONTAINER, step3_children),
    ];
    // Siegel-Abschnitt (Linked Role) ist per leerem step4_body abschaltbar —
    // Re-Aktivierung ist damit reine TOML-Aenderung ohne Deploy.
    if !config.texts.step4_body.trim().is_empty() {
        components.push(container(
            RANG_GUIDE_COMPONENT_ID_STEP4_CONTAINER,
            step4_children,
        ));
    }
    components.push(container(
        RANG_GUIDE_COMPONENT_ID_EXTRAS_CONTAINER,
        extras_children,
    ));

    chunk_rang_guide_components(components, &banner_candidates, hero_banner)
}

/// `**Titel**\nBody` — oder nur der Body, wenn der Titel leer ist (Ueberschrift
/// steckt dann im Schritt-Banner).
fn section_text(title: &str, body: &str) -> String {
    let title = title.trim();
    if title.is_empty() {
        body.to_string()
    } else {
        format!("**{title}**\n{body}")
    }
}

fn chunk_rang_guide_components(
    components: Vec<Value>,
    banner_candidates: &[RangGuideBannerOutput],
    hero_banner: Option<RangGuideBannerOutput>,
) -> Vec<RangGuideBuiltMessage> {
    let mut chunks = Vec::<Vec<Value>>::new();
    let mut current = Vec::<Value>::new();
    let mut current_text_chars = 0usize;
    let mut current_component_count = 0usize;

    for component in components {
        let component_text_chars = text_display_chars_in_value(&component);
        let component_count = component_count_in_value(&component);
        if !current.is_empty()
            && (current_text_chars + component_text_chars > RANG_GUIDE_TEXT_CHAR_BUDGET
                || current_component_count + component_count > RANG_GUIDE_COMPONENT_BUDGET)
        {
            chunks.push(std::mem::take(&mut current));
            current_text_chars = 0;
            current_component_count = 0;
        }
        current_text_chars += component_text_chars;
        current_component_count += component_count;
        current.push(component);
    }
    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
        .into_iter()
        .map(|components| {
            // Attachments gehoeren zu der Nachricht, deren Components sie per
            // attachment://-URL referenzieren; IDs pro Nachricht ab 0.
            let mut refs = Vec::new();
            for component in &components {
                collect_attachment_refs(component, &mut refs);
            }
            let attachments = refs
                .iter()
                .filter_map(|name| {
                    banner_candidates
                        .iter()
                        .find(|banner| &banner.filename == name)
                })
                .enumerate()
                .map(|(index, banner)| RangGuidePayloadAttachment {
                    id: index as u8,
                    filename: banner.filename.clone(),
                    relative_path: banner.relative_path.clone(),
                })
                .collect();
            let has_hero = hero_banner
                .as_ref()
                .is_some_and(|banner| refs.iter().any(|name| name == &banner.filename));
            RangGuideBuiltMessage {
                banner: if has_hero { hero_banner.clone() } else { None },
                payload: RangGuideMessagePayload {
                    flags: RANG_GUIDE_COMPONENTS_V2_FLAG,
                    allowed_mentions: RangGuideAllowedMentions { parse: Vec::new() },
                    components,
                    attachments,
                },
            }
        })
        .collect()
}

fn collect_attachment_refs(value: &Value, refs: &mut Vec<String>) {
    if let Some(name) = value
        .get("media")
        .and_then(|media| media.get("url"))
        .and_then(Value::as_str)
        .and_then(|url| url.strip_prefix("attachment://"))
    {
        if !refs.iter().any(|existing| existing == name) {
            refs.push(name.to_string());
        }
    }
    for key in ["components", "items"] {
        if let Some(children) = value.get(key).and_then(Value::as_array) {
            for child in children {
                collect_attachment_refs(child, refs);
            }
        }
    }
}

fn optional_rang_guide_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> Option<RangGuideBannerOutput> {
    let relative_path = format!("{RANG_GUIDE_BANNER_DIR}/{filename}");
    let path = repo_root.join(&relative_path);
    if !path.is_file() {
        warnings.push(format!(
            "Rang-Guide-Banner `{relative_path}` fehlt; Publish laeuft ohne Banner"
        ));
        return None;
    }
    Some(RangGuideBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present: true,
    })
}

fn planned_rang_guide_action(
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

fn rang_guide_message_key(message_index: usize) -> String {
    if message_index == 0 {
        "rang-guide".to_string()
    } else {
        format!("rang-guide:{message_index}")
    }
}

fn validate_rang_guide_message_budget(payload: &RangGuideMessagePayload) -> Result<(), String> {
    let text_chars = text_display_chars(&payload.components);
    if text_chars > RANG_GUIDE_TEXT_CHAR_BUDGET {
        return Err(format!(
            "Rang-Guide-Payload ueberschreitet Textbudget: {text_chars}/{RANG_GUIDE_TEXT_CHAR_BUDGET}"
        ));
    }
    if max_text_display_chars(&payload.components) > MESSAGE_TEXT_DISPLAY_CHAR_LIMIT {
        return Err(format!(
            "Rang-Guide-Payload ueberschreitet TextDisplay-Limit {MESSAGE_TEXT_DISPLAY_CHAR_LIMIT}"
        ));
    }
    let component_count = component_count(&payload.components);
    if component_count > RANG_GUIDE_COMPONENT_BUDGET {
        return Err(format!(
            "Rang-Guide-Payload ueberschreitet Komponentenbudget: {component_count}/{RANG_GUIDE_COMPONENT_BUDGET}"
        ));
    }
    if payload.attachments.len() > RANG_GUIDE_ATTACHMENT_BUDGET {
        return Err(format!(
            "Rang-Guide-Payload ueberschreitet Attachmentbudget: {}/{RANG_GUIDE_ATTACHMENT_BUDGET}",
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
        "accent_color": RANG_GUIDE_ACCENT_GOLD,
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

    fn write_rang_guide_texts_file(repo_root: &Path, body: &str) {
        let path = repo_root.join(RANG_GUIDE_TEXTS_FILE);
        fs::create_dir_all(path.parent().expect("texts parent")).expect("mkdir texts parent");
        fs::write(path, body).expect("write texts");
    }

    fn write_banner(repo_root: &Path) {
        write_banner_bytes(repo_root, b"png");
    }

    fn write_named_banner(repo_root: &Path, filename: &str) {
        let path = repo_root.join(format!("{RANG_GUIDE_BANNER_DIR}/{filename}"));
        fs::create_dir_all(path.parent().expect("banner parent")).expect("mkdir banner parent");
        fs::write(path, filename.as_bytes()).expect("write banner");
    }

    fn write_all_banners(repo_root: &Path) {
        write_banner(repo_root);
        for filename in RANG_GUIDE_STEP_BANNER_FILENAMES {
            write_named_banner(repo_root, filename);
        }
        write_named_banner(repo_root, RANG_GUIDE_HELP_BANNER_FILENAME);
    }

    fn write_banner_bytes(repo_root: &Path, bytes: &[u8]) {
        let path = repo_root.join(format!(
            "{RANG_GUIDE_BANNER_DIR}/{RANG_GUIDE_HERO_FILENAME}"
        ));
        fs::create_dir_all(path.parent().expect("banner parent")).expect("mkdir banner parent");
        fs::write(path, bytes).expect("write banner");
    }

    fn all_custom_ids(output: &RangGuidePublishOutput) -> Vec<String> {
        output
            .messages
            .iter()
            .flat_map(|message| collect_component_custom_ids(&message.payload.components))
            .collect()
    }

    #[test]
    fn rang_guide_texts_temp_toml_mergt_fehlende_felder_auf_compile_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_rang_guide_texts_file(
            temp.path(),
            r#"
[texts]
step2_body = "Runtime Schritt 2"

[buttons]
friend_code = "Runtime Freundescode"
unlink = "Runtime Verknüpfung entfernen"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_rang_guide_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.hero_intro, RANG_GUIDE_HERO_INTRO);
        assert_eq!(config.texts.step2_body, "Runtime Schritt 2");
        assert_eq!(config.texts.step4_body, RANG_GUIDE_STEP4_BODY);
        assert_eq!(config.buttons.friend_code, "Runtime Freundescode");
        assert_eq!(config.buttons.rankcheck, RANG_GUIDE_RANKCHECK_BUTTON_LABEL);
        assert_eq!(config.buttons.unlink, "Runtime Verknüpfung entfernen");
        assert_eq!(config.urls.linked_role_login, LINKED_ROLE_LOGIN_URL_DEFAULT);
    }

    #[test]
    fn rang_guide_texts_parse_fehler_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_rang_guide_texts_file(temp.path(), "[texts]\nhero_intro = [\n");
        let mut warnings = Vec::new();
        let err =
            load_rang_guide_runtime_config(temp.path(), &mut warnings).expect_err("parse error");
        assert!(err.contains("konnte nicht geparst werden"));
    }

    #[test]
    fn rang_guide_texts_fehlende_datei_nutzt_defaults_mit_warnung() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut warnings = Vec::new();
        let config = load_rang_guide_runtime_config(temp.path(), &mut warnings).expect("defaults");
        assert_eq!(config.texts.hero_intro, RANG_GUIDE_HERO_INTRO);
        assert!(warnings
            .iter()
            .any(|warning| warning.contains("Compile-Defaults")));
    }

    #[test]
    fn rang_guide_texts_toml_mergt_teilangaben_und_url() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_rang_guide_texts_file(
            temp.path(),
            r#"
[texts]
step2_body = "Platzhalter: Runtime Schritt 2"

[urls]
linked_role_login = "https://example.invalid/linked-role"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_rang_guide_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.hero_intro, RANG_GUIDE_HERO_INTRO);
        assert_eq!(config.texts.step2_body, "Platzhalter: Runtime Schritt 2");
        assert_eq!(
            config.urls.linked_role_login,
            "https://example.invalid/linked-role"
        );
    }

    #[test]
    fn rang_guide_payload_haelt_budgets_reihenfolge_mentions_und_custom_ids() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner(temp.path());
        let output = build_rang_guide_publish_output(
            temp.path(),
            &[],
            Some(RANG_GUIDE_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("output");
        assert_eq!(output.messages.len(), 1);
        let payload = &output.messages[0].payload;
        assert_eq!(payload.flags, RANG_GUIDE_COMPONENTS_V2_FLAG);
        assert!(payload.allowed_mentions.parse.is_empty());
        assert_eq!(payload.attachments.len(), 1);
        assert_eq!(payload.attachments[0].filename, RANG_GUIDE_HERO_FILENAME);

        let hero_container = &payload.components[0];
        let hero_children = hero_container["components"]
            .as_array()
            .expect("hero children");
        assert_eq!(
            hero_container["accent_color"],
            json!(RANG_GUIDE_ACCENT_GOLD)
        );
        assert_eq!(hero_children[0]["type"], json!(12));
        assert_eq!(hero_children[1]["type"], json!(10));
        assert_eq!(
            hero_children[0]["items"][0]["media"]["url"],
            format!("attachment://{RANG_GUIDE_HERO_FILENAME}")
        );

        assert_eq!(
            all_custom_ids(&output),
            vec![
                STEAM_LINK_OPEN_CUSTOM_ID.to_string(),
                STEAM_LINK_FRIEND_CODE_CUSTOM_ID.to_string(),
                STEAM_LINK_RANKCHECK_CUSTOM_ID.to_string(),
                STEAM_LINK_UNLINK_CUSTOM_ID.to_string(),
            ]
        );
        assert_eq!(
            payload.components[4]["components"][1]["components"][0]["style"],
            json!(5)
        );
        assert_eq!(
            payload.components[4]["components"][1]["components"][0]["url"],
            LINKED_ROLE_LOGIN_URL_DEFAULT
        );
        validate_rang_guide_message_budget(payload).expect("budget");
        assert!(component_count(&payload.components) <= RANG_GUIDE_COMPONENT_BUDGET);
        assert!(text_display_chars(&payload.components) <= RANG_GUIDE_TEXT_CHAR_BUDGET);
        assert!(has_rang_guide_v2_marker(&payload.components));
    }

    #[test]
    fn rang_guide_buttons_bleiben_in_ihren_sektionen_und_unlink_ist_letzter_help_button() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output =
            build_rang_guide_publish_output(temp.path(), &[], None, None, true).expect("output");
        let payload = &output.messages[0].payload;

        let step1_buttons = payload.components[1]["components"][1]["components"]
            .as_array()
            .expect("step1 buttons");
        assert_eq!(step1_buttons.len(), 1);
        assert_eq!(step1_buttons[0]["custom_id"], STEAM_LINK_OPEN_CUSTOM_ID);
        assert_eq!(
            step1_buttons[0]["label"],
            RANG_GUIDE_STEAM_OPEN_BUTTON_LABEL
        );
        assert_eq!(step1_buttons[0]["style"], json!(1));

        let help_buttons = payload.components[5]["components"][1]["components"]
            .as_array()
            .expect("help buttons");
        assert_eq!(help_buttons.len(), 3);
        assert_eq!(
            help_buttons[0]["custom_id"],
            STEAM_LINK_FRIEND_CODE_CUSTOM_ID
        );
        assert_eq!(
            help_buttons[0]["label"],
            RANG_GUIDE_FRIEND_CODE_BUTTON_LABEL
        );
        assert_eq!(help_buttons[0]["style"], json!(2));
        assert_eq!(help_buttons[1]["custom_id"], STEAM_LINK_RANKCHECK_CUSTOM_ID);
        assert_eq!(help_buttons[1]["label"], RANG_GUIDE_RANKCHECK_BUTTON_LABEL);
        assert_eq!(help_buttons[1]["style"], json!(2));
        assert_eq!(help_buttons[2]["custom_id"], "steam_link_panel:unlink");
        assert_eq!(help_buttons[2]["label"], "🔓 Verknüpfung entfernen");
        assert_eq!(help_buttons[2]["style"], json!(4));
    }

    #[test]
    fn rang_guide_schritt_banner_media_zuerst_und_leerer_titel_ohne_fettzeile() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_all_banners(temp.path());
        write_rang_guide_texts_file(
            temp.path(),
            r#"
[texts]
step1_title = ""
"#,
        );
        let output =
            build_rang_guide_publish_output(temp.path(), &[], None, None, true).expect("output");
        assert_eq!(output.messages.len(), 1);
        let payload = &output.messages[0].payload;
        assert_eq!(payload.attachments.len(), 6);
        for (index, attachment) in payload.attachments.iter().enumerate() {
            assert_eq!(attachment.id as usize, index);
        }
        // Jeder Abschnitts-Container beginnt mit seinem Banner (type 12)
        for container_index in 0..=5 {
            assert_eq!(
                payload.components[container_index]["components"][0]["type"],
                json!(12),
                "Container {container_index} ohne Banner-Media"
            );
        }
        // Leerer Titel: Text startet direkt mit dem Body statt einer Fettzeile
        let step1_text = payload.components[1]["components"][1]["content"]
            .as_str()
            .expect("step1 text");
        assert!(!step1_text.starts_with("**"));
        validate_rang_guide_message_budget(payload).expect("budget");
    }

    #[test]
    fn rang_guide_attachments_haengen_am_referenzierenden_chunk() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_all_banners(temp.path());
        let long_step2 = format!("Platzhalter: Schritt 2 lang. {}", "Satz. ".repeat(400));
        let long_step3 = format!("Platzhalter: Schritt 3 lang. {}", "Satz. ".repeat(400));
        write_rang_guide_texts_file(
            temp.path(),
            &format!(
                r#"
[texts]
step2_body = "{long_step2}"
step3_body = "{long_step3}"
"#
            ),
        );

        let output =
            build_rang_guide_publish_output(temp.path(), &[], None, None, true).expect("output");
        assert!(output.messages.len() > 1);
        for message in &output.messages {
            let mut refs = Vec::new();
            for component in &message.payload.components {
                collect_attachment_refs(component, &mut refs);
            }
            let filenames: Vec<String> = message
                .payload
                .attachments
                .iter()
                .map(|attachment| attachment.filename.clone())
                .collect();
            assert_eq!(refs, filenames, "Attachments passen nicht zu Referenzen");
            for (index, attachment) in message.payload.attachments.iter().enumerate() {
                assert_eq!(attachment.id as usize, index);
            }
            validate_rang_guide_message_budget(&message.payload).expect("budget");
        }
        assert!(output.messages[0].banner.is_some());
        assert!(output.messages[1..].iter().all(|m| m.banner.is_none()));
    }

    #[test]
    fn rang_guide_leerer_step4_body_laesst_siegel_abschnitt_weg() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_all_banners(temp.path());
        write_rang_guide_texts_file(temp.path(), "[texts]\nstep4_body = \"\"\n");
        let output =
            build_rang_guide_publish_output(temp.path(), &[], None, None, true).expect("output");
        assert_eq!(output.messages.len(), 1);
        let payload = &output.messages[0].payload;
        assert_eq!(payload.components.len(), 5);
        let blob = serde_json::to_string(&payload.components).expect("json");
        assert!(!blob.contains(LINKED_ROLE_LOGIN_URL_DEFAULT));
        assert!(!blob.contains("rang-guide-schritt-4.png"));
        assert_eq!(payload.attachments.len(), 5);
        assert_eq!(
            all_custom_ids(&output),
            vec![
                STEAM_LINK_OPEN_CUSTOM_ID.to_string(),
                STEAM_LINK_FRIEND_CODE_CUSTOM_ID.to_string(),
                STEAM_LINK_RANKCHECK_CUSTOM_ID.to_string(),
                STEAM_LINK_UNLINK_CUSTOM_ID.to_string(),
            ]
        );
    }

    #[test]
    fn rang_guide_payload_hash_aendert_sich_bei_banner_bytes() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner-one");
        let first = build_rang_guide_publish_output(
            temp.path(),
            &[7001],
            Some(RANG_GUIDE_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("first output");

        write_banner_bytes(temp.path(), b"banner-two");
        let changed = build_rang_guide_publish_output(
            temp.path(),
            &[7001],
            Some(RANG_GUIDE_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("changed output");

        assert_eq!(changed.messages[0].action, "planned_edit");
        assert_ne!(changed.payload_hash, first.payload_hash);
        assert!(!rang_guide_payload_is_unchanged(&changed));
    }

    #[test]
    fn rang_guide_budget_grenzen_sind_exakt_inklusiv() {
        let payload = |components: Vec<Value>, attachment_count: usize| RangGuideMessagePayload {
            flags: RANG_GUIDE_COMPONENTS_V2_FLAG,
            allowed_mentions: RangGuideAllowedMentions { parse: Vec::new() },
            components,
            attachments: (0..attachment_count)
                .map(|index| RangGuidePayloadAttachment {
                    id: index as u8,
                    filename: format!("banner-{index}.png"),
                    relative_path: format!("assets/welcome-banners/banner-{index}.png"),
                })
                .collect(),
        };

        validate_rang_guide_message_budget(&payload(
            vec![text_display(1, "x".repeat(RANG_GUIDE_TEXT_CHAR_BUDGET))],
            RANG_GUIDE_ATTACHMENT_BUDGET,
        ))
        .expect("3500 chars and 9 attachments are valid");
        validate_rang_guide_message_budget(&payload(
            (0..RANG_GUIDE_COMPONENT_BUDGET)
                .map(|index| text_display(index as u64 + 1, String::new()))
                .collect(),
            0,
        ))
        .expect("35 components are valid");

        assert!(validate_rang_guide_message_budget(&payload(
            vec![text_display(1, "x".repeat(RANG_GUIDE_TEXT_CHAR_BUDGET + 1))],
            0,
        ))
        .is_err());
        assert!(validate_rang_guide_message_budget(&payload(
            (0..=RANG_GUIDE_COMPONENT_BUDGET)
                .map(|index| text_display(index as u64 + 1, String::new()))
                .collect(),
            0,
        ))
        .is_err());
        assert!(validate_rang_guide_message_budget(&payload(
            vec![text_display(1, String::new())],
            RANG_GUIDE_ATTACHMENT_BUDGET + 1,
        ))
        .is_err());
    }

    #[test]
    fn rang_guide_payload_fehlendes_banner_warnt_und_publisht_ohne_attachment() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output =
            build_rang_guide_publish_output(temp.path(), &[], None, None, true).expect("output");
        assert!(output.messages[0].payload.attachments.is_empty());
        assert!(output.messages[0].banner.is_none());
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("ohne Banner")));
    }

    #[test]
    fn rang_guide_chunking_bleibt_an_semantischen_top_level_schritten() {
        let temp = tempfile::tempdir().expect("tempdir");
        let long_step2 = format!("Platzhalter: Schritt 2 lang. {}", "Satz. ".repeat(400));
        let long_step3 = format!("Platzhalter: Schritt 3 lang. {}", "Satz. ".repeat(400));
        write_rang_guide_texts_file(
            temp.path(),
            &format!(
                r#"
[texts]
step2_body = "{long_step2}"
step3_body = "{long_step3}"
"#
            ),
        );

        let output =
            build_rang_guide_publish_output(temp.path(), &[], None, None, true).expect("output");

        assert!(output.messages.len() > 1);
        for message in &output.messages {
            validate_rang_guide_message_budget(&message.payload).expect("budget");
            assert!(has_rang_guide_v2_marker(&message.payload.components));
        }
        assert_eq!(
            output.messages[0].payload.components[0]["components"][0]["content"],
            RANG_GUIDE_HERO_INTRO
        );
        assert_eq!(
            output
                .messages
                .last()
                .expect("last")
                .payload
                .components
                .last()
                .expect("last component")["components"][1]["components"][2]["custom_id"],
            STEAM_LINK_UNLINK_CUSTOM_ID
        );
    }

    #[test]
    fn rang_guide_storage_erkennt_edit_noop_und_formatwechsel() {
        let temp = tempfile::tempdir().expect("tempdir");
        let first = build_rang_guide_publish_output(
            temp.path(),
            &[7001],
            Some(RANG_GUIDE_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("first");
        let unchanged = build_rang_guide_publish_output(
            temp.path(),
            &[7001],
            Some(RANG_GUIDE_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("unchanged");

        assert!(!unchanged.repost_required);
        assert_eq!(unchanged.messages[0].action, "planned_no_op");
        assert!(rang_guide_payload_is_unchanged(&unchanged));

        let wrong_format =
            build_rang_guide_publish_output(temp.path(), &[7001], Some("1"), None, true)
                .expect("wrong format");
        assert!(wrong_format.repost_required);
        assert_eq!(wrong_format.messages[0].action, "planned_repost");

        let kv_loss = build_rang_guide_publish_output(
            temp.path(),
            &[],
            Some(RANG_GUIDE_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("kv loss");
        assert!(kv_loss.repost_required);
        assert_eq!(kv_loss.messages[0].action, "planned_post");
    }

    #[test]
    fn rang_guide_legacy_cleanup_loescht_nie_fremde_autoren() {
        let bot_user_id = 42;
        let base = RangGuideLegacyMessage {
            message_id: 7001,
            author_id: bot_user_id,
            flags: 0,
            has_embeds: true,
            custom_ids: vec![STEAM_LINK_OPEN_CUSTOM_ID.to_string()],
        };
        assert!(is_legacy_rank_guide_cleanup_candidate(&base, bot_user_id));

        let foreign = RangGuideLegacyMessage {
            author_id: 99,
            ..base.clone()
        };
        assert!(!is_legacy_rank_guide_cleanup_candidate(
            &foreign,
            bot_user_id
        ));

        let v2 = RangGuideLegacyMessage {
            flags: RANG_GUIDE_COMPONENTS_V2_FLAG,
            ..base.clone()
        };
        assert!(!is_legacy_rank_guide_cleanup_candidate(&v2, bot_user_id));

        let no_embed = RangGuideLegacyMessage {
            has_embeds: false,
            ..base.clone()
        };
        assert!(!is_legacy_rank_guide_cleanup_candidate(
            &no_embed,
            bot_user_id
        ));

        let unrelated_button = RangGuideLegacyMessage {
            custom_ids: vec!["router_spawn_casual".to_string()],
            ..base
        };
        assert!(!is_legacy_rank_guide_cleanup_candidate(
            &unrelated_button,
            bot_user_id
        ));
    }
}
