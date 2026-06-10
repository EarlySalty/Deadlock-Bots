//! TempVoice — Join-to-create-Lanes mit Owner-Lifecycle.
//!
//! Port des Verhaltens-Kerns von `cogs/tempvoice/` (core.py 2785 Zeilen):
//! - [`logic`] — pure Rang-/Namens-Logik (CPython-Referenzwerte in Tests)
//! - [`store`] — Bestands-Tabellen (lanes, bans, presets, prefs)
//! - [`engine`] — Event-Verarbeitung hinter dem [`engine::LanePort`]-Trait

pub mod engine;
pub mod logic;
pub mod store;

pub use engine::{LanePort, StagingRules, TempVoiceConfig, TempVoiceEngine};
pub use store::TempVoiceStore;
