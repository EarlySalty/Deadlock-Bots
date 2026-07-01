//! dl-community — Phase 6/7-Dienste.
//!
//! - [`tags`] — Tag-System (Single Source of Truth für User-/Mod-Tags);
//!   versorgt TempVoice-Tag-Filter, Moderation (Ragebaiter) und Onboarding.
//! - FAQ, Clips, Leave-Survey, Bug-Reporter folgen.

pub mod ai_onboarding;
pub mod clips;
pub mod coaching;
pub mod coaching_requests;
mod db;
pub mod dm_assistant;
pub mod faq;
pub mod feedback_hub;
pub mod invites;
pub mod leave_survey;
pub mod onboarding;
pub mod privacy;
pub mod privacy_ui;
pub mod reaction_roles;
pub mod retention;
pub mod tags;
pub mod tags_ui;
