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
pub const SUPPORT_COMPONENT_ID_STRIPE_BUTTON: u64 = 33_006;
pub const SUPPORT_COMPONENT_ID_WHY_SEP_1: u64 = 33_011;
pub const SUPPORT_COMPONENT_ID_WHY_BODY: u64 = 33_012;
pub const SUPPORT_COMPONENT_ID_WHY_SEP_2: u64 = 33_013;
pub const SUPPORT_COMPONENT_ID_WHY_AI: u64 = 33_014;
pub const SUPPORT_COMPONENT_ID_BENEFITS_CONTAINER: u64 = 33_020;
pub const SUPPORT_COMPONENT_ID_BENEFITS_STEAM: u64 = 33_021;
pub const SUPPORT_COMPONENT_ID_BENEFITS_SEP_1: u64 = 33_022;
pub const SUPPORT_COMPONENT_ID_BENEFITS_BRAIN: u64 = 33_023;
pub const SUPPORT_COMPONENT_ID_BENEFITS_SEP_2: u64 = 33_024;
pub const SUPPORT_COMPONENT_ID_BENEFITS_DISCORD: u64 = 33_025;
pub const SUPPORT_COMPONENT_ID_BENEFITS_SEP_3: u64 = 33_026;
pub const SUPPORT_COMPONENT_ID_BENEFITS_PLANNED: u64 = 33_027;
pub const SUPPORT_COMPONENT_ID_BENEFITS_SEP_4: u64 = 33_028;
pub const SUPPORT_COMPONENT_ID_BENEFITS_FREE: u64 = 33_029;
pub const SUPPORT_COMPONENT_ID_PAY_CONTAINER: u64 = 33_030;
pub const SUPPORT_COMPONENT_ID_PAY_BODY: u64 = 33_031;
pub const SUPPORT_COMPONENT_ID_PAY_SEP: u64 = 33_032;
pub const SUPPORT_COMPONENT_ID_PAY_THANKS: u64 = 33_033;
pub const SUPPORT_COMPONENT_ID_PAY_SEP_ACTIONS: u64 = 33_034;

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
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;
const SUPPORT_MESSAGE_COUNT: usize = 3;

pub const SUPPORT_DONATE_URL_DEFAULT: &str = "https://ko-fi.com/deutschedeadlockcommunity";
pub const SUPPORT_INTRO: &str = "\
# Server unterstützen\n\
\n\
Ich baue einen großen Teil von dem, was hier läuft, selbst. Steam Bot, Twitch Bot, Patchnotes, Deadlock Brain, die Website, Turniere und die übrigen Community-Systeme. Das sind jede Woche viele Stunden, und der Betrieb kostet laufend Geld.";
pub const SUPPORT_WHY: &str = "\
## Warum ich Unterstützung brauche\n\
\n\
Server, APIs, Infrastruktur und die KI-Coding-Abos liegen je nach Monat ungefähr bei **50 bis 250 €**, manchmal auch darüber. 2026 sind bis zum 8. November schon **über 1.600 €** zusammengekommen. Meine Arbeitszeit ist darin nicht enthalten.\n\
\n\
Ich habe mir oft überlegt, wie sich das langfristig tragen lässt. So wie bisher kann ich die Kosten nicht dauerhaft alleine stemmen.";
pub const SUPPORT_AI: &str = "\
## Zu den KI-Abos\n\
\n\
Die Abos kosten Geld. Bei diesem Umfang sind sie aber deutlich günstiger, als dieselbe Arbeitszeit auch nur zum Mindestlohn an Entwickler zu zahlen.\n\
\n\
Die Tools entwickeln die Projekte nicht alleine. Planung, Architektur, Umsetzung, Tests, Fehlersuche und Betrieb bleiben bei mir. Die Abos machen diesen Umfang überhaupt erst bezahlbar.\n\
\n\
Was an Unterstützung reinkommt, geht zurück in Infrastruktur und Entwicklung. Gewinn ist nicht das Ziel.";
pub const SUPPORT_STEAM: &str = "\
## Steam Bot\n\
\n\
Der Steam Bot wird auf ein Supporter-Modell umgestellt. Das gilt für die Steam-Verknüpfung, die automatischen Rang-Rollen, Rang-Abfragen, Match-Abfragen und die weiteren Funktionen des Bots.\n\
\n\
Die Anbindung ist aufwendig. Deadlock ändert sich, und die Pflege läuft mit.\n\
\n\
Geplant sind erweiterte Match-Abfragen, ähnlich dem priorisierten Abruf bei der Deadlock API: https://deadlock-api.com/patron\n\
Echtzeit verspreche ich nicht. Was möglich ist, hängt von Steam und dem Game Coordinator ab.";
pub const SUPPORT_BRAIN: &str = "\
## Deadlock Brain\n\
\n\
Für Supporter ist vorgesehen, das Deadlock Brain ohne zusätzliches Abo und ohne Nutzungslimit zu nutzen. Das ist noch nicht eingerichtet. Bis dahin gilt das normale Tageslimit.\n\
\n\
Geplant, und noch nicht fertig, ist ein Build-Helfer. Du wählst einen Helden und bekommst sinnvolle Items und Builds, dazu eine Erklärung, warum ein Item passt. Du kannst gegen bestimmte Gegner bauen. Die Vorschläge sollen sich an Patches anpassen. Der Helfer soll beim Verstehen helfen, die Entscheidung bleibt bei dir.";
pub const SUPPORT_DISCORD: &str = "\
## Auf Discord\n\
\n\
Supporter bekommen eine eigene Rolle, private Supporter-Channels im VIP-Bereich und Zugang zum Streamer-VC.\n\
\n\
Geplant, und noch nicht eingerichtet, sind Einblicke in das, was gerade gebaut wird, Abstimmungen über neue Funktionen und früherer Zugang zu Betatests.\n\
\n\
Moderatoren-Chats, interne Teambereiche und Moderationsrechte gibt es darüber nicht.";
pub const SUPPORT_PLANNED: &str = "\
## Was noch kommt\n\
\n\
**Patch-Watchlist.** Du suchst dir Helden und Items aus und bekommst Bescheid, wenn ein Patch genau daran etwas ändert.\n\
\n\
**Weitere Match-Abfragen** im Steam Bot, soweit sie sich zuverlässig bauen lassen.\n\
\n\
Ein eigenes Spielerprofil baue ich nicht. Dafür gibt es Statlocker und ähnliche Seiten. Eine eigene Funktion für Tierlisten und Builds gibt es ebenfalls nicht.";
pub const SUPPORT_FREE: &str = "\
## Was kostenlos bleibt\n\
\n\
Der Server wird nicht kostenpflichtig. Kostenlos bleiben Mitspieler suchen, die normalen Voice-Channels, Community-Aktivitäten, reguläre Turniere, Scrims und das normale Coaching.\n\
\n\
Bei Turnieren und Coaching bringt ein Abo keinen Vorteil.";
pub const SUPPORT_PAY: &str = "\
## So kannst du unterstützen\n\
\n\
**Discord-Server-Abo.** Über den Servernamen, dort wo Discord die Server-Abos anbietet.\n\
\n\
**Server-Boost.** Zählt vorerst ebenfalls als Unterstützung.\n\
\n\
**Ko-fi.** Einmalig oder monatlich. Dafür gibt es die Unterstützer-Rolle für 31 Tage. Nach dem Eingang schalte ich sie manuell frei, in der Regel innerhalb eines Tages.\n\
\n\
**Stripe-Abo für den Server.** Ein eigenes Abo, getrennt von den Abos des Steam Bots und des Twitch Bots. Der Button erscheint erst, wenn die Zahlung deinem Discord-Account zugeordnet wird und die Rolle nach Ablauf oder Kündigung wieder entzogen wird.\n\
\n\
**Twitch-Bot-Abo.** Der Twitch Bot hat eigene kostenpflichtige Abos. Wer so ein Abo hat, soll dieselben Discord-Supporter-Vorteile bekommen, ohne ein zweites Abo. Die automatische Freischaltung ist noch nicht eingerichtet. Bis dahin reicht eine kurze Nachricht, dann trage ich es manuell ein.";
pub const SUPPORT_THANKS: &str = "Danke, dass du den Server und die Projekte mitträgst.";
pub const SUPPORT_DONATE_BUTTON_LABEL: &str = "Über Ko-fi unterstützen";
pub const SUPPORT_STRIPE_BUTTON_LABEL: &str = "Supporter-Abo abschließen";

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
    stripe_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportTexts {
    intro: String,
    why: String,
    ai: String,
    steam: String,
    brain: String,
    discord: String,
    planned: String,
    free: String,
    pay: String,
    thanks: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportButtons {
    donate: String,
    stripe: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedSupportUrls {
    donate: String,
    stripe: Option<String>,
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
    #[serde(default)]
    stripe: SupportStripeToml,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportBodyTextsToml {
    title: Option<String>,
    boost: Option<String>,
    donation: Option<String>,
    benefits: Option<String>,
    intro: Option<String>,
    why: Option<String>,
    ai: Option<String>,
    steam: Option<String>,
    brain: Option<String>,
    discord: Option<String>,
    planned: Option<String>,
    free: Option<String>,
    pay: Option<String>,
    thanks: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportButtonsToml {
    donate: Option<String>,
    stripe: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportUrlsToml {
    donate: Option<String>,
    stripe: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SupportStripeToml {
    #[serde(default)]
    enabled: bool,
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
    Ok(ResolvedSupportConfig::from_defaults().merge(file, warnings))
}

impl ResolvedSupportConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedSupportTexts {
                intro: SUPPORT_INTRO.to_string(),
                why: SUPPORT_WHY.to_string(),
                ai: SUPPORT_AI.to_string(),
                steam: SUPPORT_STEAM.to_string(),
                brain: SUPPORT_BRAIN.to_string(),
                discord: SUPPORT_DISCORD.to_string(),
                planned: SUPPORT_PLANNED.to_string(),
                free: SUPPORT_FREE.to_string(),
                pay: SUPPORT_PAY.to_string(),
                thanks: SUPPORT_THANKS.to_string(),
            },
            buttons: ResolvedSupportButtons {
                donate: SUPPORT_DONATE_BUTTON_LABEL.to_string(),
                stripe: SUPPORT_STRIPE_BUTTON_LABEL.to_string(),
            },
            urls: ResolvedSupportUrls {
                donate: SUPPORT_DONATE_URL_DEFAULT.to_string(),
                stripe: None,
            },
            stripe_enabled: false,
        }
    }

    fn merge(mut self, file: SupportTextsToml, warnings: &mut Vec<String>) -> Self {
        apply_optional(&mut self.texts.intro, file.texts.intro);
        apply_optional(&mut self.texts.why, file.texts.why);
        apply_optional(&mut self.texts.ai, file.texts.ai);
        apply_optional(&mut self.texts.steam, file.texts.steam);
        apply_optional(&mut self.texts.brain, file.texts.brain);
        apply_optional(&mut self.texts.discord, file.texts.discord);
        apply_optional(&mut self.texts.planned, file.texts.planned);
        apply_optional(&mut self.texts.free, file.texts.free);
        apply_optional(&mut self.texts.pay, file.texts.pay);
        apply_optional(&mut self.texts.thanks, file.texts.thanks);
        apply_optional(&mut self.buttons.donate, file.buttons.donate);
        apply_optional(&mut self.buttons.stripe, file.buttons.stripe);
        if let Some(donate) = file.urls.donate {
            let donate = donate.trim();
            if is_https_url(donate) {
                self.urls.donate = donate.to_string();
            } else {
                warnings.push(
                    "Ko-fi-Button behält die Standard-URL: [urls].donate muss eine https-Adresse mit Host sein"
                        .to_string(),
                );
            }
        }
        if let Some(stripe) = file.urls.stripe {
            if !stripe.is_empty() {
                self.urls.stripe = Some(stripe);
            }
        }
        self.stripe_enabled = file.stripe.enabled;
        if file.texts.title.is_some()
            || file.texts.boost.is_some()
            || file.texts.donation.is_some()
            || file.texts.benefits.is_some()
        {
            warnings.push(
                "Support-Textdatei enthält alte Felder (title, boost, donation, benefits). Die werden nicht mehr angezeigt."
                    .to_string(),
            );
        }
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
    let messages = vec![
        why_message(banner, config),
        section_message(
            SUPPORT_COMPONENT_ID_BENEFITS_CONTAINER,
            &[
                (SUPPORT_COMPONENT_ID_BENEFITS_STEAM, &config.texts.steam),
                (SUPPORT_COMPONENT_ID_BENEFITS_BRAIN, &config.texts.brain),
                (SUPPORT_COMPONENT_ID_BENEFITS_DISCORD, &config.texts.discord),
                (SUPPORT_COMPONENT_ID_BENEFITS_PLANNED, &config.texts.planned),
                (SUPPORT_COMPONENT_ID_BENEFITS_FREE, &config.texts.free),
            ],
        ),
        pay_message(config, warnings),
    ];
    debug_assert_eq!(messages.len(), SUPPORT_MESSAGE_COUNT);
    messages
}

fn why_message(
    banner: Option<SupportBannerOutput>,
    config: &ResolvedSupportConfig,
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
    children.extend(section_children(&[
        (SUPPORT_COMPONENT_ID_MAIN_TEXT, &config.texts.intro),
        (SUPPORT_COMPONENT_ID_WHY_BODY, &config.texts.why),
        (SUPPORT_COMPONENT_ID_WHY_AI, &config.texts.ai),
    ]));
    message_from_container(
        SUPPORT_COMPONENT_ID_MAIN_CONTAINER,
        banner,
        children,
        attachments,
    )
}

fn section_message(container_id: u64, blocks: &[(u64, &str)]) -> SupportBuiltMessage {
    message_from_container(container_id, None, section_children(blocks), Vec::new())
}

fn pay_message(config: &ResolvedSupportConfig, warnings: &mut Vec<String>) -> SupportBuiltMessage {
    let mut children = section_children(&[
        (SUPPORT_COMPONENT_ID_PAY_BODY, &config.texts.pay),
        (SUPPORT_COMPONENT_ID_PAY_THANKS, &config.texts.thanks),
    ]);
    let mut buttons = vec![link_button(
        SUPPORT_COMPONENT_ID_DONATE_BUTTON,
        &config.buttons.donate,
        &config.urls.donate,
    )];
    if let Some(button) = stripe_button(config, warnings) {
        buttons.push(button);
    }
    if !children.is_empty() {
        children.push(separator(SUPPORT_COMPONENT_ID_PAY_SEP_ACTIONS, 2));
    }
    children.push(action_row(SUPPORT_COMPONENT_ID_ACTION_ROW, buttons));
    message_from_container(
        SUPPORT_COMPONENT_ID_PAY_CONTAINER,
        None,
        children,
        Vec::new(),
    )
}

fn stripe_button(config: &ResolvedSupportConfig, warnings: &mut Vec<String>) -> Option<Value> {
    let Some(url) = config
        .urls
        .stripe
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    else {
        if config.stripe_enabled {
            warnings.push(
                "Stripe-Button aus: [stripe].enabled ist true, aber [urls].stripe fehlt."
                    .to_string(),
            );
        }
        return None;
    };
    if !is_buy_stripe_link(url) {
        warnings.push(
            "Stripe-Button aus: [urls].stripe muss ein https://buy.stripe.com/-Payment-Link des Server-Supporter-Abos sein"
                .to_string(),
        );
        return None;
    }
    if !config.stripe_enabled {
        warnings.push(
            "Stripe-Button aus: [urls].stripe ist gesetzt, [stripe].enabled bleibt false, bis die Zahlung dem Discord-Account zugeordnet wird und die Rolle nach Ablauf oder Kündigung entzogen wird."
                .to_string(),
        );
        return None;
    }
    warnings.push(
        "Stripe-Button ist sichtbar. Dieses Panel prüft nicht, ob die Zahlung dem Discord-Account zugeordnet wird und ob die Rolle nach Ablauf oder Kündigung entzogen wird."
            .to_string(),
    );
    Some(link_button(
        SUPPORT_COMPONENT_ID_STRIPE_BUTTON,
        &config.buttons.stripe,
        url,
    ))
}

fn same_id_set(left: &[u64], right: &[u64]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn is_https_url(url: &str) -> bool {
    if url.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    !host.is_empty()
        && !host.contains('@')
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
}

fn is_buy_stripe_link(url: &str) -> bool {
    if url.chars().count() > 512 {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://buy.stripe.com/") else {
        return false;
    };
    let slug = rest.split(['?', '#']).next().unwrap_or("");
    let slug = slug.trim_matches('/');
    !slug.is_empty()
        && slug
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
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
            children.push(separator(separator_id_for(*text_id), 1));
        }
    }
    children
}

fn separator_id_for(text_id: u64) -> u64 {
    match text_id {
        SUPPORT_COMPONENT_ID_MAIN_TEXT => SUPPORT_COMPONENT_ID_WHY_SEP_1,
        SUPPORT_COMPONENT_ID_WHY_BODY => SUPPORT_COMPONENT_ID_WHY_SEP_2,
        SUPPORT_COMPONENT_ID_BENEFITS_STEAM => SUPPORT_COMPONENT_ID_BENEFITS_SEP_1,
        SUPPORT_COMPONENT_ID_BENEFITS_BRAIN => SUPPORT_COMPONENT_ID_BENEFITS_SEP_2,
        SUPPORT_COMPONENT_ID_BENEFITS_DISCORD => SUPPORT_COMPONENT_ID_BENEFITS_SEP_3,
        SUPPORT_COMPONENT_ID_BENEFITS_PLANNED => SUPPORT_COMPONENT_ID_BENEFITS_SEP_4,
        SUPPORT_COMPONENT_ID_PAY_BODY => SUPPORT_COMPONENT_ID_PAY_SEP,
        other => other.saturating_add(100),
    }
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
        let label = value.get("label").and_then(Value::as_str).unwrap_or("");
        let label_chars = label.chars().count();
        if label.trim().is_empty() || label_chars > 80 {
            return Err(format!(
                "Support-Button-Label muss 1 bis 80 Zeichen haben, hat {label_chars}"
            ));
        }
        let url = value.get("url").and_then(Value::as_str).unwrap_or("");
        if !is_https_url(url) || url.chars().count() > 512 {
            return Err(
                "Support-Button-URL muss https sein, einen Host haben und höchstens 512 Zeichen lang sein"
                    .to_string(),
            );
        }
    }
    if let Some(children) = value.get("components").and_then(Value::as_array) {
        for child in children {
            validate_component_shape(child)?;
        }
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
    #[test]
    fn button_prueft_genau_die_gesendeten_zeichen() {
        for url in [
            "https://ko-fi.com/a b",
            " https://ko-fi.com/x",
            "https://ko-fi.com/x\n",
            "https://user@ko-fi.com/x",
        ] {
            assert!(!is_https_url(url), "{url}");
        }
        assert!(is_https_url("https://ko-fi.com/deutschedeadlockcommunity"));
        let padded = serde_json::json!({
            "type": 2,
            "label": format!("{}   ", "a".repeat(80)),
            "url": "https://ko-fi.com/x",
        });
        assert!(validate_component_shape(&padded).is_err());
        let long_stripe = format!("https://buy.stripe.com/{}", "a".repeat(500));
        assert!(!is_buy_stripe_link(&long_stripe));
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

    fn link_buttons(payload: &SupportMessagePayload) -> Vec<Value> {
        let mut buttons = Vec::new();
        fn walk(value: &Value, buttons: &mut Vec<Value>) {
            if value.get("type").and_then(Value::as_u64) == Some(2) {
                buttons.push(value.clone());
            }
            if let Some(children) = value.get("components").and_then(Value::as_array) {
                for child in children {
                    walk(child, buttons);
                }
            }
        }
        for component in &payload.components {
            walk(component, &mut buttons);
        }
        buttons
    }

    fn stored_ids() -> [u64; SUPPORT_MESSAGE_COUNT] {
        [7001, 7002, 7003]
    }

    #[test]
    fn support_texts_temp_toml_mergt_fehlende_felder_auf_compile_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(
            temp.path(),
            r#"
[texts]
pay = "Runtime Zahlung"

[buttons]
donate = "Runtime Button"

[urls]
donate = "https://example.invalid/kofi"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_support_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.intro, SUPPORT_INTRO);
        assert_eq!(config.texts.pay, "Runtime Zahlung");
        assert_eq!(config.texts.why, SUPPORT_WHY);
        assert_eq!(config.buttons.donate, "Runtime Button");
        assert_eq!(config.urls.donate, "https://example.invalid/kofi");
        assert!(!config.stripe_enabled);
    }

    #[test]
    fn alte_felder_bleiben_parsebar_und_werden_nicht_angezeigt() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(
            temp.path(),
            r#"
[texts]
title = "Alt"
benefits = "Lese-Zugriff auf den Community-Moderatoren-Chat"
"#,
        );
        let output = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("output");
        let rendered = output
            .messages
            .iter()
            .flat_map(|message| text_contents(&message.payload))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!rendered.contains("Community-Moderatoren-Chat"));
        assert!(rendered.contains("Moderationsrechte gibt es darüber nicht"));
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
    fn support_payload_ist_drei_goldene_karten_mit_banner_und_kofi() {
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
        assert_eq!(output.messages.len(), SUPPORT_MESSAGE_COUNT);
        assert_eq!(output.messages[0].message_key, "support");
        assert_eq!(output.messages[1].message_key, "support:vorteile");
        assert_eq!(output.messages[2].message_key, "support:zahlung");
        assert!(output.repost_required);

        let why = &output.messages[0].payload;
        assert_eq!(why.flags, SUPPORT_COMPONENTS_V2_FLAG);
        assert!(why.allowed_mentions.parse.is_empty());
        assert_eq!(why.attachments.len(), 1);
        assert_eq!(why.components[0]["type"], json!(17));
        assert_eq!(
            why.components[0]["accent_color"],
            json!(SUPPORT_ACCENT_GOLD)
        );
        assert_eq!(why.components[0]["components"][0]["type"], json!(12));
        assert_eq!(
            why.components[0]["components"][0]["items"][0]["media"]["url"],
            format!("attachment://{SUPPORT_HERO_FILENAME}")
        );
        let why_text = text_contents(why);
        assert_eq!(
            why_text,
            vec![
                SUPPORT_INTRO.to_string(),
                SUPPORT_WHY.to_string(),
                SUPPORT_AI.to_string()
            ]
        );
        assert!(why_text.iter().any(|text| text.contains("1.600 €")));
        assert!(why
            .components
            .iter()
            .any(|component| component_has_type(component, 14)));

        let benefits = text_contents(&output.messages[1].payload);
        assert_eq!(
            benefits,
            vec![
                SUPPORT_STEAM.to_string(),
                SUPPORT_BRAIN.to_string(),
                SUPPORT_DISCORD.to_string(),
                SUPPORT_PLANNED.to_string(),
                SUPPORT_FREE.to_string()
            ]
        );
        assert!(benefits
            .iter()
            .any(|text| text.contains("deadlock-api.com/patron")));
        assert!(benefits
            .iter()
            .any(|text| text.contains("Echtzeit verspreche ich nicht")));
        assert!(benefits
            .iter()
            .any(|text| text.contains("noch nicht fertig")));
        assert!(benefits.iter().any(|text| text.contains("Statlocker")));
        assert!(benefits
            .iter()
            .any(|text| text.contains("noch nicht eingerichtet")));
        let free = benefits
            .iter()
            .find(|text| text.starts_with("## Was kostenlos bleibt"))
            .expect("free section");
        assert!(!free.to_lowercase().contains("steam"));

        let pay = &output.messages[2].payload;
        let pay_text = text_contents(pay);
        assert_eq!(
            pay_text,
            vec![SUPPORT_PAY.to_string(), SUPPORT_THANKS.to_string()]
        );
        assert!(pay_text
            .iter()
            .any(|text| text.contains("noch nicht eingerichtet")));
        let buttons = link_buttons(pay);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0]["style"], json!(5));
        assert_eq!(buttons[0]["label"], SUPPORT_DONATE_BUTTON_LABEL);
        assert_eq!(buttons[0]["url"], SUPPORT_DONATE_URL_DEFAULT);
        assert!(buttons[0].get("custom_id").is_none());
        assert_eq!(
            pay.components[0]["accent_color"],
            json!(SUPPORT_ACCENT_GOLD)
        );

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

    fn component_has_type(value: &Value, kind: u64) -> bool {
        if value.get("type").and_then(Value::as_u64) == Some(kind) {
            return true;
        }
        value
            .get("components")
            .and_then(Value::as_array)
            .is_some_and(|children| children.iter().any(|child| component_has_type(child, kind)))
    }

    #[test]
    fn stripe_button_erscheint_nur_mit_link_und_freigabe() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_support_texts_file(
            temp.path(),
            r#"[urls]
stripe = "https://buy.stripe.com/test-server-supporter"

[stripe]
enabled = true
"#,
        );
        let output = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("output");
        let buttons = link_buttons(&output.messages[2].payload);
        assert_eq!(buttons.len(), 2);
        assert_eq!(buttons[1]["id"], json!(SUPPORT_COMPONENT_ID_STRIPE_BUTTON));
        assert_eq!(buttons[1]["label"], json!(SUPPORT_STRIPE_BUTTON_LABEL));
        assert_eq!(
            buttons[1]["url"],
            "https://buy.stripe.com/test-server-supporter"
        );
        assert_eq!(buttons[1]["style"], json!(5));
        assert!(buttons[1].get("custom_id").is_none());
        assert!(output.warnings.iter().any(|warning| {
            warning.contains("prüft nicht, ob die Zahlung dem Discord-Account zugeordnet wird")
        }));

        write_support_texts_file(
            temp.path(),
            r#"[urls]
stripe = "https://buy.stripe.com/test-server-supporter"
"#,
        );
        let held = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("enabled defaults false");
        assert_eq!(link_buttons(&held.messages[2].payload).len(), 1);
        assert!(held
            .warnings
            .iter()
            .any(|warning| warning.contains("[stripe].enabled bleibt false")));

        write_support_texts_file(
            temp.path(),
            r#"[urls]
stripe = "https://example.com/no-payment-link"

[stripe]
enabled = true
"#,
        );
        let invalid = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("invalid link leaves Ko-fi active");
        assert_eq!(link_buttons(&invalid.messages[2].payload).len(), 1);
        assert!(invalid
            .warnings
            .iter()
            .any(|warning| warning.contains("Stripe-Button aus")));

        write_support_texts_file(
            temp.path(),
            r#"[stripe]
enabled = true
"#,
        );
        let missing = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("enabled without url leaves Ko-fi active");
        assert_eq!(link_buttons(&missing.messages[2].payload).len(), 1);
        assert!(missing
            .warnings
            .iter()
            .any(|warning| warning.contains("[urls].stripe fehlt")));
    }

    #[test]
    fn support_payload_hash_noop_und_banner_bytes_aendern_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner-one");
        let first = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
            true,
        )
        .expect("first output");
        assert_eq!(first.messages[0].action, "planned_edit");

        let same = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("same output");
        assert_eq!(same.messages[0].action, "planned_no_op");
        assert_eq!(same.messages[2].action, "planned_no_op");
        assert!(support_payload_is_unchanged(&same));
        assert!(!same.repost_required);

        write_banner_bytes(temp.path(), b"banner-two");
        let changed = build_support_publish_output(
            temp.path(),
            &stored_ids(),
            Some(SUPPORT_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("changed output");
        assert_eq!(changed.messages[0].action, "planned_edit");
        assert_ne!(changed.payload_hash, first.payload_hash);
    }

    #[test]
    fn eine_gespeicherte_nachricht_wird_als_repost_geplant() {
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_support_publish_output(
            temp.path(),
            &[7001],
            Some(SUPPORT_PAYLOAD_FORMAT),
            None,
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
    fn repo_support_texts_stimmen_mit_den_defaults_ueberein() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut warnings = Vec::new();
        let config = load_support_runtime_config(&repo, &mut warnings).expect("repo config");
        assert!(
            warnings.is_empty(),
            "Repo-Textdatei soll ohne Warnung laden: {warnings:?}"
        );
        assert_eq!(config.texts.intro, SUPPORT_INTRO);
        assert_eq!(config.texts.why, SUPPORT_WHY);
        assert_eq!(config.texts.ai, SUPPORT_AI);
        assert_eq!(config.texts.steam, SUPPORT_STEAM);
        assert_eq!(config.texts.brain, SUPPORT_BRAIN);
        assert_eq!(config.texts.discord, SUPPORT_DISCORD);
        assert_eq!(config.texts.planned, SUPPORT_PLANNED);
        assert_eq!(config.texts.free, SUPPORT_FREE);
        assert_eq!(config.texts.pay, SUPPORT_PAY);
        assert_eq!(config.texts.thanks, SUPPORT_THANKS);
        assert_eq!(config.buttons.donate, SUPPORT_DONATE_BUTTON_LABEL);
        assert_eq!(config.buttons.stripe, SUPPORT_STRIPE_BUTTON_LABEL);
        assert_eq!(config.urls.donate, SUPPORT_DONATE_URL_DEFAULT);
        assert_eq!(config.urls.stripe, None);
        assert!(!config.stripe_enabled);
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
            vec![link_button(2, &long_label, "https://ko-fi.com/x")],
            Vec::new(),
        );
        let overflow_error = validate_support_message_budget(&overflow.payload).expect_err("label");
        assert!(overflow_error.contains("1 bis 80"));

        let bare = message_from_container(
            1,
            None,
            vec![link_button(2, "Ko-fi", "https://")],
            Vec::new(),
        );
        let bare_error = validate_support_message_budget(&bare.payload).expect_err("url");
        assert!(bare_error.contains("https"));
    }
}
