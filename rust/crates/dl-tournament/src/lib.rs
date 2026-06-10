//! dl-tournament — Phase 8 des Rewrites.
//!
//! - [`balancer`] — Team-Aufteilung nach Rang-Scores (Port von
//!   `cogs/deadlock_team_balancer.py`).
//! - Custom-Games-Flow, Turnier-Store und Turnier-Web (8767) folgen.

pub mod balancer;
pub mod store;
pub mod web;
