use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const FAQ_CHANNEL_ID: u64 = 1_491_953_161_747_955_853;
pub const FAQ_CHANNEL_NAME: &str = "server-faq";
pub const FAQ_TEXTS_FILE: &str = "assets/faq_texts.toml";
pub const FAQ_BANNER_DIR: &str = "assets/welcome-banners";
pub const FAQ_HERO_FILENAME: &str = "faq-hero.png";
pub const FAQ_PAYLOAD_FORMAT: &str = "2";
pub const FAQ_PAYLOAD_FORMAT_KEY: &str = "faq_v2_payload_format";
pub const FAQ_PAYLOAD_HASH_KEY: &str = "faq_v2_payload_hash";
pub const FAQ_MESSAGE_ID_PREFIX: &str = "faq_v2_message_id_";
pub const FAQ_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const FAQ_EPHEMERAL_FLAG: u64 = 1 << 6;
pub const FAQ_ACCENT_GOLD: u64 = 0xC8A86B;

pub const FAQ_COMPONENT_ID_HERO_MEDIA: u64 = 34_001;
pub const FAQ_COMPONENT_ID_MAIN_CONTAINER: u64 = 34_002;
pub const FAQ_COMPONENT_ID_MAIN_TEXT: u64 = 34_003;
pub const FAQ_COMPONENT_ID_ACTION_ROW: u64 = 34_004;
pub const FAQ_COMPONENT_ID_SELECT: u64 = 34_005;
pub const FAQ_COMPONENT_ID_EPHEMERAL_CONTAINER: u64 = 34_006;
pub const FAQ_COMPONENT_ID_EPHEMERAL_TEXT: u64 = 34_007;

pub const FAQ_MARKER_COMPONENT_IDS: &[u64] = &[
    FAQ_COMPONENT_ID_HERO_MEDIA,
    FAQ_COMPONENT_ID_MAIN_CONTAINER,
    FAQ_COMPONENT_ID_MAIN_TEXT,
    FAQ_COMPONENT_ID_ACTION_ROW,
    FAQ_COMPONENT_ID_SELECT,
];

pub const FAQ_TEXT_CHAR_BUDGET: usize = 3500;
pub const FAQ_COMPONENT_BUDGET: usize = 35;
pub const FAQ_ATTACHMENT_BUDGET: usize = 9;
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;

pub const FAQ_SELECT_CUSTOM_ID: &str = "faq:show";
pub const FAQ_MAIN_TITLE: &str = "**❓ Server-FAQ · Deutsche Deadlock Community**";
pub const FAQ_MAIN_BODY: &str = "Wähl unten deine Frage aus — die Antwort siehst nur du.\nNichts Passendes dabei? Stell deine Frage in <#1426220702054355077> oder mach ein Ticket in <#1483136301271355532> auf.";
pub const FAQ_SELECT_PLACEHOLDER: &str = "Wähl deine Frage …";
pub const FAQ_UNKNOWN_VALUE_MESSAGE: &str =
    "Diese Frage kenne ich nicht mehr — Panel wird gleich aktualisiert.";

pub const FAQ_ANSWER_F1: &str = "**🔑 Wie bekomme ich einen Deadlock-Invite?**\nDer Weg ist kurz: Verknüpf deinen Steam-Account in <#1398021105339334666> (der Guide dort führt dich durch) und nimm danach die Freundschaftsanfrage unseres Steam-Bots an — dein Invite kommt dann automatisch. Alternativ frag einfach in <#1426220702054355077> nach einem Invite — pack am besten direkt deinen Steam-Freundescode dazu, dann kann dich jemand persönlich einladen.";
pub const FAQ_ANSWER_F2: &str = "**🤝 Muss ich dem Bot eine Steam-Freundschaftsanfrage schicken?**\nNormalerweise nicht — nach dem Verknüpfen schickt unser Bot **dir** eine Anfrage, du musst sie auf Steam nur annehmen. Kam nichts an? Dann geh den Weg selbst: Steam → Freunde → „Freund hinzufügen\" → Freundescode **820142646** eingeben — damit findest du unseren Bot eindeutig, unabhängig vom Anzeigenamen. Sobald die Freundschaft steht, läuft dein Invite automatisch weiter.";
pub const FAQ_ANSWER_F3: &str = "**⏳ Wie lange dauert mein Invite?**\nSobald deine Steam-Freundschaft mit dem Bot bestätigt ist, dauert es in der Regel nur ein paar Stunden — oft schneller. Unsicher, ob alles durch ist? Über das Panel in <#1398021105339334666> kannst du deine Verknüpfung jederzeit neu prüfen lassen. Wenn es deutlich länger hängt, schau in „Mein Invite hängt — was tun?\".";
pub const FAQ_ANSWER_F4: &str = "**🛠️ Mein Invite hängt — was tun?**\nFast immer liegt es an einem von drei Punkten: Steam ist noch nicht verknüpft (<#1398021105339334666>), die Freundschaftsanfrage des Bots wurde auf Steam noch nicht angenommen, oder dein Steam-Account ist „limited\" (siehe die Frage dazu). Geh die drei kurz durch — wenn es dann immer noch klemmt, schreib in <#1426220702054355077> oder mach ein Ticket in <#1483136301271355532> auf: Da schaut ein Mensch drauf.";
pub const FAQ_ANSWER_F5: &str = "**💌 Kann mich ein Mensch direkt einladen?**\nJa — Mitglieder mit verknüpftem Steam-Account können Freunde persönlich einladen. Frag am besten direkt die Person, die dich hergeholt hat, oder schreib in <#1426220702054355077>. Und wenn gerade kein Mensch greifbar ist: Der Bot übernimmt automatisch, du musst nichts extra tun.";
pub const FAQ_ANSWER_F6: &str = "**🚧 Steam sagt „limited account\" — warum kein Invite?**\nDas ist eine Beschränkung von Valve, kein Fehler bei uns: „Limited\" sind Steam-Accounts, die noch nie mindestens 5 $ im Steam-Store ausgegeben haben — solche Accounts können über unseren Weg keine Playtest-Einladung erhalten. Sobald du einmalig für 5 $ irgendwas auf Steam gekauft hast, fällt die Sperre weg. Wenn du unsicher bist, mach ein Ticket in <#1483136301271355532> auf — das Team geht die Optionen mit dir durch.";
pub const FAQ_ANSWER_F7: &str = "**🔗 Wie verknüpfe ich meinen Steam-Account?**\nGeh in <#1398021105339334666> — der Guide dort führt dich Schritt für Schritt durch: Button klicken, auf der offiziellen Steam-Seite einloggen, Freundschaftsanfrage des Bots annehmen, fertig. Über dasselbe Panel kannst du den Stand jederzeit neu prüfen lassen.";
pub const FAQ_ANSWER_F8: &str = "**🛡️ Was passiert bei der Verifizierung mit meinen Daten?**\nWas du davon hast: deine echte Rang-Rolle (hält sich ab dann von selbst aktuell), Zugang zu den Ranked-Funktionen (Ranked-Lanes öffnen und Ranked-Gesuchen beitreten) und du kannst Freunde per Invite reinholen. Zur Technik: Die Verknüpfung läuft über den offiziellen Steam-Login (OpenID) — wir sehen nie dein Passwort und haben keinerlei Zugriff auf deinen Account, wir bekommen nur deine Steam-ID. Deine Daten kannst du jederzeit exportieren oder löschen lassen.";
pub const FAQ_ANSWER_F9: &str = "**🏆 Wie bekomme oder ändere ich meine Rang-Rolle?**\nZwei Stufen: Beim Onboarding gibst du deinen Rang selbst an und bekommst die passende Rolle — das kannst du jederzeit im „Kanäle & Rollen\"-Tab ändern. Verknüpfst du zusätzlich deinen Steam-Account in <#1398021105339334666>, bekommst du deine echte Rang-Rolle aus dem Spiel, die sich ab dann automatisch aktuell hält — da musst du nie wieder etwas anfassen.";
pub const FAQ_ANSWER_F10: &str = "**🌱 Ich bin ganz neu — hilft mir jemand?**\nJa, genau dafür sind wir da: In der Neue-Spieler-Lane erwartet niemand, dass du schon irgendwas kannst — spring einfach rein. Dazu gibt's unser komplett kostenloses Coaching (siehe die Frage dazu), und in <#1426220702054355077> ist keine Frage zu einfach. Sag dazu, dass du neu bist — dann holen dich alle da ab, wo du stehst.";
pub const FAQ_ANSWER_F11: &str = "**🎓 Wie funktioniert das kostenlose Coaching?**\nKomplett kostenlos: Erfahrene Spieler aus der Community nehmen sich Zeit für dich — von den Grundlagen bis zum Rang-Aufstieg. Anmelden kannst du dich über den Coaching-Bereich auf unserer Website (https://deutsche-deadlock-community.de/coaching); deine Anfrage landet direkt beim Coach-Team hier im Discord und ein Coach übernimmt sie.";
pub const FAQ_ANSWER_F12: &str = "**🎯 Wo finde ich Mitspieler?**\nZwei Wege: Poste ein Gesuch in <#1522769149208821881> — Modus, Rang-Bereich und Spielzeit wählst du einfach per Klick, und mit 🔔 kannst du dich benachrichtigen lassen, sobald ein passendes Gesuch reinkommt. Oder spring direkt in eine Voice-Lane: Über das Panel in <#1513468476365209670> öffnest du eine Lane für Normale Lane, Ranked oder Street Brawl (für Ranked brauchst du die Steam-Verknüpfung). Reinsetzen ist ausdrücklich erlaubt — du musst niemanden um Erlaubnis fragen.";
pub const FAQ_ANSWER_F13: &str = "**🎙️ Wie funktionieren die Voice-Lanes?**\nDu darfst in jede rein — unsere Lanes haben keine Türsteher. Wer eine Lane öffnet, wählt den Modus; bei Ranked ist der Rang-Bereich eine Ansage, kein Schloss. Die Regel dahinter ist einfach: Der Lane-Ersteller entscheidet, wer bleibt — bei Streit entscheiden die Mods. Die Chill-Lanes sind unser rang-egales Wohnzimmer, die Neue-Spieler-Lane ist für alle, die gerade erst anfangen. Nur zuhören ist übrigens auch völlig okay.";
pub const FAQ_ANSWER_F14: &str = "**🔔 Pings & Benachrichtigungen einstellen**\nOben links über dem Kanalbaum findest du „Kanäle & Rollen\" — dort schaltest du Benachrichtigungs-Rollen selbst an oder aus und kannst auch deine Onboarding-Angaben jederzeit ändern (das sind reine Angaben über dich, keine Berechtigungen). Für einzelne Kanäle gilt der Discord-Standard: Rechtsklick auf den Kanal → Benachrichtigungen anpassen.";
pub const FAQ_ANSWER_F15: &str = "**🧭 Wo fange ich an — und wo kann ich fragen?**\nFolg einfach der Server-Guide-Checkliste oben im Kanalbaum: Sag Hallo in <#1426220702054355077>, verknüpf deinen Steam-Account, such dir Mitspieler — in der Reihenfolge, ohne Zeitdruck. Und für alles andere gilt: <#1426220702054355077> ist genau dafür da. Es gibt keine dummen Fragen, und hier antworten dir echte Menschen — normalerweise noch am selben Tag.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FaqDefaultEntry {
    pub value: &'static str,
    pub label: &'static str,
    pub emoji: &'static str,
    pub answer: &'static str,
}

pub const FAQ_DEFAULT_ENTRIES: &[FaqDefaultEntry] = &[
    FaqDefaultEntry {
        value: "f1",
        label: "Wie bekomme ich einen Deadlock-Invite?",
        emoji: "🔑",
        answer: FAQ_ANSWER_F1,
    },
    FaqDefaultEntry {
        value: "f2",
        label: "Muss ich dem Bot eine Steam-Freundschaftsanfrage schicken?",
        emoji: "🤝",
        answer: FAQ_ANSWER_F2,
    },
    FaqDefaultEntry {
        value: "f3",
        label: "Wie lange dauert mein Invite?",
        emoji: "⏳",
        answer: FAQ_ANSWER_F3,
    },
    FaqDefaultEntry {
        value: "f4",
        label: "Mein Invite hängt — was tun?",
        emoji: "🛠️",
        answer: FAQ_ANSWER_F4,
    },
    FaqDefaultEntry {
        value: "f5",
        label: "Kann mich ein Mensch direkt einladen?",
        emoji: "💌",
        answer: FAQ_ANSWER_F5,
    },
    FaqDefaultEntry {
        value: "f6",
        label: "Steam sagt „limited account\" — warum kein Invite?",
        emoji: "🚧",
        answer: FAQ_ANSWER_F6,
    },
    FaqDefaultEntry {
        value: "f7",
        label: "Wie verknüpfe ich meinen Steam-Account?",
        emoji: "🔗",
        answer: FAQ_ANSWER_F7,
    },
    FaqDefaultEntry {
        value: "f8",
        label: "Was passiert bei der Verifizierung mit meinen Daten?",
        emoji: "🛡️",
        answer: FAQ_ANSWER_F8,
    },
    FaqDefaultEntry {
        value: "f9",
        label: "Wie bekomme oder ändere ich meine Rang-Rolle?",
        emoji: "🏆",
        answer: FAQ_ANSWER_F9,
    },
    FaqDefaultEntry {
        value: "f10",
        label: "Ich bin ganz neu — hilft mir jemand?",
        emoji: "🌱",
        answer: FAQ_ANSWER_F10,
    },
    FaqDefaultEntry {
        value: "f11",
        label: "Wie funktioniert das kostenlose Coaching?",
        emoji: "🎓",
        answer: FAQ_ANSWER_F11,
    },
    FaqDefaultEntry {
        value: "f12",
        label: "Wo finde ich Mitspieler?",
        emoji: "🎯",
        answer: FAQ_ANSWER_F12,
    },
    FaqDefaultEntry {
        value: "f13",
        label: "Wie funktionieren die Voice-Lanes?",
        emoji: "🎙️",
        answer: FAQ_ANSWER_F13,
    },
    FaqDefaultEntry {
        value: "f14",
        label: "Pings & Benachrichtigungen einstellen",
        emoji: "🔔",
        answer: FAQ_ANSWER_F14,
    },
    FaqDefaultEntry {
        value: "f15",
        label: "Wo fange ich an — und wo kann ich fragen?",
        emoji: "🧭",
        answer: FAQ_ANSWER_F15,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaqPublishOutput {
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
    pub messages: Vec<FaqMessageOutput>,
    pub stored_message_ids: Vec<u64>,
    pub posted_message_ids: Vec<u64>,
    pub edited_message_ids: Vec<u64>,
    pub deleted_message_ids: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaqMessageOutput {
    pub message_index: usize,
    pub message_key: String,
    pub action: String,
    pub stored_message_id: Option<u64>,
    pub message_id: Option<u64>,
    pub banner: Option<FaqBannerOutput>,
    pub payload: FaqMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaqBannerOutput {
    pub filename: String,
    pub relative_path: String,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaqMessagePayload {
    pub flags: u64,
    pub allowed_mentions: FaqAllowedMentions,
    pub components: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FaqPayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaqAllowedMentions {
    pub parse: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FaqPayloadAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FaqV2Message {
    pub message_id: u64,
    pub author_id: u64,
    pub flags: u64,
    pub components: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFaqConfig {
    texts: ResolvedFaqTexts,
    options: Vec<ResolvedFaqOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFaqTexts {
    title: String,
    body: String,
    placeholder: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedFaqOption {
    value: String,
    label: String,
    emoji: String,
    answer: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaqTextsToml {
    #[serde(default)]
    texts: FaqBodyTextsToml,
    #[serde(default)]
    options: Vec<FaqOptionToml>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaqBodyTextsToml {
    title: Option<String>,
    body: Option<String>,
    placeholder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FaqOptionToml {
    value: String,
    label: Option<String>,
    emoji: Option<String>,
    answer: Option<String>,
}

#[derive(Debug, Clone)]
struct FaqBuiltMessage {
    banner: Option<FaqBannerOutput>,
    payload: FaqMessagePayload,
}

#[cfg(test)]
pub fn faq_message_id_key(message_index: usize) -> String {
    format!("{FAQ_MESSAGE_ID_PREFIX}{message_index}")
}

pub fn build_faq_publish_output(
    repo_root: &Path,
    stored_message_ids: &[u64],
    stored_payload_format: Option<&str>,
    stored_payload_hash: Option<&str>,
    dry_run: bool,
) -> Result<FaqPublishOutput, String> {
    let mut warnings = Vec::new();
    let config = load_faq_runtime_config(repo_root, &mut warnings)?;
    let built_messages = faq_messages(repo_root, &config, &mut warnings);

    let mut messages = Vec::new();
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored_message_ids.get(message_index).copied();
        messages.push(FaqMessageOutput {
            message_index,
            message_key: faq_message_key(message_index),
            action: if dry_run {
                planned_faq_action(stored_message_id, false, false)
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
        validate_faq_message_budget(&message.payload)?;
    }

    let payload_hash = faq_payload_hash(repo_root, &messages)?;
    let repost_required =
        !faq_storage_matches(stored_payload_format, stored_message_ids, messages.len());
    let hash_matches = stored_payload_hash == Some(payload_hash.as_str());
    if dry_run {
        for message in &mut messages {
            message.action =
                planned_faq_action(message.stored_message_id, repost_required, hash_matches);
        }
    }

    Ok(FaqPublishOutput {
        guild_id: dl_server_as_code::DEFAULT_GUILD_ID,
        channel_id: FAQ_CHANNEL_ID,
        channel_name: FAQ_CHANNEL_NAME.to_string(),
        dry_run,
        payload_format: FAQ_PAYLOAD_FORMAT.to_string(),
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

pub fn faq_storage_matches(
    stored_payload_format: Option<&str>,
    stored_message_ids: &[u64],
    expected_message_count: usize,
) -> bool {
    stored_payload_format == Some(FAQ_PAYLOAD_FORMAT)
        && stored_message_ids.len() == expected_message_count
}

pub fn faq_payload_is_unchanged(output: &FaqPublishOutput) -> bool {
    output.stored_payload_hash.as_deref() == Some(output.payload_hash.as_str())
}

pub fn faq_payload_hash(repo_root: &Path, messages: &[FaqMessageOutput]) -> Result<String, String> {
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
                            "FAQ-Attachment `{}` konnte fuer Hash nicht gelesen werden: {err}",
                            path.display()
                        )
                    })?;
                    Ok(FaqPayloadHashAttachment {
                        id: attachment.id,
                        filename: attachment.filename.as_str(),
                        sha256: format!("{:x}", Sha256::digest(bytes)),
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(FaqPayloadHashMessage {
                payload: &message.payload,
                attachment_hashes,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let raw = serde_json::to_vec(&payloads)
        .map_err(|err| format!("FAQ-Payload konnte nicht serialisiert werden: {err}"))?;
    Ok(format!("{:x}", Sha256::digest(raw)))
}

#[derive(Serialize)]
struct FaqPayloadHashMessage<'a> {
    payload: &'a FaqMessagePayload,
    attachment_hashes: Vec<FaqPayloadHashAttachment<'a>>,
}

#[derive(Serialize)]
struct FaqPayloadHashAttachment<'a> {
    id: u8,
    filename: &'a str,
    sha256: String,
}

pub fn is_faq_v2_message(message: &FaqV2Message, bot_user_id: u64) -> bool {
    message.author_id == bot_user_id
        && message.flags & FAQ_COMPONENTS_V2_FLAG != 0
        && has_faq_v2_marker(&message.components)
}

pub fn has_faq_v2_marker(components: &[Value]) -> bool {
    collect_component_ids(components)
        .iter()
        .any(|component_id| FAQ_MARKER_COMPONENT_IDS.contains(component_id))
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

pub fn faq_answer_for_value(value: &str) -> Option<&'static str> {
    FAQ_DEFAULT_ENTRIES
        .iter()
        .find(|entry| entry.value == value)
        .map(|entry| entry.answer)
}

pub fn register_components(router: &mut dl_discord::InteractionRouter) {
    router.on_custom_id(FAQ_SELECT_CUSTOM_ID, Arc::new(FaqSelectHandler));
}

struct FaqSelectHandler;

#[async_trait::async_trait]
impl dl_discord::InteractionHandler for FaqSelectHandler {
    async fn handle(&self, interaction: dl_discord::BridgeInteraction) -> dl_discord::BridgeReply {
        faq_section_reply(interaction.values.first().map(String::as_str))
    }
}

pub fn faq_section_reply(value: Option<&str>) -> dl_discord::BridgeReply {
    let Some(text) = value.and_then(faq_answer_for_value) else {
        return dl_discord::BridgeReply {
            content: Some(FAQ_UNKNOWN_VALUE_MESSAGE.to_string()),
            ephemeral: true,
            allowed_mentions: Some(empty_allowed_mentions_value()),
            ..dl_discord::BridgeReply::default()
        };
    };
    dl_discord::BridgeReply {
        components: Some(json!([container(
            FAQ_COMPONENT_ID_EPHEMERAL_CONTAINER,
            vec![text_display(
                FAQ_COMPONENT_ID_EPHEMERAL_TEXT,
                text.to_string()
            )],
        )])),
        ephemeral: true,
        message_flags: Some(FAQ_EPHEMERAL_FLAG | FAQ_COMPONENTS_V2_FLAG),
        allowed_mentions: Some(empty_allowed_mentions_value()),
        fallback: Some(Box::new(faq_section_fallback_reply(text))),
        ..dl_discord::BridgeReply::default()
    }
}

pub fn faq_section_fallback_reply(text: &str) -> dl_discord::BridgeReply {
    dl_discord::BridgeReply {
        embeds: vec![json!({
            "description": text,
            "color": FAQ_ACCENT_GOLD,
        })],
        ephemeral: true,
        allowed_mentions: Some(empty_allowed_mentions_value()),
        ..dl_discord::BridgeReply::default()
    }
}

fn empty_allowed_mentions_value() -> Value {
    json!({ "parse": [] })
}

fn load_faq_runtime_config(
    repo_root: &Path,
    warnings: &mut Vec<String>,
) -> Result<ResolvedFaqConfig, String> {
    let path = repo_root.join(FAQ_TEXTS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            warnings.push(format!(
                "FAQ-Textdatei `{FAQ_TEXTS_FILE}` fehlt; Compile-Defaults werden verwendet"
            ));
            return Ok(ResolvedFaqConfig::from_defaults());
        }
        Err(err) => {
            return Err(format!(
                "FAQ-Textdatei `{FAQ_TEXTS_FILE}` konnte nicht gelesen werden: {err}"
            ));
        }
    };

    let file = toml::from_str::<FaqTextsToml>(&raw).map_err(|err| {
        format!("FAQ-Textdatei `{FAQ_TEXTS_FILE}` konnte nicht geparst werden: {err}")
    })?;
    ResolvedFaqConfig::from_defaults().merge(file)
}

impl ResolvedFaqConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedFaqTexts {
                title: FAQ_MAIN_TITLE.to_string(),
                body: FAQ_MAIN_BODY.to_string(),
                placeholder: FAQ_SELECT_PLACEHOLDER.to_string(),
            },
            options: FAQ_DEFAULT_ENTRIES
                .iter()
                .map(|entry| ResolvedFaqOption {
                    value: entry.value.to_string(),
                    label: entry.label.to_string(),
                    emoji: entry.emoji.to_string(),
                    answer: entry.answer.to_string(),
                })
                .collect(),
        }
    }

    fn merge(mut self, file: FaqTextsToml) -> Result<Self, String> {
        apply_optional(&mut self.texts.title, file.texts.title);
        apply_optional(&mut self.texts.body, file.texts.body);
        apply_optional(&mut self.texts.placeholder, file.texts.placeholder);

        let mut seen = BTreeSet::new();
        for option in file.options {
            if !seen.insert(option.value.clone()) {
                return Err(format!(
                    "FAQ-Textdatei `{FAQ_TEXTS_FILE}` enthaelt value `{}` mehrfach",
                    option.value
                ));
            }
            let Some(target) = self
                .options
                .iter_mut()
                .find(|entry| entry.value == option.value)
            else {
                return Err(format!(
                    "FAQ-Textdatei `{FAQ_TEXTS_FILE}` enthaelt unbekannten value `{}`",
                    option.value
                ));
            };
            apply_optional(&mut target.label, option.label);
            apply_optional(&mut target.emoji, option.emoji);
            apply_optional(&mut target.answer, option.answer);
        }
        Ok(self)
    }
}

fn apply_optional(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}

fn faq_messages(
    repo_root: &Path,
    config: &ResolvedFaqConfig,
    warnings: &mut Vec<String>,
) -> Vec<FaqBuiltMessage> {
    let banner = optional_faq_banner(repo_root, FAQ_HERO_FILENAME, warnings);
    let mut components = Vec::new();
    let mut attachments = Vec::new();
    if let Some(banner) = &banner {
        components.push(media_gallery(FAQ_COMPONENT_ID_HERO_MEDIA, &banner.filename));
        attachments.push(FaqPayloadAttachment {
            id: 0,
            filename: banner.filename.clone(),
            relative_path: banner.relative_path.clone(),
        });
    }

    components.push(container(
        FAQ_COMPONENT_ID_MAIN_CONTAINER,
        vec![
            text_display(
                FAQ_COMPONENT_ID_MAIN_TEXT,
                section_text(&config.texts.title, &config.texts.body),
            ),
            action_row(
                FAQ_COMPONENT_ID_ACTION_ROW,
                vec![string_select(
                    FAQ_COMPONENT_ID_SELECT,
                    FAQ_SELECT_CUSTOM_ID,
                    &config.texts.placeholder,
                    &config.options,
                )],
            ),
        ],
    ));

    vec![FaqBuiltMessage {
        banner,
        payload: FaqMessagePayload {
            flags: FAQ_COMPONENTS_V2_FLAG,
            allowed_mentions: FaqAllowedMentions { parse: Vec::new() },
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

fn optional_faq_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> Option<FaqBannerOutput> {
    let relative_path = format!("{FAQ_BANNER_DIR}/{filename}");
    let path = repo_root.join(&relative_path);
    if !path.is_file() {
        warnings.push(format!(
            "FAQ-Banner `{relative_path}` fehlt; Publish laeuft ohne Banner"
        ));
        return None;
    }
    Some(FaqBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present: true,
    })
}

fn planned_faq_action(
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

fn faq_message_key(message_index: usize) -> String {
    if message_index == 0 {
        "faq".to_string()
    } else {
        format!("faq:{message_index}")
    }
}

fn validate_faq_message_budget(payload: &FaqMessagePayload) -> Result<(), String> {
    let text_chars = text_display_chars(&payload.components);
    if text_chars > FAQ_TEXT_CHAR_BUDGET {
        return Err(format!(
            "FAQ-Payload ueberschreitet Textbudget: {text_chars}/{FAQ_TEXT_CHAR_BUDGET}"
        ));
    }
    if max_text_display_chars(&payload.components) > MESSAGE_TEXT_DISPLAY_CHAR_LIMIT {
        return Err(format!(
            "FAQ-Payload ueberschreitet TextDisplay-Limit {MESSAGE_TEXT_DISPLAY_CHAR_LIMIT}"
        ));
    }
    let component_count = component_count(&payload.components);
    if component_count > FAQ_COMPONENT_BUDGET {
        return Err(format!(
            "FAQ-Payload ueberschreitet Komponentenbudget: {component_count}/{FAQ_COMPONENT_BUDGET}"
        ));
    }
    if payload.attachments.len() > FAQ_ATTACHMENT_BUDGET {
        return Err(format!(
            "FAQ-Payload ueberschreitet Attachmentbudget: {}/{FAQ_ATTACHMENT_BUDGET}",
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
        "accent_color": FAQ_ACCENT_GOLD,
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

fn string_select(
    id: u64,
    custom_id: &str,
    placeholder: &str,
    options: &[ResolvedFaqOption],
) -> Value {
    json!({
        "type": 3,
        "id": id,
        "custom_id": custom_id,
        "placeholder": placeholder,
        "min_values": 1,
        "max_values": 1,
        "options": options.iter().map(select_option).collect::<Vec<_>>(),
    })
}

fn select_option(option: &ResolvedFaqOption) -> Value {
    json!({
        "label": option.label.as_str(),
        "value": option.value.as_str(),
        "emoji": {
            "name": option.emoji.as_str(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dl_discord::InteractionHandler;
    use std::path::Path;
    use std::path::PathBuf;

    fn write_faq_texts_file(repo_root: &Path, body: &str) {
        let path = repo_root.join(FAQ_TEXTS_FILE);
        fs::create_dir_all(path.parent().expect("texts parent")).expect("mkdir texts parent");
        fs::write(path, body).expect("write texts");
    }

    fn write_banner_bytes(repo_root: &Path, bytes: &[u8]) {
        let path = repo_root.join(format!("{FAQ_BANNER_DIR}/{FAQ_HERO_FILENAME}"));
        fs::create_dir_all(path.parent().expect("banner parent")).expect("mkdir banner parent");
        fs::write(path, bytes).expect("write banner");
    }

    #[test]
    fn faq_texts_temp_toml_mergt_fehlende_felder_auf_compile_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_faq_texts_file(
            temp.path(),
            r#"
[texts]
body = "Runtime Body"

[[options]]
value = "f7"
label = "Runtime Label"
answer = "Runtime Answer"
"#,
        );
        let mut warnings = Vec::new();
        let config = load_faq_runtime_config(temp.path(), &mut warnings).expect("config");
        assert_eq!(config.texts.title, FAQ_MAIN_TITLE);
        assert_eq!(config.texts.body, "Runtime Body");
        assert_eq!(config.texts.placeholder, FAQ_SELECT_PLACEHOLDER);
        assert_eq!(config.options.len(), 15);
        assert_eq!(config.options[6].value, "f7");
        assert_eq!(config.options[6].label, "Runtime Label");
        assert_eq!(config.options[6].emoji, "🔗");
        assert_eq!(config.options[6].answer, "Runtime Answer");
        assert_eq!(config.options[0].answer, FAQ_ANSWER_F1);
    }

    #[test]
    fn faq_texts_parse_fehler_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_faq_texts_file(temp.path(), "[texts]\nbody = [\n");
        let mut warnings = Vec::new();
        let err = load_faq_runtime_config(temp.path(), &mut warnings).expect_err("parse error");
        assert!(err.contains("konnte nicht geparst werden"));
    }

    #[test]
    fn faq_texts_unbekannter_value_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_faq_texts_file(
            temp.path(),
            r#"
[[options]]
value = "f99"
label = "Nope"
"#,
        );
        let mut warnings = Vec::new();
        let err = load_faq_runtime_config(temp.path(), &mut warnings).expect_err("unknown value");
        assert!(err.contains("unbekannten value `f99`"));
    }

    #[test]
    fn faq_texts_seed_toml_parst_zu_compile_defaults() {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let mut warnings = Vec::new();
        let config = load_faq_runtime_config(&repo_root, &mut warnings).expect("config");
        assert!(warnings.is_empty());
        assert_eq!(config.texts.title, FAQ_MAIN_TITLE);
        assert_eq!(config.texts.body, FAQ_MAIN_BODY);
        assert_eq!(config.texts.placeholder, FAQ_SELECT_PLACEHOLDER);
        assert_eq!(config.options.len(), FAQ_DEFAULT_ENTRIES.len());
        for (resolved, expected) in config.options.iter().zip(FAQ_DEFAULT_ENTRIES) {
            assert_eq!(resolved.value, expected.value);
            assert_eq!(resolved.label, expected.label);
            assert_eq!(resolved.emoji, expected.emoji);
            assert_eq!(resolved.answer, expected.answer);
        }
    }

    #[test]
    fn faq_payload_haelt_v2_mentions_banner_select_und_marker() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner");
        let output =
            build_faq_publish_output(temp.path(), &[], Some(FAQ_PAYLOAD_FORMAT), None, true)
                .expect("output");
        assert_eq!(output.messages.len(), 1);
        let payload = &output.messages[0].payload;
        assert_eq!(payload.flags, FAQ_COMPONENTS_V2_FLAG);
        assert!(payload.allowed_mentions.parse.is_empty());
        assert_eq!(payload.attachments.len(), 1);
        assert_eq!(payload.attachments[0].filename, FAQ_HERO_FILENAME);
        assert_eq!(payload.components[0]["type"], json!(12));
        assert_eq!(
            payload.components[0]["items"][0]["media"]["url"],
            format!("attachment://{FAQ_HERO_FILENAME}")
        );
        assert_eq!(
            payload.components[1]["accent_color"],
            json!(FAQ_ACCENT_GOLD)
        );
        assert_eq!(
            payload.components[1]["components"][0]["content"],
            format!("{FAQ_MAIN_TITLE}\n{FAQ_MAIN_BODY}")
        );
        let select = &payload.components[1]["components"][1]["components"][0];
        assert_eq!(select["type"], json!(3));
        assert_eq!(select["custom_id"], FAQ_SELECT_CUSTOM_ID);
        assert_eq!(select["placeholder"], FAQ_SELECT_PLACEHOLDER);
        assert_eq!(select["min_values"], json!(1));
        assert_eq!(select["max_values"], json!(1));
        let options = select["options"].as_array().expect("options");
        assert_eq!(options.len(), 15);
        for (option, expected) in options.iter().zip(FAQ_DEFAULT_ENTRIES) {
            assert_eq!(option["value"], expected.value);
            assert_eq!(option["label"], expected.label);
            assert_eq!(option["emoji"]["name"], expected.emoji);
        }
        assert_eq!(
            collect_component_custom_ids(&payload.components),
            vec![FAQ_SELECT_CUSTOM_ID.to_string()]
        );
        validate_faq_message_budget(payload).expect("budget");
        assert!(has_faq_v2_marker(&payload.components));
    }

    #[test]
    fn faq_payload_hash_noop_und_banner_bytes_aendern_hash() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_banner_bytes(temp.path(), b"banner-one");
        let first =
            build_faq_publish_output(temp.path(), &[7001], Some(FAQ_PAYLOAD_FORMAT), None, true)
                .expect("first output");

        let same = build_faq_publish_output(
            temp.path(),
            &[7001],
            Some(FAQ_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("same output");
        assert_eq!(same.messages[0].action, "planned_no_op");
        assert!(faq_payload_is_unchanged(&same));

        write_banner_bytes(temp.path(), b"banner-two");
        let changed = build_faq_publish_output(
            temp.path(),
            &[7001],
            Some(FAQ_PAYLOAD_FORMAT),
            Some(&first.payload_hash),
            true,
        )
        .expect("changed output");
        assert_eq!(changed.messages[0].action, "planned_edit");
        assert_ne!(changed.payload_hash, first.payload_hash);
    }

    #[test]
    fn faq_values_mappen_auf_alle_antworten_und_unknown() {
        for entry in FAQ_DEFAULT_ENTRIES {
            assert_eq!(faq_answer_for_value(entry.value), Some(entry.answer));
        }
        assert_eq!(faq_answer_for_value("f16"), None);
        assert_eq!(faq_answer_for_value(""), None);
    }

    #[tokio::test]
    async fn faq_select_reply_ist_ephemeral_v2_mit_allowed_mentions_und_v1_fallback() {
        let handler = FaqSelectHandler;
        let reply = handler
            .handle(dl_discord::BridgeInteraction {
                custom_id: FAQ_SELECT_CUSTOM_ID.to_string(),
                values: vec!["f12".to_string()],
                ..dl_discord::BridgeInteraction::default()
            })
            .await;
        assert_eq!(
            reply.message_flags,
            Some(FAQ_EPHEMERAL_FLAG | FAQ_COMPONENTS_V2_FLAG)
        );
        assert!(!reply.update_message);
        assert_eq!(reply.allowed_mentions, Some(json!({"parse": []})));
        let components = reply.components.expect("components");
        assert_eq!(components[0]["type"], json!(17));
        assert_eq!(components[0]["accent_color"], json!(FAQ_ACCENT_GOLD));
        assert_eq!(components[0]["components"][0]["content"], FAQ_ANSWER_F12);
        let fallback = reply.fallback.expect("fallback");
        assert!(fallback.ephemeral);
        assert!(fallback.components.is_none());
        assert_eq!(fallback.allowed_mentions, Some(json!({"parse": []})));
        assert_eq!(fallback.embeds[0]["color"], json!(FAQ_ACCENT_GOLD));
        assert_eq!(fallback.embeds[0]["description"], FAQ_ANSWER_F12);
    }

    #[tokio::test]
    async fn faq_select_unknown_value_liefert_ephemere_fehlermeldung_ohne_update() {
        let handler = FaqSelectHandler;
        let reply = handler
            .handle(dl_discord::BridgeInteraction {
                custom_id: FAQ_SELECT_CUSTOM_ID.to_string(),
                values: vec!["f99".to_string()],
                ..dl_discord::BridgeInteraction::default()
            })
            .await;
        assert_eq!(reply.content.as_deref(), Some(FAQ_UNKNOWN_VALUE_MESSAGE));
        assert!(reply.ephemeral);
        assert!(!reply.update_message);
        assert_eq!(reply.allowed_mentions, Some(json!({"parse": []})));
    }
}
