use std::fs;
use std::io::ErrorKind;
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
const FUNDING_BAR_SEGMENTS: i64 = 20;
const CUSTOM_ID_MAX_CHARS: usize = 100;
const BUTTON_LABEL_MAX_CHARS: usize = 80;

pub const SUPPORT_INTRO: &str = "\
# Server unterstützen\n\
\n\
Hier steht, wie du den Server und die Projekte unterstützen kannst. Jeder Weg zählt gleich.";
pub const SUPPORT_PERKS: &str = "\
## Was du jetzt bekommst\n\
\n\
**Mit den Streamern zusammen spielen.** Als Unterstützer bist du ein vertrauenswürdiges Mitglied der Community. Du kommst in den Streamer-VC und kannst dort mit den Streamern zusammen zocken.\n\
\n\
**Die Rolle Server Unterstützer.** Damit sieht jeder, dass du den Server mitträgst.";
pub const SUPPORT_STEAM: &str = "\
## Steam Bot\n\
\n\
Als Nächstes wird der Steam Bot zur Funktion für Unterstützer. Dann holt er deine Matches bevorzugt ab, ähnlich wie bei https://deadlock-api.com/patron: schnellere Updates, deine ganze Match-Historie und dein Rang direkt von Steam.\n\
\n\
Echtzeit verspreche ich nicht, was möglich ist, hängt von Steam ab.";
pub const SUPPORT_PLANNED: &str = "\
## Was noch kommt\n\
\n\
Das ist geplant und noch nicht fertig:\n\
\n\
- **Patch-Watchlist.** Du suchst dir Helden und Items aus und bekommst Bescheid, wenn ein Patch genau daran etwas ändert.\n\
- **Build-Helfer im Deadlock Brain.** Du wählst einen Helden und bekommst passende Items mit einer Erklärung, auch gegen bestimmte Gegner.\n\
- **Mehr Match-Funktionen im Steam Bot.**\n\
- **Turniere und Scrims.** Unterstützer testen neue Turnierfunktionen als Erste. Dazu kommen zusätzliche Community-Abende und Fun-Turniere.\n\
- **Coaching-Material** für Unterstützer.\n\
- **Mitbestimmen.** Unterstützer stimmen ab, was als Nächstes gebaut wird.";
pub const SUPPORT_FREE: &str = "\
## Was kostenlos bleibt\n\
\n\
Der Server wird nicht kostenpflichtig. Mitspieler suchen, die normalen Voice-Channels, Community-Aktivitäten, reguläre Turniere, Scrims und das normale Coaching bleiben frei.\n\
\n\
Bei regulären Turnieren und beim normalen Coaching bringt Unterstützung keinen Vorteil.";
pub const SUPPORT_FUNDING: &str = "\
## Monatsziel\n\
\n\
{balken} {prozent} %\n\
**{stand} von {ziel} im Monat**\n\
-# Hier zählen die laufenden Abos und die einmaligen Beträge aus diesem Monat.";
pub const SUPPORT_PAY: &str = "\
## So kannst du unterstützen\n\
\n\
**Monatlich über Stripe.** {stufen} im Monat.{rolle} Kündigen kannst du jederzeit selbst über den Button unten.\n\
\n\
**Einmalig.** Den Betrag wählst du selbst, den Mindestbetrag zeigt dir die Zahlungsseite.{rolle}\n\
\n\
**Server-Boost.** Zählt vorerst als Unterstützung.{rolle}";
pub const SUPPORT_TWITCH: &str = "\
**Twitch-Sub.** Ein Abo bei {kanaele} zählt, genauso ein Abo bei einem Streamer, der den bezahlten Twitch Bot nutzt. Verbinde dafür einmal deinen Twitch-Account über den Button unten, den Rest prüft der Bot automatisch.";
pub const SUPPORT_THANKS: &str = "Danke, dass du den Server und die Projekte mitträgst ❤️";
pub const SUPPORT_BECOME_BUTTON_LABEL: &str = "Supporter werden";
pub const SUPPORT_TIP_BUTTON_LABEL: &str = "Einmalig unterstützen";
pub const SUPPORT_MANAGE_BUTTON_LABEL: &str = "Abo verwalten oder kündigen";
pub const SUPPORT_TWITCH_BUTTON_LABEL: &str = "Twitch verbinden";
const SUPPORT_ROLE_NOTE: &str = " Die Rolle kommt automatisch.";

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

pub fn join_alternatives(labels: Vec<String>) -> String {
    match labels.split_last() {
        None => String::new(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} oder {last}", rest.join(", ")),
    }
}

pub fn format_whole_euro(eur: u64) -> String {
    format!("{} €", group_thousands(eur))
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
    plus: Option<String>,
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
            &self.plus,
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
    format!("{SUPPORT_MESSAGE_ID_PREFIX}{message_index}")
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

pub fn build_support_publish_output(
    repo_root: &Path,
    stored: &SupportStoredState<'_>,
    funding: SupportFunding,
    options: &SupportPanelOptions,
    dry_run: bool,
) -> Result<SupportPublishOutput, String> {
    let mut warnings = Vec::new();
    let config = load_support_runtime_config(repo_root, &mut warnings)?;
    placeholder_warnings(&config.texts, options, &mut warnings);
    let built_messages = support_messages(repo_root, &config, funding, options, &mut warnings);

    let mut messages = Vec::new();
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored.message_ids.get(message_index).copied();
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
) -> Option<Vec<(u64, &'a SupportMessageOutput)>> {
    if current.messages.len() != next.messages.len() || next.messages.len() != message_ids.len() {
        return None;
    }
    Some(
        current
            .messages
            .iter()
            .zip(&next.messages)
            .zip(message_ids)
            .filter(|((left, right), _)| left.payload != right.payload)
            .map(|((_, right), message_id)| (*message_id, right))
            .collect(),
    )
}

pub fn support_payload_is_unchanged(output: &SupportPublishOutput) -> bool {
    output.stored_payload_hash.as_deref() == Some(output.payload_hash.as_str())
}

pub fn render_funding_text(template: &str, funding: SupportFunding) -> String {
    template
        .replace("{balken}", &funding_bar(funding))
        .replace("{prozent}", &funding_percent(funding).to_string())
        .replace("{stand}", &format_euro(funding.raised_cents))
        .replace("{ziel}", &format_euro(funding.goal_cents))
}

fn funding_percent(funding: SupportFunding) -> i64 {
    if funding.goal_cents <= 0 {
        return 0;
    }
    funding.raised_cents.max(0).saturating_mul(100) / funding.goal_cents
}

fn funding_bar(funding: SupportFunding) -> String {
    let raised = funding.raised_cents.max(0);
    let filled = if funding.goal_cents <= 0 {
        0
    } else {
        let exact = raised.saturating_mul(FUNDING_BAR_SEGMENTS) / funding.goal_cents;
        let at_least_one = if raised > 0 { exact.max(1) } else { exact };
        at_least_one.min(FUNDING_BAR_SEGMENTS)
    };
    let empty = FUNDING_BAR_SEGMENTS - filled;
    format!(
        "`{}{}`",
        "█".repeat(filled as usize),
        "░".repeat(empty as usize)
    )
}

pub fn format_euro(cents: i64) -> String {
    let sign = if cents < 0 { "-" } else { "" };
    let abs = cents.unsigned_abs();
    let euros = group_thousands(abs / 100);
    match abs % 100 {
        0 => format!("{sign}{euros} €"),
        rest => format!("{sign}{euros},{rest:02} €"),
    }
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
                "Support-Textdatei enthält alte Felder (Ko-fi, Warum, KI-Abos, Steam Plus, Stripe-Link oder ältere Abschnitte). Die werden nicht mehr angezeigt."
                    .to_string(),
            );
        }
        self
    }
}

const FUNDING_PLACEHOLDERS: &[&str] = &["balken", "prozent", "stand", "ziel"];
const PAY_PLACEHOLDERS: &[&str] = &["stufen", "rolle"];
const TWITCH_PLACEHOLDERS: &[&str] = &["kanaele"];

fn placeholder_warnings(
    texts: &ResolvedSupportTexts,
    options: &SupportPanelOptions,
    warnings: &mut Vec<String>,
) {
    let twitch_required: &[&str] = if options.twitch {
        TWITCH_PLACEHOLDERS
    } else {
        &[]
    };
    let checks: [(&str, &str, &[&str], &[&str]); 9] = [
        (&texts.intro, "intro", &[], &[]),
        (&texts.perks, "perks", &[], &[]),
        (&texts.steam, "steam", &[], &[]),
        (&texts.planned, "planned", &[], &[]),
        (&texts.free, "free", &[], &[]),
        (
            &texts.funding,
            "funding",
            FUNDING_PLACEHOLDERS,
            FUNDING_PLACEHOLDERS,
        ),
        (&texts.pay, "pay", PAY_PLACEHOLDERS, PAY_PLACEHOLDERS),
        (
            &texts.twitch,
            "twitch",
            twitch_required,
            TWITCH_PLACEHOLDERS,
        ),
        (&texts.thanks, "thanks", &[], &[]),
    ];
    for (text, field, required, allowed) in checks {
        let found = placeholder_names(text);
        for missing in required.iter().filter(|name| !found.contains(name)) {
            warnings.push(format!(
                "Support-Textdatei: [texts].{field} enthält keinen Platzhalter {{{missing}}}; der Wert fehlt dann im Panel."
            ));
        }
        for unknown in found.iter().filter(|name| !allowed.contains(name)) {
            warnings.push(format!(
                "Support-Textdatei: [texts].{field} enthält den unbekannten Platzhalter {{{unknown}}}; er erscheint dann wörtlich im Panel."
            ));
        }
    }
}

fn placeholder_names(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        let name = &after[..end];
        if !name.is_empty() && name.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_') {
            names.push(name);
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
    repo_root: &Path,
    config: &ResolvedSupportConfig,
    funding: SupportFunding,
    options: &SupportPanelOptions,
    warnings: &mut Vec<String>,
) -> Vec<SupportBuiltMessage> {
    let banner = optional_support_banner(repo_root, SUPPORT_HERO_FILENAME, warnings);
    let messages = vec![
        main_message(banner, config, options),
        message_from_container(
            SUPPORT_COMPONENT_ID_BENEFITS_CONTAINER,
            None,
            section_children(&[
                (SUPPORT_COMPONENT_ID_PLANNED, &config.texts.planned),
                (SUPPORT_COMPONENT_ID_FREE, &config.texts.free),
            ]),
            Vec::new(),
        ),
        pay_message(config, funding, options),
    ];
    debug_assert_eq!(messages.len(), SUPPORT_MESSAGE_COUNT);
    messages
}

fn main_message(
    banner: Option<SupportBannerOutput>,
    config: &ResolvedSupportConfig,
    options: &SupportPanelOptions,
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
            relative_path: banner.relative_path.clone(),
        });
    }
    let perks = if options.role_grant {
        config.texts.perks.as_str()
    } else {
        ""
    };
    children.extend(section_children(&[
        (SUPPORT_COMPONENT_ID_MAIN_TEXT, &config.texts.intro),
        (SUPPORT_COMPONENT_ID_PERKS, perks),
        (SUPPORT_COMPONENT_ID_STEAM, &config.texts.steam),
    ]));
    message_from_container(
        SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
        banner,
        children,
        attachments,
    )
}

fn render_pay_text(texts: &ResolvedSupportTexts, options: &SupportPanelOptions) -> String {
    let role = if options.role_grant {
        SUPPORT_ROLE_NOTE
    } else {
        ""
    };
    let mut parts = vec![texts
        .pay
        .replace("{stufen}", &format_tier_list(&options.tiers_eur))
        .replace("{rolle}", role)];
    if options.twitch {
        parts.push(texts.twitch.replace("{kanaele}", &twitch_channel_list()));
    }
    parts.retain(|part| !part.trim().is_empty());
    parts.join("\n\n")
}

fn pay_message(
    config: &ResolvedSupportConfig,
    funding: SupportFunding,
    options: &SupportPanelOptions,
) -> SupportBuiltMessage {
    let funding_text = render_funding_text(&config.texts.funding, funding);
    let pay_text = render_pay_text(&config.texts, options);
    let mut children = section_children(&[
        (SUPPORT_COMPONENT_ID_FUNDING, &funding_text),
        (SUPPORT_COMPONENT_ID_PAY_BODY, &pay_text),
        (SUPPORT_COMPONENT_ID_PAY_THANKS, &config.texts.thanks),
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
    if options.twitch {
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
    if label.trim().is_empty() || label_chars > BUTTON_LABEL_MAX_CHARS {
        return Err(format!(
            "Support-Button-Label muss 1 bis {BUTTON_LABEL_MAX_CHARS} Zeichen haben, hat {label_chars}"
        ));
    }
    let custom_id = value.get("custom_id").and_then(Value::as_str).unwrap_or("");
    if custom_id.is_empty()
        || custom_id.chars().count() > CUSTOM_ID_MAX_CHARS
        || value.get("url").is_some()
    {
        return Err(format!(
            "Support-Button braucht eine custom_id mit 1 bis {CUSTOM_ID_MAX_CHARS} Zeichen und keine URL"
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
        let with_url = json!({
            "type": 2,
            "style": 1,
            "label": "Link",
            "custom_id": "x",
            "url": "https://example.com/x",
        });
        assert!(validate_component_shape(&with_url).is_err());
        let no_custom_id = json!({"type": 2, "style": 1, "label": "Supporter werden"});
        assert!(validate_component_shape(&no_custom_id).is_err());
        let long_custom_id = interactive_button(1, 1, "x", &"a".repeat(101));
        assert!(validate_component_shape(&long_custom_id).is_err());
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
        let rendered = output
            .messages
            .iter()
            .flat_map(|message| text_contents(&message.payload))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!rendered.contains("Warum ich Unterstützung brauche"));
        assert!(!rendered.contains("KI-Abos"));
        assert!(!rendered.to_lowercase().contains("ko-fi"));
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
        assert!(main_text[1]
            .starts_with("## Was du jetzt bekommst\n\n**Mit den Streamern zusammen spielen.**"));
        assert!(main_text[1].contains("vertrauenswürdiges Mitglied"));
        assert!(main_text[2].contains("deadlock-api.com/patron"));

        let benefits = text_contents(&output.messages[1].payload);
        assert_eq!(
            benefits,
            vec![SUPPORT_PLANNED.to_string(), SUPPORT_FREE.to_string()]
        );
        assert!(!benefits[1].to_lowercase().contains("steam"));

        let pay = &output.messages[2].payload;
        let pay_text = text_contents(pay);
        assert_eq!(pay_text.len(), 3);
        assert!(pay_text[0].contains("`██████░░░░░░░░░░░░░░` 30 %"));
        assert!(pay_text[0].contains("**15 € von 50 € im Monat**"));
        assert_eq!(
            pay_text[1],
            SUPPORT_PAY
                .replace("{stufen}", "5 €, 10 €, 20 € oder 50 €")
                .replace("{rolle}", SUPPORT_ROLE_NOTE)
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

        let rendered = output
            .messages
            .iter()
            .flat_map(|message| {
                let labels = buttons(&message.payload)
                    .into_iter()
                    .filter_map(|button| button["label"].as_str().map(str::to_string));
                text_contents(&message.payload).into_iter().chain(labels)
            })
            .collect::<Vec<_>>()
            .join("\n");
        for banned in [
            "Ko-fi",
            "KI-Abos",
            "Warum ich",
            "Server-Abo",
            "admin.",
            "\u{2014}",
        ] {
            assert!(
                !rendered.to_lowercase().contains(&banned.to_lowercase()),
                "{banned}"
            );
        }

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
        assert!(pay_text[1].ends_with(
            "Verbinde dafür einmal deinen Twitch-Account über den Button unten, den Rest prüft der Bot automatisch."
        ));
        assert!(pay_text[1].contains("Ein Abo bei earlysalty oder dach_lock zählt"));
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
    fn balken_zeigt_betrag_prozent_und_mindestens_ein_segment() {
        let template = "{balken} {prozent} % {stand} von {ziel}";
        assert_eq!(
            render_funding_text(template, funding(0)),
            "`░░░░░░░░░░░░░░░░░░░░` 0 % 0 € von 50 €"
        );
        assert_eq!(
            render_funding_text(template, funding(50)),
            "`█░░░░░░░░░░░░░░░░░░░` 1 % 0,50 € von 50 €"
        );
        assert_eq!(
            render_funding_text(template, funding(7550)),
            "`████████████████████` 151 % 75,50 € von 50 €"
        );
        let big = SupportFunding {
            raised_cents: 123_456_700,
            goal_cents: 0,
        };
        assert_eq!(
            render_funding_text(template, big),
            "`░░░░░░░░░░░░░░░░░░░░` 0 % 1.234.567 € von 0 €"
        );
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
        assert_eq!(output.messages[0].action, "planned_repost");
        assert_eq!(output.messages[1].action, "planned_post");
        assert_eq!(output.messages[2].action, "planned_post");
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

        let long_label = "x".repeat(81);
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
        assert!(overflow_error.contains("1 bis 80"));
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
        assert_eq!(granted.matches(SUPPORT_ROLE_NOTE.trim()).count(), 3);
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
            .split("\n\n")
            .find(|part| part.starts_with("**Einmalig.**"))
            .expect("Einmalig-Absatz");
        assert!(!once.chars().any(|ch| ch.is_ascii_digit()), "{once}");
        assert!(once.contains("Zahlungsseite"), "{once}");
        assert!(!text.contains('{'), "{text}");
    }

    #[test]
    fn fehlende_platzhalter_werden_je_feld_gemeldet() {
        let mut texts = ResolvedSupportConfig::from_defaults().texts;
        let mut warnings = Vec::new();
        placeholder_warnings(&texts, &options(), &mut warnings);
        assert!(warnings.is_empty(), "{warnings:?}");
        texts.funding = "{stand}".to_string();
        placeholder_warnings(&texts, &options(), &mut warnings);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings
            .iter()
            .all(|warning| warning.contains("[texts].funding")));
        warnings.clear();
        texts = ResolvedSupportConfig::from_defaults().texts;
        texts.pay = format!("{} ab {{mindestbetrag}} für {{tage}} Tage", texts.pay);
        texts.thanks = "Danke {name}".to_string();
        texts.twitch = "Twitch".to_string();
        placeholder_warnings(&texts, &options(), &mut warnings);
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
        placeholder_warnings(&texts, &with_twitch, &mut warnings);
        assert!(
            warnings
                .iter()
                .any(|warning| warning
                    .contains("[texts].twitch enthält keinen Platzhalter {kanaele}"))
        );
        assert_eq!(placeholder_names("{a} {B} {} {x_y} {"), vec!["a", "x_y"]);
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
