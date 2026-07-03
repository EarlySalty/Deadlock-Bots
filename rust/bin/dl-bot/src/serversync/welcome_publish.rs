use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dl_server_as_code::{ChannelKind, ChannelSpec, GuildModel, OverwriteKey, RoleSpec, TargetKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use serenity::all::Permissions;

pub const WELCOME_CHANNEL_NAME: &str = "willkommen";
pub const WELCOME_BANNER_DIR: &str = "assets/welcome-banners";
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
            channel_key: "twitch",
            description: "Live-Alerts und News von unserem Twitch-Kanal.",
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
        aliases: &["Moderator", "Community Moderator"],
    },
    WelcomeTeamRoleGroupSpec {
        key: "coach",
        aliases: &["Coach"],
    },
];

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
}

pub fn welcome_repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

pub fn welcome_sections() -> &'static [WelcomeSectionDefinition] {
    WELCOME_SECTION_DEFINITIONS
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
    let team_roles = resolve_team_roles(model, team_members, &mut warnings);
    let navigation_embeds = navigation_embeds(model, &mut warnings);
    let quickstart_buttons = quickstart_buttons(model)?;

    let mut sections = Vec::new();
    for definition in WELCOME_SECTION_DEFINITIONS {
        let marker = welcome_marker(definition.id);
        let banner = definition
            .banner_filename
            .map(|filename| welcome_banner(repo_root, filename, &mut warnings));
        let payload = match definition.id {
            "hero" => payload_with_optional_banner(
                WELCOME_TEXTS.hero_intro,
                WELCOME_TEXTS.section_titles.hero,
                &marker,
                banner.as_ref(),
                Vec::new(),
            ),
            "navigation" => WelcomeMessagePayload {
                content: String::new(),
                embeds: navigation_embeds.clone(),
                components: Vec::new(),
                attachments: Vec::new(),
            },
            "team" => team_payload(&team_roles, &marker, banner.as_ref()),
            "socials" => payload_with_optional_banner(
                WELCOME_TEXTS.socials_intro,
                WELCOME_TEXTS.section_titles.socials,
                &marker,
                banner.as_ref(),
                vec![button_row(vec![
                    link_button(WELCOME_TEXTS.buttons.website, WELCOME_LINK_URLS.website),
                    link_button(WELCOME_TEXTS.buttons.twitch, WELCOME_LINK_URLS.twitch),
                    link_button(WELCOME_TEXTS.buttons.coaching, WELCOME_LINK_URLS.coaching),
                    link_button(
                        WELCOME_TEXTS.buttons.server_invite,
                        WELCOME_LINK_URLS.server_invite,
                    ),
                ])],
            ),
            "quickstart" => WelcomeMessagePayload {
                content: WELCOME_TEXTS.quickstart_intro.to_string(),
                embeds: vec![text_embed(
                    WELCOME_TEXTS.section_titles.quickstart,
                    None,
                    &marker,
                    None,
                )],
                components: vec![button_row(quickstart_buttons.clone())],
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
    marker: &str,
    banner: Option<&WelcomeBannerOutput>,
) -> WelcomeMessagePayload {
    let fields = team_roles
        .iter()
        .filter_map(|role| {
            let name = role.matched_role_name.as_ref()?;
            let value = if role.member_mentions.is_empty() {
                WELCOME_TEXTS.empty_team_role_members.to_string()
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
        .then(|| WELCOME_TEXTS.empty_team_role_members.to_string());
    let mut embed = embed_base(
        WELCOME_TEXTS.section_titles.team,
        description.as_deref(),
        marker,
    );
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

fn navigation_embeds(model: &GuildModel, warnings: &mut Vec<String>) -> Vec<Value> {
    let mut embeds = Vec::new();
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
                    "value": channel_description(&channel.name),
                    "inline": false,
                })
            })
            .collect::<Vec<_>>();
        let marker = if embeds.is_empty() {
            welcome_marker("navigation")
        } else {
            String::new()
        };
        let mut embed = embed_base(
            &category.name,
            None,
            if marker.is_empty() {
                WELCOME_TEXTS.section_titles.navigation
            } else {
                &marker
            },
        );
        if marker.is_empty() {
            embed["footer"] = json!({ "text": WELCOME_TEXTS.section_titles.navigation });
        }
        embed["fields"] = Value::Array(fields);
        embeds.push(embed);
    }

    if embeds.is_empty() {
        warnings.push("Welcome-Navigation enthaelt keine oeffentlichen Kanaele".to_string());
        return vec![text_embed(
            WELCOME_TEXTS.section_titles.navigation,
            Some(WELCOME_TEXTS.empty_navigation),
            &welcome_marker("navigation"),
            None,
        )];
    }
    embeds
}

fn quickstart_buttons(model: &GuildModel) -> Result<Vec<Value>, String> {
    Ok(vec![
        channel_link_button(
            WELCOME_TEXTS.buttons.rules,
            model.guild_id,
            require_channel_id(model, "regelwerk")?,
        ),
        channel_link_button(
            WELCOME_TEXTS.buttons.rank,
            model.guild_id,
            require_channel_id(model, "deadlock-rang")?,
        ),
        channel_link_button(
            WELCOME_TEXTS.buttons.support,
            model.guild_id,
            require_channel_id(model, "server-support")?,
        ),
    ])
}

fn resolve_team_roles(
    model: &GuildModel,
    team_members: &[WelcomeTeamMember],
    warnings: &mut Vec<String>,
) -> Vec<WelcomeTeamRoleOutput> {
    WELCOME_TEAM_ROLE_GROUPS
        .iter()
        .map(|group| {
            let matched = resolve_role(model, group.aliases);
            if matched.is_none() {
                warnings.push(format!(
                    "Team-Rolle `{}` wurde im Live-Guild-Modell nicht gefunden",
                    group.key
                ));
            }
            let member_ids = matched
                .map(|role| members_for_role(team_members, role.role_id))
                .unwrap_or_default();
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
                matched_role_id: matched.map(|role| role.role_id),
                matched_role_name: matched.map(|role| role.name.clone()),
                member_ids,
                member_mentions,
            }
        })
        .collect()
}

fn resolve_role<'a>(model: &'a GuildModel, aliases: &[&str]) -> Option<&'a RoleSpec> {
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
    candidates.first().copied()
}

fn members_for_role(team_members: &[WelcomeTeamMember], role_id: u64) -> Vec<u64> {
    let mut member_ids = team_members
        .iter()
        .filter(|member| !member.bot && member.role_ids.contains(&role_id))
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

fn channel_description(channel_name: &str) -> &'static str {
    let normalized = normalized_name(channel_name);
    WELCOME_TEXTS
        .channel_descriptions
        .iter()
        .find(|entry| normalized_name(entry.channel_key) == normalized)
        .map_or(WELCOME_TEXTS.default_channel_description, |entry| {
            entry.description
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

fn embed_base(title: &str, description: Option<&str>, marker: &str) -> Value {
    let mut embed = json!({
        "title": title,
        "footer": { "text": marker },
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
    fn welcome_idempotenz_priorisiert_storage_und_nutzt_marker_fallback() {
        assert_eq!(
            welcome_candidate_message_ids(Some(10), Some(11)),
            vec![10, 11]
        );
        assert_eq!(welcome_candidate_message_ids(None, Some(11)), vec![11]);
        assert_eq!(welcome_candidate_message_ids(Some(10), Some(10)), vec![10]);
    }

    #[test]
    fn welcome_team_rollen_werden_in_hierarchie_aufgeloest() {
        let mut model = base_model();
        model.roles.insert(
            30,
            role(30, "Community Moderator", Permissions::empty(), 30),
        );
        model
            .roles
            .insert(31, role(31, "Coach", Permissions::empty(), 10));
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
        assert_eq!(
            moderator.matched_role_name.as_deref(),
            Some("Community Moderator")
        );
        assert_eq!(moderator.member_mentions, vec!["<@100>"]);
        let coach = output
            .team_roles
            .iter()
            .find(|role| role.key == "coach")
            .expect("coach");
        assert_eq!(coach.member_mentions, vec!["<@101>"]);
    }
}
