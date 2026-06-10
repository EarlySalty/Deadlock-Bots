//! Brücken zu den Schwester-Bots.
//!
//! - [`steam`]: der dünne Discord-Arm des Rust-Steam-Bots — Port von
//!   `cogs/steam_bridge.py`. Wire-Vertrag: `POST {STEAM_BOT_API_URL}/events/discord`
//!   mit kinds `interaction`/`slash_command`/`member_remove`/`admin_command`.
//! - `twitch` (folgt in Phase 3b): live_bridge + streamer_link_matcher.

pub mod glue;
pub mod matcher;
pub mod steam;
pub mod twitch;
