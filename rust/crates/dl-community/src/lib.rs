//! dl-community — Phase 6/7-Dienste.
//!
//! - [`tags`] — Tag-System (Single Source of Truth für User-/Mod-Tags);
//!   versorgt TempVoice-Tag-Filter und Moderation (Ragebaiter).
//! - FAQ, Clips, Leave-Survey, Bug-Reporter folgen.

pub mod clips;
pub mod coaching;
pub mod coaching_requests;
mod db;
pub mod feedback_hub;
pub mod invite_lounge;
pub mod invites;
pub mod knowledge_client;
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
