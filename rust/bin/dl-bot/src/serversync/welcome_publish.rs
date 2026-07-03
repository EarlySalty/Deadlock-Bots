use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use dl_server_as_code::{ChannelKind, ChannelSpec, GuildModel, OverwriteKey, RoleSpec, TargetKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::Permissions;

pub const WELCOME_CHANNEL_NAME: &str = "willkommen";
pub const WELCOME_BANNER_DIR: &str = "assets/welcome-banners";
pub const WELCOME_TEXTS_FILE: &str = "assets/welcome_texts.toml";
pub const WELCOME_MARKER_PREFIX: &str = "serversync:welcome:";

pub const WELCOME_LINK_URLS: WelcomeLinkUrls = WelcomeLinkUrls {
    website: "https://earlysalty.com",
    twitch: "https://www.twitch.tv/earlysalty",
    coaching: "https://earlysalty.com/coaching",
    server_invite: "https://discord.com/channels/1289721245281292288/1464736918951432222",
};

pub const WELCOME_TEXTS: WelcomeTextTable = WelcomeTextTable {
    section_titles: WelcomeSectionTitles {
        hero: "Willkommen",
        navigation: "Kanal-Navigation",
        team: "Community-Team",
        socials: "Links & Socials",
        quickstart: "Schnellstart",
    },
    hero_intro: "Willkommen bei der Deutschen Deadlock Community — Deutschlands Anlaufstelle für alles rund um Deadlock. Hier findest du Mitspieler, kostenloses Coaching, Turniere und eine Community, die das Spiel genauso ernst oder locker nimmt wie du. Dieser Kanal zeigt dir, wo was ist.",
    empty_navigation: "Die Navigation wird gerade neu aufgebaut — schau gleich nochmal rein.",
    default_channel_description: "Beschreibung folgt.",
    channel_descriptions: &[
        WelcomeChannelDescription {
            channel_key: "regelwerk",
            description: "Die Regeln des Servers. Einmal lesen, dann weißt du Bescheid.",
        },
        WelcomeChannelDescription {
            channel_key: "willkommen",
            description: "Du bist hier: der Überblick über den ganzen Server.",
        },
        WelcomeChannelDescription {
            channel_key: "deadlock-rang",
            description: "Steam verknüpfen und deine Rang-Rolle automatisch bekommen.",
        },
        WelcomeChannelDescription {
            channel_key: "server-support",
            description: "Fragen oder Probleme mit dem Server? Hier entlang.",
        },
        WelcomeChannelDescription {
            channel_key: "deadlock-invite",
            description: "Unser Einladungslink zum Weitergeben.",
        },
        WelcomeChannelDescription {
            channel_key: "ankündigungen",
            description: "Alles Wichtige aus der Community, kompakt.",
        },
        WelcomeChannelDescription {
            channel_key: "patchnotes",
            description: "Deadlock-Patchnotes, aufbereitet auf Deutsch.",
        },
        WelcomeChannelDescription {
            channel_key: "dev-updates",
            description: "Was sich an unseren Bots und der Website tut.",
        },
        WelcomeChannelDescription {
            channel_key: "streamer-updates",
            description: "Wer aus der Community gerade live ist.",
        },
        WelcomeChannelDescription {
            channel_key: "deadlock-streamer",
            description: "Unsere Twitch-Streamer: Live-Alerts und Highlights.",
        },
        WelcomeChannelDescription {
            channel_key: "allgemein",
            description: "Der Hauptchat für alles rund um Deadlock.",
        },
        WelcomeChannelDescription {
            channel_key: "off-topic",
            description: "Alles, was nichts mit Deadlock zu tun hat.",
        },
        WelcomeChannelDescription {
            channel_key: "frag-die-community",
            description: "Deine Frage, die Antworten der Community.",
        },
        WelcomeChannelDescription {
            channel_key: "memes",
            description: "Memes rein, Niveau egal, Hauptsache lustig.",
        },
        WelcomeChannelDescription {
            channel_key: "leaks",
            description: "Leaks und Datamining-Fundstücke zu Deadlock.",
        },
        WelcomeChannelDescription {
            channel_key: "kreativ-ecke",
            description: "Alles Selbstgemachte: Art, Edits, Mods, Movement-Clips.",
        },
        WelcomeChannelDescription {
            channel_key: "gameplay-clips",
            description: "Deine besten (und schlimmsten) Momente.",
        },
        WelcomeChannelDescription {
            channel_key: "yt-videos",
            description: "Videos aus der Community und rund um Deadlock.",
        },
        WelcomeChannelDescription {
            channel_key: "feedback-kanal",
            description: "Ideen und Verbesserungsvorschläge für den Server.",
        },
        WelcomeChannelDescription {
            channel_key: "bot-spam",
            description: "Bot-Befehle testen, ohne die Chats vollzumüllen.",
        },
        WelcomeChannelDescription {
            channel_key: "mitspieler-suche",
            description: "Finde Mitspieler für Ranked, Casual oder einfach eine Runde.",
        },
        WelcomeChannelDescription {
            channel_key: "game-guides-und-tipps",
            description: "Guides, Tricks und Wissenswertes rund ums Spiel.",
        },
        WelcomeChannelDescription {
            channel_key: "custom-games-chat",
            description: "Organisation rund um Custom Games und Events.",
        },
        WelcomeChannelDescription {
            channel_key: "rank-ups",
            description: "Automatische Glückwünsche bei Rang-Aufstiegen.",
        },
        WelcomeChannelDescription {
            channel_key: "scrim-planung",
            description: "Scrim-Termine und Team-Aufstellungen — pro Scrim ein Thread.",
        },
        WelcomeChannelDescription {
            channel_key: "code-of-conduct",
            description: "Die Spielregeln fürs Coaching-Programm.",
        },
        WelcomeChannelDescription {
            channel_key: "ich-brauch-einen-coach",
            description: "Hier meldest du dich für kostenloses Coaching an.",
        },
        WelcomeChannelDescription {
            channel_key: "coaching-chat",
            description: "Austausch zwischen Coaches und Coachees.",
        },
        WelcomeChannelDescription {
            channel_key: "erfahrungsberichte",
            description: "Was andere aus ihrem Coaching mitgenommen haben.",
        },
        WelcomeChannelDescription {
            channel_key: "sprachkanal-verwalten",
            description: "Deine eigene Lane per Klick verwalten.",
        },
        WelcomeChannelDescription {
            channel_key: "sprach-kanal-verwalten",
            description: "Deine eigene Lane per Klick verwalten.",
        },
        WelcomeChannelDescription {
            channel_key: "anleitung",
            description: "Kurzanleitung für die Ranked-Lanes.",
        },
        WelcomeChannelDescription {
            channel_key: "lane-eröffnen",
            description: "Beitreten — und deine eigene Lane öffnet sich automatisch.",
        },
        WelcomeChannelDescription {
            channel_key: "ranked-competitiv-lane-öffnen",
            description: "Beitreten — und deine Ranked-Lane öffnet sich automatisch.",
        },
        WelcomeChannelDescription {
            channel_key: "street-brawl-lanes",
            description: "Beitreten — und deine Street-Brawl-Lane öffnet sich automatisch.",
        },
        WelcomeChannelDescription {
            channel_key: "deadlock-router",
            description: "Der Einstieg in alle Voice-Lanes.",
        },
        WelcomeChannelDescription {
            channel_key: "neue-spieler-lane",
            description: "Voice für alle, die gerade erst anfangen.",
        },
        WelcomeChannelDescription {
            channel_key: "off-topic-voice",
            description: "Quatschen abseits von Deadlock.",
        },
        WelcomeChannelDescription {
            channel_key: "sammelpunkt",
            description: "Treffpunkt vor Custom Games.",
        },
        WelcomeChannelDescription {
            channel_key: "caster-channel",
            description: "Voice für Caster bei Custom Games und Turnieren.",
        },
        WelcomeChannelDescription {
            channel_key: "afk",
            description: "Kurz weg? Hier parkt dich der Server.",
        },
    ],
    empty_team_role_members: "aktuell unbesetzt",
    socials_intro: "Die Community gibt es auch außerhalb von Discord:",
    quickstart_intro: "Die drei wichtigsten Klicks für den Start:",
    buttons: WelcomeButtonLabels {
        website: "Website",
        twitch: "Twitch",
        coaching: "Coaching",
        server_invite: "Einladungslink",
        rules: "📜 Regelwerk lesen",
        rank: "🔗 Rang verknüpfen",
        support: "🎫 Support",
    },
};

pub const WELCOME_TEAM_ROLE_GROUPS: &[WelcomeTeamRoleGroupSpec] = &[
    WelcomeTeamRoleGroupSpec {
        key: "owner",
        aliases: &["Owner", "Server Owner", "Inhaber"],
    },
    WelcomeTeamRoleGroupSpec {
        key: "admin",
        aliases: &["Admin", "Administrator"],
    },
    WelcomeTeamRoleGroupSpec {
        key: "moderator",
        aliases: &["Moderator", "Mod"],
    },
    WelcomeTeamRoleGroupSpec {
        key: "community-moderator",
        aliases: &["Community Moderator", "Community Mod"],
    },
    WelcomeTeamRoleGroupSpec {
        key: "coach",
        aliases: &["Coach"],
    },
];

pub const WELCOME_TEAM_EXCLUDED_MEMBER_IDS: &[u64] = &[793_214_097_013_080_095];
const WELCOME_QUICKSTART_JUMP_LABEL: &str = "⬆️ Zum Anfang";
const WELCOME_QUICKSTART_JUMP_WARNING: &str =
    "Hero-Message-ID fuer Welcome-Schnellstart-Link fehlt; Button `Zum Anfang` wird weggelassen";

const NON_PUBLIC_CATEGORY_KEYS: &[&str] = &[
    "moderation",
    "streamer",
    "streamer only",
    "vip",
    "support",
    "support tickets",
    "archiv",
    "archive",
];

const EXCLUDED_NAVIGATION_CHANNEL_KEYS: &[&str] = &["nsfw", "rage room"];

const WELCOME_SECTION_DEFINITIONS: &[WelcomeSectionDefinition] = &[
    WelcomeSectionDefinition {
        id: "hero",
        message_key: "welcome:hero",
        banner_filename: Some("hero.png"),
    },
    WelcomeSectionDefinition {
        id: "navigation",
        message_key: "welcome:navigation",
        banner_filename: Some("navigation.png"),
    },
    WelcomeSectionDefinition {
        id: "team",
        message_key: "welcome:team",
        banner_filename: Some("team.png"),
    },
    WelcomeSectionDefinition {
        id: "socials",
        message_key: "welcome:socials",
        banner_filename: Some("socials.png"),
    },
    WelcomeSectionDefinition {
        id: "quickstart",
        message_key: "welcome:quickstart",
        banner_filename: None,
    },
];

#[derive(Debug, Clone, Copy)]
pub struct WelcomeLinkUrls {
    pub website: &'static str,
    pub twitch: &'static str,
    pub coaching: &'static str,
    pub server_invite: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeTextTable {
    pub section_titles: WelcomeSectionTitles,
    pub hero_intro: &'static str,
    pub empty_navigation: &'static str,
    pub default_channel_description: &'static str,
    pub channel_descriptions: &'static [WelcomeChannelDescription],
    pub empty_team_role_members: &'static str,
    pub socials_intro: &'static str,
    pub quickstart_intro: &'static str,
    pub buttons: WelcomeButtonLabels,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeSectionTitles {
    pub hero: &'static str,
    pub navigation: &'static str,
    pub team: &'static str,
    pub socials: &'static str,
    pub quickstart: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeChannelDescription {
    pub channel_key: &'static str,
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeButtonLabels {
    pub website: &'static str,
    pub twitch: &'static str,
    pub coaching: &'static str,
    pub server_invite: &'static str,
    pub rules: &'static str,
    pub rank: &'static str,
    pub support: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeConfig {
    texts: ResolvedWelcomeTextTable,
    urls: ResolvedWelcomeLinkUrls,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeLinkUrls {
    website: String,
    twitch: String,
    coaching: String,
    server_invite: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeTextTable {
    section_titles: ResolvedWelcomeSectionTitles,
    hero_intro: String,
    empty_navigation: String,
    default_channel_description: String,
    channel_descriptions: Vec<ResolvedWelcomeChannelDescription>,
    empty_team_role_members: String,
    socials_intro: String,
    quickstart_intro: String,
    buttons: ResolvedWelcomeButtonLabels,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeSectionTitles {
    hero: String,
    navigation: String,
    team: String,
    socials: String,
    quickstart: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeChannelDescription {
    channel_key: String,
    description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeButtonLabels {
    website: String,
    twitch: String,
    coaching: String,
    server_invite: String,
    rules: String,
    rank: String,
    support: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeTextsToml {
    #[serde(default)]
    titles: WelcomeTitlesToml,
    #[serde(default)]
    texts: WelcomeBodyTextsToml,
    #[serde(default)]
    urls: WelcomeUrlsToml,
    #[serde(default)]
    buttons: WelcomeButtonsToml,
    #[serde(default)]
    channel: Option<Vec<WelcomeChannelToml>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeTitlesToml {
    hero: Option<String>,
    navigation: Option<String>,
    team: Option<String>,
    socials: Option<String>,
    quickstart: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeBodyTextsToml {
    hero_intro: Option<String>,
    empty_navigation: Option<String>,
    default_channel_description: Option<String>,
    empty_team_role_members: Option<String>,
    socials_intro: Option<String>,
    quickstart_intro: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeUrlsToml {
    website: Option<String>,
    twitch: Option<String>,
    coaching: Option<String>,
    server_invite: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeButtonsToml {
    website: Option<String>,
    twitch: Option<String>,
    coaching: Option<String>,
    server_invite: Option<String>,
    rules: Option<String>,
    rank: Option<String>,
    support: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeChannelToml {
    key: String,
    description: String,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeTeamRoleGroupSpec {
    pub key: &'static str,
    pub aliases: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeSectionDefinition {
    pub id: &'static str,
    pub message_key: &'static str,
    pub banner_filename: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WelcomeTeamMember {
    pub user_id: u64,
    pub role_ids: Vec<u64>,
    pub bot: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomePublishOutput {
    pub guild_id: u64,
    pub channel_id: u64,
    pub channel_name: String,
    pub dry_run: bool,
    pub warnings: Vec<String>,
    pub team_roles: Vec<WelcomeTeamRoleOutput>,
    pub sections: Vec<WelcomeSectionOutput>,
    pub stored_message_ids: BTreeMap<String, u64>,
    pub posted_message_ids: BTreeMap<String, u64>,
    pub edited_message_ids: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomeTeamRoleOutput {
    pub key: String,
    pub aliases: Vec<String>,
    pub matched_role_id: Option<u64>,
    pub matched_role_name: Option<String>,
    pub member_ids: Vec<u64>,
    pub member_mentions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomeSectionOutput {
    pub section_id: String,
    pub message_key: String,
    pub marker: String,
    pub action: String,
    pub stored_message_id: Option<u64>,
    pub message_id: Option<u64>,
    pub banner: Option<WelcomeBannerOutput>,
    pub payload: WelcomeMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomeBannerOutput {
    pub filename: String,
    pub relative_path: String,
    pub present: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomeMessagePayload {
    pub content: String,
    pub embeds: Vec<Value>,
    pub components: Vec<Value>,
    pub attachments: Vec<WelcomePayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomePayloadAttachment {
    pub id: u8,
    pub filename: String,
    #[serde(skip)]
    pub relative_path: String,
}

pub fn welcome_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn welcome_sections() -> &'static [WelcomeSectionDefinition] {
    WELCOME_SECTION_DEFINITIONS
}

fn load_welcome_runtime_config(
    repo_root: &Path,
    warnings: &mut Vec<String>,
) -> Result<ResolvedWelcomeConfig, String> {
    let path = repo_root.join(WELCOME_TEXTS_FILE);
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            warnings.push(format!(
                "Welcome-Textdatei `{WELCOME_TEXTS_FILE}` fehlt; Compile-Defaults werden verwendet"
            ));
            return Ok(ResolvedWelcomeConfig::from_defaults());
        }
        Err(err) => {
            return Err(format!(
                "Welcome-Textdatei `{WELCOME_TEXTS_FILE}` konnte nicht gelesen werden: {err}"
            ));
        }
    };

    let file = toml::from_str::<WelcomeTextsToml>(&raw).map_err(|err| {
        format!("Welcome-Textdatei `{WELCOME_TEXTS_FILE}` konnte nicht geparst werden: {err}")
    })?;
    Ok(ResolvedWelcomeConfig::from_defaults().merge(file))
}

impl ResolvedWelcomeConfig {
    fn from_defaults() -> Self {
        Self {
            texts: ResolvedWelcomeTextTable::from_defaults(),
            urls: ResolvedWelcomeLinkUrls::from_defaults(),
        }
    }

    fn merge(mut self, file: WelcomeTextsToml) -> Self {
        apply_optional(&mut self.texts.section_titles.hero, file.titles.hero);
        apply_optional(
            &mut self.texts.section_titles.navigation,
            file.titles.navigation,
        );
        apply_optional(&mut self.texts.section_titles.team, file.titles.team);
        apply_optional(&mut self.texts.section_titles.socials, file.titles.socials);
        apply_optional(
            &mut self.texts.section_titles.quickstart,
            file.titles.quickstart,
        );

        apply_optional(&mut self.texts.hero_intro, file.texts.hero_intro);
        apply_optional(
            &mut self.texts.empty_navigation,
            file.texts.empty_navigation,
        );
        apply_optional(
            &mut self.texts.default_channel_description,
            file.texts.default_channel_description,
        );
        apply_optional(
            &mut self.texts.empty_team_role_members,
            file.texts.empty_team_role_members,
        );
        apply_optional(&mut self.texts.socials_intro, file.texts.socials_intro);
        apply_optional(
            &mut self.texts.quickstart_intro,
            file.texts.quickstart_intro,
        );

        apply_optional(&mut self.urls.website, file.urls.website);
        apply_optional(&mut self.urls.twitch, file.urls.twitch);
        apply_optional(&mut self.urls.coaching, file.urls.coaching);
        apply_optional(&mut self.urls.server_invite, file.urls.server_invite);

        apply_optional(&mut self.texts.buttons.website, file.buttons.website);
        apply_optional(&mut self.texts.buttons.twitch, file.buttons.twitch);
        apply_optional(&mut self.texts.buttons.coaching, file.buttons.coaching);
        apply_optional(
            &mut self.texts.buttons.server_invite,
            file.buttons.server_invite,
        );
        apply_optional(&mut self.texts.buttons.rules, file.buttons.rules);
        apply_optional(&mut self.texts.buttons.rank, file.buttons.rank);
        apply_optional(&mut self.texts.buttons.support, file.buttons.support);

        if let Some(channels) = file.channel {
            self.texts.channel_descriptions = channels
                .into_iter()
                .map(|channel| ResolvedWelcomeChannelDescription {
                    channel_key: channel.key,
                    description: channel.description,
                })
                .collect();
        }

        self
    }
}

impl ResolvedWelcomeLinkUrls {
    fn from_defaults() -> Self {
        Self {
            website: WELCOME_LINK_URLS.website.to_string(),
            twitch: WELCOME_LINK_URLS.twitch.to_string(),
            coaching: WELCOME_LINK_URLS.coaching.to_string(),
            server_invite: WELCOME_LINK_URLS.server_invite.to_string(),
        }
    }
}

impl ResolvedWelcomeTextTable {
    fn from_defaults() -> Self {
        Self {
            section_titles: ResolvedWelcomeSectionTitles {
                hero: WELCOME_TEXTS.section_titles.hero.to_string(),
                navigation: WELCOME_TEXTS.section_titles.navigation.to_string(),
                team: WELCOME_TEXTS.section_titles.team.to_string(),
                socials: WELCOME_TEXTS.section_titles.socials.to_string(),
                quickstart: WELCOME_TEXTS.section_titles.quickstart.to_string(),
            },
            hero_intro: WELCOME_TEXTS.hero_intro.to_string(),
            empty_navigation: WELCOME_TEXTS.empty_navigation.to_string(),
            default_channel_description: WELCOME_TEXTS.default_channel_description.to_string(),
            channel_descriptions: WELCOME_TEXTS
                .channel_descriptions
                .iter()
                .map(|entry| ResolvedWelcomeChannelDescription {
                    channel_key: entry.channel_key.to_string(),
                    description: entry.description.to_string(),
                })
                .collect(),
            empty_team_role_members: WELCOME_TEXTS.empty_team_role_members.to_string(),
            socials_intro: WELCOME_TEXTS.socials_intro.to_string(),
            quickstart_intro: WELCOME_TEXTS.quickstart_intro.to_string(),
            buttons: ResolvedWelcomeButtonLabels {
                website: WELCOME_TEXTS.buttons.website.to_string(),
                twitch: WELCOME_TEXTS.buttons.twitch.to_string(),
                coaching: WELCOME_TEXTS.buttons.coaching.to_string(),
                server_invite: WELCOME_TEXTS.buttons.server_invite.to_string(),
                rules: WELCOME_TEXTS.buttons.rules.to_string(),
                rank: WELCOME_TEXTS.buttons.rank.to_string(),
                support: WELCOME_TEXTS.buttons.support.to_string(),
            },
        }
    }
}

fn apply_optional(target: &mut String, value: Option<String>) {
    if let Some(value) = value {
        *target = value;
    }
}

pub fn welcome_message_id_key(section_id: &str) -> String {
    format!("welcome_message_id_{section_id}")
}

pub fn welcome_marker(section_id: &str) -> String {
    format!("{WELCOME_MARKER_PREFIX}{section_id}")
}

pub fn welcome_section_id_from_marker(marker: &str) -> Option<&'static str> {
    let section_id = marker.strip_prefix(WELCOME_MARKER_PREFIX)?;
    WELCOME_SECTION_DEFINITIONS
        .iter()
        .find(|section| section.id == section_id)
        .map(|section| section.id)
}

pub fn welcome_section_id_from_title(title: &str) -> Option<&'static str> {
    [
        ("hero", WELCOME_TEXTS.section_titles.hero),
        ("navigation", WELCOME_TEXTS.section_titles.navigation),
        ("team", WELCOME_TEXTS.section_titles.team),
        ("socials", WELCOME_TEXTS.section_titles.socials),
        ("quickstart", WELCOME_TEXTS.section_titles.quickstart),
    ]
    .into_iter()
    .find(|(_, section_title)| *section_title == title)
    .map(|(section_id, _)| section_id)
}

pub fn welcome_candidate_message_ids(
    stored_message_id: Option<u64>,
    discovered_message_id: Option<u64>,
) -> Vec<u64> {
    let mut candidates = Vec::new();
    if let Some(id) = stored_message_id {
        candidates.push(id);
    }
    if let Some(id) = discovered_message_id {
        candidates.push(id);
    }
    candidates.sort_unstable();
    candidates.dedup();
    if let Some(stored) = stored_message_id {
        candidates.sort_by_key(|id| usize::from(*id != stored));
    }
    candidates
}

pub fn build_welcome_publish_output(
    model: &GuildModel,
    team_members: &[WelcomeTeamMember],
    repo_root: &Path,
    stored_message_ids: &BTreeMap<String, u64>,
    dry_run: bool,
) -> Result<WelcomePublishOutput, String> {
    let welcome_channel =
        resolve_single_channel(model, WELCOME_CHANNEL_NAME)?.ok_or_else(|| {
            "Kanal `willkommen` wurde im Live-Guild-Modell nicht gefunden".to_string()
        })?;
    let mut warnings = Vec::new();
    let config = load_welcome_runtime_config(repo_root, &mut warnings)?;
    let texts = &config.texts;
    let urls = &config.urls;
    let team_roles = resolve_team_roles(model, team_members, &mut warnings);
    let (navigation_embeds, navigation_attachments) =
        navigation_embeds(model, repo_root, texts, &mut warnings);
    let quickstart_buttons = quickstart_buttons(model, texts)?;
    let hero_message_id = stored_message_ids.get("hero").copied();
    let hero_jump_url = hero_message_id
        .map(|message_id| hero_message_url(model.guild_id, welcome_channel.channel_id, message_id));
    if hero_jump_url.is_none() {
        warnings.push(WELCOME_QUICKSTART_JUMP_WARNING.to_string());
    }

    let mut sections = Vec::new();
    for definition in WELCOME_SECTION_DEFINITIONS {
        let marker = welcome_marker(definition.id);
        let banner = definition
            .banner_filename
            .map(|filename| welcome_banner(repo_root, filename, &mut warnings));
        let payload = match definition.id {
            "hero" => payload_with_optional_banner(
                &texts.hero_intro,
                &texts.section_titles.hero,
                &marker,
                banner.as_ref(),
                Vec::new(),
            ),
            "navigation" => WelcomeMessagePayload {
                content: String::new(),
                embeds: navigation_embeds.clone(),
                components: Vec::new(),
                attachments: navigation_attachments.clone(),
            },
            "team" => team_payload(&team_roles, texts, &marker, banner.as_ref()),
            "socials" => payload_with_optional_banner(
                &texts.socials_intro,
                &texts.section_titles.socials,
                &marker,
                banner.as_ref(),
                vec![button_row(vec![
                    link_button(&texts.buttons.website, &urls.website),
                    link_button(&texts.buttons.twitch, &urls.twitch),
                    link_button(&texts.buttons.coaching, &urls.coaching),
                    link_button(&texts.buttons.server_invite, &urls.server_invite),
                ])],
            ),
            "quickstart" => WelcomeMessagePayload {
                content: texts.quickstart_intro.clone(),
                embeds: vec![text_embed(
                    &texts.section_titles.quickstart,
                    None,
                    &marker,
                    None,
                )],
                components: quickstart_components(
                    quickstart_buttons.clone(),
                    hero_jump_url.as_deref(),
                ),
                attachments: Vec::new(),
            },
            other => return Err(format!("Unbekannte Welcome-Sektion `{other}`")),
        };
        let stored_message_id = stored_message_ids.get(definition.id).copied();
        sections.push(WelcomeSectionOutput {
            section_id: definition.id.to_string(),
            message_key: definition.message_key.to_string(),
            marker,
            action: if dry_run {
                if stored_message_id.is_some() {
                    "planned_edit".to_string()
                } else {
                    "planned_post".to_string()
                }
            } else {
                "pending".to_string()
            },
            stored_message_id,
            message_id: stored_message_id,
            banner,
            payload,
        });
    }

    Ok(WelcomePublishOutput {
        guild_id: model.guild_id,
        channel_id: welcome_channel.channel_id,
        channel_name: welcome_channel.name.clone(),
        dry_run,
        warnings,
        team_roles,
        sections,
        stored_message_ids: stored_message_ids.clone(),
        posted_message_ids: BTreeMap::new(),
        edited_message_ids: BTreeMap::new(),
    })
}

pub fn refresh_quickstart_jump_button(
    output: &mut WelcomePublishOutput,
    hero_message_id: Option<u64>,
) {
    let hero_jump_url = hero_message_id
        .map(|message_id| hero_message_url(output.guild_id, output.channel_id, message_id));
    if hero_jump_url.is_some() {
        output
            .warnings
            .retain(|warning| warning != WELCOME_QUICKSTART_JUMP_WARNING);
    } else if !output
        .warnings
        .iter()
        .any(|warning| warning == WELCOME_QUICKSTART_JUMP_WARNING)
    {
        output
            .warnings
            .push(WELCOME_QUICKSTART_JUMP_WARNING.to_string());
    }
    for section in &mut output.sections {
        if section.section_id != "quickstart" {
            continue;
        }
        let Some(first_row) = section.payload.components.first() else {
            continue;
        };
        let Some(components) = first_row
            .get("components")
            .and_then(Value::as_array)
            .cloned()
        else {
            continue;
        };
        let primary_buttons = components
            .into_iter()
            .filter(|button| {
                button.get("label").and_then(Value::as_str) != Some(WELCOME_QUICKSTART_JUMP_LABEL)
            })
            .collect::<Vec<_>>();
        section.payload.components =
            quickstart_components(primary_buttons, hero_jump_url.as_deref());
    }
}

fn welcome_banner(
    repo_root: &Path,
    filename: &str,
    warnings: &mut Vec<String>,
) -> WelcomeBannerOutput {
    let relative_path = format!("{WELCOME_BANNER_DIR}/{filename}");
    let present = repo_root.join(&relative_path).is_file();
    if !present {
        warnings.push(format!(
            "Welcome-Banner `{relative_path}` fehlt; Sektion wird ohne Bild gebaut"
        ));
    }
    WelcomeBannerOutput {
        filename: filename.to_string(),
        relative_path,
        present,
    }
}

fn divider_attachment_for_category(
    repo_root: &Path,
    category_name: &str,
    attachment_id: usize,
    warnings: &mut Vec<String>,
) -> Option<WelcomePayloadAttachment> {
    let filename = divider_filename_for_category(category_name)?;
    let relative_path = format!("{WELCOME_BANNER_DIR}/{filename}");
    if !repo_root.join(&relative_path).is_file() {
        warnings.push(format!(
            "Welcome-Divider `{relative_path}` fehlt; Kategorie `{category_name}` wird ohne Bild gebaut"
        ));
        return None;
    }
    let id = u8::try_from(attachment_id).unwrap_or(u8::MAX);
    Some(WelcomePayloadAttachment {
        id,
        filename: filename.to_string(),
        relative_path,
    })
}

fn divider_filename_for_category(category_name: &str) -> Option<&'static str> {
    match normalized_name(category_name).as_str() {
        "information" => Some("divider-information.png"),
        "medien" => Some("divider-medien.png"),
        "community" => Some("divider-community.png"),
        "deadlock" => Some("divider-deadlock.png"),
        "coaching" => Some("divider-coaching.png"),
        "deadlock router" => Some("divider-router.png"),
        "custom game" => Some("divider-custom.png"),
        _ => None,
    }
}

fn payload_with_optional_banner(
    content: &str,
    title: &str,
    marker: &str,
    banner: Option<&WelcomeBannerOutput>,
    components: Vec<Value>,
) -> WelcomeMessagePayload {
    WelcomeMessagePayload {
        content: content.to_string(),
        embeds: vec![text_embed(title, None, marker, banner)],
        components,
        attachments: payload_attachments(banner),
    }
}

fn team_payload(
    team_roles: &[WelcomeTeamRoleOutput],
    texts: &ResolvedWelcomeTextTable,
    marker: &str,
    banner: Option<&WelcomeBannerOutput>,
) -> WelcomeMessagePayload {
    let fields = team_roles
        .iter()
        .filter_map(|role| {
            let name = role.matched_role_name.as_ref()?;
            let value = if role.member_mentions.is_empty() {
                texts.empty_team_role_members.clone()
            } else {
                truncate_embed_field(&role.member_mentions.join(" "))
            };
            Some(json!({
                "name": name,
                "value": value,
                "inline": false,
            }))
        })
        .collect::<Vec<_>>();

    let description = fields
        .is_empty()
        .then(|| texts.empty_team_role_members.clone());
    let mut embed = embed_base(&texts.section_titles.team, description.as_deref(), marker);
    if !fields.is_empty() {
        embed["fields"] = Value::Array(fields);
    }
    if let Some(image) = banner_image(banner) {
        embed["image"] = image;
    }

    WelcomeMessagePayload {
        content: String::new(),
        embeds: vec![embed],
        components: Vec::new(),
        attachments: payload_attachments(banner),
    }
}

fn navigation_embeds(
    model: &GuildModel,
    repo_root: &Path,
    texts: &ResolvedWelcomeTextTable,
    warnings: &mut Vec<String>,
) -> (Vec<Value>, Vec<WelcomePayloadAttachment>) {
    let mut embeds = Vec::new();
    let mut attachments = Vec::new();
    let mut categories = model.categories.values().collect::<Vec<_>>();
    categories.sort_by_key(|category| (category.position, category.category_id));
    for category in categories {
        if !is_public_category_name(&category.name) {
            continue;
        }
        let mut channels = model
            .channels
            .values()
            .filter(|channel| channel.parent_category_id == Some(category.category_id))
            .filter(|channel| is_navigation_channel(model, channel))
            .collect::<Vec<_>>();
        channels.sort_by_key(|channel| (channel.position, channel.channel_id));
        if channels.is_empty() {
            continue;
        }
        let fields = channels
            .into_iter()
            .map(|channel| {
                json!({
                    "name": format!("<#{}>", channel.channel_id),
                    "value": channel_description(texts, &channel.name),
                    "inline": false,
                })
            })
            .collect::<Vec<_>>();
        let mut embed = embed_base(&category.name, None, "");
        if let Some(attachment) =
            divider_attachment_for_category(repo_root, &category.name, attachments.len(), warnings)
        {
            embed["image"] = json!({
                "url": format!("attachment://{}", attachment.filename),
            });
            attachments.push(attachment);
        }
        embed["fields"] = Value::Array(fields);
        embeds.push(embed);
    }

    if embeds.is_empty() {
        warnings.push("Welcome-Navigation enthaelt keine oeffentlichen Kanaele".to_string());
        return (
            vec![text_embed(
                &texts.section_titles.navigation,
                Some(&texts.empty_navigation),
                &welcome_marker("navigation"),
                None,
            )],
            Vec::new(),
        );
    }
    (embeds, attachments)
}

fn quickstart_buttons(
    model: &GuildModel,
    texts: &ResolvedWelcomeTextTable,
) -> Result<Vec<Value>, String> {
    Ok(vec![
        channel_link_button(
            &texts.buttons.rules,
            model.guild_id,
            require_channel_id(model, "regelwerk")?,
        ),
        channel_link_button(
            &texts.buttons.rank,
            model.guild_id,
            require_channel_id(model, "deadlock-rang")?,
        ),
        channel_link_button(
            &texts.buttons.support,
            model.guild_id,
            require_channel_id(model, "server-support")?,
        ),
    ])
}

fn quickstart_components(primary_buttons: Vec<Value>, hero_jump_url: Option<&str>) -> Vec<Value> {
    let mut rows = vec![button_row(primary_buttons)];
    if let Some(url) = hero_jump_url {
        rows.push(button_row(vec![link_button(
            WELCOME_QUICKSTART_JUMP_LABEL,
            url,
        )]));
    }
    rows
}

fn hero_message_url(guild_id: u64, channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
}

fn resolve_team_roles(
    model: &GuildModel,
    team_members: &[WelcomeTeamMember],
    warnings: &mut Vec<String>,
) -> Vec<WelcomeTeamRoleOutput> {
    WELCOME_TEAM_ROLE_GROUPS
        .iter()
        .map(|group| {
            let matched = resolve_roles(model, group.aliases);
            if matched.is_empty() {
                warnings.push(format!(
                    "Team-Rolle `{}` wurde im Live-Guild-Modell nicht gefunden",
                    group.key
                ));
            }
            let role_ids = matched.iter().map(|role| role.role_id).collect::<Vec<_>>();
            let member_ids = members_for_roles(team_members, &role_ids);
            let member_mentions = member_ids
                .iter()
                .map(|id| format!("<@{id}>"))
                .collect::<Vec<_>>();
            WelcomeTeamRoleOutput {
                key: group.key.to_string(),
                aliases: group
                    .aliases
                    .iter()
                    .map(|alias| (*alias).to_string())
                    .collect(),
                matched_role_id: matched.first().map(|role| role.role_id),
                matched_role_name: matched.first().map(|role| role.name.clone()),
                member_ids,
                member_mentions,
            }
        })
        .collect()
}

fn resolve_roles<'a>(model: &'a GuildModel, aliases: &[&str]) -> Vec<&'a RoleSpec> {
    let normalized_aliases = aliases
        .iter()
        .map(|alias| normalized_name(alias))
        .collect::<BTreeSet<_>>();
    let mut candidates = model
        .roles
        .values()
        .filter(|role| normalized_aliases.contains(&normalized_name(&role.name)))
        .collect::<Vec<_>>();
    candidates.sort_by_key(|role| (role.managed, -role.position, role.role_id));
    candidates
}

fn members_for_roles(team_members: &[WelcomeTeamMember], role_ids: &[u64]) -> Vec<u64> {
    let mut member_ids = team_members
        .iter()
        .filter(|member| {
            !member.bot
                && !WELCOME_TEAM_EXCLUDED_MEMBER_IDS.contains(&member.user_id)
                && member
                    .role_ids
                    .iter()
                    .any(|role_id| role_ids.contains(role_id))
        })
        .map(|member| member.user_id)
        .collect::<Vec<_>>();
    member_ids.sort_unstable();
    member_ids.dedup();
    member_ids
}

fn resolve_single_channel<'a>(
    model: &'a GuildModel,
    name: &str,
) -> Result<Option<&'a ChannelSpec>, String> {
    let expected = normalized_name(name);
    let mut candidates = model
        .channels
        .values()
        .filter(|channel| normalized_name(&channel.name) == expected)
        .collect::<Vec<_>>();
    candidates.sort_by_key(|channel| channel.channel_id);
    match candidates.as_slice() {
        [] => Ok(None),
        [channel] => Ok(Some(channel)),
        _ => Err(format!(
            "Kanal `{name}` ist im Live-Guild-Modell mehrdeutig: {}",
            candidates
                .iter()
                .map(|channel| format!("{}:{}", channel.channel_id, channel.name))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn require_channel_id(model: &GuildModel, name: &str) -> Result<u64, String> {
    resolve_single_channel(model, name)?
        .map(|channel| channel.channel_id)
        .ok_or_else(|| format!("Kanal `{name}` wurde im Live-Guild-Modell nicht gefunden"))
}

fn is_public_category_name(name: &str) -> bool {
    let normalized = normalized_name(name);
    !NON_PUBLIC_CATEGORY_KEYS
        .iter()
        .any(|hidden| normalized == *hidden || normalized.contains(hidden))
}

fn is_navigation_channel(model: &GuildModel, model_channel: &ChannelSpec) -> bool {
    matches!(
        model_channel.kind,
        ChannelKind::Text | ChannelKind::News | ChannelKind::Forum
    ) && !EXCLUDED_NAVIGATION_CHANNEL_KEYS
        .iter()
        .any(|name| normalized_name(&model_channel.name) == *name)
        && !model_channel.nsfw
        && everyone_can_view(model, model_channel.channel_id)
}

fn channel_description<'a>(texts: &'a ResolvedWelcomeTextTable, channel_name: &str) -> &'a str {
    let normalized = normalized_name(channel_name);
    texts
        .channel_descriptions
        .iter()
        .find(|entry| normalized_name(&entry.channel_key) == normalized)
        .map_or(texts.default_channel_description.as_str(), |entry| {
            entry.description.as_str()
        })
}

fn text_embed(
    title: &str,
    description: Option<&str>,
    marker: &str,
    banner: Option<&WelcomeBannerOutput>,
) -> Value {
    let mut embed = embed_base(title, description, marker);
    if let Some(image) = banner_image(banner) {
        embed["image"] = image;
    }
    embed
}

fn embed_base(title: &str, description: Option<&str>, _marker: &str) -> Value {
    let mut embed = json!({
        "title": title,
    });
    if let Some(description) = description.filter(|value| !value.is_empty()) {
        embed["description"] = json!(description);
    }
    embed
}

fn banner_image(banner: Option<&WelcomeBannerOutput>) -> Option<Value> {
    let banner = banner.filter(|banner| banner.present)?;
    Some(json!({
        "url": format!("attachment://{}", banner.filename),
    }))
}

fn payload_attachments(banner: Option<&WelcomeBannerOutput>) -> Vec<WelcomePayloadAttachment> {
    banner
        .filter(|banner| banner.present)
        .map(|banner| WelcomePayloadAttachment {
            id: 0,
            filename: banner.filename.clone(),
            relative_path: banner.relative_path.clone(),
        })
        .into_iter()
        .collect()
}

fn button_row(buttons: Vec<Value>) -> Value {
    json!({
        "type": 1,
        "components": buttons,
    })
}

fn link_button(label: &str, url: &str) -> Value {
    json!({
        "type": 2,
        "style": 5,
        "label": label,
        "url": url,
    })
}

fn channel_link_button(label: &str, guild_id: u64, channel_id: u64) -> Value {
    link_button(
        label,
        &format!("https://discord.com/channels/{guild_id}/{channel_id}"),
    )
}

fn truncate_embed_field(value: &str) -> String {
    const MAX_FIELD_VALUE_CHARS: usize = 1024;
    if value.chars().count() <= MAX_FIELD_VALUE_CHARS {
        return value.to_string();
    }
    value
        .chars()
        .take(MAX_FIELD_VALUE_CHARS.saturating_sub(4))
        .chain(" ...".chars())
        .collect()
}

fn everyone_can_view(model: &GuildModel, channel_id: u64) -> bool {
    everyone_permissions_for_channel(model, channel_id).contains(Permissions::VIEW_CHANNEL)
}

fn everyone_permissions_for_channel(model: &GuildModel, channel_id: u64) -> Permissions {
    let mut permissions = model
        .roles
        .get(&model.guild_id)
        .or_else(|| model.roles.values().find(|role| role.name == "@everyone"))
        .map(|role| Permissions::from_bits_truncate(role.permissions_bitmask))
        .unwrap_or_default();

    if let Some(parent_id) = model
        .channels
        .get(&channel_id)
        .and_then(|channel| channel.parent_category_id)
    {
        apply_everyone_overwrite(model, parent_id, &mut permissions);
    }
    apply_everyone_overwrite(model, channel_id, &mut permissions);
    permissions
}

fn apply_everyone_overwrite(model: &GuildModel, channel_id: u64, permissions: &mut Permissions) {
    let key = OverwriteKey {
        channel_id,
        target_kind: TargetKind::Role,
        target_id: model.guild_id,
    };
    if let Some(overwrite) = model.overwrites.get(&key) {
        *permissions &= !Permissions::from_bits_truncate(overwrite.deny_bits);
        *permissions |= Permissions::from_bits_truncate(overwrite.allow_bits);
    }
}

fn normalized_name(name: &str) -> String {
    name.chars()
        .flat_map(char::to_lowercase)
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use dl_server_as_code::{
        CategorySpec, ChannelKind, ChannelSpec, GuildModel, OverwriteKey, PermissionOverwriteSpec,
        RoleSpec, TargetKind,
    };

    use super::*;

    fn role(id: u64, name: &str, permissions: Permissions, position: i32) -> RoleSpec {
        RoleSpec {
            guild_id: 1,
            role_id: id,
            name: name.to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: permissions.bits(),
            position,
        }
    }

    fn category(id: u64, name: &str, position: i32) -> CategorySpec {
        CategorySpec {
            guild_id: 1,
            category_id: id,
            name: name.to_string(),
            position,
        }
    }

    fn channel(id: u64, name: &str, parent: u64, position: i32) -> ChannelSpec {
        ChannelSpec {
            guild_id: 1,
            channel_id: id,
            name: name.to_string(),
            kind: ChannelKind::Text,
            topic: None,
            position,
            parent_category_id: Some(parent),
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            status: None,
        }
    }

    fn base_model() -> GuildModel {
        let mut model = GuildModel::new(1);
        model
            .roles
            .insert(1, role(1, "@everyone", Permissions::VIEW_CHANNEL, 0));
        model
            .categories
            .insert(10, category(10, "🏛️ ─ INFORMATION ─", 1));
        model
            .categories
            .insert(11, category(11, "💬 ─ COMMUNITY ─", 2));
        model
            .categories
            .insert(12, category(12, "🎟️ ─ SUPPORT ─", 3));
        for (id, name, parent, position) in [
            (20, "🧭willkommen", 10, 1),
            (21, "📜regelwerk", 10, 2),
            (22, "🔗deadlock-rang", 10, 3),
            (23, "🎫server-support", 10, 4),
            (24, "🌐allgemein", 11, 1),
        ] {
            model
                .channels
                .insert(id, channel(id, name, parent, position));
        }
        model
    }

    fn write_welcome_texts_file(repo_root: &Path, body: &str) {
        let path = repo_root.join(WELCOME_TEXTS_FILE);
        std::fs::create_dir_all(path.parent().expect("texts parent")).expect("assets dir");
        std::fs::write(path, body).expect("welcome texts file");
    }

    #[test]
    fn welcome_texts_seed_toml_parst_zu_compile_defaults() {
        let mut warnings = Vec::new();
        let config = load_welcome_runtime_config(&welcome_repo_root(), &mut warnings)
            .expect("welcome texts seed");

        assert_eq!(config, ResolvedWelcomeConfig::from_defaults());
        assert!(warnings.is_empty(), "unerwartete Warnungen: {warnings:?}");
    }

    #[test]
    fn welcome_texts_parse_fehler_schlaegt_hart_fehl() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_welcome_texts_file(temp.path(), "[titles]\nhero = [\n");

        let mut warnings = Vec::new();
        let err = load_welcome_runtime_config(temp.path(), &mut warnings).expect_err("parse err");

        assert!(err.contains("konnte nicht geparst werden"), "{err}");
        assert!(warnings.is_empty(), "unerwartete Warnungen: {warnings:?}");
    }

    #[test]
    fn welcome_texts_fehlende_datei_nutzt_defaults_mit_warnung() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut warnings = Vec::new();
        let config =
            load_welcome_runtime_config(temp.path(), &mut warnings).expect("default config");

        assert_eq!(config, ResolvedWelcomeConfig::from_defaults());
        assert!(warnings
            .iter()
            .any(|warning| warning.contains(WELCOME_TEXTS_FILE)));
    }

    #[test]
    fn welcome_texts_toml_mergt_teilangaben_und_ersetzt_channel_liste() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_welcome_texts_file(
            temp.path(),
            r#"
[titles]
hero = "Neuer Hero"

[texts]
quickstart_intro = "Neue Schnellstart-Zeile"

[urls]
twitch = "https://example.invalid/twitch"

[buttons]
rank = "Custom Rank"

[[channel]]
key = "allgemein"
description = "Custom allgemein"
"#,
        );

        let mut warnings = Vec::new();
        let config = load_welcome_runtime_config(temp.path(), &mut warnings).expect("merged");

        assert!(warnings.is_empty(), "unerwartete Warnungen: {warnings:?}");
        assert_eq!(config.texts.section_titles.hero, "Neuer Hero");
        assert_eq!(
            config.texts.section_titles.navigation,
            WELCOME_TEXTS.section_titles.navigation
        );
        assert_eq!(config.texts.quickstart_intro, "Neue Schnellstart-Zeile");
        assert_eq!(
            config.texts.default_channel_description,
            WELCOME_TEXTS.default_channel_description
        );
        assert_eq!(config.urls.website, WELCOME_LINK_URLS.website);
        assert_eq!(config.urls.twitch, "https://example.invalid/twitch");
        assert_eq!(config.texts.buttons.rank, "Custom Rank");
        assert_eq!(config.texts.buttons.rules, WELCOME_TEXTS.buttons.rules);
        assert_eq!(
            config.texts.channel_descriptions,
            vec![ResolvedWelcomeChannelDescription {
                channel_key: "allgemein".to_string(),
                description: "Custom allgemein".to_string(),
            }]
        );
    }

    #[test]
    fn welcome_navigation_filtert_nicht_oeffentliche_und_ventil_kanaele() {
        let mut model = base_model();
        model.channels.insert(25, channel(25, "😡rage-room", 11, 2));
        let mut nsfw = channel(26, "nsfw", 11, 3);
        nsfw.nsfw = true;
        model.channels.insert(26, nsfw);
        model
            .channels
            .insert(27, channel(27, "ticket-eröffnen", 12, 1));
        let mut voice = channel(28, "dynamische-lane", 11, 4);
        voice.kind = ChannelKind::Voice;
        model.channels.insert(28, voice);
        model.categories.insert(13, category(13, "Street Brawl", 4));
        model.overwrites.insert(
            OverwriteKey {
                channel_id: 12,
                target_kind: TargetKind::Role,
                target_id: 1,
            },
            PermissionOverwriteSpec {
                guild_id: 1,
                key: OverwriteKey {
                    channel_id: 12,
                    target_kind: TargetKind::Role,
                    target_id: 1,
                },
                allow_bits: 0,
                deny_bits: Permissions::VIEW_CHANNEL.bits(),
            },
        );

        let output = build_welcome_publish_output(
            &model,
            &[],
            tempfile::tempdir().expect("tempdir").path(),
            &BTreeMap::new(),
            true,
        )
        .expect("welcome output");
        let navigation = output
            .sections
            .iter()
            .find(|section| section.section_id == "navigation")
            .expect("navigation");
        let text = serde_json::to_string(&navigation.payload.embeds).expect("embeds json");

        assert!(text.contains("<#21>"));
        assert!(text.contains("<#24>"));
        assert!(!text.contains("<#25>"));
        assert!(!text.contains("<#26>"));
        assert!(!text.contains("<#27>"));
        assert!(!text.contains("<#28>"));
        assert!(!text.contains("Street Brawl"));
    }

    #[test]
    fn welcome_banner_fehlt_degradiert_zu_warnung_ohne_image() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_welcome_publish_output(&model, &[], temp.path(), &BTreeMap::new(), true)
            .expect("welcome output");

        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("assets/welcome-banners/hero.png")));
        let hero = output
            .sections
            .iter()
            .find(|section| section.section_id == "hero")
            .expect("hero");
        assert_eq!(
            hero.banner.as_ref().map(|banner| banner.present),
            Some(false)
        );
        assert!(hero.payload.attachments.is_empty());
        assert!(hero.payload.embeds[0].get("image").is_none());
    }

    #[test]
    fn welcome_payloads_haben_keine_sichtbaren_marker_footer() {
        let model = base_model();
        let output = build_welcome_publish_output(
            &model,
            &[],
            tempfile::tempdir().expect("tempdir").path(),
            &BTreeMap::new(),
            true,
        )
        .expect("welcome output");

        for section in &output.sections {
            for embed in &section.payload.embeds {
                assert!(
                    embed.get("footer").is_none(),
                    "Sektion {} darf keinen sichtbaren Marker-Footer haben",
                    section.section_id
                );
            }
        }
    }

    #[test]
    fn welcome_navigation_divider_nutzen_attachment_urls() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let banner_dir = temp.path().join(WELCOME_BANNER_DIR);
        std::fs::create_dir_all(&banner_dir).expect("banner dir");
        for filename in ["divider-information.png", "divider-community.png"] {
            std::fs::write(banner_dir.join(filename), b"png").expect("divider");
        }

        let output = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::from([("hero".to_string(), 7001)]),
            true,
        )
        .expect("welcome output");
        let navigation = output
            .sections
            .iter()
            .find(|section| section.section_id == "navigation")
            .expect("navigation");

        assert_eq!(
            navigation
                .payload
                .attachments
                .iter()
                .map(|attachment| (attachment.id, attachment.filename.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "divider-information.png"), (1, "divider-community.png")]
        );
        let images = navigation
            .payload
            .embeds
            .iter()
            .filter_map(|embed| {
                embed
                    .get("image")
                    .and_then(|image| image.get("url"))
                    .and_then(Value::as_str)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            images,
            vec![
                "attachment://divider-information.png",
                "attachment://divider-community.png"
            ]
        );
    }

    #[test]
    fn welcome_idempotenz_priorisiert_storage_und_nutzt_marker_fallback() {
        assert_eq!(
            welcome_candidate_message_ids(Some(10), Some(11)),
            vec![10, 11]
        );
        assert_eq!(welcome_candidate_message_ids(None, Some(11)), vec![11]);
        assert_eq!(welcome_candidate_message_ids(Some(10), Some(10)), vec![10]);
        assert_eq!(
            welcome_section_id_from_title(WELCOME_TEXTS.section_titles.hero),
            Some("hero")
        );
    }

    #[test]
    fn welcome_team_rollen_sammeln_aliase_und_filtern_ausschluesse() {
        let mut model = base_model();
        model.roles.insert(
            30,
            role(30, "Community Moderator", Permissions::empty(), 30),
        );
        model
            .roles
            .insert(31, role(31, "Coach", Permissions::empty(), 10));
        model
            .roles
            .insert(32, role(32, "Community Mod", Permissions::empty(), 20));
        model
            .roles
            .insert(33, role(33, "Moderator", Permissions::empty(), 40));
        let members = vec![
            WelcomeTeamMember {
                user_id: 100,
                role_ids: vec![30],
                bot: false,
            },
            WelcomeTeamMember {
                user_id: 101,
                role_ids: vec![31],
                bot: false,
            },
            WelcomeTeamMember {
                user_id: 102,
                role_ids: vec![30],
                bot: true,
            },
            WelcomeTeamMember {
                user_id: 103,
                role_ids: vec![33],
                bot: false,
            },
            WelcomeTeamMember {
                user_id: 104,
                role_ids: vec![32],
                bot: false,
            },
            WelcomeTeamMember {
                user_id: WELCOME_TEAM_EXCLUDED_MEMBER_IDS[0],
                role_ids: vec![30, 31, 33],
                bot: false,
            },
        ];

        let output = build_welcome_publish_output(
            &model,
            &members,
            tempfile::tempdir().expect("tempdir").path(),
            &BTreeMap::new(),
            true,
        )
        .expect("welcome output");

        let moderator = output
            .team_roles
            .iter()
            .find(|role| role.key == "moderator")
            .expect("moderator");
        assert_eq!(moderator.matched_role_name.as_deref(), Some("Moderator"));
        assert_eq!(moderator.member_mentions, vec!["<@103>"]);
        let community_moderator = output
            .team_roles
            .iter()
            .find(|role| role.key == "community-moderator")
            .expect("community moderator");
        assert_eq!(
            community_moderator.member_mentions,
            vec!["<@100>", "<@104>"]
        );
        let coach = output
            .team_roles
            .iter()
            .find(|role| role.key == "coach")
            .expect("coach");
        assert_eq!(coach.member_mentions, vec!["<@101>"]);
    }

    #[test]
    fn welcome_quickstart_jump_button_nutzt_hero_message_id() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::from([("hero".to_string(), 7001)]),
            true,
        )
        .expect("welcome output");
        let quickstart = output
            .sections
            .iter()
            .find(|section| section.section_id == "quickstart")
            .expect("quickstart");
        assert_eq!(quickstart.payload.components.len(), 2);
        assert_eq!(
            quickstart.payload.components[1]["components"][0]["label"],
            WELCOME_QUICKSTART_JUMP_LABEL
        );
        assert_eq!(
            quickstart.payload.components[1]["components"][0]["url"],
            "https://discord.com/channels/1/20/7001"
        );

        let mut without_hero =
            build_welcome_publish_output(&model, &[], temp.path(), &BTreeMap::new(), true)
                .expect("welcome output");
        assert!(without_hero
            .warnings
            .iter()
            .any(|warning| warning == WELCOME_QUICKSTART_JUMP_WARNING));
        refresh_quickstart_jump_button(&mut without_hero, Some(7002));
        let quickstart = without_hero
            .sections
            .iter()
            .find(|section| section.section_id == "quickstart")
            .expect("quickstart");
        assert_eq!(
            quickstart.payload.components[1]["components"][0]["url"],
            "https://discord.com/channels/1/20/7002"
        );
        assert!(!without_hero
            .warnings
            .iter()
            .any(|warning| warning == WELCOME_QUICKSTART_JUMP_WARNING));
    }

    #[test]
    fn welcome_deadlock_streamer_beschreibung_ist_gesetzt() {
        let texts = ResolvedWelcomeTextTable::from_defaults();

        assert_eq!(
            channel_description(&texts, "🎥deadlock-streamer"),
            "Unsere Twitch-Streamer: Live-Alerts und Highlights."
        );
    }
}
