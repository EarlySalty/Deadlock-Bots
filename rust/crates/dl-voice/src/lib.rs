//! Voice-Cluster — Phase 4 des Rewrites.
//!
//! Im Python-Original hören fünf Cogs unkoordiniert auf
//! `voice_state_update`; hier sind alle Teilsysteme Subscriber des EINEN
//! Voice-Dispatchers aus dl-discord.
//!
//! - [`tracker`] (4a): Session-Tracking → `voice_stats` + `voice_session_log`
//!   (die Datenbasis für Public-Stats, LFG, Retention).
//! - TempVoice, Rank-Lanes, Voice-Status, Reaction-DM, Steam-Nudge folgen
//!   in 4b/4c.

pub mod adaptive;
mod db;
pub mod feedback;
pub mod glue;
pub mod nudge;
pub mod rank;
pub mod rename_queue;
pub mod router;
pub mod stats;
pub mod status;
pub mod tempvoice;
pub mod tracker;
