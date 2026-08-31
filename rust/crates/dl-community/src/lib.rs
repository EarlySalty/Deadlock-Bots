//! dl-community — Phase 6/7-Dienste.
//!
//! - [`tags`] — Tag-System (Single Source of Truth für User-/Mod-Tags);
//!   versorgt TempVoice-Tag-Filter, Moderation (Ragebaiter) und Concierge.
//! - FAQ, Clips, Leave-Survey, Bug-Reporter folgen.

pub mod clips;
pub mod coaching;
pub mod coaching_requests;
pub mod concierge;
mod db;
pub mod faq;
pub mod feedback_hub;
pub mod invite_lounge;
pub mod invites;
mod knowledge_client;
pub mod leave_survey;
pub mod onboarding;
pub mod privacy;
pub mod privacy_ui;
pub mod reaction_roles;
pub mod retention;
pub mod scrim_signup;
pub mod tags;
pub mod tags_ui;
pub mod team_applications;
pub mod voice_change_hint;
