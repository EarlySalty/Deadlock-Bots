use std::collections::{BTreeMap, BTreeSet};

use serenity::all::Permissions;

use crate::model::{
    ChannelKind, DiscordId, DocumentedException, DynamicNamespace, GuildModel, NamespaceMatch,
    ObjectKind, OverwriteKey, PermissionOverwriteSpec, RoleSpec, TargetKind,
};
use crate::{Result, ServerAsCodeError};

const ROLE_COMMUNITY_MODERATOR: &str = "Community Moderator";
const ROLE_DEADLOCK_PATCHNOTES: &str = "Deadlock Patchnotes";
const ROLE_DL_RANG: &str = "DL-Rang";
const ROLE_TICKET_TOOL: &str = "Ticket Tool";
const ROLE_TURNIER_CASTER: &str = "Turnier Caster";
const ROLE_TURNIER_MODERATION: &str = "Turnier Moderation";
const ROLE_COACH: &str = "Coach";
const ROLE_VIP: &str = "VIP";
const ROLE_SERVER_BOOSTER: &str = "Server Booster";
const ROLE_SERVER_UNTERSTUETZER: &str = "Server Unterstützer";
const ROLE_STREAMER: &str = "Streamer";
const ROLE_STREAMER_VC_ZUGRIFF: &str = "Streamer VC Zugriff";
const ROLE_VC_MOVE_RECHTE: &str = "VC Move Rechte";
const ROLE_COACHING_FEEDBACK: &str = "Coaching Feedback";
const ROLE_INVITE_GAST: &str = "Invite-Gast";
const ROLE_FRISCHLING: &str = "Frischling";
const ROLE_STREAMS: &str = "Streams";
const CATEGORY_MODERATION: &str = "🛡️ ─ MODERATION ─";
const CATEGORY_INFORMATION: &str = "🏛️ ─ INFORMATION ─";
const CATEGORY_MEDIEN: &str = "📺 ─ MEDIEN ─";
const CATEGORY_COMMUNITY: &str = "💬 ─ COMMUNITY ─";
const CATEGORY_DEADLOCK: &str = "🎮 ─ DEADLOCK ─";
const CATEGORY_COACHING: &str = "🎓 ─ COACHING ─";
const CATEGORY_VIP: &str = "💎 ─ VIP ─";
const CATEGORY_STREAMER: &str = "🎥 ─ STREAMER ─";
const CATEGORY_CHILL: &str = "Chill";
const CATEGORY_RANKED: &str = "Ranked";
const CATEGORY_NEUE_SPIELER: &str = "Neue Spieler";
const CATEGORY_SUPPORT: &str = "🎟️ ─ SUPPORT ─";
const CATEGORY_ARCHIV: &str = "📦 ─ ARCHIV ─";
const CHANNEL_KREATIV_ECKE: &str = "kreativ-ecke";
const CHANNEL_LFG_FORUM: &str = "🎯mitspieler-suche";
const CHANNEL_LFG_ARCHIVE: &str = "archiv-mitspieler-suche";
const CHANNEL_WILLKOMMEN: &str = "willkommen";
const CHANNEL_SCRIM_PLANUNG: &str = "scrim-planung";
const TOPIC_WILLKOMMEN: &str =
    "Dein Startpunkt: Was es hier gibt, wer dahinter steckt und wie du loslegst.";
const TOPIC_KREATIV_ECKE: &str =
    "Alles Selbstgemachte rund um Deadlock: Art, Edits, Mods, Movement-Clips.";
const TOPIC_SCRIM_PLANUNG: &str = "Scrim-Termine und Team-Aufstellungen — pro Scrim ein Thread.";
const LFG_FORUM_DEFAULT_AUTO_ARCHIVE_DURATION: i32 = 1440;
const LFG_CHANNEL_ALIASES: &[&str] = &["spieler-suche", "mitspieler-suche"];
const LFG_FORUM_CUTOVER_ENV: &str = "DL_LFG_FORUM_CUTOVER";

const WELLE2B_MARKER_ROLES: &[&str] = &[ROLE_INVITE_GAST, ROLE_FRISCHLING];
const WELLE2B_PING_ROLE_TEMPLATE_CANDIDATES: &[&str] = &[
    "Patchnotes Ping Rolle",
    "Spieler-Suche Ping Rolle",
    "Events & Turniere Ping Rolle",
    "Events und Turniere Ping Rolle",
    "Custom Games Ping Rolle",
    "Patchnotes",
    "Spielersuche",
    "Events & Turniere",
    "Custom Games",
];

const USER_BAN_X3_COACHING_VIEW_ONLY: DiscordId = 364_796_363_709_349_912;

const EXPECTED_CATEGORIES: &[&str] = &[
    CATEGORY_MODERATION,
    CATEGORY_INFORMATION,
    CATEGORY_MEDIEN,
    CATEGORY_COMMUNITY,
    CATEGORY_DEADLOCK,
    CATEGORY_COACHING,
    CATEGORY_VIP,
    CATEGORY_STREAMER,
    CATEGORY_CHILL,
    "Deadlock Router",
    "Street Brawl",
    CATEGORY_RANKED,
    CATEGORY_NEUE_SPIELER,
    "Custom Game",
    "AFK",
    CATEGORY_SUPPORT,
    CATEGORY_ARCHIV,
];

const USER_BAN_ONE_TO_ONE_EXCEPTION_IDS: &[DiscordId] = &[
    685_573_558_281_175_043,
    496_268_533_496_545_283,
    601_742_833_438_818_357,
];

const DOCUMENTED_STRUCTURE_MOVES: &[(&str, &str)] = &[
    ("regelwerk", CATEGORY_INFORMATION),
    ("willkommen", CATEGORY_INFORMATION),
    ("deadlock-rang", CATEGORY_INFORMATION),
    ("server-support", CATEGORY_INFORMATION),
    ("ankündigungen", CATEGORY_INFORMATION),
    ("patchnotes", CATEGORY_INFORMATION),
    ("dev-updates", CATEGORY_INFORMATION),
    ("streamer-updates", CATEGORY_MEDIEN),
    ("twitch", CATEGORY_INFORMATION),
    ("deadlock-streamer", CATEGORY_MEDIEN),
    ("guides-und-tipps", CATEGORY_INFORMATION),
    ("game-guides-und-tipps", CATEGORY_INFORMATION),
    ("allgemein", CATEGORY_COMMUNITY),
    ("off-topic", CATEGORY_COMMUNITY),
    ("deadlock-invite", CATEGORY_COMMUNITY),
    ("frag-die-community", CATEGORY_COMMUNITY),
    ("memes", CATEGORY_COMMUNITY),
    ("leaks", CATEGORY_COMMUNITY),
    ("kreativ-ecke", CATEGORY_COMMUNITY),
    ("gameplay-clips", CATEGORY_MEDIEN),
    ("yt-videos", CATEGORY_MEDIEN),
    ("feedback", CATEGORY_COMMUNITY),
    ("bot-spam", CATEGORY_COMMUNITY),
    ("rage-room", CATEGORY_COMMUNITY),
    ("nsfw", CATEGORY_COMMUNITY),
    ("mitspieler-suche", CATEGORY_DEADLOCK),
    ("custom-games", CATEGORY_DEADLOCK),
    ("rank-ups", CATEGORY_DEADLOCK),
    ("scrim-planung", CATEGORY_COACHING),
];

const WELLE2B_ARCHIVE_CHANNEL_NAMES: &[&str] =
    &["server-faq", "movement", "deadlock-art", "mods", "food"];
const WELLE2B_KREATIV_SOURCE_CHANNEL_NAMES: &[&str] = &["movement", "deadlock-art", "mods", "food"];
const WELLE3_ARCHIVE_CHANNEL_NAMES: &[&str] = &["sprach-kanal-verwalten", "anleitung"];

// Matching-only Alias-Tabelle fuer Live-Namen, die nicht 1:1 aus Dekoration/Case
// ableitbar sind. Keine dieser Aliases schreibt Namen ins Soll-Modell.
// Live-Dry-Run 2026-07-02: alte Kategorienamen matchen weiter auf dieselbe ID;
// W3.1 schreibt danach nur den Trennernamen ins Soll-Modell.
const CATEGORY_MATCH_ALIASES: &[(&str, &str)] = &[
    ("moderation", CATEGORY_MODERATION),
    ("eingangsbereich", CATEGORY_INFORMATION),
    ("information", CATEGORY_INFORMATION),
    ("medien", CATEGORY_MEDIEN),
    ("neuigkeiten", CATEGORY_MEDIEN),
    ("chat", CATEGORY_COMMUNITY),
    ("community", CATEGORY_COMMUNITY),
    ("sonstiges", CATEGORY_DEADLOCK),
    ("deadlock", CATEGORY_DEADLOCK),
    ("coaching", CATEGORY_COACHING),
    ("vip", CATEGORY_VIP),
    ("streamer", CATEGORY_STREAMER),
    ("streamer only", CATEGORY_STREAMER),
    ("chill lanes", CATEGORY_CHILL),
    ("chill", CATEGORY_CHILL),
    ("competitiv lanes", CATEGORY_RANKED),
    ("competitive lanes", CATEGORY_RANKED),
    ("ranked", CATEGORY_RANKED),
    ("neue spieler lanes", CATEGORY_NEUE_SPIELER),
    ("neue spieler", CATEGORY_NEUE_SPIELER),
    ("support/tickets", CATEGORY_SUPPORT),
    ("support", CATEGORY_SUPPORT),
    ("archiv", CATEGORY_ARCHIV),
];

const FUNCTIONAL_EXCEPTION_USER_IDS: &[DiscordId] = &[
    271_549_384_787_755_008,
    702_594_328_328_929_331,
    335_030_285_240_631_296,
];

const FUNCTIONAL_DELETE_USER_IDS: &[DiscordId] = &[
    557_628_352_828_014_614,
    1_355_078_189_894_078_597,
    246_716_116_498_513_920,
    247_139_694_398_144_513,
    936_257_113_720_758_402,
    698_246_721_003_585_566,
    279_971_744_964_542_464,
    886_533_600_189_755_392,
    1_207_015_550_409_121_855,
];

const TEAM_CAPTAIN_USER_IDS: &[DiscordId] = &[193_685_907_071_696_896, 503_957_305_164_038_156];
const COACH_CHAT_USER_ID: DiscordId = 318_453_395_335_675_904;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesiredDerivation {
    pub desired: GuildModel,
    pub dynamic_namespaces: Vec<DynamicNamespace>,
    pub exceptions: Vec<DocumentedException>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesiredModelOptions {
    pub welle2b_archive_enabled: bool,
    pub lfg_forum_cutover_enabled: bool,
}

impl Default for DesiredModelOptions {
    fn default() -> Self {
        Self {
            welle2b_archive_enabled: false,
            lfg_forum_cutover_enabled: env_flag_enabled(LFG_FORUM_CUTOVER_ENV),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionOverwriteProfile {
    pub allow_bits: u64,
    pub deny_bits: u64,
}

impl PermissionOverwriteProfile {
    fn new(allow: Permissions, deny: Permissions) -> Self {
        Self {
            allow_bits: allow.bits(),
            deny_bits: deny.bits(),
        }
    }
}

/// §2 + Owner-Entscheidung 6.1: Guild-Level-Basis fuer @everyone ohne TTS und ohne private Threads.
pub fn everyone_basis_bits() -> u64 {
    (Permissions::VIEW_CHANNEL
        | Permissions::SEND_MESSAGES
        | Permissions::READ_MESSAGE_HISTORY
        | Permissions::ADD_REACTIONS
        | Permissions::EMBED_LINKS
        | Permissions::ATTACH_FILES
        | Permissions::USE_EXTERNAL_EMOJIS
        | Permissions::USE_EXTERNAL_STICKERS
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::SEND_MESSAGES_IN_THREADS
        | Permissions::SEND_POLLS
        | Permissions::SEND_VOICE_MESSAGES
        | Permissions::CONNECT
        | Permissions::SPEAK
        | Permissions::STREAM
        | Permissions::USE_VAD
        | Permissions::SET_VOICE_CHANNEL_STATUS
        | Permissions::USE_EMBEDDED_ACTIVITIES
        | Permissions::REQUEST_TO_SPEAK
        | Permissions::CHANGE_NICKNAME
        | Permissions::CREATE_INSTANT_INVITE
        | Permissions::USE_APPLICATION_COMMANDS)
        .bits()
}

/// §1/P0: oeffentlich, keine Kanal-Overwrites; Rechte kommen aus der @everyone-Basis.
pub fn p0_public_profile() -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(Permissions::empty(), Permissions::empty())
}

/// §1/P1: Verlautbarung, alle lesen, niemand schreibt oder schreibt in Public Threads.
pub fn p1_announcement_profile() -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(
        Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
        public_write_denies(),
    )
}

/// §1/P2: Panel-Bot darf im ansonsten read-only Kanal senden und Embeds setzen.
pub fn p2_panel_bot_profile() -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(
        Permissions::SEND_MESSAGES | Permissions::EMBED_LINKS,
        Permissions::empty(),
    )
}

/// §1/F: @everyone sieht einen Funktionsbereich nicht.
pub fn f_everyone_hidden_profile() -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(Permissions::empty(), Permissions::VIEW_CHANNEL)
}

/// §1/F: Funktionsrollen sehen den Bereich und duerfen Voice-Kanaele betreten.
pub fn f_role_visibility_profile() -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(
        Permissions::VIEW_CHANNEL | Permissions::CONNECT,
        Permissions::empty(),
    )
}

pub fn derive_desired_model(actual: &GuildModel) -> Result<DesiredDerivation> {
    derive_desired_model_with_options(actual, DesiredModelOptions::default())
}

pub fn derive_desired_model_with_options(
    actual: &GuildModel,
    options: DesiredModelOptions,
) -> Result<DesiredDerivation> {
    let mut ctx = RuleContext::new(actual, options);
    let mut desired = actual.clone();

    apply_documented_renames(&mut desired)?;
    apply_category_names(&mut desired, &mut ctx);
    apply_everyone_basis(&mut desired, &mut ctx);
    apply_welle2b_roles(&mut desired);
    ensure_welle3_static_channels(&mut desired, &mut ctx);
    apply_documented_structure_moves(&mut desired, &mut ctx);
    if options.lfg_forum_cutover_enabled {
        apply_welle34b_lfg_structure_delta(&mut desired, &mut ctx);
    }

    for category_name in EXPECTED_CATEGORIES {
        if ctx.category_id(category_name).is_none() {
            ctx.warn_once(
                "missing_category",
                category_name,
                format!("Kategorie `{category_name}` aus dem Soll-Dokument wurde im Ist-Modell nicht gefunden"),
            );
        }
    }
    for category in actual.categories.values() {
        if !EXPECTED_CATEGORIES
            .iter()
            .any(|expected| category_matches_expected(&category.name, expected))
            && !category_matches_expected(&category.name, CATEGORY_ARCHIV)
            && !is_resolved_empty_category(&category.name)
        {
            ctx.warn_once(
                "unknown_category",
                &category.name,
                format!("Kategorie `{}` ist im Ist vorhanden, aber in §3 nicht dokumentiert; sie bleibt unveraendert", category.name),
            );
        }
    }

    apply_category_if_present(
        &mut desired,
        &mut ctx,
        CATEGORY_MODERATION,
        apply_moderation,
    );
    apply_category_if_present(
        &mut desired,
        &mut ctx,
        CATEGORY_INFORMATION,
        apply_information,
    );
    apply_category_if_present(&mut desired, &mut ctx, CATEGORY_COMMUNITY, apply_community);
    apply_category_if_present(&mut desired, &mut ctx, CATEGORY_DEADLOCK, apply_deadlock);
    apply_category_if_present(&mut desired, &mut ctx, CATEGORY_COACHING, apply_coaching);
    apply_category_if_present(&mut desired, &mut ctx, CATEGORY_VIP, apply_vip);
    apply_category_if_present(&mut desired, &mut ctx, CATEGORY_STREAMER, apply_streamer);
    apply_category_if_present(&mut desired, &mut ctx, CATEGORY_CHILL, apply_chill_lanes);
    apply_category_if_present(
        &mut desired,
        &mut ctx,
        "Deadlock Router",
        apply_deadlock_router,
    );
    apply_category_if_present(&mut desired, &mut ctx, "Street Brawl", apply_street_brawl);
    apply_category_if_present(
        &mut desired,
        &mut ctx,
        CATEGORY_RANKED,
        apply_competitiv_lanes,
    );
    apply_category_if_present(
        &mut desired,
        &mut ctx,
        CATEGORY_NEUE_SPIELER,
        apply_neue_spieler_lanes,
    );
    apply_category_if_present(&mut desired, &mut ctx, "Custom Game", apply_custom_game);
    apply_category_if_present(&mut desired, &mut ctx, "AFK", apply_afk_category);
    apply_category_if_present(
        &mut desired,
        &mut ctx,
        CATEGORY_SUPPORT,
        apply_support_tickets,
    );
    if options.welle2b_archive_enabled {
        apply_category_if_present(&mut desired, &mut ctx, "Alt", apply_alt_category);
        apply_category_if_present(&mut desired, &mut ctx, "Beta Zugang", apply_alt_category);
        apply_welle2b_archive_rules(&mut desired, &mut ctx);
    }
    apply_welle3_archive_rules(&mut desired, &mut ctx);
    apply_global_channel_overrides(&mut desired, &mut ctx);
    remove_empty_resolved_categories(&mut desired, &mut ctx);

    let dynamic_namespaces = build_dynamic_namespaces(&ctx);
    let exceptions = build_documented_exceptions(&desired, &ctx);

    Ok(DesiredDerivation {
        desired,
        dynamic_namespaces,
        exceptions,
        warnings: ctx.warnings,
    })
}

struct RuleContext<'a> {
    actual: &'a GuildModel,
    lfg_forum_cutover_enabled: bool,
    category_ids: BTreeMap<String, DiscordId>,
    role_ids: BTreeMap<String, DiscordId>,
    role_prefix_ids: Vec<(String, DiscordId)>,
    warnings: Vec<String>,
    warning_keys: BTreeSet<String>,
}

impl<'a> RuleContext<'a> {
    fn new(actual: &'a GuildModel, options: DesiredModelOptions) -> Self {
        let mut category_ids = BTreeMap::new();
        let mut role_ids = BTreeMap::new();
        let mut role_prefix_ids = Vec::new();
        let mut warnings = Vec::new();
        let mut warning_keys = BTreeSet::new();

        for category in actual.categories.values() {
            let match_key = category_match_key(&category.name);
            if category_ids
                .insert(match_key.clone(), category.category_id)
                .is_some()
            {
                let key = format!("duplicate_category:{match_key}");
                if warning_keys.insert(key) {
                    warnings.push(format!(
                        "Kategorie `{}` kollidiert beim Matching mehrfach; die letzte ID gewinnt fuer die Regelableitung",
                        category.name
                    ));
                }
            }
        }

        for role in actual.roles.values() {
            if role_ids.insert(role.name.clone(), role.role_id).is_some() {
                let key = format!("duplicate_role:{}", role.name);
                if warning_keys.insert(key) {
                    warnings.push(format!(
                        "Rolle `{}` kommt mehrfach vor; die letzte ID gewinnt fuer die Regelableitung",
                        role.name
                    ));
                }
            }
            role_prefix_ids.push((role.name.clone(), role.role_id));
        }

        Self {
            actual,
            lfg_forum_cutover_enabled: options.lfg_forum_cutover_enabled,
            category_ids,
            role_ids,
            role_prefix_ids,
            warnings,
            warning_keys,
        }
    }

    fn category_id(&self, name: &str) -> Option<DiscordId> {
        self.category_ids.get(&category_match_key(name)).copied()
    }

    fn role_id(&mut self, name: &str) -> Option<DiscordId> {
        if let Some(id) = self.role_ids.get(name).copied() {
            return Some(id);
        }
        self.warn_once(
            "missing_role",
            name,
            format!("Rolle `{name}` fuer Soll-Overwrite wurde im Ist-Modell nicht gefunden"),
        );
        None
    }

    fn role_id_by_prefix(&mut self, prefix: &str) -> Option<DiscordId> {
        if let Some((_, id)) = self
            .role_prefix_ids
            .iter()
            .find(|(name, _)| name.starts_with(prefix))
        {
            return Some(*id);
        }
        self.warn_once(
            "missing_role_prefix",
            prefix,
            format!("Rolle mit Prefix `{prefix}` fuer Soll-Overwrite wurde im Ist-Modell nicht gefunden"),
        );
        None
    }

    fn warn_once(&mut self, group: &str, key: &str, message: String) {
        if self.warning_keys.insert(format!("{group}:{key}")) {
            self.warnings.push(message);
        }
    }
}

fn apply_category_if_present(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    name: &str,
    apply: fn(&mut GuildModel, &mut RuleContext<'_>, DiscordId),
) {
    if let Some(category_id) = ctx.category_id(name) {
        apply(desired, ctx, category_id);
    }
}

fn apply_everyone_basis(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    if let Some(role) = desired.roles.get_mut(&desired.guild_id) {
        role.permissions_bitmask = everyone_basis_bits();
        return;
    }
    let everyone_id = desired
        .roles
        .values()
        .find(|role| role.name == "@everyone")
        .map(|role| role.role_id);
    if let Some(role_id) = everyone_id.and_then(|role_id| desired.roles.get_mut(&role_id)) {
        role_id.permissions_bitmask = everyone_basis_bits();
        return;
    }
    ctx.warn_once(
        "missing_role",
        "@everyone",
        "@everyone-Rolle wurde im Ist-Modell nicht gefunden; Guild-Level-Basisrechte bleiben unveraendert".to_string(),
    );
}

fn apply_welle2b_roles(desired: &mut GuildModel) {
    let mut next_role_id = next_synthetic_role_id(desired);
    let mut next_position = desired
        .roles
        .values()
        .map(|role| role.position)
        .max()
        .unwrap_or(0)
        + 1;

    for role_name in WELLE2B_MARKER_ROLES {
        if desired.roles.values().any(|role| role.name == *role_name) {
            continue;
        }
        let role = RoleSpec {
            guild_id: desired.guild_id,
            role_id: next_role_id,
            name: (*role_name).to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: 0,
            position: next_position,
        };
        desired.roles.insert(role.role_id, role);
        next_role_id += 1;
        next_position += 1;
    }

    if desired.roles.values().any(|role| role.name == ROLE_STREAMS) {
        return;
    }

    let template = ping_role_template(desired);
    let role = RoleSpec {
        guild_id: desired.guild_id,
        role_id: next_role_id,
        name: ROLE_STREAMS.to_string(),
        color: template.map_or(0, |role| role.color),
        hoist: template.is_some_and(|role| role.hoist),
        mentionable: template.is_some_and(|role| role.mentionable),
        managed: false,
        permissions_bitmask: 0,
        position: next_position,
    };
    desired.roles.insert(role.role_id, role);
}

fn ping_role_template(model: &GuildModel) -> Option<&RoleSpec> {
    WELLE2B_PING_ROLE_TEMPLATE_CANDIDATES
        .iter()
        .filter_map(|candidate| role_by_normalized_name(model, candidate))
        .find(|role| !role.managed)
        .or_else(|| {
            model
                .roles
                .values()
                .filter(|role| !role.managed)
                .find(|role| {
                    let key = matching_key(&role.name);
                    key.contains("ping") && key != matching_key(ROLE_STREAMS)
                })
        })
}

fn role_by_normalized_name<'a>(model: &'a GuildModel, name: &str) -> Option<&'a RoleSpec> {
    let expected = matching_key(name);
    model
        .roles
        .values()
        .find(|role| matching_key(&role.name) == expected)
}

fn next_synthetic_role_id(model: &GuildModel) -> DiscordId {
    let mut candidate = model
        .roles
        .keys()
        .copied()
        .max()
        .unwrap_or(model.guild_id)
        .saturating_add(1);
    while model.roles.contains_key(&candidate) {
        candidate = candidate.saturating_add(1);
    }
    candidate
}

fn apply_category_names(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    for category_name in EXPECTED_CATEGORIES {
        if let Some(category_id) = ctx.category_id(category_name) {
            if let Some(category) = desired.categories.get_mut(&category_id) {
                category.name = (*category_name).to_string();
            }
        }
    }
}

fn ensure_welle3_static_channels(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    if let Some(category_id) = ctx.category_id(CATEGORY_INFORMATION) {
        ensure_text_channel(
            desired,
            category_id,
            &[CHANNEL_WILLKOMMEN],
            "🧭willkommen",
            Some(TOPIC_WILLKOMMEN),
        );
    }
    if let Some(category_id) = ctx.category_id(CATEGORY_COMMUNITY) {
        ensure_text_channel(
            desired,
            category_id,
            &[CHANNEL_KREATIV_ECKE],
            "🎨kreativ-ecke",
            Some(TOPIC_KREATIV_ECKE),
        );
    } else {
        ctx.warn_once(
            "missing_static_channel_parent",
            CHANNEL_KREATIV_ECKE,
            format!(
                "Ziel-Kategorie `{CATEGORY_COMMUNITY}` fuer `{CHANNEL_KREATIV_ECKE}` wurde im Ist-Modell nicht gefunden"
            ),
        );
    }
    if let Some(category_id) = ctx.category_id(CATEGORY_COACHING) {
        ensure_text_channel(
            desired,
            category_id,
            &[CHANNEL_SCRIM_PLANUNG],
            "🗓️scrim-planung",
            Some(TOPIC_SCRIM_PLANUNG),
        );
    }
}

fn ensure_text_channel(
    desired: &mut GuildModel,
    parent_category_id: DiscordId,
    aliases: &[&str],
    desired_name: &str,
    topic: Option<&str>,
) -> DiscordId {
    if let Some(channel) = desired
        .channels
        .values_mut()
        .find(|channel| channel_matches(&channel.name, aliases))
    {
        channel.name = desired_name.to_string();
        channel.parent_category_id = Some(parent_category_id);
        channel.topic = topic.map(str::to_string);
        return channel.channel_id;
    }

    let channel_id = next_synthetic_discord_id(desired);
    let position = desired
        .channels
        .values()
        .filter(|channel| channel.parent_category_id == Some(parent_category_id))
        .map(|channel| channel.position)
        .max()
        .unwrap_or(0)
        + 1;
    desired.channels.insert(
        channel_id,
        crate::model::ChannelSpec {
            guild_id: desired.guild_id,
            channel_id,
            name: desired_name.to_string(),
            kind: ChannelKind::Text,
            topic: topic.map(str::to_string),
            position,
            parent_category_id: Some(parent_category_id),
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            default_auto_archive_duration: None,
            status: None,
        },
    );
    channel_id
}

fn apply_documented_renames(desired: &mut GuildModel) -> Result<()> {
    reject_documented_rename_collisions(desired)?;
    for channel in desired.channels.values_mut() {
        let Some(target) = documented_channel_rename(&channel.name) else {
            continue;
        };
        channel.name = target.to_string();
    }
    Ok(())
}

fn reject_documented_rename_collisions(model: &GuildModel) -> Result<()> {
    let mut by_target: BTreeMap<String, Vec<(DiscordId, String)>> = BTreeMap::new();
    for channel in model.channels.values() {
        let Some(target) = documented_channel_rename(&channel.name) else {
            continue;
        };
        by_target
            .entry(matching_key(target))
            .or_default()
            .push((channel.channel_id, channel.name.clone()));
    }

    for (target_key, mut sources) in by_target {
        sources.sort_by_key(|source| source.0);
        if sources.len() <= 1 {
            continue;
        }
        let target = documented_channel_rename(&target_key).unwrap_or(target_key.as_str());
        return Err(ServerAsCodeError::ChannelRenameCollision {
            target: target.to_string(),
            canonical_source: canonical_rename_source(target).to_string(),
            sources: sources
                .into_iter()
                .map(|(id, name)| format!("{id}:{name}"))
                .collect::<Vec<_>>()
                .join(", "),
        });
    }
    Ok(())
}

fn canonical_rename_source(target: &str) -> &str {
    match matching_key(target).as_str() {
        "regelwerk" => "regelwerk",
        "willkommen" => "willkommen",
        "deadlock-rang" => "deadlock-rang",
        "server-support" => "server-support",
        "deadlock-invite" => "deadlock-invite",
        "ankündigungen" | "ankundigungen" => "ankündigungen",
        "frag-die-community" => "frag-die-community",
        "memes" => "memes",
        "rage-room" => "rage-room",
        "mitspieler-suche" => "mitspieler-suche",
        "guides-und-tipps" => "guides-und-tipps",
        "deadlock-streamer" => "twitch",
        _ => target,
    }
}

fn apply_documented_structure_moves(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    for (channel_name, target_category_name) in DOCUMENTED_STRUCTURE_MOVES {
        let Some(target_category_id) = ctx.category_id(target_category_name) else {
            ctx.warn_once(
                "missing_move_target_category",
                target_category_name,
                format!(
                    "Ziel-Kategorie `{target_category_name}` fuer dokumentierten Umzug von `{channel_name}` wurde im Ist-Modell nicht gefunden"
                ),
            );
            continue;
        };

        for channel in desired
            .channels
            .values_mut()
            .filter(|channel| channel_matches(&channel.name, &[*channel_name]))
        {
            channel.parent_category_id = Some(target_category_id);
        }
    }
}

fn documented_channel_rename(name: &str) -> Option<&'static str> {
    match matching_key(name).as_str() {
        "hier-starten-regelwerk" | "regelwerk" => Some("📜regelwerk"),
        "willkommen" => Some("🧭willkommen"),
        "rang-auswahl" | "deadlock-rang" => Some("🔗deadlock-rang"),
        "lag-kompensator" | "server-support" => Some("🎫server-support"),
        "beta-zugang" | "deadlock-invite" => Some("💌deadlock-invite"),
        "ankündigungen" | "ankundigungen" => Some("📢ankündigungen"),
        "patchnotes" => Some("📝patchnotes"),
        "dev-updates" => Some("🐛dev-updates"),
        "streamer-updates" => Some("🎥streamer-updates"),
        "twitch" | "deadlock-streamer" => Some("🎥deadlock-streamer"),
        "allgemein" => Some("🌐allgemein"),
        "off-topic" => Some("🎲off-topic"),
        "community-fragen" | "frag-die-community" => Some("💬frag-die-community"),
        "memes-channel" | "memes" => Some("💀memes"),
        "leaks" => Some("👀leaks"),
        "kreativ-ecke" => Some("🎨kreativ-ecke"),
        "gameplay-clips" => Some("📼gameplay-clips"),
        "yt-videos" => Some("📺yt-videos"),
        "feedback" => Some("❓feedback"),
        "bot-spam" => Some("🤖bot-spam"),
        "haatteee" | "rage-room" => Some("😡rage-room"),
        "spieler-suche" | "mitspieler-suche" => Some("🎯mitspieler-suche"),
        "game-guides" | "guides-und-tipps" => Some("📖guides-und-tipps"),
        "custom-games" => Some("🧩custom-games"),
        "rank-ups" => Some("🏆rank-ups"),
        "scrim-planung" => Some("🗓️scrim-planung"),
        _ => None,
    }
}

fn apply_moderation(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    let mut category_overwrites = vec![everyone_overwrite(
        desired.guild_id,
        category_id,
        f_everyone_hidden_profile(),
    )];
    push_role_overwrite(
        &mut category_overwrites,
        desired.guild_id,
        category_id,
        ctx,
        ROLE_COMMUNITY_MODERATOR,
        f_role_visibility_profile(),
    );
    set_exact_overwrites(desired, ctx, category_id, category_overwrites);

    for channel_id in child_channel_ids(desired, category_id) {
        let name = desired.channel_name(channel_id).unwrap_or_default();
        let mut overwrites = Vec::new();
        if name == "bot-logs" {
            push_role_overwrite(
                &mut overwrites,
                desired.guild_id,
                channel_id,
                ctx,
                ROLE_DEADLOCK_PATCHNOTES,
                allow(Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES),
            );
        } else if name == "caster-chat" {
            for role in [ROLE_TURNIER_CASTER, ROLE_TURNIER_MODERATION, ROLE_COACH] {
                push_role_overwrite(
                    &mut overwrites,
                    desired.guild_id,
                    channel_id,
                    ctx,
                    role,
                    allow(Permissions::VIEW_CHANNEL),
                );
            }
        }
        set_exact_overwrites(desired, ctx, channel_id, overwrites);
    }
}

fn apply_information(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    set_exact_overwrites(
        desired,
        ctx,
        category_id,
        vec![everyone_overwrite(
            desired.guild_id,
            category_id,
            p1_announcement_profile(),
        )],
    );

    for channel_id in child_channel_ids(desired, category_id) {
        let name = desired.channel_name(channel_id).unwrap_or_default();
        if channel_matches(name, &["deadlock-rang", "rang-auswahl"]) {
            apply_deadlock_rang_channel(desired, ctx, channel_id);
        } else if channel_matches(name, &["deadlock-invite", "beta-zugang"]) {
            apply_public_channel(desired, ctx, channel_id);
        } else if channel_matches(name, &["ankündigungen", "ankundigungen"]) {
            // §6.3: Sender-Rollen bleiben wie im Ist, @everyone-Sichtbarkeit wird explizit.
            set_everyone_overwrite_retaining_others(desired, channel_id, p1_announcement_profile());
        } else if channel_matches(name, &["patchnotes"]) {
            let mut overwrites = vec![everyone_overwrite(
                desired.guild_id,
                channel_id,
                p1_announcement_profile(),
            )];
            push_role_overwrite(
                &mut overwrites,
                desired.guild_id,
                channel_id,
                ctx,
                ROLE_DEADLOCK_PATCHNOTES,
                allow(Permissions::SEND_MESSAGES),
            );
            set_exact_overwrites(desired, ctx, channel_id, overwrites);
        } else if channel_matches(
            name,
            &[
                "regelwerk",
                "hier-starten-regelwerk",
                "willkommen",
                "server-support",
                "lag-kompensator",
            ],
        ) {
            set_exact_overwrites(
                desired,
                ctx,
                channel_id,
                vec![everyone_overwrite(
                    desired.guild_id,
                    channel_id,
                    p1_announcement_profile(),
                )],
            );
        } else {
            apply_public_channel(desired, ctx, channel_id);
        }
    }
}

fn apply_community(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    apply_public_channel(desired, ctx, category_id);
    for channel_id in child_channel_ids(desired, category_id) {
        apply_public_channel(desired, ctx, channel_id);
    }
}

fn apply_deadlock(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    apply_public_channel(desired, ctx, category_id);
    for channel_id in child_channel_ids(desired, category_id) {
        if ctx.lfg_forum_cutover_enabled && is_lfg_forum_channel(desired, channel_id) {
            apply_lfg_forum_channel(desired, ctx, channel_id);
        } else {
            apply_public_channel(desired, ctx, channel_id);
        }
    }
}

fn env_flag_enabled(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

fn apply_public_channel(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    channel_id: DiscordId,
) {
    set_exact_overwrites(desired, ctx, channel_id, Vec::new());
}

fn set_everyone_overwrite_retaining_others(
    desired: &mut GuildModel,
    channel_id: DiscordId,
    profile: PermissionOverwriteProfile,
) {
    let mut overwrites = desired
        .overwrites
        .values()
        .filter(|overwrite| {
            overwrite.key.channel_id == channel_id
                && !(overwrite.key.target_kind == TargetKind::Role
                    && overwrite.key.target_id == desired.guild_id)
        })
        .cloned()
        .collect::<Vec<_>>();
    overwrites.push(everyone_overwrite(desired.guild_id, channel_id, profile));
    set_exact_overwrites_without_retained(desired, channel_id, overwrites);
}

fn sync_channel_to_category_overwrites(
    desired: &mut GuildModel,
    category_id: DiscordId,
    channel_id: DiscordId,
) {
    let overwrites = desired
        .overwrites
        .values()
        .filter(|overwrite| overwrite.key.channel_id == category_id)
        .cloned()
        .map(|mut overwrite| {
            overwrite.key.channel_id = channel_id;
            overwrite
        })
        .collect::<Vec<_>>();
    set_exact_overwrites_without_retained(desired, channel_id, overwrites);
}

fn apply_coaching(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    // §3.6 konserviert die Struktur, saniert aber Zombie-, ADMIN- und funktionale User-Overwrites.
    // §6.9/6.10 bleiben fuer X1/X2/X4 1:1; X3 wird auf den dokumentierten Coaching--VIEW-Eintrag reduziert.
    normalize_coaching_overwrites(desired, ctx, category_id);
    ensure_x3_coaching_exception(desired, ctx, category_id);

    for channel_id in child_channel_ids(desired, category_id) {
        let name = desired.channel_name(channel_id).unwrap_or_default();
        if name == "erfahrungsberichte" {
            let mut overwrites = retained_exception_overwrites(ctx, channel_id);
            push_role_overwrite(
                &mut overwrites,
                desired.guild_id,
                channel_id,
                ctx,
                ROLE_COACHING_FEEDBACK,
                allow(Permissions::SEND_MESSAGES),
            );
            set_exact_overwrites(desired, ctx, channel_id, overwrites);
        } else if channel_matches(name, &[CHANNEL_SCRIM_PLANUNG]) {
            sync_channel_to_category_overwrites(desired, category_id, channel_id);
        }
    }
}

fn apply_vip(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    let mut overwrites = vec![everyone_overwrite(
        desired.guild_id,
        category_id,
        f_everyone_hidden_profile(),
    )];
    for role in [ROLE_VIP, ROLE_SERVER_BOOSTER, ROLE_SERVER_UNTERSTUETZER] {
        push_role_overwrite(
            &mut overwrites,
            desired.guild_id,
            category_id,
            ctx,
            role,
            f_role_visibility_profile(),
        );
    }
    set_exact_overwrites(desired, ctx, category_id, overwrites);
    clear_children(desired, ctx, category_id);
}

fn apply_streamer(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    let mut overwrites = vec![everyone_overwrite(
        desired.guild_id,
        category_id,
        f_everyone_hidden_profile(),
    )];
    push_role_overwrite(
        &mut overwrites,
        desired.guild_id,
        category_id,
        ctx,
        ROLE_STREAMER,
        f_role_visibility_profile(),
    );
    set_exact_overwrites(desired, ctx, category_id, overwrites);

    for channel_id in child_channel_ids(desired, category_id) {
        let channel = ctx.actual.channels.get(&channel_id);
        if channel.is_some_and(|channel| {
            channel.kind == ChannelKind::Voice && channel.name.contains("Streamer VC")
        }) {
            let mut overwrites = Vec::new();
            for role in [
                ROLE_STREAMER_VC_ZUGRIFF,
                ROLE_VIP,
                ROLE_SERVER_BOOSTER,
                ROLE_SERVER_UNTERSTUETZER,
            ] {
                push_role_overwrite(
                    &mut overwrites,
                    desired.guild_id,
                    channel_id,
                    ctx,
                    role,
                    f_role_visibility_profile(),
                );
            }
            set_exact_overwrites(desired, ctx, channel_id, overwrites);
        } else {
            set_exact_overwrites(desired, ctx, channel_id, Vec::new());
        }
    }
}

fn apply_chill_lanes(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    apply_coach_move_category(desired, ctx, category_id);
}

fn apply_deadlock_router(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    category_id: DiscordId,
) {
    set_exact_overwrites(desired, ctx, category_id, Vec::new());
}

fn apply_street_brawl(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    apply_coach_move_category(desired, ctx, category_id);
}

fn apply_competitiv_lanes(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    category_id: DiscordId,
) {
    let mut overwrites = vec![everyone_overwrite(
        desired.guild_id,
        category_id,
        deny(Permissions::CONNECT),
    )];
    if let Some(role_id) = ctx.role_id_by_prefix("Steam Verifiziert") {
        overwrites.push(role_overwrite(
            desired.guild_id,
            category_id,
            role_id,
            allow(Permissions::CONNECT),
        ));
    }
    push_role_overwrite(
        &mut overwrites,
        desired.guild_id,
        category_id,
        ctx,
        ROLE_VC_MOVE_RECHTE,
        allow(Permissions::CONNECT),
    );
    set_exact_overwrites(desired, ctx, category_id, overwrites);
    clear_children(desired, ctx, category_id);
}

fn apply_neue_spieler_lanes(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    category_id: DiscordId,
) {
    apply_coach_move_category(desired, ctx, category_id);
    clear_children(desired, ctx, category_id);
}

fn apply_custom_game(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    set_exact_overwrites(desired, ctx, category_id, Vec::new());
    for channel_id in child_channel_ids(desired, category_id) {
        let name = desired.channel_name(channel_id).unwrap_or_default();
        if name.contains("Caster") {
            let mut overwrites = vec![everyone_overwrite(
                desired.guild_id,
                channel_id,
                deny(Permissions::CONNECT),
            )];
            push_role_overwrite(
                &mut overwrites,
                desired.guild_id,
                channel_id,
                ctx,
                ROLE_TURNIER_CASTER,
                allow(Permissions::CONNECT),
            );
            set_exact_overwrites(desired, ctx, channel_id, overwrites);
        } else {
            set_exact_overwrites(desired, ctx, channel_id, Vec::new());
        }
    }
}

fn apply_afk_category(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    set_exact_overwrites(
        desired,
        ctx,
        category_id,
        vec![everyone_overwrite(
            desired.guild_id,
            category_id,
            deny(Permissions::SPEAK | Permissions::STREAM | Permissions::SEND_MESSAGES),
        )],
    );
    clear_children(desired, ctx, category_id);
}

fn apply_support_tickets(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    category_id: DiscordId,
) {
    let mut overwrites = vec![everyone_overwrite(
        desired.guild_id,
        category_id,
        f_everyone_hidden_profile(),
    )];
    push_role_overwrite(
        &mut overwrites,
        desired.guild_id,
        category_id,
        ctx,
        ROLE_TICKET_TOOL,
        allow(
            Permissions::VIEW_CHANNEL
                | Permissions::SEND_MESSAGES
                | Permissions::MANAGE_CHANNELS
                | Permissions::MANAGE_MESSAGES
                | Permissions::EMBED_LINKS
                | Permissions::ATTACH_FILES
                | Permissions::READ_MESSAGE_HISTORY,
        ),
    );
    set_exact_overwrites(desired, ctx, category_id, overwrites);

    for channel_id in child_channel_ids(desired, category_id) {
        let name = desired.channel_name(channel_id).unwrap_or_default();
        if is_ticket_namespace_channel(name) {
            continue;
        }
        sync_channel_to_category_overwrites(desired, category_id, channel_id);
    }
}

fn apply_alt_category(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    set_exact_overwrites(
        desired,
        ctx,
        category_id,
        vec![everyone_overwrite(
            desired.guild_id,
            category_id,
            f_everyone_hidden_profile(),
        )],
    );
    clear_children(desired, ctx, category_id);
}

fn apply_welle2b_archive_rules(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    let archive_category_id = ensure_archive_category(desired, ctx);
    set_exact_overwrites_without_retained(
        desired,
        archive_category_id,
        vec![everyone_overwrite(
            desired.guild_id,
            archive_category_id,
            f_everyone_hidden_profile(),
        )],
    );

    for channel_id in welle2b_archive_channel_ids(ctx, archive_category_id) {
        if let Some(channel) = desired.channels.get_mut(&channel_id) {
            channel.parent_category_id = Some(archive_category_id);
        }
        set_exact_overwrites_without_retained(
            desired,
            channel_id,
            vec![everyone_overwrite(
                desired.guild_id,
                channel_id,
                f_everyone_hidden_profile(),
            )],
        );
    }

    ensure_kreativ_ecke(desired, ctx);
}

fn apply_welle3_archive_rules(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    let channel_ids = welle3_archive_channel_ids(ctx);
    if channel_ids.is_empty() {
        return;
    }
    let archive_category_id = ensure_archive_category(desired, ctx);
    set_exact_overwrites_without_retained(
        desired,
        archive_category_id,
        vec![everyone_overwrite(
            desired.guild_id,
            archive_category_id,
            f_everyone_hidden_profile(),
        )],
    );

    for channel_id in channel_ids {
        if let Some(channel) = desired.channels.get_mut(&channel_id) {
            channel.parent_category_id = Some(archive_category_id);
        }
        set_exact_overwrites_without_retained(
            desired,
            channel_id,
            vec![everyone_overwrite(
                desired.guild_id,
                channel_id,
                f_everyone_hidden_profile(),
            )],
        );
    }
}

fn apply_welle34b_lfg_structure_delta(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    let Some(deadlock_category_id) = ctx.category_id(CATEGORY_DEADLOCK) else {
        ctx.warn_once(
            "missing_lfg_forum_parent",
            CATEGORY_DEADLOCK,
            format!("Ziel-Kategorie `{CATEGORY_DEADLOCK}` fuer `{CHANNEL_LFG_FORUM}` wurde im Ist-Modell nicht gefunden"),
        );
        return;
    };

    archive_legacy_lfg_text_channels(desired, ctx);
    ensure_lfg_forum_channel(desired, deadlock_category_id);
}

fn archive_legacy_lfg_text_channels(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    let channel_ids = ctx
        .actual
        .channels
        .values()
        .filter(|channel| {
            channel.kind != ChannelKind::Forum
                && channel_matches(&channel.name, LFG_CHANNEL_ALIASES)
        })
        .map(|channel| channel.channel_id)
        .collect::<Vec<_>>();
    if channel_ids.is_empty() {
        return;
    }

    let archive_category_id = ensure_archive_category(desired, ctx);
    set_exact_overwrites_without_retained(
        desired,
        archive_category_id,
        vec![everyone_overwrite(
            desired.guild_id,
            archive_category_id,
            f_everyone_hidden_profile(),
        )],
    );

    for channel_id in channel_ids {
        if let Some(channel) = desired.channels.get_mut(&channel_id) {
            channel.name = CHANNEL_LFG_ARCHIVE.to_string();
            channel.parent_category_id = Some(archive_category_id);
            channel.default_auto_archive_duration = None;
        }
        set_exact_overwrites_without_retained(
            desired,
            channel_id,
            vec![everyone_overwrite(
                desired.guild_id,
                channel_id,
                f_everyone_hidden_profile(),
            )],
        );
    }
}

fn ensure_lfg_forum_channel(
    desired: &mut GuildModel,
    deadlock_category_id: DiscordId,
) -> DiscordId {
    if let Some(channel) = desired.channels.values_mut().find(|channel| {
        channel.kind == ChannelKind::Forum && channel_matches(&channel.name, LFG_CHANNEL_ALIASES)
    }) {
        channel.name = CHANNEL_LFG_FORUM.to_string();
        channel.parent_category_id = Some(deadlock_category_id);
        channel.topic = None;
        channel.default_auto_archive_duration = Some(LFG_FORUM_DEFAULT_AUTO_ARCHIVE_DURATION);
        return channel.channel_id;
    }

    let channel_id = next_synthetic_discord_id(desired);
    let position = desired
        .channels
        .values()
        .filter(|channel| channel.parent_category_id == Some(deadlock_category_id))
        .map(|channel| channel.position)
        .max()
        .unwrap_or(0)
        + 1;
    desired.channels.insert(
        channel_id,
        crate::model::ChannelSpec {
            guild_id: desired.guild_id,
            channel_id,
            name: CHANNEL_LFG_FORUM.to_string(),
            kind: ChannelKind::Forum,
            topic: None,
            position,
            parent_category_id: Some(deadlock_category_id),
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            default_auto_archive_duration: Some(LFG_FORUM_DEFAULT_AUTO_ARCHIVE_DURATION),
            status: None,
        },
    );
    channel_id
}

fn is_lfg_forum_channel(model: &GuildModel, channel_id: DiscordId) -> bool {
    model.channels.get(&channel_id).is_some_and(|channel| {
        channel.kind == ChannelKind::Forum && channel_matches(&channel.name, LFG_CHANNEL_ALIASES)
    })
}

fn apply_lfg_forum_channel(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    channel_id: DiscordId,
) {
    let forum_manage_profile = allow(
        Permissions::VIEW_CHANNEL
            | Permissions::READ_MESSAGE_HISTORY
            | Permissions::SEND_MESSAGES
            | Permissions::CREATE_PUBLIC_THREADS
            | Permissions::CREATE_PRIVATE_THREADS
            | Permissions::SEND_MESSAGES_IN_THREADS
            | Permissions::MANAGE_THREADS
            | Permissions::MANAGE_MESSAGES
            | Permissions::EMBED_LINKS
            | Permissions::ATTACH_FILES,
    );
    let mut overwrites = vec![everyone_overwrite(
        desired.guild_id,
        channel_id,
        PermissionOverwriteProfile::new(
            Permissions::SEND_MESSAGES_IN_THREADS,
            Permissions::SEND_MESSAGES
                | Permissions::CREATE_PUBLIC_THREADS
                | Permissions::CREATE_PRIVATE_THREADS,
        ),
    )];
    push_role_overwrite(
        &mut overwrites,
        desired.guild_id,
        channel_id,
        ctx,
        ROLE_TICKET_TOOL,
        forum_manage_profile,
    );
    for role_name in [
        ROLE_COMMUNITY_MODERATOR,
        ROLE_TURNIER_MODERATION,
        ROLE_COACH,
        ROLE_VC_MOVE_RECHTE,
    ] {
        push_role_overwrite_if_present(
            &mut overwrites,
            desired.guild_id,
            channel_id,
            ctx,
            role_name,
            forum_manage_profile,
        );
    }
    set_exact_overwrites(desired, ctx, channel_id, overwrites);
}

fn ensure_archive_category(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) -> DiscordId {
    if let Some(category_id) = desired
        .categories
        .values()
        .find(|category| category_matches_expected(&category.name, CATEGORY_ARCHIV))
        .map(|category| category.category_id)
    {
        return category_id;
    }

    let category_id = next_synthetic_discord_id(desired);
    let position = desired
        .categories
        .values()
        .map(|category| category.position)
        .max()
        .unwrap_or(0)
        + 1;
    desired.categories.insert(
        category_id,
        crate::model::CategorySpec {
            guild_id: desired.guild_id,
            category_id,
            name: CATEGORY_ARCHIV.to_string(),
            position,
        },
    );
    ctx.category_ids
        .insert(category_match_key(CATEGORY_ARCHIV), category_id);
    category_id
}

fn welle2b_archive_channel_ids(
    ctx: &mut RuleContext<'_>,
    archive_category_id: DiscordId,
) -> BTreeSet<DiscordId> {
    let mut channel_ids = BTreeSet::new();
    channel_ids.extend(child_channel_ids(ctx.actual, archive_category_id));
    if let Some(alt_category_id) = ctx.category_id("Alt") {
        channel_ids.extend(
            ctx.actual
                .channels
                .values()
                .filter(|channel| channel.parent_category_id == Some(alt_category_id))
                .map(|channel| channel.channel_id),
        );
    }

    for channel in ctx.actual.channels.values() {
        if is_welle2b_archive_channel_name(&channel.name) {
            channel_ids.insert(channel.channel_id);
        }
    }
    channel_ids
}

fn welle3_archive_channel_ids(ctx: &RuleContext<'_>) -> BTreeSet<DiscordId> {
    let parent_category_ids = ["Street Brawl", CATEGORY_RANKED, CATEGORY_CHILL]
        .into_iter()
        .filter_map(|category_name| ctx.category_id(category_name))
        .collect::<BTreeSet<_>>();
    ctx.actual
        .channels
        .values()
        .filter(|channel| {
            channel
                .parent_category_id
                .is_some_and(|parent_id| parent_category_ids.contains(&parent_id))
        })
        .filter(|channel| channel_matches(&channel.name, WELLE3_ARCHIVE_CHANNEL_NAMES))
        .map(|channel| channel.channel_id)
        .collect()
}

fn is_welle2b_archive_channel_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if lower.starts_with("faq-") {
        return true;
    }
    if lower.contains("beta-invite") {
        return !lower.contains("log");
    }
    channel_matches(name, WELLE2B_ARCHIVE_CHANNEL_NAMES)
}

fn ensure_kreativ_ecke(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    let parent_category_id = kreativ_source_category_id(ctx);

    if let Some(channel) = desired
        .channels
        .values_mut()
        .find(|channel| channel_matches(&channel.name, &[CHANNEL_KREATIV_ECKE]))
    {
        // Nach der Archivierung liegen die Quellkanaele im Archiv und liefern
        // keine Kategorie mehr — ein bestehender Kanal behaelt dann seine.
        if let Some(parent_category_id) = parent_category_id {
            channel.parent_category_id = Some(parent_category_id);
        }
        channel.topic = Some(TOPIC_KREATIV_ECKE.to_string());
        return;
    }

    let Some(parent_category_id) = parent_category_id else {
        ctx.warn_once(
            "missing_kreativ_source",
            CHANNEL_KREATIV_ECKE,
            "Kreativ-Quellkanaele `movement`, `deadlock-art`, `mods`, `food` wurden im Ist-Modell nicht mit Kategorie gefunden; `kreativ-ecke` wird nicht automatisch angelegt".to_string(),
        );
        return;
    };

    let channel_id = next_synthetic_discord_id(desired);
    let position = desired
        .channels
        .values()
        .filter(|channel| channel.parent_category_id == Some(parent_category_id))
        .map(|channel| channel.position)
        .max()
        .unwrap_or(0)
        + 1;
    desired.channels.insert(
        channel_id,
        crate::model::ChannelSpec {
            guild_id: desired.guild_id,
            channel_id,
            name: "🎨kreativ-ecke".to_string(),
            kind: ChannelKind::Text,
            topic: Some(TOPIC_KREATIV_ECKE.to_string()),
            position,
            parent_category_id: Some(parent_category_id),
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            default_auto_archive_duration: None,
            status: None,
        },
    );
}

fn kreativ_source_category_id(ctx: &mut RuleContext<'_>) -> Option<DiscordId> {
    // Bereits archivierte Quellkanaele zaehlen nicht als Herkunft — sonst
    // wuerde `kreativ-ecke` selbst ins Archiv gezogen.
    let archive_category_ids: BTreeSet<DiscordId> = ctx
        .actual
        .categories
        .values()
        .filter(|category| category_matches_expected(&category.name, CATEGORY_ARCHIV))
        .map(|category| category.category_id)
        .collect();
    let mut parents = ctx
        .actual
        .channels
        .values()
        .filter(|channel| channel_matches(&channel.name, WELLE2B_KREATIV_SOURCE_CHANNEL_NAMES))
        .filter_map(|channel| channel.parent_category_id)
        .filter(|parent_id| !archive_category_ids.contains(parent_id))
        .collect::<BTreeSet<_>>();
    let first = parents.pop_first();
    if !parents.is_empty() {
        ctx.warn_once(
            "multiple_kreativ_source_categories",
            CHANNEL_KREATIV_ECKE,
            "`movement`, `deadlock-art`, `mods`, `food` liegen im Ist-Modell in mehreren Kategorien; `kreativ-ecke` nutzt die erste gefundene Kategorie".to_string(),
        );
    }
    first
}

fn apply_global_channel_overrides(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    let channel_ids: Vec<_> = desired.channels.keys().copied().collect();
    for channel_id in channel_ids {
        let Some(name) = desired.channel_name(channel_id).map(str::to_string) else {
            continue;
        };
        let name = name.as_str();
        if channel_matches(name, &["AFK"]) && desired.channel_parent(channel_id).is_none() {
            set_exact_overwrites(
                desired,
                ctx,
                channel_id,
                vec![everyone_overwrite(
                    desired.guild_id,
                    channel_id,
                    deny(Permissions::SPEAK | Permissions::STREAM | Permissions::SEND_MESSAGES),
                )],
            );
        }
    }
}

fn apply_deadlock_rang_channel(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    channel_id: DiscordId,
) {
    let mut overwrites = vec![everyone_overwrite(
        desired.guild_id,
        channel_id,
        p1_announcement_profile(),
    )];
    push_role_overwrite(
        &mut overwrites,
        desired.guild_id,
        channel_id,
        ctx,
        ROLE_DL_RANG,
        p2_panel_bot_profile(),
    );
    set_exact_overwrites(desired, ctx, channel_id, overwrites);
}

fn apply_coach_move_category(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    category_id: DiscordId,
) {
    let mut overwrites = Vec::new();
    push_role_overwrite(
        &mut overwrites,
        desired.guild_id,
        category_id,
        ctx,
        ROLE_COACH,
        allow(Permissions::MOVE_MEMBERS),
    );
    set_exact_overwrites(desired, ctx, category_id, overwrites);
}

fn clear_children(desired: &mut GuildModel, ctx: &mut RuleContext<'_>, category_id: DiscordId) {
    for channel_id in child_channel_ids(desired, category_id) {
        set_exact_overwrites(desired, ctx, channel_id, Vec::new());
    }
}

fn normalize_coaching_overwrites(
    desired: &mut GuildModel,
    ctx: &mut RuleContext<'_>,
    category_id: DiscordId,
) {
    let mut object_ids: BTreeSet<_> = child_channel_ids(desired, category_id)
        .into_iter()
        .collect();
    object_ids.insert(category_id);

    let existing: Vec<_> = desired
        .overwrites
        .values()
        .filter(|overwrite| object_ids.contains(&overwrite.key.channel_id))
        .cloned()
        .collect();
    desired
        .overwrites
        .retain(|key, _| !object_ids.contains(&key.channel_id));

    let mut replacements = Vec::new();
    for overwrite in existing {
        if overwrite.allow_bits == 0 && overwrite.deny_bits == 0 {
            continue;
        }
        if overwrite.key.channel_id == category_id
            && is_coach_or_moderation_role_overwrite(ctx.actual, &overwrite)
        {
            desired.overwrites.insert(overwrite.key.clone(), overwrite);
            continue;
        }
        if is_admin_role_overwrite(ctx.actual, &overwrite) {
            continue;
        }
        if overwrite.key.target_kind == TargetKind::Member
            && overwrite.key.target_id == USER_BAN_X3_COACHING_VIEW_ONLY
        {
            continue;
        }
        if is_coaching_user_replacement(&overwrite) {
            if let Some(replacement) = coaching_user_role_replacement(ctx, &overwrite) {
                replacements.push(replacement);
            } else {
                // Wenn die Ersatzrolle nicht aufloesbar ist, bleibt der originale User-Overwrite
                // unveraendert erhalten; die Warnung markiert den Fall zur manuellen Klaerung.
                desired.overwrites.insert(overwrite.key.clone(), overwrite);
            }
            continue;
        }
        if FUNCTIONAL_DELETE_USER_IDS.contains(&overwrite.key.target_id)
            && overwrite.key.target_kind == TargetKind::Member
        {
            continue;
        }

        let normalized = if is_user_ban_one_to_one(overwrite.key.target_id)
            && overwrite.key.target_kind == TargetKind::Member
        {
            overwrite
        } else {
            normalize_overwide_coaching_bits(overwrite)
        };
        desired
            .overwrites
            .insert(normalized.key.clone(), normalized);
    }

    for replacement in replacements {
        desired
            .overwrites
            .insert(replacement.key.clone(), replacement);
    }
}

fn ensure_x3_coaching_exception(
    desired: &mut GuildModel,
    ctx: &RuleContext<'_>,
    category_id: DiscordId,
) {
    let had_x3_overwrite = ctx.actual.overwrites.values().any(|overwrite| {
        overwrite.key.target_kind == TargetKind::Member
            && overwrite.key.target_id == USER_BAN_X3_COACHING_VIEW_ONLY
            && (overwrite.allow_bits != 0 || overwrite.deny_bits != 0)
    });
    if !had_x3_overwrite {
        return;
    }
    let overwrite = member_overwrite(
        desired.guild_id,
        category_id,
        USER_BAN_X3_COACHING_VIEW_ONLY,
        deny(Permissions::VIEW_CHANNEL),
    );
    desired.overwrites.insert(overwrite.key.clone(), overwrite);
}

fn is_admin_role_overwrite(actual: &GuildModel, overwrite: &PermissionOverwriteSpec) -> bool {
    if overwrite.key.target_kind != TargetKind::Role {
        return false;
    }
    actual
        .roles
        .get(&overwrite.key.target_id)
        .is_some_and(|role| {
            Permissions::from_bits_truncate(role.permissions_bitmask)
                .contains(Permissions::ADMINISTRATOR)
        })
}

fn is_coach_or_moderation_role_overwrite(
    actual: &GuildModel,
    overwrite: &PermissionOverwriteSpec,
) -> bool {
    if overwrite.key.target_kind != TargetKind::Role {
        return false;
    }
    actual
        .roles
        .get(&overwrite.key.target_id)
        .is_some_and(|role| {
            let key = matching_key(&role.name);
            key.contains("coach")
                || key.contains("moderator")
                || key.contains("moderation")
                || key.contains("mod")
                || Permissions::from_bits_truncate(role.permissions_bitmask)
                    .contains(Permissions::ADMINISTRATOR)
        })
}

fn is_coaching_user_replacement(overwrite: &PermissionOverwriteSpec) -> bool {
    overwrite.key.target_kind == TargetKind::Member
        && (TEAM_CAPTAIN_USER_IDS.contains(&overwrite.key.target_id)
            || overwrite.key.target_id == COACH_CHAT_USER_ID)
}

fn coaching_user_role_replacement(
    ctx: &mut RuleContext<'_>,
    overwrite: &PermissionOverwriteSpec,
) -> Option<PermissionOverwriteSpec> {
    let role_name = if overwrite.key.target_id == COACH_CHAT_USER_ID {
        let channel_name = ctx
            .actual
            .channel_name(overwrite.key.channel_id)
            .unwrap_or_default();
        if channel_name == "coach-chat" {
            Some(ROLE_COACH.to_string())
        } else {
            None
        }
    } else {
        ctx.actual
            .channel_name(overwrite.key.channel_id)
            .and_then(team_role_name_for_channel)
    };

    let Some(role_name) = role_name else {
        ctx.warn_once(
            "missing_coaching_replacement_mapping",
            &format!("{}:{}", overwrite.key.target_id, overwrite.key.channel_id),
            format!(
                "Coaching-User-Overwrite {} auf Objekt {} ist in §4.2 als zu ersetzen markiert, aber keinem Team-/Coach-Kanal zuordenbar; Ersatz nicht möglich, manuell klären",
                overwrite.key.target_id, overwrite.key.channel_id
            ),
        );
        return None;
    };

    let Some(role_id) = ctx.role_ids.get(&role_name).copied() else {
        ctx.warn_once(
            "missing_coaching_replacement_role",
            &format!(
                "{}:{}:{}",
                role_name, overwrite.key.target_id, overwrite.key.channel_id
            ),
            format!(
                "Coaching-User-Overwrite {} auf Objekt {} sollte durch Rolle `{role_name}` ersetzt werden, aber die Ersatz-Rolle wurde im Ist-Modell nicht gefunden; Ersatz nicht möglich, manuell klären",
                overwrite.key.target_id, overwrite.key.channel_id
            ),
        );
        return None;
    };
    let normalized = normalize_overwide_coaching_bits(overwrite.clone());
    Some(role_overwrite(
        normalized.guild_id,
        normalized.key.channel_id,
        role_id,
        PermissionOverwriteProfile {
            allow_bits: normalized.allow_bits,
            deny_bits: normalized.deny_bits,
        },
    ))
}

fn team_role_name_for_channel(channel_name: &str) -> Option<String> {
    let normalized = channel_name.to_lowercase().replace(['_', '-'], " ");
    if normalized.contains("leo") {
        return Some("Team Leo".to_string());
    }
    if normalized.contains("deniz") {
        return Some("Team Deniz".to_string());
    }
    for team_number in 1..=4 {
        if normalized.contains(&format!("team {team_number}"))
            || normalized.contains(&format!("team{team_number}"))
        {
            return Some(format!("Team {team_number}"));
        }
    }
    None
}

fn normalize_overwide_coaching_bits(
    mut overwrite: PermissionOverwriteSpec,
) -> PermissionOverwriteSpec {
    let deny = Permissions::from_bits_truncate(overwrite.deny_bits);
    if overwrite.deny_bits.count_ones() >= 20 {
        if deny.contains(Permissions::VIEW_CHANNEL) {
            overwrite.allow_bits = 0;
            overwrite.deny_bits = Permissions::VIEW_CHANNEL.bits();
            return overwrite;
        }
        if deny.contains(Permissions::CONNECT) {
            overwrite.allow_bits = 0;
            overwrite.deny_bits = Permissions::CONNECT.bits();
            return overwrite;
        }
    }

    let allow = Permissions::from_bits_truncate(overwrite.allow_bits);
    if overwrite.allow_bits.count_ones() >= 20 {
        let mut minimal = Permissions::empty();
        for bit in [
            Permissions::VIEW_CHANNEL,
            Permissions::SEND_MESSAGES,
            Permissions::CONNECT,
            Permissions::MOVE_MEMBERS,
        ] {
            if allow.contains(bit) {
                minimal |= bit;
            }
        }
        if !minimal.is_empty() {
            overwrite.allow_bits = minimal.bits();
        }
    }

    overwrite
}

fn set_exact_overwrites(
    desired: &mut GuildModel,
    ctx: &RuleContext<'_>,
    channel_id: DiscordId,
    overwrites: Vec<PermissionOverwriteSpec>,
) {
    let retained = retained_exception_overwrites(ctx, channel_id);
    set_exact_overwrites_without_retained(
        desired,
        channel_id,
        retained.into_iter().chain(overwrites).collect(),
    );
}

fn set_exact_overwrites_without_retained(
    desired: &mut GuildModel,
    channel_id: DiscordId,
    overwrites: Vec<PermissionOverwriteSpec>,
) {
    desired
        .overwrites
        .retain(|key, _| key.channel_id != channel_id);
    for overwrite in overwrites {
        desired.overwrites.insert(overwrite.key.clone(), overwrite);
    }
}

fn retained_exception_overwrites(
    ctx: &RuleContext<'_>,
    channel_id: DiscordId,
) -> Vec<PermissionOverwriteSpec> {
    ctx.actual
        .overwrites
        .values()
        .filter(|overwrite| overwrite.key.channel_id == channel_id)
        .filter(|overwrite| is_documented_exception_overwrite(ctx, overwrite))
        .cloned()
        .collect()
}

fn is_documented_exception_overwrite(
    ctx: &RuleContext<'_>,
    overwrite: &PermissionOverwriteSpec,
) -> bool {
    if is_sammelpunkt_custom_ping_overwrite(ctx.actual, overwrite) {
        return true;
    }
    if overwrite.key.target_kind != TargetKind::Member {
        return false;
    }
    if overwrite.allow_bits == 0 && overwrite.deny_bits == 0 {
        return false;
    }
    let target_id = overwrite.key.target_id;
    if is_user_ban_one_to_one(target_id) {
        return true;
    }
    if is_x3_coaching_view_exception(ctx, overwrite) {
        return true;
    }
    if FUNCTIONAL_EXCEPTION_USER_IDS.contains(&target_id) {
        return is_functional_exception_channel(ctx.actual, overwrite.key.channel_id);
    }
    false
}

fn is_sammelpunkt_custom_ping_overwrite(
    actual: &GuildModel,
    overwrite: &PermissionOverwriteSpec,
) -> bool {
    if overwrite.key.target_kind != TargetKind::Role {
        return false;
    }
    if !actual
        .channel_name(overwrite.key.channel_id)
        .is_some_and(|name| channel_matches(name, &["Sammelpunkt"]))
    {
        return false;
    }
    actual
        .roles
        .get(&overwrite.key.target_id)
        .is_some_and(|role| {
            let key = matching_key(&role.name);
            (key.contains("funny") || key.contains("grind"))
                && key.contains("custom")
                && key.contains("ping")
        })
}

fn build_documented_exceptions(
    desired: &GuildModel,
    ctx: &RuleContext<'_>,
) -> Vec<DocumentedException> {
    let mut exceptions = Vec::new();
    for overwrite in desired.overwrites.values() {
        if overwrite.key.target_kind != TargetKind::Member {
            continue;
        }
        if overwrite.allow_bits == 0 && overwrite.deny_bits == 0 {
            continue;
        }
        let target_id = overwrite.key.target_id;
        if FUNCTIONAL_DELETE_USER_IDS.contains(&target_id) {
            continue;
        }
        let exception_group =
            if is_user_ban_one_to_one(target_id) || is_x3_coaching_view_exception(ctx, overwrite) {
                Some("user-ban")
            } else if FUNCTIONAL_EXCEPTION_USER_IDS.contains(&target_id)
                && is_functional_exception_channel(ctx.actual, overwrite.key.channel_id)
            {
                Some("functional-user-overwrite")
            } else {
                None
            };
        let Some(exception_group) = exception_group else {
            continue;
        };

        exceptions.push(DocumentedException {
            exception_id: None,
            exception_key: format!(
                "{exception_group}-{}-{}",
                target_id, overwrite.key.channel_id
            ),
            object_kind: ObjectKind::PermissionOverwrite,
            channel_id: Some(overwrite.key.channel_id),
            target_kind: Some(overwrite.key.target_kind),
            target_id: Some(target_id),
            allow_bits: Some(overwrite.allow_bits),
            deny_bits: Some(overwrite.deny_bits),
            reason: match exception_group {
                "user-ban" => "Aus dem Ist-Zustand übernommene Nutzer-Sperre (Rechte-Migration \
                               2026-07-02, §4.1). Konkreter Anlass und Review-Datum trägt das \
                               Mod-Team nach (§6.15); bis dahin bleibt die Sperre unverändert \
                               bestehen."
                    .to_string(),
                _ => "Aus dem Ist-Zustand übernommene funktionale Nutzer-Sonderrechte \
                      (Rechte-Migration 2026-07-02, §4.2/§6.8). Zweck und Review-Datum trägt \
                      das Mod-Team nach (§6.15)."
                    .to_string(),
            },
        });
    }
    exceptions.sort_by(|left, right| left.exception_key.cmp(&right.exception_key));
    exceptions
}

fn is_user_ban_one_to_one(target_id: DiscordId) -> bool {
    USER_BAN_ONE_TO_ONE_EXCEPTION_IDS.contains(&target_id)
}

fn is_x3_coaching_view_exception(
    ctx: &RuleContext<'_>,
    overwrite: &PermissionOverwriteSpec,
) -> bool {
    overwrite.key.target_kind == TargetKind::Member
        && overwrite.key.target_id == USER_BAN_X3_COACHING_VIEW_ONLY
        && ctx.category_id("Coaching") == Some(overwrite.key.channel_id)
        && overwrite.allow_bits == 0
        && overwrite.deny_bits == Permissions::VIEW_CHANNEL.bits()
}

fn is_functional_exception_channel(actual: &GuildModel, channel_id: DiscordId) -> bool {
    let channel_name = actual.channel_name(channel_id).unwrap_or_default();
    channel_matches(channel_name, &["moderator-only", "bot-logs"])
}

fn build_dynamic_namespaces(ctx: &RuleContext<'_>) -> Vec<DynamicNamespace> {
    let mut namespaces = Vec::new();
    for (key, category_name, system_name) in [
        ("tempvoice_chill_lanes", CATEGORY_CHILL, "dl-voice"),
        ("tempvoice_deadlock_router", "Deadlock Router", "dl-voice"),
        ("tempvoice_street_brawl", "Street Brawl", "dl-voice"),
    ] {
        if let Some(category_id) = ctx.category_id(category_name) {
            namespaces.push(DynamicNamespace {
                namespace_id: None,
                namespace_key: key.to_string(),
                system_name: system_name.to_string(),
                match_rule: NamespaceMatch::ParentCategory(category_id),
            });
        }
    }
    namespaces.push(DynamicNamespace {
        namespace_id: None,
        namespace_key: "ticket_channels".to_string(),
        system_name: "TicketTool".to_string(),
        match_rule: NamespaceMatch::NamePattern(r"^(ticket|closed)-[0-9]+$".to_string()),
    });
    namespaces.push(DynamicNamespace {
        namespace_id: None,
        namespace_key: "faq_channels".to_string(),
        system_name: "AI-Onboarding/FAQ".to_string(),
        match_rule: NamespaceMatch::NamePrefix("faq-".to_string()),
    });
    namespaces.push(DynamicNamespace {
        namespace_id: None,
        namespace_key: "coaching_scrim_team_channels".to_string(),
        system_name: "Coaching/Scrim".to_string(),
        match_rule: NamespaceMatch::NamePattern(r"^(leo-team|deniz-team|team-[0-9]+)$".to_string()),
    });
    namespaces.push(DynamicNamespace {
        namespace_id: None,
        namespace_key: "bot_pate_fallback".to_string(),
        system_name: "Bot-Pate".to_string(),
        match_rule: NamespaceMatch::NamePrefix("fallback-".to_string()),
    });
    namespaces
}

fn child_channel_ids(actual: &GuildModel, category_id: DiscordId) -> Vec<DiscordId> {
    actual
        .channels
        .values()
        .filter(|channel| channel.parent_category_id == Some(category_id))
        .map(|channel| channel.channel_id)
        .collect()
}

fn public_write_denies() -> Permissions {
    Permissions::SEND_MESSAGES
        | Permissions::CREATE_PUBLIC_THREADS
        | Permissions::SEND_MESSAGES_IN_THREADS
}

fn allow(bits: Permissions) -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(bits, Permissions::empty())
}

fn deny(bits: Permissions) -> PermissionOverwriteProfile {
    PermissionOverwriteProfile::new(Permissions::empty(), bits)
}

fn everyone_overwrite(
    guild_id: DiscordId,
    channel_id: DiscordId,
    profile: PermissionOverwriteProfile,
) -> PermissionOverwriteSpec {
    PermissionOverwriteSpec {
        guild_id,
        key: OverwriteKey {
            channel_id,
            target_kind: TargetKind::Role,
            target_id: guild_id,
        },
        allow_bits: profile.allow_bits,
        deny_bits: profile.deny_bits,
    }
}

fn role_overwrite(
    guild_id: DiscordId,
    channel_id: DiscordId,
    role_id: DiscordId,
    profile: PermissionOverwriteProfile,
) -> PermissionOverwriteSpec {
    PermissionOverwriteSpec {
        guild_id,
        key: OverwriteKey {
            channel_id,
            target_kind: TargetKind::Role,
            target_id: role_id,
        },
        allow_bits: profile.allow_bits,
        deny_bits: profile.deny_bits,
    }
}

fn member_overwrite(
    guild_id: DiscordId,
    channel_id: DiscordId,
    user_id: DiscordId,
    profile: PermissionOverwriteProfile,
) -> PermissionOverwriteSpec {
    PermissionOverwriteSpec {
        guild_id,
        key: OverwriteKey {
            channel_id,
            target_kind: TargetKind::Member,
            target_id: user_id,
        },
        allow_bits: profile.allow_bits,
        deny_bits: profile.deny_bits,
    }
}

fn push_role_overwrite(
    overwrites: &mut Vec<PermissionOverwriteSpec>,
    guild_id: DiscordId,
    channel_id: DiscordId,
    ctx: &mut RuleContext<'_>,
    role_name: &str,
    profile: PermissionOverwriteProfile,
) {
    if let Some(role_id) = ctx.role_id(role_name) {
        overwrites.push(role_overwrite(guild_id, channel_id, role_id, profile));
    }
}

fn push_role_overwrite_if_present(
    overwrites: &mut Vec<PermissionOverwriteSpec>,
    guild_id: DiscordId,
    channel_id: DiscordId,
    ctx: &RuleContext<'_>,
    role_name: &str,
    profile: PermissionOverwriteProfile,
) {
    if let Some(role_id) = ctx.role_ids.get(role_name).copied() {
        overwrites.push(role_overwrite(guild_id, channel_id, role_id, profile));
    }
}

fn channel_matches(name: &str, candidates: &[&str]) -> bool {
    let name = matching_key(name);
    candidates
        .iter()
        .any(|candidate| matching_key(candidate) == name)
}

fn category_matches_expected(name: &str, expected: &str) -> bool {
    category_match_key(name) == matching_key(expected)
}

fn is_resolved_empty_category(name: &str) -> bool {
    channel_matches(name, &["Alt", "Beta Zugang"])
}

fn remove_empty_resolved_categories(desired: &mut GuildModel, ctx: &mut RuleContext<'_>) {
    for category_name in ["Alt", "Beta Zugang"] {
        let Some(category_id) = ctx.category_id(category_name) else {
            continue;
        };
        if child_channel_ids(desired, category_id).is_empty() {
            desired.categories.remove(&category_id);
            desired
                .overwrites
                .retain(|key, _| key.channel_id != category_id);
        }
    }
}

fn category_match_key(name: &str) -> String {
    let key = matching_key(name);
    for (alias, expected) in CATEGORY_MATCH_ALIASES {
        if *alias == key {
            return matching_key(expected);
        }
    }
    key
}

fn matching_key(name: &str) -> String {
    name.trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .trim()
        .to_lowercase()
}

fn next_synthetic_discord_id(model: &GuildModel) -> DiscordId {
    let mut candidate = model
        .categories
        .keys()
        .chain(model.channels.keys())
        .chain(model.roles.keys())
        .copied()
        .max()
        .unwrap_or(model.guild_id)
        .saturating_add(1);
    while model.categories.contains_key(&candidate)
        || model.channels.contains_key(&candidate)
        || model.roles.contains_key(&candidate)
    {
        candidate = candidate.saturating_add(1);
    }
    candidate
}

fn is_ticket_namespace_channel(name: &str) -> bool {
    let Some((prefix, suffix)) = name.split_once('-') else {
        return false;
    };
    matches!(prefix, "ticket" | "closed") && suffix.chars().all(|ch| ch.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff_models;
    use crate::model::{BotMessageSpec, CategorySpec, ChannelSpec, RoleSpec};

    const GUILD_ID: u64 = 1289721245281292288;
    const CHAT_CATEGORY: u64 = 200;
    const GENERAL: u64 = 201;
    const UNKNOWN_CATEGORY: u64 = 202;
    const UNKNOWN_CHANNEL: u64 = 203;
    const EINGANGSBEREICH_CATEGORY: u64 = 204;
    const MEDIEN_CATEGORY: u64 = 205;
    const COACHING_CATEGORY: u64 = 206;
    const COACHING_SCRIM_CATEGORY_ID: u64 = 1_459_526_231_686_119_600;
    const RANG_AUSWAHL: u64 = 207;
    const STREAM_UPDATES: u64 = 208;
    const BETA_ZUGANG: u64 = 209;
    const COACH_CHAT: u64 = 210;
    const TEAM_LEO: u64 = 211;
    const BOT_MESSAGE_CHANNEL: u64 = 212;
    const VIP_CATEGORY: u64 = 213;
    const STREAMER_CATEGORY: u64 = 214;
    const DEADLOCK_ROUTER_CATEGORY: u64 = 215;
    const SUPPORT_CATEGORY: u64 = 216;
    const REGELWERK: u64 = 217;
    const BETA_ZUGANG_EMOJI: u64 = 218;
    const ALT_CATEGORY: u64 = 219;
    const ALT_CHILD: u64 = 220;
    const FAQ_USER_CHANNEL: u64 = 221;
    const SERVER_FAQ: u64 = 222;
    const BETA_INVITE_SPAM: u64 = 223;
    const BETA_INVITE_LOG: u64 = 224;
    const MOVEMENT: u64 = 225;
    const DEADLOCK_ART: u64 = 226;
    const MODS: u64 = 227;
    const FOOD: u64 = 228;
    const CUSTOM_GAME_CATEGORY: u64 = 229;
    const SAMMELPUNKT: u64 = 230;
    const EXISTING_ARCHIVE_CATEGORY: u64 = 231;
    const ALREADY_ARCHIVED_CHANNEL: u64 = 232;
    const DEADLOCK_CATEGORY: u64 = 233;
    const TWITCH: u64 = 234;
    const DEADLOCK_STREAMER: u64 = 235;
    const MITSPIELER_SUCHE: u64 = 236;
    const GAME_GUIDES: u64 = 237;
    const CUSTOM_GAMES: u64 = 238;
    const RANK_UPS: u64 = 239;
    const STREET_BRAWL_CATEGORY: u64 = 240;
    const RANKED_CATEGORY: u64 = 241;
    const CHILL_CATEGORY: u64 = 242;
    const STREET_BRAWL_PANEL: u64 = 243;
    const RANKED_PANEL: u64 = 244;
    const CHILL_PANEL: u64 = 245;
    const RANKED_ANLEITUNG: u64 = 246;
    const ROUTER_PANEL: u64 = 247;
    const BANNED_USER: u64 = 685_573_558_281_175_043;
    const BANNED_X2: u64 = 496_268_533_496_545_283;
    const BANNED_X4: u64 = 601_742_833_438_818_357;
    const ADMIN_ROLE: u64 = 300;
    const TEAM_LEO_ROLE: u64 = 301;
    const COACH_ROLE: u64 = 302;
    const DL_RANG_ROLE: u64 = 303;
    const OLD_ARCHIVE_ROLE: u64 = 304;
    const FUNNY_CUSTOM_PING_ROLE: u64 = 305;
    const GRIND_CUSTOM_PING_ROLE: u64 = 306;
    const COMMUNITY_MOD_ROLE: u64 = 307;
    const BOT_ROLE: u64 = 308;

    fn category(id: u64, name: &str) -> CategorySpec {
        CategorySpec {
            guild_id: GUILD_ID,
            category_id: id,
            name: name.to_string(),
            position: 1,
        }
    }

    fn channel(id: u64, name: &str, parent: Option<u64>) -> ChannelSpec {
        ChannelSpec {
            guild_id: GUILD_ID,
            channel_id: id,
            name: name.to_string(),
            kind: ChannelKind::Text,
            topic: None,
            position: 1,
            parent_category_id: parent,
            nsfw: false,
            bitrate: None,
            user_limit: None,
            rate_limit_per_user: None,
            default_auto_archive_duration: None,
            status: None,
        }
    }

    fn role(id: u64, name: &str, bits: u64) -> RoleSpec {
        RoleSpec {
            guild_id: GUILD_ID,
            role_id: id,
            name: name.to_string(),
            color: 0,
            hoist: false,
            mentionable: false,
            managed: false,
            permissions_bitmask: bits,
            position: 1,
        }
    }

    fn role_with_flags(
        id: u64,
        name: &str,
        bits: u64,
        color: i32,
        hoist: bool,
        mentionable: bool,
        position: i32,
    ) -> RoleSpec {
        RoleSpec {
            guild_id: GUILD_ID,
            role_id: id,
            name: name.to_string(),
            color,
            hoist,
            mentionable,
            managed: false,
            permissions_bitmask: bits,
            position,
        }
    }

    fn role_by_name<'a>(model: &'a GuildModel, name: &str) -> Option<&'a RoleSpec> {
        model.roles.values().find(|role| role.name == name)
    }

    fn overwrite(
        channel_id: u64,
        target_kind: TargetKind,
        target_id: u64,
        allow_bits: u64,
        deny_bits: u64,
    ) -> PermissionOverwriteSpec {
        PermissionOverwriteSpec {
            guild_id: GUILD_ID,
            key: OverwriteKey {
                channel_id,
                target_kind,
                target_id,
            },
            allow_bits,
            deny_bits,
        }
    }

    fn desired_options(
        welle2b_archive_enabled: bool,
        lfg_forum_cutover_enabled: bool,
    ) -> DesiredModelOptions {
        DesiredModelOptions {
            welle2b_archive_enabled,
            lfg_forum_cutover_enabled,
        }
    }

    fn actual_model() -> GuildModel {
        let mut model = GuildModel::new(GUILD_ID);
        model.roles.insert(GUILD_ID, role(GUILD_ID, "@everyone", 0));
        model
            .categories
            .insert(CHAT_CATEGORY, category(CHAT_CATEGORY, "Chat"));
        model
            .channels
            .insert(GENERAL, channel(GENERAL, "allgemein", Some(CHAT_CATEGORY)));
        model
    }

    fn archive_candidate_model() -> GuildModel {
        let mut model = actual_model();
        model
            .roles
            .insert(OLD_ARCHIVE_ROLE, role(OLD_ARCHIVE_ROLE, "Altrolle", 0));
        model
            .categories
            .insert(ALT_CATEGORY, category(ALT_CATEGORY, "Alt"));
        for (id, name, parent) in [
            (ALT_CHILD, "low-elo-ranked", Some(ALT_CATEGORY)),
            (FAQ_USER_CHANNEL, "faq-testuser", Some(CHAT_CATEGORY)),
            (SERVER_FAQ, "server-faq", Some(CHAT_CATEGORY)),
            (BETA_INVITE_SPAM, "beta-invite-spam", Some(CHAT_CATEGORY)),
            (BETA_INVITE_LOG, "beta-invite-log", Some(CHAT_CATEGORY)),
            (MOVEMENT, "movement", Some(CHAT_CATEGORY)),
            (DEADLOCK_ART, "deadlock-art", Some(CHAT_CATEGORY)),
            (MODS, "mods", Some(CHAT_CATEGORY)),
            (FOOD, "food", Some(CHAT_CATEGORY)),
        ] {
            model.channels.insert(id, channel(id, name, parent));
            model.overwrites.insert(
                OverwriteKey {
                    channel_id: id,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                },
                overwrite(
                    id,
                    TargetKind::Role,
                    GUILD_ID,
                    0,
                    Permissions::VIEW_CHANNEL.bits(),
                ),
            );
        }
        let old_allow = overwrite(
            ALT_CHILD,
            TargetKind::Role,
            OLD_ARCHIVE_ROLE,
            Permissions::VIEW_CHANNEL.bits(),
            0,
        );
        model.overwrites.insert(old_allow.key.clone(), old_allow);
        model
    }

    fn documented_categories_model() -> GuildModel {
        let mut model = actual_model();
        model.categories.insert(
            EINGANGSBEREICH_CATEGORY,
            category(EINGANGSBEREICH_CATEGORY, "Eingangsbereich"),
        );
        model
            .categories
            .insert(MEDIEN_CATEGORY, category(MEDIEN_CATEGORY, "Medien"));
        model
            .categories
            .insert(DEADLOCK_CATEGORY, category(DEADLOCK_CATEGORY, "Sonstiges"));
        model
            .categories
            .insert(COACHING_CATEGORY, category(COACHING_CATEGORY, "Coaching"));
        model
    }

    #[test]
    fn live_category_matching_erkennt_emoji_case_und_explizite_aliases() {
        for (live_name, expected_name) in [
            ("🗨️Chat", CATEGORY_COMMUNITY),
            ("Sonstiges", CATEGORY_DEADLOCK),
            ("🎖️Coaching", CATEGORY_COACHING),
            ("vip", CATEGORY_VIP),
            ("Streamer Only", CATEGORY_STREAMER),
            ("Chill Lanes", CATEGORY_CHILL),
            ("Competitiv Lanes", CATEGORY_RANKED),
            ("Neue Spieler Lanes", CATEGORY_NEUE_SPIELER),
            ("🛜Deadlock Router", "Deadlock Router"),
            ("❓Support", CATEGORY_SUPPORT),
            ("📦 Archiv", CATEGORY_ARCHIV),
        ] {
            assert!(
                category_matches_expected(live_name, expected_name),
                "{live_name:?} muss {expected_name:?} matchen"
            );
        }
    }

    #[test]
    fn documented_channel_renames_erkennen_emoji_deko_und_alt_namen() {
        for (live_name, target_name) in [
            ("⚖️hier-starten-regelwerk", "📜regelwerk"),
            ("hier-starten-regelwerk", "📜regelwerk"),
            ("rang-auswahl", "🔗deadlock-rang"),
            ("lag-kompensator", "🎫server-support"),
            ("community-fragen", "💬frag-die-community"),
            ("memes-channel", "💀memes"),
            ("haatteee", "😡rage-room"),
            ("spieler-suche", "🎯mitspieler-suche"),
            ("game-guides", "📖guides-und-tipps"),
            ("twitch", "🎥deadlock-streamer"),
            ("🎥twitch", "🎥deadlock-streamer"),
            ("beta-zugang", "💌deadlock-invite"),
            ("🔑beta-zugang", "💌deadlock-invite"),
        ] {
            assert_eq!(
                documented_channel_rename(live_name),
                Some(target_name),
                "{live_name:?} muss nach {target_name:?} umbenennen"
            );
        }
    }

    #[test]
    fn welle2b_archivregeln_bleiben_ohne_flag_inaktiv() -> anyhow::Result<()> {
        let actual = archive_candidate_model();

        let derived = derive_desired_model_with_options(
            &actual,
            DesiredModelOptions {
                welle2b_archive_enabled: false,
                lfg_forum_cutover_enabled: false,
            },
        )?;

        assert!(derived
            .desired
            .categories
            .values()
            .all(|category| category.name != CATEGORY_ARCHIV));
        assert!(derived
            .desired
            .channels
            .values()
            .any(|channel| channel.name == "🎨kreativ-ecke"));
        assert_eq!(
            derived.desired.channels[&ALT_CHILD].parent_category_id,
            Some(ALT_CATEGORY)
        );
        assert!(derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: ALT_CHILD,
            target_kind: TargetKind::Role,
            target_id: GUILD_ID,
        }));
        Ok(())
    }

    #[test]
    fn kreativ_ecke_bleibt_nach_archivierung_in_ihrer_kategorie() -> anyhow::Result<()> {
        // Post-Archiv-Zustand: Quellkanaele liegen bereits im Archiv,
        // kreativ-ecke existiert ausserhalb — sie darf NICHT nachgezogen werden.
        let mut actual = archive_candidate_model();
        const ARCHIV_CATEGORY: u64 = 999_001;
        const KREATIV_ECKE: u64 = 999_002;
        actual
            .categories
            .insert(ARCHIV_CATEGORY, category(ARCHIV_CATEGORY, "📦 Archiv"));
        for source_id in [MOVEMENT, DEADLOCK_ART, MODS, FOOD] {
            actual
                .channels
                .get_mut(&source_id)
                .expect("source channel")
                .parent_category_id = Some(ARCHIV_CATEGORY);
        }
        actual.channels.insert(
            KREATIV_ECKE,
            channel(KREATIV_ECKE, "kreativ-ecke", Some(CHAT_CATEGORY)),
        );

        let derived = derive_desired_model_with_options(
            &actual,
            DesiredModelOptions {
                welle2b_archive_enabled: true,
                lfg_forum_cutover_enabled: false,
            },
        )?;

        assert_eq!(
            derived.desired.channels[&KREATIV_ECKE].parent_category_id,
            Some(CHAT_CATEGORY),
            "kreativ-ecke darf nicht ins Archiv gezogen werden"
        );
        Ok(())
    }

    #[test]
    fn welle2b_archivregeln_erstellen_archiv_und_kreativ_ecke() -> anyhow::Result<()> {
        let actual = archive_candidate_model();

        let derived = derive_desired_model_with_options(
            &actual,
            DesiredModelOptions {
                welle2b_archive_enabled: true,
                lfg_forum_cutover_enabled: false,
            },
        )?;
        let archive_id = derived
            .desired
            .categories
            .values()
            .find(|category| category.name == CATEGORY_ARCHIV)
            .expect("archive category")
            .category_id;

        let archive_overwrites = derived
            .desired
            .overwrites
            .values()
            .filter(|overwrite| overwrite.key.channel_id == archive_id)
            .collect::<Vec<_>>();
        assert_eq!(archive_overwrites.len(), 1);
        assert_eq!(archive_overwrites[0].key.target_kind, TargetKind::Role);
        assert_eq!(archive_overwrites[0].key.target_id, GUILD_ID);
        assert_eq!(archive_overwrites[0].allow_bits, 0);
        assert_eq!(
            archive_overwrites[0].deny_bits,
            Permissions::VIEW_CHANNEL.bits()
        );

        for channel_id in [
            ALT_CHILD,
            FAQ_USER_CHANNEL,
            SERVER_FAQ,
            BETA_INVITE_SPAM,
            MOVEMENT,
            DEADLOCK_ART,
            MODS,
            FOOD,
        ] {
            assert_eq!(
                derived.desired.channels[&channel_id].parent_category_id,
                Some(archive_id),
                "channel {channel_id} muss ins Archiv"
            );
            let channel_overwrites = derived
                .desired
                .overwrites
                .values()
                .filter(|overwrite| overwrite.key.channel_id == channel_id)
                .collect::<Vec<_>>();
            assert_eq!(
                channel_overwrites.len(),
                1,
                "channel {channel_id} braucht exakt die materialisierte Archiv-Sperre"
            );
            assert_eq!(channel_overwrites[0].key.target_kind, TargetKind::Role);
            assert_eq!(channel_overwrites[0].key.target_id, GUILD_ID);
            assert_eq!(channel_overwrites[0].allow_bits, 0);
            assert_eq!(
                channel_overwrites[0].deny_bits,
                Permissions::VIEW_CHANNEL.bits()
            );
        }
        let diff = diff_models(
            &derived.desired,
            &actual,
            &derived.dynamic_namespaces,
            &derived.exceptions,
        )?;
        assert!(
            diff.blocked.iter().all(|blocked| {
                ![
                    ALT_CHILD,
                    FAQ_USER_CHANNEL,
                    SERVER_FAQ,
                    BETA_INVITE_SPAM,
                    MOVEMENT,
                    DEADLOCK_ART,
                    MODS,
                    FOOD,
                ]
                .contains(&blocked.change.object.object_id)
            }),
            "Archiv-Deletes duerfen wegen materialisierter Sperre nicht blocken: {:?}",
            diff.blocked
        );
        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Delete
                && change.object.kind == ObjectKind::PermissionOverwrite
                && change.object.channel_id == Some(ALT_CHILD)
                && change.object.target_id == Some(OLD_ARCHIVE_ROLE)
        }));
        assert_eq!(
            derived.desired.channels[&BETA_INVITE_LOG].parent_category_id,
            Some(CHAT_CATEGORY)
        );

        let kreativ = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == "🎨kreativ-ecke")
            .expect("kreativ-ecke");
        assert_eq!(kreativ.parent_category_id, Some(CHAT_CATEGORY));
        assert_eq!(kreativ.topic.as_deref(), Some(TOPIC_KREATIV_ECKE));
        Ok(())
    }

    #[test]
    fn welle2b_archivregeln_materialisieren_faq_trotz_namespace_filter() -> anyhow::Result<()> {
        let mut actual = archive_candidate_model();
        actual.overwrites.remove(&OverwriteKey {
            channel_id: FAQ_USER_CHANNEL,
            target_kind: TargetKind::Role,
            target_id: GUILD_ID,
        });

        let derived = derive_desired_model_with_options(
            &actual,
            DesiredModelOptions {
                welle2b_archive_enabled: true,
                lfg_forum_cutover_enabled: false,
            },
        )?;
        let archive_id = derived
            .desired
            .categories
            .values()
            .find(|category| category.name == CATEGORY_ARCHIV)
            .expect("archive category")
            .category_id;
        let diff = diff_models(
            &derived.desired,
            &actual,
            &derived.dynamic_namespaces,
            &derived.exceptions,
        )?;

        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Update
                && change.object.kind == ObjectKind::Channel
                && change.object.object_id == FAQ_USER_CHANNEL
                && change.fields.iter().any(|field| {
                    field.field == "parent_category_id"
                        && field.desired == serde_json::json!(archive_id)
                })
        }));
        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Create
                && change.object.kind == ObjectKind::PermissionOverwrite
                && change.object.channel_id == Some(FAQ_USER_CHANNEL)
                && change.object.target_kind == Some(TargetKind::Role)
                && change.object.target_id == Some(GUILD_ID)
        }));
        assert!(!diff.filtered.iter().any(|filtered| {
            filtered.change.object.kind == ObjectKind::PermissionOverwrite
                && filtered.change.object.channel_id == Some(FAQ_USER_CHANNEL)
                && filtered.change.action == crate::diff::DiffAction::Create
        }));
        Ok(())
    }

    #[test]
    fn welle2b_archivregeln_materialisieren_bestehende_archiv_kinder() -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual.categories.insert(
            EXISTING_ARCHIVE_CATEGORY,
            category(EXISTING_ARCHIVE_CATEGORY, "📦 Archiv"),
        );
        actual.channels.insert(
            ALREADY_ARCHIVED_CHANNEL,
            channel(
                ALREADY_ARCHIVED_CHANNEL,
                "historischer-kanal",
                Some(EXISTING_ARCHIVE_CATEGORY),
            ),
        );

        let derived = derive_desired_model_with_options(
            &actual,
            DesiredModelOptions {
                welle2b_archive_enabled: true,
                lfg_forum_cutover_enabled: false,
            },
        )?;

        assert_eq!(
            derived.desired.channels[&ALREADY_ARCHIVED_CHANNEL].parent_category_id,
            Some(EXISTING_ARCHIVE_CATEGORY)
        );
        let overwrite = derived
            .desired
            .overwrites
            .get(&OverwriteKey {
                channel_id: ALREADY_ARCHIVED_CHANNEL,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            })
            .expect("materialized archive deny");
        assert_eq!(overwrite.allow_bits, 0);
        assert_eq!(overwrite.deny_bits, Permissions::VIEW_CHANNEL.bits());
        Ok(())
    }

    #[test]
    fn dynamic_namespaces_enthalten_faq_und_coaching_scrim() -> anyhow::Result<()> {
        let actual = actual_model();

        let derived = derive_desired_model(&actual)?;
        let keys = derived
            .dynamic_namespaces
            .iter()
            .map(|namespace| namespace.namespace_key.as_str())
            .collect::<BTreeSet<_>>();

        assert!(keys.contains("faq_channels"));
        assert!(keys.contains("coaching_scrim_team_channels"));
        assert!(derived.dynamic_namespaces.iter().any(|namespace| {
            namespace.namespace_key == "coaching_scrim_team_channels"
                && namespace.match_rule
                    == NamespaceMatch::NamePattern(
                        r"^(leo-team|deniz-team|team-[0-9]+)$".to_string(),
                    )
        }));
        Ok(())
    }

    #[test]
    fn scrim_planung_create_bleibt_trotz_coaching_scrim_namespace_in_changes() -> anyhow::Result<()>
    {
        let mut actual = GuildModel::new(GUILD_ID);
        actual
            .roles
            .insert(GUILD_ID, role(GUILD_ID, "@everyone", 0));
        actual.categories.insert(
            COACHING_SCRIM_CATEGORY_ID,
            category(COACHING_SCRIM_CATEGORY_ID, "Coaching"),
        );

        let derived = derive_desired_model(&actual)?;
        let diff = diff_models(&derived.desired, &actual, &derived.dynamic_namespaces, &[])?;

        assert!(
            diff.changes.iter().any(|change| {
                change.action == crate::diff::DiffAction::Create
                    && change.object.kind == ObjectKind::Channel
                    && change
                        .desired
                        .as_ref()
                        .and_then(|value| value.get("name"))
                        .and_then(|value| value.as_str())
                        == Some("🗓️scrim-planung")
            }),
            "scrim-planung-Create fehlt in changes: changes={:?} filtered={:?}",
            diff.changes,
            diff.filtered
        );
        Ok(())
    }

    #[test]
    fn live_namensmatching_haelt_ids_und_schreibt_w3_trennernamen() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual
            .categories
            .get_mut(&CHAT_CATEGORY)
            .expect("chat")
            .name = "🗨️Chat".to_string();
        actual
            .categories
            .get_mut(&COACHING_CATEGORY)
            .expect("coaching")
            .name = "🎖️Coaching".to_string();
        actual
            .categories
            .insert(VIP_CATEGORY, category(VIP_CATEGORY, "vip"));
        actual.categories.insert(
            STREAMER_CATEGORY,
            category(STREAMER_CATEGORY, "Streamer Only"),
        );
        actual.categories.insert(
            DEADLOCK_ROUTER_CATEGORY,
            category(DEADLOCK_ROUTER_CATEGORY, "🛜Deadlock Router"),
        );
        actual
            .categories
            .insert(SUPPORT_CATEGORY, category(SUPPORT_CATEGORY, "❓Support"));
        actual
            .roles
            .insert(DL_RANG_ROLE, role(DL_RANG_ROLE, ROLE_DL_RANG, 0));
        actual.roles.insert(3001, role(3001, ROLE_VIP, 0));
        actual
            .roles
            .insert(3002, role(3002, ROLE_SERVER_BOOSTER, 0));
        actual
            .roles
            .insert(3003, role(3003, ROLE_SERVER_UNTERSTUETZER, 0));
        actual.roles.insert(3004, role(3004, ROLE_STREAMER, 0));
        actual.roles.insert(3005, role(3005, ROLE_TICKET_TOOL, 0));
        actual.channels.insert(
            REGELWERK,
            channel(
                REGELWERK,
                "⚖️hier-starten-regelwerk",
                Some(EINGANGSBEREICH_CATEGORY),
            ),
        );
        actual.channels.insert(
            BETA_ZUGANG,
            channel(BETA_ZUGANG, "beta-zugang", Some(EINGANGSBEREICH_CATEGORY)),
        );

        let derived = derive_desired_model(&actual)?;

        for missing in [
            format!("Kategorie `{CATEGORY_COMMUNITY}`"),
            format!("Kategorie `{CATEGORY_COACHING}`"),
            format!("Kategorie `{CATEGORY_VIP}`"),
            format!("Kategorie `{CATEGORY_STREAMER}`"),
            "Kategorie `Deadlock Router`".to_string(),
            format!("Kategorie `{CATEGORY_SUPPORT}`"),
        ] {
            assert!(
                !derived
                    .warnings
                    .iter()
                    .any(|warning| warning.contains(&missing)),
                "{missing} darf nicht als fehlend gewarnt werden: {:?}",
                derived.warnings
            );
        }

        assert_eq!(
            derived
                .desired
                .categories
                .get(&CHAT_CATEGORY)
                .map(|c| c.name.as_str()),
            Some(CATEGORY_COMMUNITY)
        );
        assert_eq!(
            derived
                .desired
                .categories
                .get(&VIP_CATEGORY)
                .map(|c| c.name.as_str()),
            Some(CATEGORY_VIP)
        );
        assert_eq!(
            derived
                .desired
                .categories
                .get(&STREAMER_CATEGORY)
                .map(|c| c.name.as_str()),
            Some(CATEGORY_STREAMER)
        );
        assert_eq!(
            derived
                .desired
                .categories
                .get(&SUPPORT_CATEGORY)
                .map(|c| c.name.as_str()),
            Some(CATEGORY_SUPPORT)
        );
        assert_eq!(
            derived
                .desired
                .channels
                .get(&REGELWERK)
                .map(|c| c.name.as_str()),
            Some("📜regelwerk")
        );
        assert_eq!(
            derived
                .desired
                .channels
                .get(&BETA_ZUGANG)
                .map(|channel| (channel.name.as_str(), channel.parent_category_id)),
            Some(("💌deadlock-invite", Some(CHAT_CATEGORY)))
        );
        let invite_overwrites: Vec<_> = derived
            .desired
            .overwrites
            .values()
            .filter(|overwrite| overwrite.key.channel_id == BETA_ZUGANG)
            .collect();
        assert!(
            invite_overwrites.is_empty(),
            "P0 deadlock-invite darf keine @everyone-Allows materialisieren"
        );
        Ok(())
    }

    #[test]
    fn dokumentierte_rename_kollision_beta_und_invite_wird_abgelehnt() {
        let mut actual = documented_categories_model();
        actual.channels.insert(
            BETA_ZUGANG,
            channel(BETA_ZUGANG, "beta-zugang", Some(EINGANGSBEREICH_CATEGORY)),
        );
        actual.channels.insert(
            BETA_ZUGANG_EMOJI,
            channel(
                BETA_ZUGANG_EMOJI,
                "🔑deadlock-invite",
                Some(EINGANGSBEREICH_CATEGORY),
            ),
        );

        let err = derive_desired_model(&actual).expect_err("rename collision must fail");

        assert!(err.to_string().contains("Kanal-Rename-Kollision"));
        assert!(err.to_string().contains("deadlock-invite"));
        assert!(err.to_string().contains("beta-zugang"));
    }

    #[test]
    fn dokumentierte_rename_kollision_twitch_und_deadlock_streamer_wird_abgelehnt() {
        let mut actual = documented_categories_model();
        actual
            .channels
            .insert(TWITCH, channel(TWITCH, "🎥twitch", Some(MEDIEN_CATEGORY)));
        actual.channels.insert(
            DEADLOCK_STREAMER,
            channel(
                DEADLOCK_STREAMER,
                "🎥deadlock-streamer",
                Some(MEDIEN_CATEGORY),
            ),
        );

        let err = derive_desired_model(&actual).expect_err("rename collision must fail");

        assert!(err.to_string().contains("Kanal-Rename-Kollision"));
        assert!(err.to_string().contains("deadlock-streamer"));
        assert!(err.to_string().contains("twitch"));
    }

    #[test]
    fn profile_bits_entsprechen_dokumentierten_klassen() {
        assert_eq!(
            p0_public_profile(),
            PermissionOverwriteProfile::new(Permissions::empty(), Permissions::empty())
        );
        assert!(
            Permissions::from_bits_truncate(p1_announcement_profile().allow_bits)
                .contains(Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY)
        );
        assert_eq!(
            p1_announcement_profile().deny_bits,
            (Permissions::SEND_MESSAGES
                | Permissions::CREATE_PUBLIC_THREADS
                | Permissions::SEND_MESSAGES_IN_THREADS)
                .bits()
        );
        assert_eq!(
            p2_panel_bot_profile().allow_bits,
            (Permissions::SEND_MESSAGES | Permissions::EMBED_LINKS).bits()
        );
        assert_eq!(
            f_everyone_hidden_profile().deny_bits,
            Permissions::VIEW_CHANNEL.bits()
        );
        assert_eq!(
            f_role_visibility_profile().allow_bits,
            (Permissions::VIEW_CHANNEL | Permissions::CONNECT).bits()
        );
    }

    #[test]
    fn support_kategorie_bleibt_nicht_oeffentlich() -> anyhow::Result<()> {
        const TICKET_TOOL_ROLE: u64 = 307;
        const TICKET_PANEL: u64 = 308;
        let mut actual = documented_categories_model();
        actual
            .categories
            .insert(SUPPORT_CATEGORY, category(SUPPORT_CATEGORY, "Support"));
        actual.roles.insert(
            TICKET_TOOL_ROLE,
            role(TICKET_TOOL_ROLE, ROLE_TICKET_TOOL, 0),
        );
        actual.channels.insert(
            TICKET_PANEL,
            channel(TICKET_PANEL, "ticket-eröffnen", Some(SUPPORT_CATEGORY)),
        );

        let derived = derive_desired_model(&actual)?;

        assert_eq!(
            derived.desired.categories[&SUPPORT_CATEGORY].name,
            CATEGORY_SUPPORT
        );
        let everyone = derived
            .desired
            .overwrites
            .get(&OverwriteKey {
                channel_id: SUPPORT_CATEGORY,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            })
            .expect("@everyone support deny");
        assert_eq!(everyone.allow_bits, 0);
        assert_eq!(everyone.deny_bits, Permissions::VIEW_CHANNEL.bits());
        let ticket_tool = derived
            .desired
            .overwrites
            .get(&OverwriteKey {
                channel_id: SUPPORT_CATEGORY,
                target_kind: TargetKind::Role,
                target_id: TICKET_TOOL_ROLE,
            })
            .expect("Ticket Tool support allow");
        assert!(
            Permissions::from_bits_truncate(ticket_tool.allow_bits).contains(
                Permissions::VIEW_CHANNEL
                    | Permissions::SEND_MESSAGES
                    | Permissions::MANAGE_CHANNELS
                    | Permissions::MANAGE_MESSAGES
                    | Permissions::READ_MESSAGE_HISTORY
            )
        );
        let panel_everyone = derived
            .desired
            .overwrites
            .get(&OverwriteKey {
                channel_id: TICKET_PANEL,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            })
            .expect("@everyone ticket panel deny");
        assert_eq!(panel_everyone.deny_bits, Permissions::VIEW_CHANNEL.bits());
        Ok(())
    }

    #[test]
    fn everyone_basis_enthaelt_app_commands_aber_keine_tts_oder_private_threads(
    ) -> anyhow::Result<()> {
        let actual = actual_model();
        let derived = derive_desired_model(&actual)?;
        let bits = derived
            .desired
            .roles
            .get(&GUILD_ID)
            .expect("@everyone role")
            .permissions_bitmask;
        let permissions = Permissions::from_bits_truncate(bits);

        assert!(permissions.contains(Permissions::USE_APPLICATION_COMMANDS));
        assert!(permissions.contains(Permissions::CREATE_PUBLIC_THREADS));
        assert!(permissions.contains(Permissions::SEND_MESSAGES_IN_THREADS));
        assert!(!permissions.contains(Permissions::SEND_TTS_MESSAGES));
        assert!(!permissions.contains(Permissions::CREATE_PRIVATE_THREADS));
        Ok(())
    }

    #[test]
    fn welle2b_marker_und_streams_rollen_werden_create_if_missing_angelegt() -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual.roles.insert(
            400,
            role_with_flags(
                400,
                "Patchnotes Ping Rolle",
                Permissions::ADMINISTRATOR.bits(),
                0x66ccff,
                false,
                true,
                8,
            ),
        );

        let derived = derive_desired_model(&actual)?;

        for role_name in ["Invite-Gast", "Frischling"] {
            let role = role_by_name(&derived.desired, role_name).expect("marker role");
            assert_eq!(role.permissions_bitmask, 0);
            assert!(!role.hoist);
            assert!(!role.mentionable);
            assert!(!role.managed);
        }

        let streams = role_by_name(&derived.desired, "Streams").expect("streams role");
        assert_eq!(streams.permissions_bitmask, 0);
        assert!(streams.mentionable);
        assert_eq!(streams.color, 0x66ccff);
        Ok(())
    }

    #[test]
    fn welle2b_vorhandene_rollen_bleiben_id_stabil_und_werden_nicht_dupliziert(
    ) -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual
            .roles
            .insert(401, role(401, "Invite-Gast", Permissions::empty().bits()));
        actual
            .roles
            .insert(402, role(402, "Frischling", Permissions::empty().bits()));
        actual
            .roles
            .insert(403, role_with_flags(403, "Streams", 0, 7, false, true, 4));

        let derived = derive_desired_model(&actual)?;

        for (name, id) in [("Invite-Gast", 401), ("Frischling", 402), ("Streams", 403)] {
            let matches = derived
                .desired
                .roles
                .values()
                .filter(|role| role.name == name)
                .collect::<Vec<_>>();
            assert_eq!(matches.len(), 1, "{name} darf nicht dupliziert werden");
            assert_eq!(matches[0].role_id, id);
        }
        Ok(())
    }

    #[test]
    fn p0_chat_entfernt_redundante_overwrites_aber_ban_exception_bleibt() -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: GENERAL,
                target_kind: TargetKind::Role,
                target_id: 900,
            },
            overwrite(GENERAL, TargetKind::Role, 900, 1, 2),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: GENERAL,
                target_kind: TargetKind::Member,
                target_id: BANNED_USER,
            },
            overwrite(
                GENERAL,
                TargetKind::Member,
                BANNED_USER,
                0,
                (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits(),
            ),
        );

        let derived = derive_desired_model(&actual)?;

        assert!(!derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Role,
            target_id: 900,
        }));
        assert!(derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: GENERAL,
            target_kind: TargetKind::Member,
            target_id: BANNED_USER,
        }));
        assert_eq!(derived.exceptions.len(), 1);
        assert_eq!(derived.exceptions[0].target_id, Some(BANNED_USER));
        Ok(())
    }

    #[test]
    fn dokumentierte_struktur_umzuege_setzen_parent_und_neue_klasse() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual
            .roles
            .insert(DL_RANG_ROLE, role(DL_RANG_ROLE, ROLE_DL_RANG, 0));
        actual.channels.insert(
            RANG_AUSWAHL,
            channel(RANG_AUSWAHL, "rang-auswahl", Some(CHAT_CATEGORY)),
        );
        actual.channels.insert(
            STREAM_UPDATES,
            channel(STREAM_UPDATES, "streamer-updates", Some(CHAT_CATEGORY)),
        );
        actual.channels.insert(
            BETA_ZUGANG,
            channel(BETA_ZUGANG, "beta-zugang", Some(EINGANGSBEREICH_CATEGORY)),
        );
        actual.channels.insert(
            MITSPIELER_SUCHE,
            channel(MITSPIELER_SUCHE, "spieler-suche", Some(CHAT_CATEGORY)),
        );
        actual.channels.insert(
            GAME_GUIDES,
            channel(GAME_GUIDES, "game-guides", Some(MEDIEN_CATEGORY)),
        );
        actual
            .channels
            .insert(TWITCH, channel(TWITCH, "twitch", Some(MEDIEN_CATEGORY)));
        actual.channels.insert(
            CUSTOM_GAMES,
            channel(CUSTOM_GAMES, "custom-games", Some(CHAT_CATEGORY)),
        );
        actual.channels.insert(
            RANK_UPS,
            channel(RANK_UPS, "rank-ups", Some(DEADLOCK_CATEGORY)),
        );

        let derived = derive_desired_model_with_options(&actual, desired_options(false, true))?;

        let rang = derived.desired.channels.get(&RANG_AUSWAHL).expect("rang");
        assert_eq!(rang.name, "🔗deadlock-rang");
        assert_eq!(rang.parent_category_id, Some(EINGANGSBEREICH_CATEGORY));
        assert!(derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: RANG_AUSWAHL,
            target_kind: TargetKind::Role,
            target_id: actual.guild_id,
        }));
        assert!(derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: RANG_AUSWAHL,
            target_kind: TargetKind::Role,
            target_id: DL_RANG_ROLE,
        }));

        let stream = derived
            .desired
            .channels
            .get(&STREAM_UPDATES)
            .expect("stream");
        assert_eq!(stream.name, "🎥streamer-updates");
        assert_eq!(stream.parent_category_id, Some(MEDIEN_CATEGORY));
        assert!(
            !derived.desired.overwrites.contains_key(&OverwriteKey {
                channel_id: STREAM_UPDATES,
                target_kind: TargetKind::Role,
                target_id: actual.guild_id,
            }),
            "stream-updates erbt die P1-Rechte aus INFORMATION"
        );

        let invite = derived.desired.channels.get(&BETA_ZUGANG).expect("invite");
        assert_eq!(invite.name, "💌deadlock-invite");
        assert_eq!(invite.parent_category_id, Some(CHAT_CATEGORY));
        let invite_overwrites: Vec<_> = derived
            .desired
            .overwrites
            .values()
            .filter(|overwrite| overwrite.key.channel_id == BETA_ZUGANG)
            .collect();
        assert!(
            invite_overwrites.is_empty(),
            "P0 deadlock-invite darf keine @everyone-Allows materialisieren"
        );
        let archive_id = derived
            .desired
            .categories
            .values()
            .find(|category| category.name == CATEGORY_ARCHIV)
            .expect("archive category")
            .category_id;
        assert_eq!(
            derived
                .desired
                .channels
                .get(&MITSPIELER_SUCHE)
                .map(|channel| (
                    channel.name.as_str(),
                    channel.kind.clone(),
                    channel.parent_category_id
                )),
            Some((
                "archiv-mitspieler-suche",
                ChannelKind::Text,
                Some(archive_id)
            ))
        );
        let lfg_forum = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == "🎯mitspieler-suche")
            .expect("lfg forum");
        assert_eq!(lfg_forum.kind, ChannelKind::Forum);
        assert_eq!(lfg_forum.parent_category_id, Some(DEADLOCK_CATEGORY));
        assert_eq!(
            derived
                .desired
                .channels
                .get(&GAME_GUIDES)
                .map(|channel| (channel.name.as_str(), channel.parent_category_id)),
            Some(("📖guides-und-tipps", Some(EINGANGSBEREICH_CATEGORY)))
        );
        assert_eq!(
            derived
                .desired
                .channels
                .get(&TWITCH)
                .map(|channel| (channel.name.as_str(), channel.parent_category_id)),
            Some(("🎥deadlock-streamer", Some(MEDIEN_CATEGORY)))
        );
        assert_eq!(
            derived
                .desired
                .channels
                .get(&CUSTOM_GAMES)
                .map(|channel| (channel.name.as_str(), channel.parent_category_id)),
            Some(("🧩custom-games", Some(DEADLOCK_CATEGORY)))
        );
        assert_eq!(
            derived
                .desired
                .channels
                .get(&RANK_UPS)
                .map(|channel| (channel.name.as_str(), channel.parent_category_id)),
            Some(("🏆rank-ups", Some(DEADLOCK_CATEGORY)))
        );
        assert_eq!(
            derived.desired.categories[&MEDIEN_CATEGORY].name, CATEGORY_MEDIEN,
            "Medien-Kategorie bleibt erhalten und wird umbenannt"
        );
        Ok(())
    }

    #[test]
    fn welle3_legacy_lane_hilfskanaele_wandern_ins_archiv() -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual.categories.insert(
            STREET_BRAWL_CATEGORY,
            category(STREET_BRAWL_CATEGORY, "Street Brawl"),
        );
        actual
            .categories
            .insert(RANKED_CATEGORY, category(RANKED_CATEGORY, "Ranked"));
        actual
            .categories
            .insert(CHILL_CATEGORY, category(CHILL_CATEGORY, "Chill"));
        actual.categories.insert(
            DEADLOCK_ROUTER_CATEGORY,
            category(DEADLOCK_ROUTER_CATEGORY, "Deadlock Router"),
        );
        for (id, name, parent) in [
            (
                STREET_BRAWL_PANEL,
                "🚧sprach-kanal-verwalten",
                STREET_BRAWL_CATEGORY,
            ),
            (RANKED_PANEL, "🚧sprach-kanal-verwalten", RANKED_CATEGORY),
            (CHILL_PANEL, "🚧sprach-kanal-verwalten", CHILL_CATEGORY),
            (RANKED_ANLEITUNG, "🧾anleitung", RANKED_CATEGORY),
            (
                ROUTER_PANEL,
                "📖sprachkanal-verwalten",
                DEADLOCK_ROUTER_CATEGORY,
            ),
        ] {
            actual.channels.insert(id, channel(id, name, Some(parent)));
        }

        let derived = derive_desired_model(&actual)?;
        let archive_id = derived
            .desired
            .categories
            .values()
            .find(|category| category.name == CATEGORY_ARCHIV)
            .expect("archive category")
            .category_id;

        for channel_id in [
            STREET_BRAWL_PANEL,
            RANKED_PANEL,
            CHILL_PANEL,
            RANKED_ANLEITUNG,
        ] {
            assert_eq!(
                derived.desired.channels[&channel_id].parent_category_id,
                Some(archive_id),
                "legacy lane helper {channel_id} muss ins Archiv"
            );
            let overwrite = derived
                .desired
                .overwrites
                .get(&OverwriteKey {
                    channel_id,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                })
                .expect("archive deny");
            assert_eq!(overwrite.allow_bits, 0);
            assert_eq!(overwrite.deny_bits, Permissions::VIEW_CHANNEL.bits());
        }
        assert_eq!(
            derived.desired.channels[&ROUTER_PANEL].parent_category_id,
            Some(DEADLOCK_ROUTER_CATEGORY),
            "Router-Verwaltungskanal bleibt aktiv"
        );
        Ok(())
    }

    #[test]
    fn welle34b_lfg_cutover_aus_laesst_textkanal_ohne_lfg_delta() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual.channels.insert(
            MITSPIELER_SUCHE,
            channel(
                MITSPIELER_SUCHE,
                "🎯mitspieler-suche",
                Some(DEADLOCK_CATEGORY),
            ),
        );

        let derived = derive_desired_model_with_options(&actual, desired_options(false, false))?;

        let lfg = derived
            .desired
            .channels
            .get(&MITSPIELER_SUCHE)
            .expect("lfg text channel");
        assert_eq!(lfg.kind, ChannelKind::Text);
        assert_eq!(lfg.name, "🎯mitspieler-suche");
        assert_eq!(lfg.parent_category_id, Some(DEADLOCK_CATEGORY));
        assert!(derived
            .desired
            .channels
            .values()
            .all(|channel| channel.name != CHANNEL_LFG_ARCHIVE));
        assert!(!derived.desired.channels.values().any(|channel| {
            channel.kind == ChannelKind::Forum && channel.name == CHANNEL_LFG_FORUM
        }));

        let diff = diff_models(
            &derived.desired,
            &actual,
            &derived.dynamic_namespaces,
            &derived.exceptions,
        )?;
        assert!(
            !diff.changes.iter().any(|change| {
                change.object.kind == ObjectKind::Channel
                    && (change.object.object_id == MITSPIELER_SUCHE
                        || change
                            .desired
                            .as_ref()
                            .and_then(|desired| desired.get("name"))
                            == Some(&serde_json::json!(CHANNEL_LFG_FORUM))
                        || change
                            .desired
                            .as_ref()
                            .and_then(|desired| desired.get("name"))
                            == Some(&serde_json::json!(CHANNEL_LFG_ARCHIVE)))
            }),
            "Cutover aus darf kein LFG-Strukturdelta erzeugen: {:#?}",
            diff.changes
        );
        Ok(())
    }

    #[test]
    fn welle34b_lfg_forum_wird_neu_angelegt_und_altkanal_archiviert() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual.roles.insert(
            COMMUNITY_MOD_ROLE,
            role(COMMUNITY_MOD_ROLE, ROLE_COMMUNITY_MODERATOR, 0),
        );
        actual
            .roles
            .insert(BOT_ROLE, role(BOT_ROLE, ROLE_TICKET_TOOL, 0));
        actual.channels.insert(
            MITSPIELER_SUCHE,
            channel(MITSPIELER_SUCHE, "spieler-suche", Some(DEADLOCK_CATEGORY)),
        );

        let derived = derive_desired_model_with_options(&actual, desired_options(false, true))?;
        let archive_id = derived
            .desired
            .categories
            .values()
            .find(|category| category.name == CATEGORY_ARCHIV)
            .expect("archive category")
            .category_id;
        let lfg_forum = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == "🎯mitspieler-suche")
            .expect("lfg forum");

        assert_ne!(
            lfg_forum.channel_id, MITSPIELER_SUCHE,
            "das neue Forum darf nicht die ID des alten Textkanals wiederverwenden"
        );
        assert_eq!(lfg_forum.kind, ChannelKind::Forum);
        assert_eq!(lfg_forum.parent_category_id, Some(DEADLOCK_CATEGORY));
        assert_eq!(lfg_forum.default_auto_archive_duration, Some(1440));

        let old = derived
            .desired
            .channels
            .get(&MITSPIELER_SUCHE)
            .expect("old lfg text channel");
        assert_eq!(old.kind, ChannelKind::Text);
        assert_eq!(old.name, "archiv-mitspieler-suche");
        assert_eq!(old.parent_category_id, Some(archive_id));
        let archive_overwrite = derived
            .desired
            .overwrites
            .get(&OverwriteKey {
                channel_id: MITSPIELER_SUCHE,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            })
            .expect("old lfg archive deny");
        assert_eq!(archive_overwrite.allow_bits, 0);
        assert_eq!(
            archive_overwrite.deny_bits,
            Permissions::VIEW_CHANNEL.bits()
        );

        let diff = diff_models(
            &derived.desired,
            &actual,
            &derived.dynamic_namespaces,
            &derived.exceptions,
        )?;
        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Create
                && change.object.kind == ObjectKind::Channel
                && change
                    .desired
                    .as_ref()
                    .and_then(|desired| desired.get("kind"))
                    == Some(&serde_json::json!("forum"))
        }));
        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Update
                && change.object.kind == ObjectKind::Channel
                && change.object.object_id == MITSPIELER_SUCHE
                && change
                    .fields
                    .iter()
                    .any(|field| field.field == "parent_category_id")
        }));
        assert!(
            !diff.changes.iter().any(|change| {
                change.action == crate::diff::DiffAction::Update
                    && change.object.kind == ObjectKind::Channel
                    && change.object.object_id == MITSPIELER_SUCHE
                    && change.fields.iter().any(|field| field.field == "kind")
            }),
            "der alte Textkanal darf niemals als Text->Forum-Typupdate geplant werden: {:#?}",
            diff.changes
        );
        Ok(())
    }

    #[test]
    fn welle34b_lfg_cutover_mit_exakter_namenskollision_erstellt_forum_und_archiviert_text(
    ) -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual.channels.insert(
            MITSPIELER_SUCHE,
            channel(
                MITSPIELER_SUCHE,
                "🎯mitspieler-suche",
                Some(DEADLOCK_CATEGORY),
            ),
        );

        let derived = derive_desired_model_with_options(&actual, desired_options(false, true))?;
        let archive_id = derived
            .desired
            .categories
            .values()
            .find(|category| category.name == CATEGORY_ARCHIV)
            .expect("archive category")
            .category_id;
        let old = derived
            .desired
            .channels
            .get(&MITSPIELER_SUCHE)
            .expect("old lfg text channel");
        assert_eq!(old.kind, ChannelKind::Text);
        assert_eq!(old.name, CHANNEL_LFG_ARCHIVE);
        assert_eq!(old.parent_category_id, Some(archive_id));
        let lfg_forum = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == CHANNEL_LFG_FORUM && channel.kind == ChannelKind::Forum)
            .expect("lfg forum");
        assert_ne!(lfg_forum.channel_id, MITSPIELER_SUCHE);

        let diff = diff_models(
            &derived.desired,
            &actual,
            &derived.dynamic_namespaces,
            &derived.exceptions,
        )?;
        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Create
                && change.object.kind == ObjectKind::Channel
                && change
                    .desired
                    .as_ref()
                    .and_then(|desired| desired.get("kind"))
                    == Some(&serde_json::json!("forum"))
        }));
        assert!(diff.changes.iter().any(|change| {
            change.action == crate::diff::DiffAction::Update
                && change.object.kind == ObjectKind::Channel
                && change.object.object_id == MITSPIELER_SUCHE
                && change.fields.iter().any(|field| field.field == "name")
        }));
        assert!(
            !diff.changes.iter().any(|change| {
                change.action == crate::diff::DiffAction::Update
                    && change.object.kind == ObjectKind::Channel
                    && change.object.object_id == MITSPIELER_SUCHE
                    && change.fields.iter().any(|field| field.field == "kind")
            }),
            "exakte Namenskollision darf kein Text->Forum-Typupdate planen: {:#?}",
            diff.changes
        );
        Ok(())
    }

    #[test]
    fn welle34b_lfg_forum_rechte_blocken_freie_posts_aber_erlauben_antworten() -> anyhow::Result<()>
    {
        let mut actual = documented_categories_model();
        actual.roles.insert(
            COMMUNITY_MOD_ROLE,
            role(COMMUNITY_MOD_ROLE, ROLE_COMMUNITY_MODERATOR, 0),
        );
        actual
            .roles
            .insert(BOT_ROLE, role(BOT_ROLE, ROLE_TICKET_TOOL, 0));
        actual.channels.insert(
            MITSPIELER_SUCHE,
            channel(MITSPIELER_SUCHE, "spieler-suche", Some(DEADLOCK_CATEGORY)),
        );

        let derived = derive_desired_model_with_options(&actual, desired_options(false, true))?;
        let lfg_forum = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == "🎯mitspieler-suche")
            .expect("lfg forum");
        let everyone = derived
            .desired
            .overwrites
            .get(&OverwriteKey {
                channel_id: lfg_forum.channel_id,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            })
            .expect("@everyone overwrite");
        let everyone_allow = Permissions::from_bits_truncate(everyone.allow_bits);
        let everyone_deny = Permissions::from_bits_truncate(everyone.deny_bits);
        assert!(everyone_allow.contains(Permissions::SEND_MESSAGES_IN_THREADS));
        assert!(everyone_deny.contains(Permissions::SEND_MESSAGES));
        assert!(everyone_deny.contains(Permissions::CREATE_PUBLIC_THREADS));
        assert!(everyone_deny.contains(Permissions::CREATE_PRIVATE_THREADS));
        assert!(!everyone_deny.contains(Permissions::SEND_MESSAGES_IN_THREADS));

        for (role_id, role_name) in [
            (COMMUNITY_MOD_ROLE, ROLE_COMMUNITY_MODERATOR),
            (BOT_ROLE, ROLE_TICKET_TOOL),
        ] {
            let overwrite = derived
                .desired
                .overwrites
                .get(&OverwriteKey {
                    channel_id: lfg_forum.channel_id,
                    target_kind: TargetKind::Role,
                    target_id: role_id,
                })
                .unwrap_or_else(|| panic!("{role_name} overwrite"));
            let allow = Permissions::from_bits_truncate(overwrite.allow_bits);
            assert!(allow.contains(Permissions::SEND_MESSAGES));
            assert!(allow.contains(Permissions::CREATE_PUBLIC_THREADS));
            assert!(allow.contains(Permissions::MANAGE_THREADS));
        }
        Ok(())
    }

    #[test]
    fn welle3_static_channels_und_trennernamen_werden_kodiert() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual
            .roles
            .insert(COACH_ROLE, role(COACH_ROLE, ROLE_COACH, 0));
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: COACHING_CATEGORY,
                target_kind: TargetKind::Role,
                target_id: GUILD_ID,
            },
            overwrite(
                COACHING_CATEGORY,
                TargetKind::Role,
                GUILD_ID,
                0,
                Permissions::VIEW_CHANNEL.bits(),
            ),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: COACHING_CATEGORY,
                target_kind: TargetKind::Role,
                target_id: COACH_ROLE,
            },
            overwrite(
                COACHING_CATEGORY,
                TargetKind::Role,
                COACH_ROLE,
                (Permissions::VIEW_CHANNEL | Permissions::MANAGE_CHANNELS).bits(),
                0,
            ),
        );

        let derived = derive_desired_model(&actual)?;

        assert_eq!(
            derived.desired.categories[&EINGANGSBEREICH_CATEGORY].name,
            CATEGORY_INFORMATION
        );
        assert_eq!(
            derived.desired.categories[&MEDIEN_CATEGORY].name,
            CATEGORY_MEDIEN
        );
        assert_eq!(
            derived.desired.categories[&CHAT_CATEGORY].name,
            CATEGORY_COMMUNITY
        );
        assert_eq!(
            derived.desired.categories[&DEADLOCK_CATEGORY].name,
            CATEGORY_DEADLOCK
        );
        assert_eq!(
            derived.desired.categories[&COACHING_CATEGORY].name,
            CATEGORY_COACHING
        );

        let willkommen = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == "🧭willkommen")
            .expect("willkommen");
        assert_eq!(
            willkommen.parent_category_id,
            Some(EINGANGSBEREICH_CATEGORY)
        );
        assert_eq!(willkommen.topic.as_deref(), Some(TOPIC_WILLKOMMEN));
        assert_eq!(
            derived
                .desired
                .overwrites
                .get(&OverwriteKey {
                    channel_id: willkommen.channel_id,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                })
                .map(|overwrite| (overwrite.allow_bits, overwrite.deny_bits)),
            Some((
                p1_announcement_profile().allow_bits,
                p1_announcement_profile().deny_bits
            ))
        );

        let scrim = derived
            .desired
            .channels
            .values()
            .find(|channel| channel.name == "🗓️scrim-planung")
            .expect("scrim-planung");
        assert_eq!(scrim.parent_category_id, Some(COACHING_CATEGORY));
        assert_eq!(scrim.topic.as_deref(), Some(TOPIC_SCRIM_PLANUNG));
        assert_eq!(
            derived
                .desired
                .overwrites
                .get(&OverwriteKey {
                    channel_id: scrim.channel_id,
                    target_kind: TargetKind::Role,
                    target_id: GUILD_ID,
                })
                .map(|overwrite| overwrite.deny_bits),
            Some(Permissions::VIEW_CHANNEL.bits())
        );
        assert_eq!(
            derived
                .desired
                .overwrites
                .get(&OverwriteKey {
                    channel_id: scrim.channel_id,
                    target_kind: TargetKind::Role,
                    target_id: COACH_ROLE,
                })
                .map(|overwrite| overwrite.allow_bits),
            Some((Permissions::VIEW_CHANNEL | Permissions::MANAGE_CHANNELS).bits())
        );
        Ok(())
    }

    #[test]
    fn coaching_normalisiert_admin_user_replacements_und_x3() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual.roles.insert(
            ADMIN_ROLE,
            role(ADMIN_ROLE, "Moderator", Permissions::ADMINISTRATOR.bits()),
        );
        actual
            .roles
            .insert(TEAM_LEO_ROLE, role(TEAM_LEO_ROLE, "Team Leo", 0));
        actual
            .roles
            .insert(COACH_ROLE, role(COACH_ROLE, ROLE_COACH, 0));
        let category_mod_overwrite = overwrite(
            COACHING_CATEGORY,
            TargetKind::Role,
            ADMIN_ROLE,
            Permissions::MANAGE_CHANNELS.bits(),
            0,
        );
        let category_coach_overwrite = overwrite(
            COACHING_CATEGORY,
            TargetKind::Role,
            COACH_ROLE,
            (Permissions::MANAGE_CHANNELS | Permissions::MOVE_MEMBERS).bits(),
            0,
        );
        actual.overwrites.insert(
            category_mod_overwrite.key.clone(),
            category_mod_overwrite.clone(),
        );
        actual.overwrites.insert(
            category_coach_overwrite.key.clone(),
            category_coach_overwrite.clone(),
        );
        actual.channels.insert(
            COACH_CHAT,
            channel(COACH_CHAT, "coach-chat", Some(COACHING_CATEGORY)),
        );
        actual.channels.insert(
            TEAM_LEO,
            channel(TEAM_LEO, "leo-team", Some(COACHING_CATEGORY)),
        );

        let full_deny = Permissions::all().bits();
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: COACH_CHAT,
                target_kind: TargetKind::Role,
                target_id: ADMIN_ROLE,
            },
            overwrite(COACH_CHAT, TargetKind::Role, ADMIN_ROLE, 0, full_deny),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: COACH_CHAT,
                target_kind: TargetKind::Member,
                target_id: COACH_CHAT_USER_ID,
            },
            overwrite(
                COACH_CHAT,
                TargetKind::Member,
                COACH_CHAT_USER_ID,
                full_deny,
                0,
            ),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: TEAM_LEO,
                target_kind: TargetKind::Member,
                target_id: TEAM_CAPTAIN_USER_IDS[0],
            },
            overwrite(
                TEAM_LEO,
                TargetKind::Member,
                TEAM_CAPTAIN_USER_IDS[0],
                (Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES).bits(),
                0,
            ),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: TEAM_LEO,
                target_kind: TargetKind::Member,
                target_id: USER_BAN_X3_COACHING_VIEW_ONLY,
            },
            overwrite(
                TEAM_LEO,
                TargetKind::Member,
                USER_BAN_X3_COACHING_VIEW_ONLY,
                0,
                full_deny,
            ),
        );

        let derived = derive_desired_model(&actual)?;

        assert_eq!(
            derived.desired.overwrites.get(&category_mod_overwrite.key),
            Some(&category_mod_overwrite)
        );
        assert_eq!(
            derived
                .desired
                .overwrites
                .get(&category_coach_overwrite.key),
            Some(&category_coach_overwrite)
        );
        assert!(!derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: COACH_CHAT,
            target_kind: TargetKind::Role,
            target_id: ADMIN_ROLE,
        }));
        assert!(!derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: COACH_CHAT,
            target_kind: TargetKind::Member,
            target_id: COACH_CHAT_USER_ID,
        }));
        assert!(derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: COACH_CHAT,
            target_kind: TargetKind::Role,
            target_id: COACH_ROLE,
        }));
        assert!(!derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: TEAM_LEO,
            target_kind: TargetKind::Member,
            target_id: TEAM_CAPTAIN_USER_IDS[0],
        }));
        assert!(derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: TEAM_LEO,
            target_kind: TargetKind::Role,
            target_id: TEAM_LEO_ROLE,
        }));
        assert!(!derived.desired.overwrites.contains_key(&OverwriteKey {
            channel_id: TEAM_LEO,
            target_kind: TargetKind::Member,
            target_id: USER_BAN_X3_COACHING_VIEW_ONLY,
        }));
        assert_eq!(
            derived
                .desired
                .overwrites
                .get(&OverwriteKey {
                    channel_id: COACHING_CATEGORY,
                    target_kind: TargetKind::Member,
                    target_id: USER_BAN_X3_COACHING_VIEW_ONLY,
                })
                .map(|overwrite| (overwrite.allow_bits, overwrite.deny_bits)),
            Some((0, Permissions::VIEW_CHANNEL.bits()))
        );
        assert!(derived.exceptions.iter().any(|exception| {
            exception.target_id == Some(USER_BAN_X3_COACHING_VIEW_ONLY)
                && exception.channel_id == Some(COACHING_CATEGORY)
                && exception.deny_bits == Some(Permissions::VIEW_CHANNEL.bits())
        }));
        Ok(())
    }

    #[test]
    fn sammelpunkt_funny_grind_custom_ping_overwrites_bleiben_im_soll() -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual.categories.insert(
            CUSTOM_GAME_CATEGORY,
            category(CUSTOM_GAME_CATEGORY, "Custom Game"),
        );
        actual.channels.insert(
            SAMMELPUNKT,
            channel(SAMMELPUNKT, "Sammelpunkt", Some(CUSTOM_GAME_CATEGORY)),
        );
        actual.roles.insert(
            FUNNY_CUSTOM_PING_ROLE,
            role(FUNNY_CUSTOM_PING_ROLE, "Funny Custom Ping", 0),
        );
        actual.roles.insert(
            GRIND_CUSTOM_PING_ROLE,
            role(GRIND_CUSTOM_PING_ROLE, "Grind Custom Ping", 0),
        );
        let funny = overwrite(
            SAMMELPUNKT,
            TargetKind::Role,
            FUNNY_CUSTOM_PING_ROLE,
            Permissions::CONNECT.bits(),
            0,
        );
        let grind = overwrite(
            SAMMELPUNKT,
            TargetKind::Role,
            GRIND_CUSTOM_PING_ROLE,
            Permissions::CONNECT.bits(),
            0,
        );
        actual.overwrites.insert(funny.key.clone(), funny.clone());
        actual.overwrites.insert(grind.key.clone(), grind.clone());

        let derived = derive_desired_model(&actual)?;

        assert_eq!(derived.desired.overwrites.get(&funny.key), Some(&funny));
        assert_eq!(derived.desired.overwrites.get(&grind.key), Some(&grind));
        Ok(())
    }

    #[test]
    fn coaching_bewahrt_user_overwrite_bei_fehlender_ersatzrolle() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual.channels.insert(
            TEAM_LEO,
            channel(TEAM_LEO, "leo-team", Some(COACHING_CATEGORY)),
        );
        let key = OverwriteKey {
            channel_id: TEAM_LEO,
            target_kind: TargetKind::Member,
            target_id: TEAM_CAPTAIN_USER_IDS[0],
        };
        let original = overwrite(
            TEAM_LEO,
            TargetKind::Member,
            TEAM_CAPTAIN_USER_IDS[0],
            0b1010,
            0b0101,
        );
        actual.overwrites.insert(key.clone(), original.clone());

        let derived = derive_desired_model(&actual)?;

        assert_eq!(derived.desired.overwrites.get(&key), Some(&original));
        assert!(derived.warnings.iter().any(|warning| {
            warning.contains("Team Leo") && warning.contains("Ersatz nicht möglich, manuell klären")
        }));
        Ok(())
    }

    #[test]
    fn unbekannte_kategorie_bleibt_unveraendert_und_warnt() -> anyhow::Result<()> {
        let mut actual = actual_model();
        actual.categories.insert(
            UNKNOWN_CATEGORY,
            category(UNKNOWN_CATEGORY, "Undokumentiert"),
        );
        actual.channels.insert(
            UNKNOWN_CHANNEL,
            channel(UNKNOWN_CHANNEL, "custom", Some(UNKNOWN_CATEGORY)),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: UNKNOWN_CHANNEL,
                target_kind: TargetKind::Role,
                target_id: 999,
            },
            overwrite(UNKNOWN_CHANNEL, TargetKind::Role, 999, 7, 11),
        );

        let derived = derive_desired_model(&actual)?;

        assert!(derived
            .warnings
            .iter()
            .any(|warning| warning.contains("Undokumentiert")));
        assert_eq!(
            derived.desired.overwrites.get(&OverwriteKey {
                channel_id: UNKNOWN_CHANNEL,
                target_kind: TargetKind::Role,
                target_id: 999,
            }),
            actual.overwrites.get(&OverwriteKey {
                channel_id: UNKNOWN_CHANNEL,
                target_kind: TargetKind::Role,
                target_id: 999,
            })
        );
        Ok(())
    }

    #[test]
    fn transformation_entfernt_keine_strukturobjekte() -> anyhow::Result<()> {
        let mut actual = documented_categories_model();
        actual.channels.insert(
            RANG_AUSWAHL,
            channel(RANG_AUSWAHL, "rang-auswahl", Some(CHAT_CATEGORY)),
        );
        actual.channels.insert(
            BOT_MESSAGE_CHANNEL,
            channel(BOT_MESSAGE_CHANNEL, "bot-panel", Some(CHAT_CATEGORY)),
        );
        actual.bot_messages.insert(
            (BOT_MESSAGE_CHANNEL, "panel-main".to_string()),
            BotMessageSpec {
                guild_id: GUILD_ID,
                channel_id: BOT_MESSAGE_CHANNEL,
                message_key: "panel-main".to_string(),
                message_kind: "panel".to_string(),
                message_id: Some(42),
                content_hash: Some("hash-a".to_string()),
            },
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: GENERAL,
                target_kind: TargetKind::Member,
                target_id: BANNED_X2,
            },
            overwrite(GENERAL, TargetKind::Member, BANNED_X2, 0, 123),
        );
        actual.overwrites.insert(
            OverwriteKey {
                channel_id: COACHING_CATEGORY,
                target_kind: TargetKind::Member,
                target_id: BANNED_X4,
            },
            overwrite(COACHING_CATEGORY, TargetKind::Member, BANNED_X4, 0, 456),
        );

        let derived = derive_desired_model(&actual)?;

        let expected_categories = actual.categories.keys().collect::<BTreeSet<_>>();
        assert_eq!(
            derived.desired.categories.keys().collect::<BTreeSet<_>>(),
            expected_categories
        );
        for channel_id in actual.channels.keys() {
            assert!(
                derived.desired.channels.contains_key(channel_id),
                "bestehender Kanal {channel_id} muss erhalten bleiben"
            );
        }
        for channel_name in ["🧭willkommen", "🎨kreativ-ecke", "🗓️scrim-planung"] {
            assert!(
                derived
                    .desired
                    .channels
                    .values()
                    .any(|channel| channel.name == channel_name),
                "W3.1-Kanal {channel_name} muss im Soll-Modell existieren"
            );
        }
        let actual_role_names = actual
            .roles
            .values()
            .map(|role| role.name.as_str())
            .collect::<BTreeSet<_>>();
        let desired_role_names = derived
            .desired
            .roles
            .values()
            .map(|role| role.name.as_str())
            .collect::<BTreeSet<_>>();
        for role_name in actual_role_names {
            assert!(
                desired_role_names.contains(role_name),
                "bestehende Rolle {role_name} muss erhalten bleiben"
            );
        }
        for role_name in ["Invite-Gast", "Frischling", "Streams"] {
            assert!(
                desired_role_names.contains(role_name),
                "Welle2b-Rolle {role_name} muss im Soll-Modell existieren"
            );
        }
        assert_eq!(
            derived.desired.bot_messages.keys().collect::<BTreeSet<_>>(),
            actual.bot_messages.keys().collect::<BTreeSet<_>>()
        );
        for (key, actual_message) in &actual.bot_messages {
            assert_eq!(
                serde_json::to_vec(derived.desired.bot_messages.get(key).expect("message"))?,
                serde_json::to_vec(actual_message)?
            );
        }
        for (channel_id, actual_channel) in &actual.channels {
            if *channel_id == RANG_AUSWAHL {
                continue;
            }
            assert_eq!(
                derived
                    .desired
                    .channels
                    .get(channel_id)
                    .map(|channel| channel.parent_category_id),
                Some(actual_channel.parent_category_id),
                "Parent von nicht umgezogenem Kanal {channel_id} muss erhalten bleiben"
            );
        }
        for key in [
            OverwriteKey {
                channel_id: GENERAL,
                target_kind: TargetKind::Member,
                target_id: BANNED_X2,
            },
            OverwriteKey {
                channel_id: COACHING_CATEGORY,
                target_kind: TargetKind::Member,
                target_id: BANNED_X4,
            },
        ] {
            assert_eq!(
                serde_json::to_vec(derived.desired.overwrites.get(&key).expect("desired"))?,
                serde_json::to_vec(actual.overwrites.get(&key).expect("actual"))?
            );
        }
        Ok(())
    }
}
