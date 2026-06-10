//! dl-activity — Phase 5 des Rewrites.
//!
//! - [`analyzer`] — Aktivitätsmuster + Co-Spieler-Graph aus
//!   `voice_session_log` (Datenbasis für LFG, Retention, Stats).
//! - LFG, player_finder (Flag aus) und Retention folgen.

pub mod analyzer;
pub mod glue;
pub mod lfg;
