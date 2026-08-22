//! Gemeinsame Guild-Konstanten.
//!
//! Der alte Wizard, die AI-Tour und die Welcome-DM-Flows sind entfernt.
//! Zugang läuft über Discords natives Onboarding. Fragen beantwortet der
//! Concierge. Diese Datei hält nur noch IDs, die Journey und Server-Sync
//! brauchen.

pub const MAIN_GUILD_ID: u64 = 1289721245281292288;
pub const RULES_CHANNEL_ID: u64 = 1315684135175716975;
pub const ONBOARD_COMPLETE_ROLE_ID: u64 = 1304216250649415771;

pub const ROLE_STREAMER_ONBOARD_ID: u64 = 1468365558293598268;
pub const ROLE_STREAMER_PARTNER_ID: u64 = 1411798947936342097;
pub const ROLE_LFG_PING_ID: u64 = 1407086020331311144;
pub const ROLE_CUSTOM_GAMES_PING_ID: u64 = 1407085699374649364;
pub const ROLE_PATCHNOTES_PING_ID: u64 = 1330994309524357140;
pub const ROLE_RANKED_ID: u64 = 1420466763262591120;
pub const ROLE_CASUAL_ID: u64 = 1420466468746690621;

/// Hinweis für übrig gebliebene Alt-Buttons in Discord.
pub const LEGACY_ONBOARDING_RETIRED_HINT: &str =
    "Der alte Einstieg ist aus. Schreib mir einfach eine Nachricht, ich helfe dir weiter.";
