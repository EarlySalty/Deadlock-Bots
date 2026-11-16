use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use dl_core::runtime_config::SUPPORTER_TWITCH_CHANNELS;

pub const SUPPORT_CHANNEL_ID: u64 = 1_483_136_301_271_355_532;
pub const SUPPORT_CHANNEL_NAME: &str = "server-support";
pub const SUPPORT_TEXTS_FILE: &str = "assets/support_texts.toml";
pub const SUPPORT_BANNER_DIR: &str = "assets/welcome-banners";
pub const SUPPORT_HERO_FILENAME: &str = "support-hero.png";
pub const SUPPORT_PAYLOAD_FORMAT: &str = "3";
pub const SUPPORT_PAYLOAD_FORMAT_KEY: &str = "support_v2_payload_format";
pub const SUPPORT_PAYLOAD_HASH_KEY: &str = "support_v2_payload_hash";
pub const SUPPORT_MESSAGE_ID_PREFIX: &str = "support_v2_message_id_";
pub const SUPPORT_FUNDING_LIVE_KEY: &str = "support_v3_funding_live";
pub const SUPPORT_PANEL_OPTIONS_KEY: &str = "support_v3_panel_options";
pub const SUPPORT_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const SUPPORT_ACCENT_GOLD: u64 = 0xC8A86B;
#[cfg(test)]
pub const SUPPORT_OWNER_MESSAGE_ID: u64 = 1_483_147_756_192_403_467;

pub const SUPPORT_BUTTON_BECOME: &str = "support:become";
pub const SUPPORT_BUTTON_TIP: &str = "support:tip";
pub const SUPPORT_BUTTON_MANAGE: &str = "support:manage";
pub const SUPPORT_BUTTON_TWITCH: &str = "support:twitch";

pub const SUPPORT_COMPONENT_ID_HERO_MEDIA: u64 = 33_001;
pub const SUPPORT_COMPONENT_ID_MAIN_CONTAINER: u64 = 33_002;
pub const SUPPORT_COMPONENT_ID_MAIN_TEXT: u64 = 33_003;
pub const SUPPORT_COMPONENT_ID_ACTION_ROW: u64 = 33_004;
pub const SUPPORT_COMPONENT_ID_BECOME_BUTTON: u64 = 33_005;
pub const SUPPORT_COMPONENT_ID_TIP_BUTTON: u64 = 33_006;
pub const SUPPORT_COMPONENT_ID_MANAGE_BUTTON: u64 = 33_007;
pub const SUPPORT_COMPONENT_ID_TWITCH_BUTTON: u64 = 33_008;
pub const SUPPORT_COMPONENT_ID_PERKS: u64 = 33_014;
pub const SUPPORT_COMPONENT_ID_BENEFITS_CONTAINER: u64 = 33_020;
pub const SUPPORT_COMPONENT_ID_STEAM: u64 = 33_021;
pub const SUPPORT_COMPONENT_ID_PLANNED: u64 = 33_027;
pub const SUPPORT_COMPONENT_ID_FREE: u64 = 33_029;
pub const SUPPORT_COMPONENT_ID_PAY_CONTAINER: u64 = 33_030;
pub const SUPPORT_COMPONENT_ID_PAY_BODY: u64 = 33_031;
pub const SUPPORT_COMPONENT_ID_PAY_THANKS: u64 = 33_033;
pub const SUPPORT_COMPONENT_ID_PAY_SEP_ACTIONS: u64 = 33_034;
pub const SUPPORT_COMPONENT_ID_FUNDING: u64 = 33_035;

pub const SUPPORT_MARKER_COMPONENT_IDS: &[u64] = &[
    SUPPORT_COMPONENT_ID_HERO_MEDIA,
    SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
    SUPPORT_COMPONENT_ID_MAIN_TEXT,
    SUPPORT_COMPONENT_ID_ACTION_ROW,
    SUPPORT_COMPONENT_ID_BENEFITS_CONTAINER,
    SUPPORT_COMPONENT_ID_PAY_CONTAINER,
];

pub const SUPPORT_TEXT_CHAR_BUDGET: usize = 3500;
pub const SUPPORT_COMPONENT_BUDGET: usize = 35;
pub const SUPPORT_ATTACHMENT_BUDGET: usize = 9;
pub const SUPPORT_TIER_BUTTON_PREFIX: &str = "support:tier:";
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;
const SUPPORT_MESSAGE_COUNT: usize = 3;
const FUNDING_BAR_SEGMENTS: i64 = 10;
const BUTTON_LABEL_MAX_CHARS: usize = 80;
const CUSTOM_ID_MAX_CHARS: usize = 100;
const TIER_ID_MAX_DIGITS: usize = u64::MAX.ilog10() as usize + 1;
const _: () = assert!(
    SUPPORT_BUTTON_BECOME.len() <= CUSTOM_ID_MAX_CHARS
        && SUPPORT_BUTTON_TIP.len() <= CUSTOM_ID_MAX_CHARS
        && SUPPORT_BUTTON_MANAGE.len() <= CUSTOM_ID_MAX_CHARS
        && SUPPORT_BUTTON_TWITCH.len() <= CUSTOM_ID_MAX_CHARS
        && SUPPORT_TIER_BUTTON_PREFIX.len() + TIER_ID_MAX_DIGITS <= CUSTOM_ID_MAX_CHARS
);
#[cfg(test)]
pub(crate) const SUPPORT_BANNED_COPY: [&str; 6] = [
    "Ko-fi",
    "KI-Abos",
    "Warum ich",
    "Server-Abo",
    "admin.",
    "\u{2014}",
];

pub const SUPPORT_INTRO: &str = "\
# Server unterstützen\n\
\n\
Damit trägst du den Server und unsere Projekte mit.";
pub const SUPPORT_PERKS: &str = "\
## Das bekommst du\n\
\n\
**Streamer-VC.** Zock mit den Streamern zusammen.\n\
**Rolle Server Unterstützer.** Jeder sieht, dass du den Server mitträgst.";
pub const SUPPORT_STEAM: &str = "\
## Als Nächstes\n\
\n\
**Steam Bot.** Holt deine Matches bevorzugt ab, mit ganzer Historie und Rang direkt von Steam.";
pub const SUPPORT_PLANNED: &str = "\
## Was noch kommt\n\
\n\
**Patch-Watchlist.** Du erfährst, wenn ein Patch deine Helden oder Items ändert.\n\
**Build-Helfer im Deadlock Brain.** Passende Items für deinen Helden, auch gegen bestimmte Gegner.\n\
**Turniere.** Neue Turnierfunktionen zuerst testen, dazu Fun-Turniere.\n\
**Coaching-Material.** Nur für Unterstützer.\n\
**Mitbestimmen.** Du stimmst ab, was als Nächstes gebaut wird.";
pub const SUPPORT_FREE: &str = "\
## Bleibt kostenlos\n\
\n\
Mitspieler suchen, Voice-Channels, Community-Abende, reguläre Turniere, Scrims und Coaching.";
pub const SUPPORT_FUNDING: &str = "\
**Monatsziel:** {stand} / {ziel}\n\
{balken}";
pub const SUPPORT_PAY: &str = "\
## So unterstützt du\n\
\n\
**Monatlich.** {stufen} im Monat, jederzeit kündbar.\n\
**Einmalig.** Betrag frei wählbar.\n\
**Server-Boost.** Zählt auch.";
pub const SUPPORT_TWITCH: &str = "**Twitch-Sub.** Ein Abo bei {kanaele} zählt auch.";
pub const SUPPORT_THANKS: &str = "Danke, dass du den Server und die Projekte mitträgst ❤️";
pub const SUPPORT_BECOME_BUTTON_LABEL: &str = "Supporter werden";
pub const SUPPORT_TIP_BUTTON_LABEL: &str = "Einmalig unterstützen";
pub const SUPPORT_MANAGE_BUTTON_LABEL: &str = "Abo verwalten oder kündigen";
pub const SUPPORT_TWITCH_BUTTON_LABEL: &str = "Twitch verbinden";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SupportFunding {
    pub raised_cents: i64,
    pub goal_cents: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SupportPanelOptions {
    pub tiers_eur: Vec<u64>,
    pub twitch: bool,
    pub role_grant: bool,
}

impl SupportPanelOptions {
    pub fn to_kv(&self) -> Result<String, String> {
        serde_json::to_string(self).map_err(|error| error.to_string())
    }

    pub fn from_kv(raw: &str) -> Result<Self, String> {
        serde_json::from_str(raw).map_err(|error| error.to_string())
    }
}

pub fn tier_button_id(eur: u64) -> String {
    format!("{SUPPORT_TIER_BUTTON_PREFIX}{eur}")
}

pub fn format_tier_list(tiers_eur: &[u64]) -> String {
    join_alternatives(
        tiers_eur
            .iter()
            .map(|eur| format_whole_euro(*eur))
            .collect(),
    )
}

pub fn twitch_channel_list() -> String {
    join_alternatives(
        SUPPORTER_TWITCH_CHANNELS
            .iter()
            .map(|channel| channel.to_string())
            .collect(),
    )
}

#[cfg(test)]
pub(crate) fn banned_copy_in(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    SUPPORT_BANNED_COPY
        .into_iter()
        .find(|banned| lower.contains(&banned.to_lowercase()))
}

pub fn join_alternatives(labels: Vec<String>) -> String {
    match labels.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} oder {last}", rest.join(", ")),
    }
}

pub fn format_whole_euro(eur: u64) -> String {
    with_euro_sign(&group_thousands(eur))
}

impl SupportFunding {
    pub fn to_kv(self) -> String {
        format!("{}:{}", self.raised_cents, self.goal_cents)
    }

    pub fn from_kv(raw: &str) -> Result<Self, String> {
        let (raised, goal) = raw
            .split_once(':')
            .ok_or_else(|| format!("kein Trennzeichen in `{raw}`"))?;
        let cents = |part: &str| {
            part.trim()
                .parse::<i64>()
                .map_err(|error| format!("`{part}` ist kein Centbetrag: {error}"))
        };
        Ok(Self {
            raised_cents: cents(raised)?,
            goal_cents: cents(goal)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupportHistoryPlan {
    Adopt(Vec<u64>),
    Proceed { extra_delete_ids: Vec<u64> },
    Blocked { found: usize, expected: usize },
}

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
    pub funding: SupportFunding,
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
    #[serde(skip)]
    pub bytes: Vec<u8>,
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
    pub bytes: Vec<u8>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportTexts {
    intro: String,
    perks: String,
    steam: String,
    planned: String,
    free: String,
    funding: String,
    pay: String,
    twitch: String,
    thanks: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportButtons {
    join: String,
    tip: String,
    manage: String,
    twitch: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportTextsToml {
    #[serde(default)]
    texts: SupportBodyTextsToml,
    #[serde(default)]
    buttons: SupportButtonsToml,
    urls: Option<toml::Table>,
    stripe: Option<toml::Table>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportBodyTextsToml {
    intro: Option<String>,
    perks: Option<String>,
    steam: Option<String>,
    planned: Option<String>,
    free: Option<String>,
    funding: Option<String>,
    pay: Option<String>,
    twitch: Option<String>,
    thanks: Option<String>,
    title: Option<String>,
    boost: Option<String>,
    donation: Option<String>,
    benefits: Option<String>,
    why: Option<String>,
    ai: Option<String>,
    brain: Option<String>,
    discord: Option<String>,
}

impl SupportBodyTextsToml {
    fn has_legacy_fields(&self) -> bool {
        [
            &self.title,
            &self.boost,
            &self.donation,
            &self.benefits,
            &self.why,
            &self.ai,
            &self.brain,
            &self.discord,
        ]
        .iter()
        .any(|field| field.is_some())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportButtonsToml {
    #[serde(rename = "become")]
    join: Option<String>,
    tip: Option<String>,
    manage: Option<String>,
    twitch: Option<String>,
    donate: Option<String>,
    stripe: Option<String>,
}

#[derive(Debug, Clone)]
struct SupportBuiltMessage {
    banner: Option<SupportBannerOutput>,
    payload: SupportMessagePayload,
}

#[cfg(test)]
pub fn support_message_id_key(message_index: usize) -> String {
    crate::serversync::static_v2_message_id_key(SUPPORT_MESSAGE_ID_PREFIX, message_index)
}

pub fn plan_support_history(
    stored: &[u64],
    discovered: &[u64],
    expected: usize,
) -> SupportHistoryPlan {
    if stored.len() == expected {
        if !discovered.is_empty()
            && stored
                .iter()
                .all(|message_id| !discovered.contains(message_id))
        {
            return SupportHistoryPlan::Blocked {
                found: discovered.len(),
                expected,
            };
        }
        let extra_delete_ids = discovered
            .iter()
            .copied()
            .filter(|message_id| !stored.contains(message_id))
            .collect();
        return SupportHistoryPlan::Proceed { extra_delete_ids };
    }
    if discovered.len() == expected {
        return SupportHistoryPlan::Adopt(discovered.to_vec());
    }
    let stored_matches_discovered = !stored.is_empty() && same_id_set(stored, discovered);
    if discovered.len() < expected
        && (discovered.is_empty() || stored.is_empty() || stored_matches_discovered)
    {
        let extra_delete_ids = if stored.is_empty() {
            discovered.to_vec()
        } else {
            Vec::new()
        };
        return SupportHistoryPlan::Proceed { extra_delete_ids };
    }
    SupportHistoryPlan::Blocked {
        found: discovered.len(),
        expected,
    }
}

pub struct SupportStoredState<'a> {
    pub message_ids: &'a [u64],
    pub payload_format: Option<&'a str>,
    pub payload_hash: Option<&'a str>,
}

pub struct SupportSnapshot {
    config: ResolvedSupportConfig,
    config_warnings: Vec<String>,
    banner: Option<SupportBannerOutput>,
    banner_warnings: Vec<String>,
}

pub fn load_support_snapshot(repo_root: &Path) -> Result<SupportSnapshot, String> {
    let mut config_warnings = Vec::new();
    let config = load_support_runtime_config(repo_root, &mut config_warnings)?;
    let mut banner_warnings = Vec::new();
    let banner = optional_support_banner(repo_root, SUPPORT_HERO_FILENAME, &mut banner_warnings)?;
    Ok(SupportSnapshot {
        config,
        config_warnings,
        banner,
        banner_warnings,
    })
}

pub fn build_support_publish_output(
    repo_root: &Path,
    stored: &SupportStoredState<'_>,
    funding: SupportFunding,
    options: &SupportPanelOptions,
    dry_run: bool,
) -> Result<SupportPublishOutput, String> {
    let snapshot = load_support_snapshot(repo_root)?;
    build_support_output_from(&snapshot, stored, funding, options, dry_run)
}

pub fn build_support_output_from(
    snapshot: &SupportSnapshot,
    stored: &SupportStoredState<'_>,
    funding: SupportFunding,
    options: &SupportPanelOptions,
    dry_run: bool,
) -> Result<SupportPublishOutput, String> {
    let mut warnings = snapshot.config_warnings.clone();
    let blocks = SupportBlocks::new(&snapshot.config.texts, funding, options);
    placeholder_warnings(&blocks, &mut warnings);
    warnings.extend(snapshot.banner_warnings.iter().cloned());
    let built_messages = support_messages(snapshot.banner.clone(), &snapshot.config, &blocks);
    let positional_ids = stored.message_ids.len() == built_messages.len();

    let mut messages = Vec::new();
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored
            .message_ids
            .get(message_index)
            .copied()
            .filter(|_| positional_ids);
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

    let payload_hash = support_payload_hash(&messages)?;
    let repost_required =
        !support_storage_matches(stored.payload_format, stored.message_ids, messages.len());
    let hash_matches = stored.payload_hash == Some(payload_hash.as_str());
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
        stored_payload_format: stored.payload_format.map(str::to_string),
        payload_hash,
        stored_payload_hash: stored.payload_hash.map(str::to_string),
        repost_required,
        funding,
        warnings,
        messages,
        stored_message_ids: stored.message_ids.to_vec(),
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

pub fn changed_messages<'a>(
    current: &SupportPublishOutput,
    next: &'a SupportPublishOutput,
    message_ids: &[u64],
) -> Vec<(u64, &'a SupportMessageOutput)> {
    current
        .messages
        .iter()
        .zip(&next.messages)
        .zip(message_ids)
        .filter(|((left, right), _)| left.payload != right.payload)
        .map(|((_, right), message_id)| (*message_id, right))
        .collect()
}

pub fn new_warnings<'a>(
    current: &SupportPublishOutput,
    next: &'a SupportPublishOutput,
) -> Vec<&'a String> {
    next.warnings
        .iter()
        .filter(|warning| !current.warnings.contains(warning))
        .collect()
}

pub fn support_payload_is_unchanged(output: &SupportPublishOutput) -> bool {
    output.stored_payload_hash.as_deref() == Some(output.payload_hash.as_str())
}

fn funding_block(template: &str, funding: SupportFunding) -> SupportBlock<'_> {
    SupportBlock {
        values: vec![
            (PLACEHOLDER_BAR, funding_bar(funding)),
            (PLACEHOLDER_RAISED, format_euro(funding.raised_cents)),
            (PLACEHOLDER_GOAL, format_euro(funding.goal_cents)),
        ],
        ..SupportBlock::fixed("funding", template)
    }
}

fn funding_bar(funding: SupportFunding) -> String {
    let segments = i128::from(FUNDING_BAR_SEGMENTS);
    let filled = funding_share(funding, segments).min(segments);
    format!(
        "{}{}",
        "▰".repeat(segment_count(filled)),
        "▱".repeat(segment_count(segments - filled))
    )
}

fn funding_share(funding: SupportFunding, scale: i128) -> i128 {
    let raised = i128::from(funding.raised_cents.max(0));
    let goal = i128::from(funding.goal_cents);
    if goal <= 0 {
        return 0;
    }
    let exact = raised * scale / goal;
    if raised > 0 {
        exact.max(1)
    } else {
        exact
    }
}

fn segment_count(value: i128) -> usize {
    usize::try_from(value).unwrap_or(0)
}

pub fn format_euro(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let euros = group_thousands(abs / 100);
    match abs % 100 {
        0 => with_euro_sign(&format!("{sign}{euros}")),
        rest => with_euro_sign(&format!("{sign}{euros},{rest:02}")),
    }
}

fn with_euro_sign(amount: &str) -> String {
    format!("{amount} €")
}

fn group_thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(digit);
    }
    grouped
}

fn support_payload_hash(messages: &[SupportMessageOutput]) -> Result<String, String> {
    let payloads = messages
        .iter()
        .map(|message| {
            let attachment_hashes = message
                .payload
                .attachments
                .iter()
                .map(|attachment| SupportPayloadHashAttachment {
                    id: attachment.id,
                    filename: attachment.filename.as_str(),
                    sha256: format!("{:x}", Sha256::digest(&attachment.bytes)),
                })
                .collect::<Vec<_>>();
            SupportPayloadHashMessage {
                payload: &message.payload,
                attachment_hashes,
            }
        })
        .collect::<Vec<_>>();
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
    let Some(raw) = dl_core::optional_file::read_to_string(&path)
        .map_err(|reason| format!("Support-Textdatei `{SUPPORT_TEXTS_FILE}` {reason}"))?
    else {
        warnings.push(format!(
            "Support-Textdatei `{SUPPORT_TEXTS_FILE}` fehlt; Compile-Defaults werden verwendet"
        ));
        return Ok(ResolvedSupportConfig::from_defaults());
    };

    let file = toml::from_str::<SupportTextsToml>(&raw).map_err(|err| {
        format!("Support-Textdatei `{SUPPORT_TEXTS_FILE}` konnte nicht geparst werden: {err}")
    })?;
    Ok(ResolvedSupportConfig::from_defaults().merge(file, warnings))
}

impl ResolvedSupportConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedSupportTexts {
                intro: SUPPORT_INTRO.to_string(),
                perks: SUPPORT_PERKS.to_string(),
                steam: SUPPORT_STEAM.to_string(),
                planned: SUPPORT_PLANNED.to_string(),
                free: SUPPORT_FREE.to_string(),
                funding: SUPPORT_FUNDING.to_string(),
                pay: SUPPORT_PAY.to_string(),
                twitch: SUPPORT_TWITCH.to_string(),
                thanks: SUPPORT_THANKS.to_string(),
            },
            buttons: ResolvedSupportButtons {
                join: SUPPORT_BECOME_BUTTON_LABEL.to_string(),
                tip: SUPPORT_TIP_BUTTON_LABEL.to_string(),
                manage: SUPPORT_MANAGE_BUTTON_LABEL.to_string(),
                twitch: SUPPORT_TWITCH_BUTTON_LABEL.to_string(),
            },
        }
    }

    fn merge(mut self, file: SupportTextsToml, warnings: &mut Vec<String>) -> Self {
        let legacy = file.texts.has_legacy_fields()
            || file.buttons.donate.is_some()
            || file.buttons.stripe.is_some()
            || file.urls.is_some()
            || file.stripe.is_some();
        apply_optional(&mut self.texts.intro, file.texts.intro);
        apply_optional(&mut self.texts.perks, file.texts.perks);
        apply_optional(&mut self.texts.steam, file.texts.steam);
        apply_optional(&mut self.texts.planned, file.texts.planned);
        apply_optional(&mut self.texts.free, file.texts.free);
        apply_optional(&mut self.texts.funding, file.texts.funding);
        apply_optional(&mut self.texts.pay, file.texts.pay);
        apply_optional(&mut self.texts.twitch, file.texts.twitch);
        apply_optional(&mut self.texts.thanks, file.texts.thanks);
        apply_optional(&mut self.buttons.join, file.buttons.join);
        apply_optional(&mut self.buttons.tip, file.buttons.tip);
        apply_optional(&mut self.buttons.manage, file.buttons.manage);
        apply_optional(&mut self.buttons.twitch, file.buttons.twitch);
        if legacy {
            warnings.push(
                "Support-Textdatei enthält alte Felder (Ko-fi, Warum, KI-Abos, Stripe-Link oder ältere Abschnitte). Die werden nicht mehr angezeigt."
                    .to_string(),
            );
        }
        self
    }
}

const PLACEHOLDER_BAR: &str = "balken";
const PLACEHOLDER_RAISED: &str = "stand";
const PLACEHOLDER_GOAL: &str = "ziel";
const PLACEHOLDER_TIERS: &str = "stufen";
const PLACEHOLDER_CHANNELS: &str = "kanaele";

struct SupportBlock<'a> {
    field: &'static str,
    template: &'a str,
    shown: bool,
    values: Vec<(&'static str, String)>,
}

impl<'a> SupportBlock<'a> {
    fn fixed(field: &'static str, template: &'a str) -> Self {
        Self {
            field,
            template,
            shown: true,
            values: Vec::new(),
        }
    }

    fn rendered(&self) -> String {
        if !self.shown {
            return String::new();
        }
        self.values
            .iter()
            .fold(self.template.to_string(), |text, (name, value)| {
                text.replace(&format!("{{{name}}}"), value)
            })
    }
}

struct SupportBlocks<'a> {
    intro: SupportBlock<'a>,
    perks: SupportBlock<'a>,
    steam: SupportBlock<'a>,
    planned: SupportBlock<'a>,
    free: SupportBlock<'a>,
    funding: SupportBlock<'a>,
    pay: SupportBlock<'a>,
    twitch: SupportBlock<'a>,
    thanks: SupportBlock<'a>,
}

impl<'a> SupportBlocks<'a> {
    fn new(
        texts: &'a ResolvedSupportTexts,
        funding: SupportFunding,
        options: &SupportPanelOptions,
    ) -> Self {
        Self {
            intro: SupportBlock::fixed("intro", &texts.intro),
            perks: SupportBlock {
                shown: options.role_grant,
                ..SupportBlock::fixed("perks", &texts.perks)
            },
            steam: SupportBlock::fixed("steam", &texts.steam),
            planned: SupportBlock::fixed("planned", &texts.planned),
            free: SupportBlock::fixed("free", &texts.free),
            funding: funding_block(&texts.funding, funding),
            pay: SupportBlock {
                values: vec![(PLACEHOLDER_TIERS, format_tier_list(&options.tiers_eur))],
                ..SupportBlock::fixed("pay", &texts.pay)
            },
            twitch: SupportBlock {
                shown: options.twitch,
                values: vec![(PLACEHOLDER_CHANNELS, twitch_channel_list())],
                ..SupportBlock::fixed("twitch", &texts.twitch)
            },
            thanks: SupportBlock::fixed("thanks", &texts.thanks),
        }
    }

    fn all(&self) -> [&SupportBlock<'a>; 9] {
        [
            &self.intro,
            &self.perks,
            &self.steam,
            &self.planned,
            &self.free,
            &self.funding,
            &self.pay,
            &self.twitch,
            &self.thanks,
        ]
    }
}

fn placeholder_warnings(blocks: &SupportBlocks<'_>, warnings: &mut Vec<String>) {
    for block in blocks.all().into_iter().filter(|block| block.shown) {
        let field = block.field;
        let found = placeholder_names(block.template);
        let known: Vec<&str> = block.values.iter().map(|(name, _)| *name).collect();
        for missing in known.iter().filter(|name| !found.contains(name)) {
            warnings.extend([format!(
                "Support-Textdatei: [texts].{field} enthält keinen Platzhalter {{{missing}}}; der Wert fehlt dann im Panel."
            )]);
        }
        for unknown in found.iter().filter(|name| !known.contains(name)) {
            warnings.extend([format!(
                "Support-Textdatei: [texts].{field} enthält den unbekannten Platzhalter {{{unknown}}}; er erscheint dann wörtlich im Panel."
            )]);
        }
    }
}

const PLACEHOLDER_NAME_MAX_CHARS: usize = 40;

fn placeholder_names(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        let name = &after[..end];
        if !name.is_empty()
            && name.chars().count() <= PLACEHOLDER_NAME_MAX_CHARS
            && name.chars().all(|ch| ch.is_alphanumeric() || ch == '_')
        {
            names.extend([name]);
            rest = &after[end + 1..];
        } else {
            rest = after;
        }
    }
    names
}

fn apply_optional(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn support_messages(
    banner: Option<SupportBannerOutput>,
    config: &ResolvedSupportConfig,
    blocks: &SupportBlocks<'_>,
) -> Vec<SupportBuiltMessage> {
    let messages = vec![
        main_message(banner, blocks),
        message_from_container(
            SUPPORT_COMPONENT_ID_BENEFITS_CONTAINER,
            None,
            section_children(&[
                (SUPPORT_COMPONENT_ID_PLANNED, &blocks.planned.rendered()),
                (SUPPORT_COMPONENT_ID_FREE, &blocks.free.rendered()),
            ]),
            Vec::new(),
        ),
        pay_message(config, blocks),
    ];
    debug_assert_eq!(messages.len(), SUPPORT_MESSAGE_COUNT);
    messages
}

fn main_message(
    banner: Option<SupportBannerOutput>,
    blocks: &SupportBlocks<'_>,
) -> SupportBuiltMessage {
    let mut children = Vec::new();
    let mut attachments = Vec::new();
    if let Some(banner) = &banner {
        children.push(media_gallery(
            SUPPORT_COMPONENT_ID_HERO_MEDIA,
            &banner.filename,
        ));
        attachments.push(SupportPayloadAttachment {
            id: 0,
            filename: banner.filename.clone(),
            bytes: banner.bytes.clone(),
        });
    }
    children.extend(section_children(&[
        (SUPPORT_COMPONENT_ID_MAIN_TEXT, &blocks.intro.rendered()),
        (SUPPORT_COMPONENT_ID_PERKS, &blocks.perks.rendered()),
        (SUPPORT_COMPONENT_ID_STEAM, &blocks.steam.rendered()),
    ]));
    message_from_container(
        SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
        banner,
        children,
        attachments,
    )
}

fn render_pay_text(blocks: &SupportBlocks<'_>) -> String {
    [blocks.pay.rendered(), blocks.twitch.rendered()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn pay_message(config: &ResolvedSupportConfig, blocks: &SupportBlocks<'_>) -> SupportBuiltMessage {
    let mut children = section_children(&[
        (SUPPORT_COMPONENT_ID_FUNDING, &blocks.funding.rendered()),
        (SUPPORT_COMPONENT_ID_PAY_BODY, &render_pay_text(blocks)),
        (SUPPORT_COMPONENT_ID_PAY_THANKS, &blocks.thanks.rendered()),
    ]);
    if !children.is_empty() {
        children.push(separator(SUPPORT_COMPONENT_ID_PAY_SEP_ACTIONS, 2));
    }
    let mut row = vec![
        interactive_button(
            SUPPORT_COMPONENT_ID_BECOME_BUTTON,
            1,
            &config.buttons.join,
            SUPPORT_BUTTON_BECOME,
        ),
        interactive_button(
            SUPPORT_COMPONENT_ID_TIP_BUTTON,
            2,
            &config.buttons.tip,
            SUPPORT_BUTTON_TIP,
        ),
        interactive_button(
            SUPPORT_COMPONENT_ID_MANAGE_BUTTON,
            2,
            &config.buttons.manage,
            SUPPORT_BUTTON_MANAGE,
        ),
    ];
    if blocks.twitch.shown {
        row.push(interactive_button(
            SUPPORT_COMPONENT_ID_TWITCH_BUTTON,
            2,
            &config.buttons.twitch,
            SUPPORT_BUTTON_TWITCH,
        ));
    }
    children.push(action_row(SUPPORT_COMPONENT_ID_ACTION_ROW, row));
    message_from_container(
        SUPPORT_COMPONENT_ID_PAY_CONTAINER,
        None,
        children,
        Vec::new(),
    )
}

fn same_id_set(left: &[u64], right: &[u64]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn section_children(blocks: &[(u64, &str)]) -> Vec<Value> {
    let present: Vec<&(u64, &str)> = blocks
        .iter()
        .filter(|(_, text)| !text.trim().is_empty())
        .collect();
    let mut children = Vec::new();
    for (index, (text_id, text)) in present.iter().enumerate() {
        children.push(text_display(*text_id, (*text).trim().to_string()));
        if index + 1 < present.len() {
            children.push(separator(text_id.saturating_add(100), 1));
        }
    }
    children
}

fn message_from_container(
    container_id: u64,
    banner: Option<SupportBannerOutput>,
    children: Vec<Value>,
    attachments: Vec<SupportPayloadAttachment>,
) -> SupportBuiltMessage {
    SupportBuiltMessage {
        banner,
        payload: SupportMessagePayload {
            flags: SUPPORT_COMPONENTS_V2_FLAG,
            allowed_mentions: SupportAllowedMentions { parse: Vec::new() },
            components: vec![container(container_id, children)],
            attachments,
        },
    }
}

fn optional_support_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<SupportBannerOutput>, String> {
    let relative_path = format!("{SUPPORT_BANNER_DIR}/{filename}");
    let Some(bytes) = crate::serversync::read_optional_banner(
        repo_root,
        "Support",
        &relative_path,
        "Publish laeuft ohne Banner",
        warnings,
    )?
    else {
        return Ok(None);
    };
    Ok(Some(SupportBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present: true,
        bytes,
    }))
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
    match message_index {
        0 => "support".to_string(),
        1 => "support:vorteile".to_string(),
        2 => "support:zahlung".to_string(),
        other => format!("support:{other}"),
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
    for component in &payload.components {
        validate_component_shape(component)?;
    }
    Ok(())
}

fn validate_component_shape(value: &Value) -> Result<(), String> {
    let kind = value.get("type").and_then(Value::as_u64);
    if kind == Some(17) {
        let empty = value
            .get("components")
            .and_then(Value::as_array)
            .is_none_or(|children| children.is_empty());
        if empty {
            return Err("Support-Payload hat einen leeren Container".to_string());
        }
    }
    if kind == Some(2) {
        validate_button(value)?;
    }
    if let Some(children) = value.get("components").and_then(Value::as_array) {
        for child in children {
            validate_component_shape(child)?;
        }
    }
    Ok(())
}

fn validate_button(value: &Value) -> Result<(), String> {
    let label = value.get("label").and_then(Value::as_str).unwrap_or("");
    let label_chars = label.chars().count();
    if label.trim().is_empty() {
        return Err("Support-Button-Label ist leer".to_string());
    }
    if label_chars > BUTTON_LABEL_MAX_CHARS {
        return Err(format!(
            "Support-Button-Label darf höchstens {BUTTON_LABEL_MAX_CHARS} Zeichen haben, hat {label_chars}"
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

fn separator(id: u64, spacing: u8) -> Value {
    json!({
        "type": 14,
        "id": id,
        "divider": true,
        "spacing": spacing,
    })
}

fn action_row(id: u64, components: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "id": id,
        "components": components,
    })
}

fn interactive_button(id: u64, style: u8, label: &str, custom_id: &str) -> Value {
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
    use std::fs;
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

    fn text_contents(payload: &SupportMessagePayload) -> Vec<String> {
        let mut contents = Vec::new();
        fn walk(value: &Value, contents: &mut Vec<String>) {
            if value.get("type").and_then(Value::as_u64) == Some(10) {
                if let Some(content) = value.get("content").and_then(Value::as_str) {
                    contents.push(content.to_string());
                }
            }
            if let Some(children) = value.get("components").and_then(Value::as_array) {
                for child in children {
                    walk(child, contents);
                }
            }
        }
        for component in &payload.components {
            walk(component, &mut contents);
        }
        contents
    }

    fn buttons(payload: &SupportMessagePayload) -> Vec<Value> {
        let mut found = Vec::new();
        fn walk(value: &Value, found: &mut Vec<Value>) {
            if value.get("type").and_then(Value::as_u64) == Some(2) {
                found.push(value.clone());
            }
            if let Some(children) = value.get("components").and_then(Value::as_array) {
                for child in children {
                    walk(child, found);
                }
            }
        }
        for component in &payload.components {
            walk(component, &mut found);
        }
        found
    }

    fn rendered_copy(output: &SupportPublishOutput) -> String {
        output
            .messages
            .iter()
            .flat_map(|message| {
                let labels = buttons(&message.payload)
                    .into_iter()
                    .filter_map(|button| button["label"].as_str().map(str::to_string));
                text_contents(&message.payload).into_iter().chain(labels)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn assert_no_banned_copy(output: &SupportPublishOutput, variant: &str) {
        assert_eq!(banned_copy_in(&rendered_copy(output)), None, "{variant}");
    }

    const STORED_IDS: [u64; SUPPORT_MESSAGE_COUNT] = [7001, 7002, 7003];

    fn stored<'a>(ids: &'a [u64], hash: Option<&'a str>) -> SupportStoredState<'a> {
        SupportStoredState {
            message_ids: ids,
            payload_format: Some(SUPPORT_PAYLOAD_FORMAT),
            payload_hash: hash,
        }
    }

    fn options() -> SupportPanelOptions {
        SupportPanelOptions {
            tiers_eur: dl_core::runtime_config::SUPPORTER_DEFAULT_TIERS_EUR.to_vec(),
            twitch: false,
            role_grant: true,
        }
    }

    fn funding(raised_cents: i64) -> SupportFunding {
        SupportFunding {
            raised_cents,
            goal_cents: 5000,
        }
    }

    #[test]
    fn button_prueft_genau_die_gesendeten_zeichen() {
        let padded = interactive_button(
            1,
            1,
            &format!("{}   ", "a".repeat(BUTTON_LABEL_MAX_CHARS)),
            "x",
        );
        assert!(validate_component_shape(&padded).is_err());
        let ok = interactive_button(1, 1, "x", SUPPORT_BUTTON_BECOME);
        assert!(validate_component_shape(&ok).is_ok());
    }

    #[test]
    fn gleiche_ids_in_anderer_reihenfolge_werden_ersetzt() {
        assert_eq!(
            plan_support_history(&[2, 1], &[1, 2], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: Vec::new()
            }
        );
    }

    #[test]
    fn support_texts_temp_toml_mergt_fehlende_felder_auf_compile_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(
            temp.path(),
            r#"
[texts]
pay = "Runtime Zahlung {stufen}"

[buttons]
become = "Runtime Button"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_support_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.intro, SUPPORT_INTRO);
        assert_eq!(config.texts.pay, "Runtime Zahlung {stufen}");
        assert_eq!(config.texts.perks, SUPPORT_PERKS);
        assert_eq!(config.buttons.join, "Runtime Button");
        assert_eq!(config.buttons.manage, SUPPORT_MANAGE_BUTTON_LABEL);
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn alte_felder_bleiben_parsebar_und_werden_nicht_angezeigt() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(
            temp.path(),
            r#"
[texts]
title = "Alt"
why = "Warum ich Unterstützung brauche"
ai = "Zu den KI-Abos"

[buttons]
donate = "Über Ko-fi unterstützen"

[urls]
donate = "https://ko-fi.com/deutschedeadlockcommunity"

[stripe]
enabled = false
"#,
        );
        let output = build_support_publish_output(
            temp.path(),
            &stored(&STORED_IDS, None),
            funding(0),
            &options(),
            true,
        )
        .expect("output");
        assert_no_banned_copy(&output, "alte Felder");
        let labels: Vec<String> = output
            .messages
            .iter()
            .flat_map(|message| buttons(&message.payload))
            .filter_map(|button| button["label"].as_str().map(str::to_string))
            .collect();
        assert_eq!(
            labels,
            vec![
                SUPPORT_BECOME_BUTTON_LABEL.to_string(),
                SUPPORT_TIP_BUTTON_LABEL.to_string(),
                SUPPORT_MANAGE_BUTTON_LABEL.to_string()
            ]
        );
        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("alte Felder")));
    }

    #[test]
    fn support_texts_parse_fehler_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(temp.path(), "[texts]\nintro = [\n");
        let mut warnings = Vec::new();
        let err = load_support_runtime_config(temp.path(), &mut warnings).expect_err("parse error");
        assert!(err.contains("konnte nicht geparst werden"));
    }

    #[test]
    fn support_payload_ist_drei_goldene_karten_mit_banner_balken_und_knoepfen() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner");
        let output = build_support_publish_output(
            temp.path(),
            &stored(&[], None),
            funding(1500),
            &options(),
            true,
        )
        .expect("output");
        assert_eq!(output.messages.len(), SUPPORT_MESSAGE_COUNT);
        assert_eq!(output.messages[0].message_key, "support");
        assert_eq!(output.messages[1].message_key, "support:vorteile");
        assert_eq!(output.messages[2].message_key, "support:zahlung");
        assert!(output.repost_required);

        for message in &output.messages {
            assert_eq!(
                message.payload.components[0]["accent_color"],
                json!(SUPPORT_ACCENT_GOLD)
            );
        }
        let main = &output.messages[0].payload;
        assert_eq!(main.flags, SUPPORT_COMPONENTS_V2_FLAG);
        assert!(main.allowed_mentions.parse.is_empty());
        assert_eq!(main.attachments.len(), 1);
        assert_eq!(main.components[0]["type"], json!(17));
        assert_eq!(
            main.components[0]["accent_color"],
            json!(SUPPORT_ACCENT_GOLD)
        );
        assert_eq!(
            main.components[0]["components"][0]["items"][0]["media"]["url"],
            format!("attachment://{SUPPORT_HERO_FILENAME}")
        );
        let main_text = text_contents(main);
        assert_eq!(
            main_text,
            vec![
                SUPPORT_INTRO.to_string(),
                SUPPORT_PERKS.to_string(),
                SUPPORT_STEAM.to_string()
            ]
        );

        let benefits = text_contents(&output.messages[1].payload);
        assert_eq!(
            benefits,
            vec![SUPPORT_PLANNED.to_string(), SUPPORT_FREE.to_string()]
        );
        assert!(!benefits[1].to_lowercase().contains("steam"));

        let pay = &output.messages[2].payload;
        let pay_text = text_contents(pay);
        assert_eq!(pay_text.len(), 3);
        assert_eq!(
            pay_text[0],
            SUPPORT_FUNDING
                .replace("{stand}", "15 €")
                .replace("{ziel}", "50 €")
                .replace("{balken}", "▰▰▰▱▱▱▱▱▱▱")
        );
        assert_eq!(
            pay_text[1],
            SUPPORT_PAY.replace("{stufen}", "5 €, 10 €, 20 € oder 50 €")
        );
        assert!(!pay_text[1].contains("Steam Plus"));
        assert!(!pay_text[1].contains("Tage"));
        assert!(!pay_text[1].contains("Twitch"));
        assert_eq!(pay_text[2], SUPPORT_THANKS);
        let pay_buttons = buttons(pay);
        let custom_ids: Vec<&str> = pay_buttons
            .iter()
            .filter_map(|button| button["custom_id"].as_str())
            .collect();
        assert_eq!(
            custom_ids,
            vec![
                SUPPORT_BUTTON_BECOME,
                SUPPORT_BUTTON_TIP,
                SUPPORT_BUTTON_MANAGE
            ]
        );
        assert!(pay_buttons.iter().all(|button| button.get("url").is_none()));

        assert_no_banned_copy(&output, "Standard");

        for message in &output.messages {
            validate_support_message_budget(&message.payload).expect("budget");
            assert!(has_support_v2_marker(&message.payload.components));
            let ids = collect_component_ids(&message.payload.components);
            let mut unique = ids.clone();
            unique.sort_unstable();
            unique.dedup();
            assert_eq!(
                ids.len(),
                unique.len(),
                "Komponenten-IDs sind je Nachricht eindeutig"
            );
        }
    }

    #[test]
    fn keine_panel_variante_rendert_verbotene_woerter() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner");
        for twitch in [false, true] {
            for role_grant in [false, true] {
                let variant = SupportPanelOptions {
                    twitch,
                    role_grant,
                    ..options()
                };
                let output = build_support_publish_output(
                    temp.path(),
                    &stored(&[], None),
                    funding(1500),
                    &variant,
                    true,
                )
                .expect("output");
                let rendered = rendered_copy(&output);
                assert_eq!(rendered.contains("Twitch"), twitch, "twitch={twitch}");
                assert_eq!(
                    rendered.contains("Rolle"),
                    role_grant,
                    "role_grant={role_grant}"
                );
                assert_no_banned_copy(&output, &format!("twitch={twitch} role_grant={role_grant}"));
            }
        }
    }

    #[test]
    fn twitch_erscheint_nur_wenn_eingerichtet() {
        let temp = tempfile::tempdir().expect("tempdir");
        let full = SupportPanelOptions {
            twitch: true,
            ..options()
        };
        let output =
            build_support_publish_output(temp.path(), &stored(&[], None), funding(0), &full, true)
                .expect("output");
        assert!(
            !output.warnings.iter().any(|warning| warning.contains('{')),
            "{:?}",
            output.warnings
        );
        let pay = &output.messages[2].payload;
        let pay_text = text_contents(pay);
        assert!(pay_text[1].ends_with(&format!(
            "\n{}",
            SUPPORT_TWITCH.replace("{kanaele}", "earlysalty oder dach_lock")
        )));
        let custom_ids: Vec<String> = buttons(pay)
            .iter()
            .filter_map(|button| button["custom_id"].as_str().map(str::to_string))
            .collect();
        assert_eq!(
            custom_ids.last().map(String::as_str),
            Some(SUPPORT_BUTTON_TWITCH)
        );
        let default = build_support_publish_output(
            temp.path(),
            &stored(&[], None),
            funding(0),
            &options(),
            true,
        )
        .expect("default output");
        assert_ne!(default.payload_hash, output.payload_hash);
    }

    #[test]
    fn stufenliste_liest_sich_als_satz() {
        assert_eq!(format_tier_list(&[]), "");
        assert_eq!(format_tier_list(&[5]), "5 €");
        assert_eq!(format_tier_list(&[5, 10]), "5 € oder 10 €");
        assert_eq!(
            format_tier_list(dl_core::runtime_config::SUPPORTER_DEFAULT_TIERS_EUR),
            "5 €, 10 €, 20 € oder 50 €"
        );
        assert_eq!(format_tier_list(&[5, 1000]), "5 € oder 1.000 €");
        assert_eq!(twitch_channel_list(), "earlysalty oder dach_lock");
        assert_eq!(tier_button_id(50), "support:tier:50");
    }

    #[test]
    fn balken_zeigt_betrag_und_mindestens_ein_segment() {
        let template = "{balken} {stand} von {ziel}";
        assert_eq!(
            funding_block(template, funding(0)).rendered(),
            "▱▱▱▱▱▱▱▱▱▱ 0 € von 50 €"
        );
        assert_eq!(
            funding_block(template, funding(50)).rendered(),
            "▰▱▱▱▱▱▱▱▱▱ 0,50 € von 50 €"
        );
        assert_eq!(
            funding_block(template, funding(7550)).rendered(),
            "▰▰▰▰▰▰▰▰▰▰ 75,50 € von 50 €"
        );
        let big = SupportFunding {
            raised_cents: 123_456_700,
            goal_cents: 0,
        };
        assert_eq!(
            funding_block(template, big).rendered(),
            "▱▱▱▱▱▱▱▱▱▱ 1.234.567 € von 0 €"
        );
        let tiny = SupportFunding {
            raised_cents: 500,
            goal_cents: 100_000,
        };
        assert_eq!(
            funding_block(template, tiny).rendered(),
            "▰▱▱▱▱▱▱▱▱▱ 5 € von 1.000 €"
        );
        let huge = SupportFunding {
            raised_cents: i64::MAX,
            goal_cents: 7,
        };
        assert!(funding_block(template, huge)
            .rendered()
            .starts_with("▰▰▰▰▰▰▰▰▰▰ "));
    }

    #[test]
    fn funding_kv_rundreise() {
        let value = funding(1234);
        assert_eq!(SupportFunding::from_kv(&value.to_kv()), Ok(value));
        for broken in ["kaputt", "12:x", "x:12", ""] {
            assert!(SupportFunding::from_kv(broken).is_err(), "{broken}");
        }
    }

    #[test]
    fn support_payload_hash_noop_banner_und_balken_aendern_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner-one");
        let first = build_support_publish_output(
            temp.path(),
            &stored(&STORED_IDS, None),
            funding(0),
            &options(),
            true,
        )
        .expect("first output");
        assert_eq!(first.messages[0].action, "planned_edit");

        let same = build_support_publish_output(
            temp.path(),
            &stored(&STORED_IDS, Some(&first.payload_hash)),
            funding(0),
            &options(),
            true,
        )
        .expect("same output");
        assert_eq!(same.messages[0].action, "planned_no_op");
        assert_eq!(same.messages[2].action, "planned_no_op");
        assert!(support_payload_is_unchanged(&same));
        assert!(!same.repost_required);

        let more_money = build_support_publish_output(
            temp.path(),
            &stored(&STORED_IDS, Some(&first.payload_hash)),
            funding(500),
            &options(),
            true,
        )
        .expect("funding output");
        assert_ne!(more_money.payload_hash, first.payload_hash);

        write_banner_bytes(temp.path(), b"banner-two");
        let changed = build_support_publish_output(
            temp.path(),
            &stored(&STORED_IDS, Some(&first.payload_hash)),
            funding(0),
            &options(),
            true,
        )
        .expect("changed output");
        assert_eq!(changed.messages[0].action, "planned_edit");
        assert_ne!(changed.payload_hash, first.payload_hash);
    }

    #[test]
    fn altes_format_wird_als_repost_geplant() {
        let temp = tempfile::tempdir().expect("tempdir");
        let state = SupportStoredState {
            message_ids: &STORED_IDS,
            payload_format: Some("2"),
            payload_hash: None,
        };
        let output =
            build_support_publish_output(temp.path(), &state, funding(0), &options(), true)
                .expect("output");
        assert!(output.repost_required);
        assert!(output
            .messages
            .iter()
            .all(|message| message.action == "planned_repost"));
    }

    #[test]
    fn eine_gespeicherte_nachricht_wird_als_repost_geplant() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_support_publish_output(
            temp.path(),
            &stored(&[7001], None),
            funding(0),
            &options(),
            true,
        )
        .expect("output");
        assert!(output.repost_required);
        assert_eq!(output.stored_message_ids, vec![7001]);
        assert!(
            output
                .messages
                .iter()
                .all(|message| message.action == "planned_post"
                    && message.stored_message_id.is_none())
        );
    }

    #[test]
    fn textdatei_als_kaputter_link_bricht_ab_statt_defaults_zu_nutzen() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join(SUPPORT_TEXTS_FILE);
        fs::create_dir_all(path.parent().expect("texts parent")).expect("mkdir texts parent");
        std::os::unix::fs::symlink(temp.path().join("ziel-fehlt.toml"), &path).expect("symlink");
        let Err(err) = load_support_snapshot(temp.path()) else {
            panic!("kaputter Textdatei-Link muss den Snapshot abbrechen");
        };
        assert!(err.contains("Support-Textdatei"), "{err}");
    }

    #[test]
    fn unlesbarer_banner_bricht_ab_statt_ohne_banner_zu_posten() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = load_support_snapshot(temp.path()).expect("snapshot ohne banner");
        assert!(missing.banner.is_none());
        assert!(missing
            .banner_warnings
            .iter()
            .any(|warning| warning.contains("fehlt")));

        write_banner_bytes(temp.path(), b"banner");
        let path = temp
            .path()
            .join(format!("{SUPPORT_BANNER_DIR}/{SUPPORT_HERO_FILENAME}"));
        fs::remove_file(&path).expect("remove banner");
        fs::create_dir_all(&path).expect("banner path as directory");
        let Err(err) = load_support_snapshot(temp.path()) else {
            panic!("Banner-Pfad als Ordner muss den Snapshot abbrechen");
        };
        assert!(err.contains("ist keine Datei"), "{err}");

        use std::os::unix::fs::PermissionsExt;
        fs::remove_dir(&path).expect("remove banner directory");
        write_banner_bytes(temp.path(), b"banner");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod");
        assert!(
            fs::read(&path).is_err(),
            "gesperrte Datei bleibt lesbar (Lauf als root?), der Lesefehler-Fall ist so nicht prüfbar"
        );
        let Err(err) = load_support_snapshot(temp.path()) else {
            panic!("unlesbarer Banner muss den Snapshot abbrechen");
        };
        assert!(err.contains("konnte nicht gelesen werden"), "{err}");
    }

    #[test]
    fn history_plan_adoptiert_ersetzt_oder_blockt() {
        assert_eq!(
            plan_support_history(&[1, 2, 3], &[1, 2, 3], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: Vec::new()
            }
        );
        assert_eq!(
            plan_support_history(&[1, 2, 3], &[1, 2, 3, 99], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: vec![99]
            }
        );
        assert_eq!(
            plan_support_history(&[1, 2, 3], &[4, 5, 6], 3),
            SupportHistoryPlan::Blocked {
                found: 3,
                expected: 3
            }
        );
        assert_eq!(
            plan_support_history(&[1, 2, 3], &[], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: Vec::new()
            }
        );
        assert_eq!(
            plan_support_history(&[1, 2, 3], &[1, 2], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: Vec::new()
            }
        );
        assert_eq!(
            plan_support_history(&[], &[11, 12, 13], 3),
            SupportHistoryPlan::Adopt(vec![11, 12, 13])
        );
        assert_eq!(
            plan_support_history(&[11], &[11], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: Vec::new()
            }
        );
        assert_eq!(
            plan_support_history(&[], &[11], 3),
            SupportHistoryPlan::Proceed {
                extra_delete_ids: vec![11]
            }
        );
        assert_eq!(
            plan_support_history(&[11], &[11, 99], 3),
            SupportHistoryPlan::Blocked {
                found: 2,
                expected: 3
            }
        );
    }

    #[test]
    fn luecken_in_den_gespeicherten_ids_ordnen_keiner_karte_eine_fremde_id_zu() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_support_publish_output(
            temp.path(),
            &stored(&STORED_IDS[1..], None),
            funding(0),
            &options(),
            true,
        )
        .expect("output");
        assert!(output.repost_required);
        assert_eq!(output.stored_message_ids, STORED_IDS[1..].to_vec());
        for message in &output.messages {
            assert_eq!(message.stored_message_id, None, "{}", message.message_key);
            assert_eq!(message.action, "planned_post", "{}", message.message_key);
        }
    }

    fn dry_run_card(message: &SupportMessageOutput) -> String {
        let mut blocks = Vec::new();
        for container in &message.payload.components {
            for child in container["components"].as_array().into_iter().flatten() {
                match child["type"].as_u64() {
                    Some(12) => blocks.extend(
                        child["items"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|item| item["media"]["url"].as_str())
                            .map(|url| format!("[Banner: {url}]")),
                    ),
                    Some(10) => blocks.extend(child["content"].as_str().map(str::to_string)),
                    Some(14) => blocks.extend(["---".to_string()]),
                    Some(1) => blocks.extend([child["components"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|button| {
                            format!(
                                "[{}] ({})",
                                button["label"].as_str().unwrap_or_default(),
                                button["custom_id"].as_str().unwrap_or_default()
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" | ")]),
                    _ => {}
                }
            }
        }
        blocks.join("\n\n")
    }

    #[test]
    fn dry_run_doku_entspricht_dem_render() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let path = repo.join(".tasks/2026-11-11-supporter-panel/dry-run.md");
        let snapshot = load_support_snapshot(&repo).expect("repo snapshot");
        let funding = SupportFunding {
            raised_cents: 0,
            goal_cents: dl_core::runtime_config::SUPPORTER_DEFAULT_FUNDING_GOAL_CENTS,
        };
        let mut options = SupportPanelOptions {
            tiers_eur: dl_core::runtime_config::SUPPORTER_DEFAULT_TIERS_EUR.to_vec(),
            twitch: false,
            role_grant: false,
        };
        let off = build_support_output_from(&snapshot, &stored(&[], None), funding, &options, true)
            .expect("render ohne Rolle");
        options.role_grant = true;
        let on = build_support_output_from(&snapshot, &stored(&[], None), funding, &options, true)
            .expect("render mit Rolle");
        assert!(off.warnings.is_empty(), "{:?}", off.warnings);
        assert!(on.warnings.is_empty(), "{:?}", on.warnings);
        let header = format!(
            "# Support panel dry-run\n\nGenerated by the test `dry_run_doku_entspricht_dem_render` in `rust/bin/dl-bot/src/serversync/support_publish.rs`; `SUPPORT_DRY_RUN_WRITE=1` rewrites this file, otherwise the test fails when the file differs from the render. Rendered by `build_support_output_from` with `dry_run = true`, empty KV and the repo files under `assets/`, with the dl-core defaults: tiers {}, funding {} of {}, Twitch off, `supporter_role_grant_apply` off. Warnings: none. Nothing was posted to Discord. Buttons are shown as label and custom_id. The last section is card 1 with `supporter_role_grant_apply = true`; cards 2 and 3 are the same in both renders.",
            format_tier_list(&options.tiers_eur),
            format_euro(funding.raised_cents),
            format_euro(funding.goal_cents)
        );
        let mut sections = vec![header];
        sections.extend(off.messages.iter().map(|message| {
            format!(
                "## Card {} ({}), action {}\n\n{}",
                message.message_index + 1,
                message.message_key,
                message.action,
                dry_run_card(message)
            )
        }));
        sections.extend(on.messages.first().map(|message| {
            format!(
                "## Card 1 ({}) with `supporter_role_grant_apply = true`, action {}\n\n{}",
                message.message_key,
                message.action,
                dry_run_card(message)
            )
        }));
        let generated = format!("{}\n", sections.join("\n\n"));
        if std::env::var_os("SUPPORT_DRY_RUN_WRITE").is_some() {
            fs::write(&path, &generated).expect("dry-run schreiben");
        }
        let documented = fs::read_to_string(&path).expect("dry-run lesen");
        assert_eq!(
            documented, generated,
            "dry-run.md weicht vom Render ab; mit SUPPORT_DRY_RUN_WRITE=1 neu erzeugen"
        );
    }

    #[test]
    fn repo_support_textdatei_ueberschreibt_nichts() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let raw = fs::read_to_string(repo.join(SUPPORT_TEXTS_FILE)).expect("repo textdatei");
        let file = toml::from_str::<SupportTextsToml>(&raw).expect("repo textdatei parst");
        assert!(!file.texts.has_legacy_fields());
        assert_eq!(
            [
                &file.texts.intro,
                &file.texts.perks,
                &file.texts.steam,
                &file.texts.planned,
                &file.texts.free,
                &file.texts.funding,
                &file.texts.pay,
                &file.texts.twitch,
                &file.texts.thanks,
                &file.buttons.join,
                &file.buttons.tip,
                &file.buttons.manage,
                &file.buttons.twitch,
            ]
            .iter()
            .filter(|field| field.is_some())
            .count(),
            0
        );
        let mut warnings = Vec::new();
        load_support_runtime_config(&repo, &mut warnings).expect("repo config");
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn leere_karte_und_ueberlanges_label_werden_abgelehnt() {
        let empty = message_from_container(1, None, Vec::new(), Vec::new());
        let empty_error = validate_support_message_budget(&empty.payload).expect_err("empty");
        assert!(empty_error.contains("leeren Container"));

        let long_label = "x".repeat(BUTTON_LABEL_MAX_CHARS + 1);
        let overflow = message_from_container(
            1,
            None,
            vec![action_row(
                2,
                vec![interactive_button(3, 1, &long_label, "x")],
            )],
            Vec::new(),
        );
        let overflow_error = validate_support_message_budget(&overflow.payload).expect_err("label");
        assert!(overflow_error.contains(&format!("höchstens {BUTTON_LABEL_MAX_CHARS} Zeichen")));
    }

    fn pay_text_for(options: &SupportPanelOptions) -> String {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_support_publish_output(
            temp.path(),
            &stored(&[], None),
            funding(0),
            options,
            true,
        )
        .expect("output");
        text_contents(&output.messages[2].payload)[1].clone()
    }

    #[test]
    fn rollenzusage_haengt_am_vergabeschalter() {
        let granted = pay_text_for(&options());
        assert!(!granted.contains("Rolle"), "{granted}");
        assert!(!granted.contains('{'), "{granted}");
        let manual_options = SupportPanelOptions {
            role_grant: false,
            ..options()
        };
        let manual = pay_text_for(&manual_options);
        assert!(!manual.contains("Rolle"), "{manual}");
        assert!(!manual.contains('{'), "{manual}");
        let temp = tempfile::tempdir().expect("tempdir");
        let rendered = |options: &SupportPanelOptions| {
            build_support_publish_output(temp.path(), &stored(&[], None), funding(0), options, true)
                .expect("output")
                .messages
                .iter()
                .flat_map(|message| text_contents(&message.payload))
                .collect::<Vec<_>>()
        };
        assert!(rendered(&options()).contains(&SUPPORT_PERKS.to_string()));
        let manual_texts = rendered(&manual_options);
        assert!(!manual_texts.contains(&SUPPORT_PERKS.to_string()));
        assert!(
            manual_texts.iter().all(|text| !text.contains("Rolle")),
            "{manual_texts:?}"
        );
    }

    #[test]
    fn einmalig_nennt_keine_zahlen_die_der_steam_bot_festlegt() {
        let text = pay_text_for(&options());
        let once = text
            .lines()
            .find(|line| line.starts_with("**Einmalig.**"))
            .expect("Einmalig-Zeile");
        assert!(!once.chars().any(|ch| ch.is_ascii_digit()), "{once}");
        assert!(!text.contains('{'), "{text}");
    }

    #[test]
    fn fehlende_platzhalter_werden_je_feld_gemeldet() {
        let mut texts = ResolvedSupportConfig::from_defaults().texts;
        let mut warnings = Vec::new();
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &options()),
            &mut warnings,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        texts.funding = "{stand}".to_string();
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &options()),
            &mut warnings,
        );
        assert_eq!(warnings.len(), 2, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|warning| warning.contains("[texts].funding")));
        warnings.clear();
        texts = ResolvedSupportConfig::from_defaults().texts;
        texts.pay = format!("{} ab {{mindestbetrag}} für {{tage}} Tage", texts.pay);
        texts.thanks = "Danke {name}".to_string();
        texts.twitch = "Twitch".to_string();
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &options()),
            &mut warnings,
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        for (field, name) in [
            ("pay", "{mindestbetrag}"),
            ("pay", "{tage}"),
            ("thanks", "{name}"),
        ] {
            assert!(
                warnings.iter().any(|warning| warning.contains(&format!(
                    "[texts].{field} enthält den unbekannten Platzhalter {name}"
                ))),
                "{field} {name}: {warnings:?}"
            );
        }
        warnings.clear();
        let with_twitch = SupportPanelOptions {
            twitch: true,
            ..options()
        };
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &with_twitch),
            &mut warnings,
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning
                    .contains("[texts].twitch enthält keinen Platzhalter {kanaele}"))
        );
        assert_eq!(
            placeholder_names("{a} {B} {} {x_y} {stufe2} {Stufen} {mit leer} {ä} {"),
            vec!["a", "B", "x_y", "stufe2", "Stufen", "ä"]
        );
        warnings.clear();
        texts = ResolvedSupportConfig::from_defaults().texts;
        texts.pay = texts.pay.replace("{stufen}", "{Stufen}");
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &options()),
            &mut warnings,
        );
        assert_eq!(
            warnings,
            vec![
                "Support-Textdatei: [texts].pay enthält keinen Platzhalter {stufen}; der Wert fehlt dann im Panel.".to_string(),
                "Support-Textdatei: [texts].pay enthält den unbekannten Platzhalter {Stufen}; er erscheint dann wörtlich im Panel.".to_string(),
            ]
        );
        warnings.clear();
        texts = ResolvedSupportConfig::from_defaults().texts;
        texts.pay = "{stufen}".to_string();
        texts.perks = "Rolle {name}".to_string();
        texts.twitch = "Kanäle {liste}".to_string();
        let hidden = SupportPanelOptions {
            role_grant: false,
            twitch: false,
            ..options()
        };
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &hidden),
            &mut warnings,
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        placeholder_warnings(
            &SupportBlocks::new(&texts, funding(0), &options()),
            &mut warnings,
        );
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings
            .iter()
            .any(|warning| warning
                .contains("[texts].perks enthält den unbekannten Platzhalter {name}")));
    }

    #[test]
    fn panel_optionen_rundreise() {
        let value = SupportPanelOptions {
            twitch: true,
            ..options()
        };
        let raw = value.to_kv().expect("kv");
        assert_eq!(SupportPanelOptions::from_kv(&raw), Ok(value));
        assert!(SupportPanelOptions::from_kv("kaputt").is_err());
        assert!(SupportPanelOptions::from_kv(
            r#"{"tiers_eur":[5],"plus_monthly_cents":300,"twitch":false,"role_grant":true}"#
        )
        .is_err());
    }
}
