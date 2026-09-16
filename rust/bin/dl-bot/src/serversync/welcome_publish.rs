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
pub const WELCOME_PAYLOAD_FORMAT: &str = "2";
pub const WELCOME_PAYLOAD_FORMAT_KEY: &str = "welcome_payload_format";
pub const WELCOME_COMPONENTS_V2_FLAG: u64 = 1 << 15;
pub const WELCOME_ACCENT_GOLD: u64 = 0xC8A86B;
pub const WELCOME_SUPPORT_TICKET_CHANNEL_ID: u64 = 1_459_628_609_705_738_539;

const NAVIGATION_TEXT_CHAR_BUDGET: usize = 3500;
const NAVIGATION_COMPONENT_BUDGET: usize = 35;
const NAVIGATION_ATTACHMENT_BUDGET: usize = 9;
const MESSAGE_TEXT_DISPLAY_CHAR_LIMIT: usize = 4000;

pub const WELCOME_LINK_URLS: WelcomeLinkUrls = WelcomeLinkUrls {
    website: "https://deutsche-deadlock-community.de",
    twitch: "https://www.twitch.tv/earlysalty",
    coaching: "https://deutsche-deadlock-community.de/coaching",
    streamer: "https://deutsche-deadlock-community.de/streamer",
    server_invite: "https://discord.gg/deutsche-deadlock-community-1289721245281292288",
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
    navigation_intro: "Alle Bereiche des Servers im Überblick — ein Klick auf den Kanal bringt dich direkt hin.",
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
            description: "Server unterstützen: Boosts, Spenden und Extras.",
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
    team: WelcomeTeamTexts {
        bot_group_title: "🤖 Server-Management",
        bot_description:
            "unser Bot: verwaltet Rollen, Voice-Lanes, Onboarding, Coaching und diesen Hub.",
    },
    socials_intro: "Die Community gibt es auch außerhalb von Discord:",
    quickstart_intro: "Die drei wichtigsten Klicks für den Start:",
    buttons: WelcomeButtonLabels {
        website: "Website",
        twitch: "Twitch",
        coaching: "Coaching",
        streamer: "Streamer werden",
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
        banner_filename: Some("divider-quickstart.png"),
    },
];

#[derive(Debug, Clone, Copy)]
pub struct WelcomeLinkUrls {
    pub website: &'static str,
    pub twitch: &'static str,
    pub coaching: &'static str,
    pub streamer: &'static str,
    pub server_invite: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeTextTable {
    pub section_titles: WelcomeSectionTitles,
    pub hero_intro: &'static str,
    pub empty_navigation: &'static str,
    pub navigation_intro: &'static str,
    pub default_channel_description: &'static str,
    pub channel_descriptions: &'static [WelcomeChannelDescription],
    pub empty_team_role_members: &'static str,
    pub team: WelcomeTeamTexts,
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
pub struct WelcomeTeamTexts {
    pub bot_group_title: &'static str,
    pub bot_description: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct WelcomeButtonLabels {
    pub website: &'static str,
    pub twitch: &'static str,
    pub coaching: &'static str,
    pub streamer: &'static str,
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
    streamer: String,
    server_invite: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeTextTable {
    section_titles: ResolvedWelcomeSectionTitles,
    hero_intro: String,
    empty_navigation: String,
    navigation_intro: String,
    default_channel_description: String,
    channel_descriptions: Vec<ResolvedWelcomeChannelDescription>,
    empty_team_role_members: String,
    team: ResolvedWelcomeTeamTexts,
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
struct ResolvedWelcomeTeamTexts {
    bot_group_title: String,
    bot_description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResolvedWelcomeButtonLabels {
    website: String,
    twitch: String,
    coaching: String,
    streamer: String,
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
    team: WelcomeTeamToml,
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
    navigation_intro: Option<String>,
    default_channel_description: Option<String>,
    empty_team_role_members: Option<String>,
    socials_intro: Option<String>,
    quickstart_intro: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeTeamToml {
    bot_group_title: Option<String>,
    bot_description: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeUrlsToml {
    website: Option<String>,
    twitch: Option<String>,
    coaching: Option<String>,
    streamer: Option<String>,
    server_invite: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct WelcomeButtonsToml {
    website: Option<String>,
    twitch: Option<String>,
    coaching: Option<String>,
    streamer: Option<String>,
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
    pub payload_format: String,
    pub stored_payload_format: Option<String>,
    pub repost_required: bool,
    pub warnings: Vec<String>,
    pub team_roles: Vec<WelcomeTeamRoleOutput>,
    pub bot_team: Option<WelcomeTeamBotOutput>,
    pub sections: Vec<WelcomeSectionOutput>,
    pub stored_message_ids: BTreeMap<String, Vec<u64>>,
    pub posted_message_ids: BTreeMap<String, Vec<u64>>,
    pub edited_message_ids: BTreeMap<String, Vec<u64>>,
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
pub struct WelcomeTeamBotOutput {
    pub user_id: u64,
    pub mention: String,
    pub group_title: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomeSectionOutput {
    pub section_id: String,
    pub message_index: usize,
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
    pub flags: u64,
    pub allowed_mentions: WelcomeAllowedMentions,
    pub components: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<WelcomePayloadAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WelcomeAllowedMentions {
    pub parse: Vec<String>,
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
            &mut self.texts.navigation_intro,
            file.texts.navigation_intro,
        );
        apply_optional(
            &mut self.texts.default_channel_description,
            file.texts.default_channel_description,
        );
        apply_optional(
            &mut self.texts.empty_team_role_members,
            file.texts.empty_team_role_members,
        );
        apply_optional(
            &mut self.texts.team.bot_group_title,
            file.team.bot_group_title,
        );
        apply_optional(
            &mut self.texts.team.bot_description,
            file.team.bot_description,
        );
        apply_optional(&mut self.texts.socials_intro, file.texts.socials_intro);
        apply_optional(
            &mut self.texts.quickstart_intro,
            file.texts.quickstart_intro,
        );

        apply_optional(&mut self.urls.website, file.urls.website);
        apply_optional(&mut self.urls.twitch, file.urls.twitch);
        apply_optional(&mut self.urls.coaching, file.urls.coaching);
        apply_optional(&mut self.urls.streamer, file.urls.streamer);
        apply_optional(&mut self.urls.server_invite, file.urls.server_invite);

        apply_optional(&mut self.texts.buttons.website, file.buttons.website);
        apply_optional(&mut self.texts.buttons.twitch, file.buttons.twitch);
        apply_optional(&mut self.texts.buttons.coaching, file.buttons.coaching);
        apply_optional(&mut self.texts.buttons.streamer, file.buttons.streamer);
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
            streamer: WELCOME_LINK_URLS.streamer.to_string(),
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
            navigation_intro: WELCOME_TEXTS.navigation_intro.to_string(),
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
            team: ResolvedWelcomeTeamTexts {
                bot_group_title: WELCOME_TEXTS.team.bot_group_title.to_string(),
                bot_description: WELCOME_TEXTS.team.bot_description.to_string(),
            },
            socials_intro: WELCOME_TEXTS.socials_intro.to_string(),
            quickstart_intro: WELCOME_TEXTS.quickstart_intro.to_string(),
            buttons: ResolvedWelcomeButtonLabels {
                website: WELCOME_TEXTS.buttons.website.to_string(),
                twitch: WELCOME_TEXTS.buttons.twitch.to_string(),
                coaching: WELCOME_TEXTS.buttons.coaching.to_string(),
                streamer: WELCOME_TEXTS.buttons.streamer.to_string(),
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

pub fn welcome_legacy_message_id_key(section_id: &str) -> String {
    format!("welcome_message_id_{section_id}")
}

pub fn welcome_message_id_key(section_id: &str, message_index: usize) -> String {
    format!("welcome_message_id_{section_id}_{message_index}")
}

pub fn welcome_message_id_key_prefix(section_id: &str) -> String {
    format!("welcome_message_id_{section_id}_")
}

pub fn welcome_message_key(section_message_key: &str, message_index: usize) -> String {
    if message_index == 0 {
        section_message_key.to_string()
    } else {
        format!("{section_message_key}:{message_index}")
    }
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

#[derive(Debug, Clone)]
struct WelcomeBuiltMessage {
    banner: Option<WelcomeBannerOutput>,
    payload: WelcomeMessagePayload,
}

pub fn build_welcome_publish_output(
    model: &GuildModel,
    team_members: &[WelcomeTeamMember],
    repo_root: &Path,
    stored_message_ids: &BTreeMap<String, Vec<u64>>,
    stored_payload_format: Option<&str>,
    dry_run: bool,
    bot_user_id: Option<u64>,
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
    let bot_team = bot_team_output(bot_user_id, texts, &mut warnings);
    let navigation_messages = navigation_messages(model, repo_root, texts, &mut warnings);
    let quickstart_buttons = quickstart_buttons(model, texts)?;
    let hero_message_id = stored_message_ids
        .get("hero")
        .and_then(|ids| ids.first())
        .copied();
    let hero_jump_url = hero_message_id
        .map(|message_id| hero_message_url(model.guild_id, welcome_channel.channel_id, message_id));
    if hero_jump_url.is_none() {
        warnings.push(WELCOME_QUICKSTART_JUMP_WARNING.to_string());
    }

    let mut sections = Vec::new();
    for definition in WELCOME_SECTION_DEFINITIONS {
        let built_messages = match definition.id {
            "hero" => vec![hero_message(repo_root, texts, definition, &mut warnings)],
            "navigation" => navigation_messages.clone(),
            "team" => vec![team_message(
                repo_root,
                &team_roles,
                bot_team.as_ref(),
                texts,
                definition,
                &mut warnings,
            )],
            "socials" => vec![socials_message(
                repo_root,
                texts,
                urls,
                definition,
                &mut warnings,
            )],
            "quickstart" => vec![quickstart_message(
                repo_root,
                texts,
                definition,
                quickstart_buttons.clone(),
                hero_jump_url.as_deref(),
                &mut warnings,
            )],
            other => return Err(format!("Unbekannte Welcome-Sektion `{other}`")),
        };
        append_section_outputs(
            &mut sections,
            definition,
            built_messages,
            stored_message_ids,
            dry_run,
        );
    }

    let repost_required =
        !welcome_storage_matches(stored_payload_format, stored_message_ids, &sections);
    if dry_run {
        for section in &mut sections {
            section.action = planned_welcome_action(section.stored_message_id, repost_required);
        }
    }

    Ok(WelcomePublishOutput {
        guild_id: model.guild_id,
        channel_id: welcome_channel.channel_id,
        channel_name: welcome_channel.name.clone(),
        dry_run,
        payload_format: WELCOME_PAYLOAD_FORMAT.to_string(),
        stored_payload_format: stored_payload_format.map(str::to_string),
        repost_required,
        warnings,
        team_roles,
        bot_team,
        sections,
        stored_message_ids: stored_message_ids.clone(),
        posted_message_ids: BTreeMap::new(),
        edited_message_ids: BTreeMap::new(),
    })
}

fn append_section_outputs(
    sections: &mut Vec<WelcomeSectionOutput>,
    definition: &WelcomeSectionDefinition,
    built_messages: Vec<WelcomeBuiltMessage>,
    stored_message_ids: &BTreeMap<String, Vec<u64>>,
    dry_run: bool,
) {
    for (message_index, built) in built_messages.into_iter().enumerate() {
        let stored_message_id = stored_message_ids
            .get(definition.id)
            .and_then(|ids| ids.get(message_index))
            .copied();
        sections.push(WelcomeSectionOutput {
            section_id: definition.id.to_string(),
            message_index,
            message_key: welcome_message_key(definition.message_key, message_index),
            marker: welcome_marker(definition.id),
            action: if dry_run {
                planned_welcome_action(stored_message_id, false)
            } else {
                "pending".to_string()
            },
            stored_message_id,
            message_id: stored_message_id,
            banner: built.banner,
            payload: built.payload,
        });
    }
}

fn planned_welcome_action(stored_message_id: Option<u64>, repost_required: bool) -> String {
    if repost_required && stored_message_id.is_some() {
        "planned_repost".to_string()
    } else if stored_message_id.is_some() {
        "planned_edit".to_string()
    } else {
        "planned_post".to_string()
    }
}

pub fn welcome_storage_matches(
    stored_payload_format: Option<&str>,
    stored_message_ids: &BTreeMap<String, Vec<u64>>,
    sections: &[WelcomeSectionOutput],
) -> bool {
    if stored_payload_format != Some(WELCOME_PAYLOAD_FORMAT) {
        return false;
    }
    for definition in WELCOME_SECTION_DEFINITIONS {
        let expected = sections
            .iter()
            .filter(|section| section.section_id == definition.id)
            .count();
        let stored = stored_message_ids.get(definition.id).map_or(0, Vec::len);
        if expected != stored {
            return false;
        }
    }
    true
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
        refresh_quickstart_payload(&mut section.payload, hero_jump_url.as_deref());
    }
}

fn refresh_quickstart_payload(payload: &mut WelcomeMessagePayload, hero_jump_url: Option<&str>) {
    for container in &mut payload.components {
        let Some(container_components) = container
            .get_mut("components")
            .and_then(Value::as_array_mut)
        else {
            continue;
        };
        for component in container_components {
            if component.get("type").and_then(Value::as_u64) != Some(1) {
                continue;
            }
            let Some(buttons) = component
                .get_mut("components")
                .and_then(Value::as_array_mut)
            else {
                continue;
            };
            buttons.retain(|button| {
                button.get("label").and_then(Value::as_str) != Some(WELCOME_QUICKSTART_JUMP_LABEL)
            });
            if let Some(url) = hero_jump_url {
                buttons.push(link_button(WELCOME_QUICKSTART_JUMP_LABEL, url));
            }
            return;
        }
    }
}

fn hero_message(
    repo_root: &Path,
    texts: &ResolvedWelcomeTextTable,
    definition: &WelcomeSectionDefinition,
    warnings: &mut Vec<String>,
) -> WelcomeBuiltMessage {
    let banner = section_banner(repo_root, definition, warnings);
    let mut attachments = Vec::new();
    let mut components = Vec::new();
    push_media_gallery_for_banner(&mut components, &mut attachments, banner.as_ref());
    components.push(text_display(format!(
        "## {}\n{}",
        texts.section_titles.hero, texts.hero_intro
    )));
    WelcomeBuiltMessage {
        banner,
        payload: welcome_payload(vec![container(components)], attachments),
    }
}

fn team_message(
    repo_root: &Path,
    team_roles: &[WelcomeTeamRoleOutput],
    bot_team: Option<&WelcomeTeamBotOutput>,
    texts: &ResolvedWelcomeTextTable,
    definition: &WelcomeSectionDefinition,
    warnings: &mut Vec<String>,
) -> WelcomeBuiltMessage {
    let banner = section_banner(repo_root, definition, warnings);
    let mut attachments = Vec::new();
    let mut components = Vec::new();
    push_media_gallery_for_banner(&mut components, &mut attachments, banner.as_ref());

    let mut role_blocks = team_roles
        .iter()
        .filter_map(|role| {
            let name = role.matched_role_name.as_deref()?;
            let members = if role.member_mentions.is_empty() {
                texts.empty_team_role_members.clone()
            } else {
                role.member_mentions.join(" ")
            };
            Some(format!("### {name}\n{members}"))
        })
        .collect::<Vec<_>>();
    if role_blocks.is_empty() {
        role_blocks.push(format!(
            "### {}\n{}",
            texts.section_titles.team, texts.empty_team_role_members
        ));
    }
    if let Some(bot_team) = bot_team {
        role_blocks.push(format!(
            "### {}\n{} — {}",
            bot_team.group_title, bot_team.mention, bot_team.description
        ));
    }

    let mut used_text_chars = 0usize;
    for (index, block) in role_blocks.into_iter().enumerate() {
        if index > 0 {
            components.push(separator());
        }
        let remaining = MESSAGE_TEXT_DISPLAY_CHAR_LIMIT.saturating_sub(used_text_chars);
        if remaining == 0 {
            break;
        }
        let block = truncate_to_char_budget(&block, remaining);
        used_text_chars += block.chars().count();
        components.push(text_display(block));
    }

    WelcomeBuiltMessage {
        banner,
        payload: welcome_payload(vec![container(components)], attachments),
    }
}

fn socials_message(
    repo_root: &Path,
    texts: &ResolvedWelcomeTextTable,
    urls: &ResolvedWelcomeLinkUrls,
    definition: &WelcomeSectionDefinition,
    warnings: &mut Vec<String>,
) -> WelcomeBuiltMessage {
    let banner = section_banner(repo_root, definition, warnings);
    let mut attachments = Vec::new();
    let mut components = Vec::new();
    push_media_gallery_for_banner(&mut components, &mut attachments, banner.as_ref());
    components.push(text_display(texts.socials_intro.clone()));
    components.push(button_row(vec![
        link_button(&texts.buttons.website, &urls.website),
        link_button(&texts.buttons.twitch, &urls.twitch),
        link_button(&texts.buttons.coaching, &urls.coaching),
        link_button(&texts.buttons.streamer, &urls.streamer),
        link_button(&texts.buttons.server_invite, &urls.server_invite),
    ]));

    WelcomeBuiltMessage {
        banner,
        payload: welcome_payload(vec![container(components)], attachments),
    }
}

fn quickstart_message(
    repo_root: &Path,
    texts: &ResolvedWelcomeTextTable,
    definition: &WelcomeSectionDefinition,
    primary_buttons: Vec<Value>,
    hero_jump_url: Option<&str>,
    warnings: &mut Vec<String>,
) -> WelcomeBuiltMessage {
    let banner = section_banner(repo_root, definition, warnings);
    let mut attachments = Vec::new();
    let mut components = Vec::new();
    push_media_gallery_for_banner(&mut components, &mut attachments, banner.as_ref());
    components.push(text_display(format!(
        "## {}\n{}",
        texts.section_titles.quickstart, texts.quickstart_intro
    )));
    components.extend(quickstart_components(primary_buttons, hero_jump_url));

    WelcomeBuiltMessage {
        banner,
        payload: welcome_payload(vec![container(components)], attachments),
    }
}

#[derive(Debug, Clone)]
struct NavigationCategoryMessage {
    category_name: String,
    divider_filename: Option<String>,
    divider_relative_path: Option<String>,
    channel_list: String,
}

#[derive(Debug, Clone)]
struct NavigationChunk {
    components: Vec<Value>,
    attachments: Vec<WelcomePayloadAttachment>,
    text_chars: usize,
    component_count: usize,
    category_count: usize,
}

impl NavigationChunk {
    fn new() -> Self {
        Self {
            components: Vec::new(),
            attachments: Vec::new(),
            text_chars: 0,
            component_count: 0,
            category_count: 0,
        }
    }

    fn can_add(&self, category: &NavigationCategoryMessage) -> bool {
        self.text_chars + category.text_chars() <= NAVIGATION_TEXT_CHAR_BUDGET
            && self.component_count + category.component_count() <= NAVIGATION_COMPONENT_BUDGET
            && self.attachments.len() + usize::from(category.divider_filename.is_some())
                <= NAVIGATION_ATTACHMENT_BUDGET
    }

    fn push_header(&mut self, banner: Option<&WelcomeBannerOutput>, title: &str, intro: &str) {
        let mut container_components = Vec::new();
        push_media_gallery_for_banner(&mut container_components, &mut self.attachments, banner);
        if container_components.is_empty() {
            let content = format!("## {title}");
            self.text_chars += content.chars().count();
            container_components.push(text_display(content));
        }
        if !intro.is_empty() {
            self.text_chars += intro.chars().count();
            container_components.push(text_display(intro.to_string()));
        }
        let component = container(container_components);
        self.component_count += count_component(&component);
        self.components.push(component);
    }

    fn push_empty_navigation(&mut self, empty_navigation: &str) {
        let component = container(vec![text_display(empty_navigation.to_string())]);
        self.text_chars += empty_navigation.chars().count();
        self.component_count += count_component(&component);
        self.components.push(component);
    }

    fn push_category(&mut self, category: &NavigationCategoryMessage) {
        let mut container_components = Vec::new();
        if let (Some(filename), Some(relative_path)) = (
            category.divider_filename.as_deref(),
            category.divider_relative_path.as_deref(),
        ) {
            push_media_gallery_for_attachment(
                &mut container_components,
                &mut self.attachments,
                filename,
                relative_path,
            );
        } else {
            container_components.push(text_display(format!("### {}", category.category_name)));
        }
        container_components.push(text_display(category.channel_list.clone()));
        let component = container(container_components);
        self.text_chars += category.text_chars();
        self.component_count += count_component(&component);
        self.components.push(component);
        self.category_count += 1;
    }

    fn into_message(self) -> WelcomeBuiltMessage {
        WelcomeBuiltMessage {
            banner: None,
            payload: welcome_payload(self.components, self.attachments),
        }
    }
}

impl NavigationCategoryMessage {
    fn title_chars(&self) -> usize {
        if self.divider_filename.is_some() {
            0
        } else {
            format!("### {}", self.category_name).chars().count()
        }
    }

    fn text_chars(&self) -> usize {
        self.title_chars() + self.channel_list.chars().count()
    }

    fn component_count(&self) -> usize {
        3
    }
}

fn navigation_messages(
    model: &GuildModel,
    repo_root: &Path,
    texts: &ResolvedWelcomeTextTable,
    warnings: &mut Vec<String>,
) -> Vec<WelcomeBuiltMessage> {
    let header_banner = Some(welcome_banner(repo_root, "navigation.png", warnings));
    let categories = navigation_category_messages(model, repo_root, texts, warnings);
    let mut first = NavigationChunk::new();
    first.push_header(
        header_banner.as_ref(),
        &texts.section_titles.navigation,
        &texts.navigation_intro,
    );

    if categories.is_empty() {
        warnings.push("Welcome-Navigation enthaelt keine oeffentlichen Kanaele".to_string());
        first.push_empty_navigation(&texts.empty_navigation);
        return vec![WelcomeBuiltMessage {
            banner: header_banner,
            payload: welcome_payload(first.components, first.attachments),
        }];
    }

    let mut chunks = vec![first];
    for category in categories {
        if !chunks.last().expect("nav chunk").can_add(&category) {
            chunks.push(NavigationChunk::new());
        }
        if !chunks.last().expect("nav chunk").can_add(&category)
            && chunks.last().expect("nav chunk").category_count == 0
        {
            let mut truncated = category.clone();
            let list_budget = NAVIGATION_TEXT_CHAR_BUDGET.saturating_sub(truncated.title_chars());
            truncated.channel_list = truncate_to_char_budget(&truncated.channel_list, list_budget);
            chunks
                .last_mut()
                .expect("nav chunk")
                .push_category(&truncated);
            continue;
        }
        chunks
            .last_mut()
            .expect("nav chunk")
            .push_category(&category);
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut message = chunk.into_message();
            if index == 0 {
                message.banner = header_banner.clone();
            }
            message
        })
        .collect()
}

fn navigation_category_messages(
    model: &GuildModel,
    repo_root: &Path,
    texts: &ResolvedWelcomeTextTable,
    warnings: &mut Vec<String>,
) -> Vec<NavigationCategoryMessage> {
    let mut messages = Vec::new();
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

        let channel_list = channels
            .into_iter()
            .map(|channel| {
                format!(
                    "<#{}>\n{}",
                    channel.channel_id,
                    channel_description(texts, &channel.name)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let channel_list = truncate_to_char_budget(&channel_list, NAVIGATION_TEXT_CHAR_BUDGET);

        let (divider_filename, divider_relative_path) =
            divider_attachment_for_category(repo_root, &category.name, warnings)
                .map_or((None, None), |attachment| {
                    (Some(attachment.filename), Some(attachment.relative_path))
                });
        messages.push(NavigationCategoryMessage {
            category_name: category.name.clone(),
            divider_filename,
            divider_relative_path,
            channel_list,
        });
    }
    messages
}

fn section_banner(
    repo_root: &Path,
    definition: &WelcomeSectionDefinition,
    warnings: &mut Vec<String>,
) -> Option<WelcomeBannerOutput> {
    definition
        .banner_filename
        .map(|filename| welcome_banner(repo_root, filename, warnings))
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
    Some(WelcomePayloadAttachment {
        id: 0,
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
            WELCOME_SUPPORT_TICKET_CHANNEL_ID,
        ),
    ])
}

fn quickstart_components(primary_buttons: Vec<Value>, hero_jump_url: Option<&str>) -> Vec<Value> {
    let mut buttons = primary_buttons;
    if let Some(url) = hero_jump_url {
        buttons.push(link_button(WELCOME_QUICKSTART_JUMP_LABEL, url));
    }
    vec![button_row(buttons)]
}

fn welcome_payload(
    components: Vec<Value>,
    attachments: Vec<WelcomePayloadAttachment>,
) -> WelcomeMessagePayload {
    WelcomeMessagePayload {
        flags: WELCOME_COMPONENTS_V2_FLAG,
        allowed_mentions: WelcomeAllowedMentions { parse: Vec::new() },
        components,
        attachments,
    }
}

fn push_media_gallery_for_banner(
    components: &mut Vec<Value>,
    attachments: &mut Vec<WelcomePayloadAttachment>,
    banner: Option<&WelcomeBannerOutput>,
) {
    let Some(banner) = banner.filter(|banner| banner.present) else {
        return;
    };
    push_media_gallery_for_attachment(
        components,
        attachments,
        &banner.filename,
        &banner.relative_path,
    );
}

fn push_media_gallery_for_attachment(
    components: &mut Vec<Value>,
    attachments: &mut Vec<WelcomePayloadAttachment>,
    filename: &str,
    relative_path: &str,
) {
    let id = u8::try_from(attachments.len()).unwrap_or(u8::MAX);
    attachments.push(WelcomePayloadAttachment {
        id,
        filename: filename.to_string(),
        relative_path: relative_path.to_string(),
    });
    components.push(media_gallery(filename));
}

fn container(components: Vec<Value>) -> Value {
    json!({
        "type": 17,
        "accent_color": WELCOME_ACCENT_GOLD,
        "components": components,
    })
}

fn text_display(content: String) -> Value {
    json!({
        "type": 10,
        "content": content,
    })
}

fn media_gallery(filename: &str) -> Value {
    json!({
        "type": 12,
        "items": [{
            "media": {
                "url": format!("attachment://{filename}"),
            },
        }],
    })
}

fn separator() -> Value {
    json!({
        "type": 14,
        "divider": true,
        "spacing": 1,
    })
}

#[cfg(test)]
fn welcome_payload_component_count(payload: &WelcomeMessagePayload) -> usize {
    payload.components.iter().map(count_component).sum()
}

fn count_component(component: &Value) -> usize {
    1 + component
        .get("components")
        .and_then(Value::as_array)
        .map(|components| components.iter().map(count_component).sum::<usize>())
        .unwrap_or(0)
}

#[cfg(test)]
fn welcome_payload_text_display_chars(payload: &WelcomeMessagePayload) -> usize {
    payload
        .components
        .iter()
        .map(text_display_chars_in_component)
        .sum()
}

#[cfg(test)]
fn text_display_chars_in_component(component: &Value) -> usize {
    let own_chars = if component.get("type").and_then(Value::as_u64) == Some(10) {
        component
            .get("content")
            .and_then(Value::as_str)
            .map_or(0, |content| content.chars().count())
    } else {
        0
    };
    own_chars
        + component
            .get("components")
            .and_then(Value::as_array)
            .map(|components| {
                components
                    .iter()
                    .map(text_display_chars_in_component)
                    .sum::<usize>()
            })
            .unwrap_or(0)
}

fn truncate_to_char_budget(value: &str, max_chars: usize) -> String {
    let char_count = value.chars().count();
    if char_count <= max_chars {
        return value.to_string();
    }
    if max_chars <= 4 {
        return value.chars().take(max_chars).collect();
    }
    value
        .chars()
        .take(max_chars.saturating_sub(4))
        .chain(" ...".chars())
        .collect()
}

fn hero_message_url(guild_id: u64, channel_id: u64, message_id: u64) -> String {
    format!("https://discord.com/channels/{guild_id}/{channel_id}/{message_id}")
}

fn resolve_team_roles(
    model: &GuildModel,
    team_members: &[WelcomeTeamMember],
    warnings: &mut Vec<String>,
) -> Vec<WelcomeTeamRoleOutput> {
    let mut seen_member_ids = BTreeSet::new();
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
            let member_ids = members_for_roles(team_members, &role_ids)
                .into_iter()
                .filter(|member_id| seen_member_ids.insert(*member_id))
                .collect::<Vec<_>>();
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

fn bot_team_output(
    bot_user_id: Option<u64>,
    texts: &ResolvedWelcomeTextTable,
    warnings: &mut Vec<String>,
) -> Option<WelcomeTeamBotOutput> {
    let Some(user_id) = bot_user_id else {
        warnings.push("Bot-User-ID fehlt; Bot-Team-Block wird weggelassen".to_string());
        return None;
    };
    Some(WelcomeTeamBotOutput {
        user_id,
        mention: format!("<@{user_id}>"),
        group_title: texts.team.bot_group_title.clone(),
        description: texts.team.bot_description.clone(),
    })
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
            default_auto_archive_duration: None,
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

    fn build_test_output(
        model: &GuildModel,
        team_members: &[WelcomeTeamMember],
        repo_root: &Path,
        stored_message_ids: &BTreeMap<String, Vec<u64>>,
    ) -> WelcomePublishOutput {
        build_welcome_publish_output(
            model,
            team_members,
            repo_root,
            stored_message_ids,
            None,
            true,
            Some(999),
        )
        .expect("welcome output")
    }

    fn first_section<'a>(
        output: &'a WelcomePublishOutput,
        section_id: &str,
    ) -> &'a WelcomeSectionOutput {
        output
            .sections
            .iter()
            .find(|section| section.section_id == section_id)
            .expect(section_id)
    }

    fn first_action_row_buttons(payload: &WelcomeMessagePayload) -> Vec<Value> {
        let container_components = payload.components[0]["components"]
            .as_array()
            .expect("container components");
        let row = container_components
            .iter()
            .find(|component| component.get("type").and_then(Value::as_u64) == Some(1))
            .expect("action row");
        row["components"].as_array().expect("buttons").clone()
    }

    #[test]
    fn welcome_texts_seed_toml_parst_zu_compile_defaults() {
        let raw =
            fs::read_to_string(welcome_repo_root().join(WELCOME_TEXTS_FILE)).expect("seed toml");
        let parsed = toml::from_str::<WelcomeTextsToml>(&raw).expect("seed parses");
        let channels = parsed.channel.expect("seed channels");
        assert!(
            !channels.is_empty(),
            "Seed enthaelt keine channel-Eintraege"
        );
        for channel in channels {
            assert!(!channel.key.trim().is_empty(), "channel key leer");
            assert!(
                !channel.description.trim().is_empty(),
                "channel description leer fuer {}",
                channel.key
            );
        }

        let mut warnings = Vec::new();
        let config = load_welcome_runtime_config(&welcome_repo_root(), &mut warnings)
            .expect("welcome texts seed");

        assert!(warnings.is_empty(), "unerwartete Warnungen: {warnings:?}");
        assert_eq!(
            config.texts.team.bot_group_title,
            WELCOME_TEXTS.team.bot_group_title
        );
        assert_eq!(
            config.texts.team.bot_description,
            WELCOME_TEXTS.team.bot_description
        );
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

[team]
bot_group_title = "Bot-Gruppe"

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
        assert_eq!(config.texts.team.bot_group_title, "Bot-Gruppe");
        assert_eq!(
            config.texts.team.bot_description,
            WELCOME_TEXTS.team.bot_description
        );
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

        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(&model, &[], temp.path(), &BTreeMap::new());
        let text = serde_json::to_string(
            &output
                .sections
                .iter()
                .filter(|section| section.section_id == "navigation")
                .map(|section| &section.payload.components)
                .collect::<Vec<_>>(),
        )
        .expect("components json");

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
        let output = build_test_output(&model, &[], temp.path(), &BTreeMap::new());

        assert!(output
            .warnings
            .iter()
            .any(|warning| warning.contains("assets/welcome-banners/hero.png")));
        let hero = first_section(&output, "hero");
        assert_eq!(
            hero.banner.as_ref().map(|banner| banner.present),
            Some(false)
        );
        assert!(hero.payload.attachments.is_empty());
        assert_eq!(hero.payload.components[0]["components"][0]["type"], 10);
    }

    #[test]
    fn welcome_payloads_sind_components_v2_ohne_mentions_und_embeds() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(&model, &[], temp.path(), &BTreeMap::new());

        for section in &output.sections {
            let payload = serde_json::to_value(&section.payload).expect("payload json");
            assert_eq!(payload["flags"], WELCOME_COMPONENTS_V2_FLAG);
            assert!(payload.get("content").is_none());
            assert!(payload.get("embeds").is_none());
            assert_eq!(
                payload["allowed_mentions"]["parse"]
                    .as_array()
                    .expect("parse")
                    .len(),
                0
            );
            assert!(!payload["components"]
                .as_array()
                .expect("components")
                .is_empty());
        }
    }

    #[test]
    fn welcome_banner_steht_vor_text_im_container() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let banner_dir = temp.path().join(WELCOME_BANNER_DIR);
        std::fs::create_dir_all(&banner_dir).expect("banner dir");
        std::fs::write(banner_dir.join("hero.png"), b"png").expect("hero");

        let output = build_test_output(&model, &[], temp.path(), &BTreeMap::new());
        let hero = first_section(&output, "hero");
        let components = hero.payload.components[0]["components"]
            .as_array()
            .expect("container components");

        assert_eq!(components[0]["type"], 12);
        assert_eq!(components[1]["type"], 10);
        assert_eq!(hero.payload.attachments[0].filename.as_str(), "hero.png");
    }

    #[test]
    fn welcome_navigation_divider_nutzen_media_gallery_attachment_urls() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let banner_dir = temp.path().join(WELCOME_BANNER_DIR);
        std::fs::create_dir_all(&banner_dir).expect("banner dir");
        for filename in ["divider-information.png", "divider-community.png"] {
            std::fs::write(banner_dir.join(filename), b"png").expect("divider");
        }

        let output = build_test_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::from([("hero".to_string(), vec![7001])]),
        );
        let navigation = first_section(&output, "navigation");

        assert_eq!(
            navigation
                .payload
                .attachments
                .iter()
                .map(|attachment| (attachment.id, attachment.filename.as_str()))
                .collect::<Vec<_>>(),
            vec![(0, "divider-information.png"), (1, "divider-community.png")]
        );
        let text = serde_json::to_string(&navigation.payload.components).expect("components json");
        assert!(text.contains("attachment://divider-information.png"));
        assert!(text.contains("attachment://divider-community.png"));
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

        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(&model, &members, temp.path(), &BTreeMap::new());

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
    fn welcome_team_dedupliziert_mitglieder_in_prioritaetsreihenfolge() {
        let mut model = base_model();
        model
            .roles
            .insert(30, role(30, "Moderator", Permissions::empty(), 30));
        model
            .roles
            .insert(31, role(31, "Coach", Permissions::empty(), 20));
        let members = vec![WelcomeTeamMember {
            user_id: 100,
            role_ids: vec![30, 31],
            bot: false,
        }];
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(&model, &members, temp.path(), &BTreeMap::new());

        let moderator = output
            .team_roles
            .iter()
            .find(|role| role.key == "moderator")
            .expect("moderator");
        let coach = output
            .team_roles
            .iter()
            .find(|role| role.key == "coach")
            .expect("coach");

        assert_eq!(moderator.member_mentions, vec!["<@100>"]);
        assert!(coach.member_mentions.is_empty());
    }

    #[test]
    fn welcome_bot_team_block_ist_optional_und_in_payload() {
        let mut model = base_model();
        model
            .roles
            .insert(40, role(40, "Owner", Permissions::empty(), 40));
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(&model, &[], temp.path(), &BTreeMap::new());

        assert_eq!(output.bot_team.as_ref().map(|bot| bot.user_id), Some(999));
        let team = first_section(&output, "team");
        let text = serde_json::to_string(&team.payload.components).expect("team json");
        assert!(text.contains("### Owner"));
        // Gruppen ohne existierende Live-Rolle werden nicht gerendert
        assert!(!text.contains("### Moderator"));
        assert!(text.contains("### 🤖 Server-Management"));
        assert!(text.contains("<@999> — unser Bot: verwaltet Rollen, Voice-Lanes, Onboarding, Coaching und diesen Hub."));

        let without_bot = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::new(),
            None,
            true,
            None,
        )
        .expect("welcome output");
        assert!(without_bot.bot_team.is_none());
        assert!(without_bot
            .warnings
            .iter()
            .any(|warning| warning.contains("Bot-User-ID fehlt")));
    }

    #[test]
    fn welcome_quickstart_jump_button_nutzt_hero_message_id() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::from([("hero".to_string(), vec![7001])]),
        );
        let quickstart = first_section(&output, "quickstart");
        let buttons = first_action_row_buttons(&quickstart.payload);
        assert_eq!(buttons.len(), 4);
        assert_eq!(buttons[3]["label"], WELCOME_QUICKSTART_JUMP_LABEL);
        assert_eq!(buttons[3]["url"], "https://discord.com/channels/1/20/7001");

        let mut without_hero = build_test_output(&model, &[], temp.path(), &BTreeMap::new());
        assert!(without_hero
            .warnings
            .iter()
            .any(|warning| warning == WELCOME_QUICKSTART_JUMP_WARNING));
        refresh_quickstart_jump_button(&mut without_hero, Some(7002));
        let quickstart = first_section(&without_hero, "quickstart");
        let buttons = first_action_row_buttons(&quickstart.payload);
        assert_eq!(buttons[3]["url"], "https://discord.com/channels/1/20/7002");
        assert!(!without_hero
            .warnings
            .iter()
            .any(|warning| warning == WELCOME_QUICKSTART_JUMP_WARNING));
    }

    #[test]
    fn welcome_navigation_chunking_haelt_budgetgrenzen() {
        let mut model = base_model();
        for idx in 0..30_u64 {
            let category_id = 1000 + idx;
            let channel_id = 2000 + idx;
            model.categories.insert(
                category_id,
                category(category_id, &format!("Public {idx}"), 10 + idx as i32),
            );
            model.channels.insert(
                channel_id,
                channel(channel_id, &format!("kanal-{idx}"), category_id, 1),
            );
        }
        let temp = tempfile::tempdir().expect("tempdir");
        let output = build_test_output(&model, &[], temp.path(), &BTreeMap::new());

        let nav_messages = output
            .sections
            .iter()
            .filter(|section| section.section_id == "navigation")
            .collect::<Vec<_>>();
        assert!(nav_messages.len() > 1);
        for section in nav_messages {
            assert!(
                welcome_payload_text_display_chars(&section.payload) <= NAVIGATION_TEXT_CHAR_BUDGET
            );
            assert!(
                welcome_payload_component_count(&section.payload) <= NAVIGATION_COMPONENT_BUDGET
            );
            assert!(section.payload.attachments.len() <= NAVIGATION_ATTACHMENT_BUDGET);
        }
    }

    #[test]
    fn welcome_storage_format_und_messageanzahl_steuern_repost() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let initial = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::new(),
            None,
            true,
            Some(999),
        )
        .expect("initial");
        assert!(initial.repost_required);

        let mut exact_ids = BTreeMap::<String, Vec<u64>>::new();
        for section in &initial.sections {
            exact_ids
                .entry(section.section_id.clone())
                .or_default()
                .push(7000 + section.message_index as u64);
        }
        let patchable = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &exact_ids,
            Some(WELCOME_PAYLOAD_FORMAT),
            true,
            Some(999),
        )
        .expect("patchable");
        assert!(!patchable.repost_required);

        let wrong_format = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &exact_ids,
            Some("1"),
            true,
            Some(999),
        )
        .expect("wrong format");
        assert!(wrong_format.repost_required);

        let mut missing_nav_message = exact_ids.clone();
        missing_nav_message
            .get_mut("navigation")
            .expect("navigation ids")
            .pop();
        let count_changed = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &missing_nav_message,
            Some(WELCOME_PAYLOAD_FORMAT),
            true,
            Some(999),
        )
        .expect("count changed");
        assert!(count_changed.repost_required);
    }

    #[test]
    fn welcome_repost_refresh_nutzt_neue_hero_id_fuer_quickstart() {
        let model = base_model();
        let temp = tempfile::tempdir().expect("tempdir");
        let mut output = build_welcome_publish_output(
            &model,
            &[],
            temp.path(),
            &BTreeMap::from([("hero".to_string(), vec![7001])]),
            Some("1"),
            true,
            Some(999),
        )
        .expect("welcome output");
        assert!(output.repost_required);

        refresh_quickstart_jump_button(&mut output, Some(8001));
        let quickstart = first_section(&output, "quickstart");
        let buttons = first_action_row_buttons(&quickstart.payload);
        assert_eq!(buttons[3]["url"], "https://discord.com/channels/1/20/8001");
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
