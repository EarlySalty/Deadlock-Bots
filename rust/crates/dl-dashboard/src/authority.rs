//! Mitglieds-Zugriffsprüfung für den Dashboard-Login.
//!
//! `dl-web` hat keine Discord-Gateway-Verbindung. Pythons
//! `_check_discord_member_access` liest aber den Member-Cache des Bots — in
//! Rust geht das über den Master-Broker (:8770). Diese Schicht trennt sauber:
//! - [`MemberLookup`] holt die rohen Fakten (Admin-Permission, Rollen) — per
//!   Broker in Produktion, per Mock im Test.
//! - [`decide_access`] ist die **reine** Entscheidung (Owner/Admin/Rolle →
//!   Zugriffsstufe), ohne Netzwerk und damit vollständig testbar.

use std::time::Duration;

use serde_json::Value;

use crate::config::{AccessLevel, DashboardConfig};

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Rohe Zugriffs-Fakten eines Mitglieds (wie der Broker sie liefert).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberAccessInfo {
    pub found: bool,
    pub display_name: Option<String>,
    pub is_administrator: bool,
    pub role_ids: Vec<u64>,
}

/// Schlägt den Zugriffsstatus eines Mitglieds nach.
#[async_trait::async_trait]
pub trait MemberLookup: Send + Sync {
    /// `None` bedeutet **Lookup nicht möglich** (Broker nicht erreichbar) —
    /// nicht zu verwechseln mit „gefunden, aber ohne Rechte" (`found=false`).
    async fn member_access(&self, guild_id: Option<u64>, user_id: u64) -> Option<MemberAccessInfo>;
}

/// Ergebnis der Zugriffsentscheidung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessOutcome {
    /// `None` = kein Zugriff.
    pub level: Option<AccessLevel>,
    /// Begründung (Audit/Session-Reason), wortgleich zum Original.
    pub reason: &'static str,
}

impl AccessOutcome {
    fn granted(level: AccessLevel, reason: &'static str) -> Self {
        Self {
            level: Some(level),
            reason,
        }
    }

    pub fn is_granted(&self) -> bool {
        self.level.is_some()
    }
}

/// Reine Zugriffsentscheidung — Reihenfolge wie im Original:
/// Owner → Admin-Permission → Moderator-Rolle → Dashboard-Zugriffsrollen
/// (alle Voll-Zugriff) → sonst kein Zugriff.
pub fn decide_access(
    cfg: &DashboardConfig,
    user_id: u64,
    info: &MemberAccessInfo,
) -> AccessOutcome {
    if user_id == cfg.owner_user_id {
        return AccessOutcome::granted(AccessLevel::Full, "owner_override");
    }
    if info.is_administrator {
        return AccessOutcome::granted(AccessLevel::Full, "guild_admin");
    }
    if info.role_ids.contains(&cfg.moderator_role_id) {
        return AccessOutcome::granted(AccessLevel::Full, "moderator_role");
    }
    if info
        .role_ids
        .iter()
        .any(|role| cfg.access_role_ids.contains(role))
    {
        return AccessOutcome::granted(AccessLevel::Full, "dashboard_access_role");
    }
    AccessOutcome {
        level: None,
        reason: "missing_admin_or_moderator_role",
    }
}

/// Broker-gestützter Lookup (`GET /internal/master/v1/discord/member-access`).
#[derive(Clone)]
pub struct BrokerMemberLookup {
    http: reqwest::Client,
    base: String,
}

impl BrokerMemberLookup {
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("reqwest-Client bauen");
        Self {
            http,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait::async_trait]
impl MemberLookup for BrokerMemberLookup {
    async fn member_access(&self, guild_id: Option<u64>, user_id: u64) -> Option<MemberAccessInfo> {
        let url = format!("{}/internal/master/v1/discord/member-access", self.base);
        let mut query = vec![("user_id".to_string(), user_id.to_string())];
        if let Some(gid) = guild_id {
            query.push(("guild_id".to_string(), gid.to_string()));
        }
        let response = match self.http.get(&url).query(&query).send().await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(%err, "Broker member-access nicht erreichbar");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "Broker member-access Fehler");
            return None;
        }
        let data: Value = response.json().await.ok()?;
        Some(MemberAccessInfo {
            found: data.get("found").and_then(Value::as_bool).unwrap_or(false),
            display_name: data
                .get("display_name")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|s| !s.is_empty()),
            is_administrator: data
                .get("is_administrator")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            role_ids: data
                .get("role_ids")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()))
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DashboardConfig {
        DashboardConfig::from_lookup(|_| None)
    }

    fn info(is_admin: bool, roles: &[u64]) -> MemberAccessInfo {
        MemberAccessInfo {
            found: true,
            display_name: Some("X".into()),
            is_administrator: is_admin,
            role_ids: roles.to_vec(),
        }
    }

    #[test]
    fn owner_bekommt_voll_ohne_rollen() {
        let cfg = cfg();
        let out = decide_access(&cfg, cfg.owner_user_id, &MemberAccessInfo::default());
        assert_eq!(out.level, Some(AccessLevel::Full));
        assert_eq!(out.reason, "owner_override");
    }

    #[test]
    fn admin_permission_bekommt_voll() {
        let cfg = cfg();
        let out = decide_access(&cfg, 999, &info(true, &[]));
        assert_eq!(out.level, Some(AccessLevel::Full));
        assert_eq!(out.reason, "guild_admin");
    }

    #[test]
    fn moderator_rolle_bekommt_voll() {
        let cfg = cfg();
        let out = decide_access(&cfg, 999, &info(false, &[cfg.moderator_role_id]));
        assert_eq!(out.level, Some(AccessLevel::Full));
        assert_eq!(out.reason, "moderator_role");
    }

    #[test]
    fn dashboard_zugriffsrolle_bekommt_voll_ohne_moderator() {
        let cfg = cfg();
        for role in &cfg.access_role_ids {
            let out = decide_access(&cfg, 999, &info(false, &[*role]));
            assert_eq!(out.level, Some(AccessLevel::Full), "Rolle {role}");
            assert_eq!(out.reason, "dashboard_access_role");
        }
    }

    #[test]
    fn fremder_ohne_rolle_wird_abgelehnt() {
        let cfg = cfg();
        let out = decide_access(&cfg, 999, &info(false, &[123, 456]));
        assert!(!out.is_granted());
        assert_eq!(out.reason, "missing_admin_or_moderator_role");
    }

    #[test]
    fn moderator_rolle_schlaegt_fremde_rollen() {
        let cfg = cfg();
        let out = decide_access(&cfg, 999, &info(false, &[cfg.moderator_role_id, 123]));
        assert_eq!(out.level, Some(AccessLevel::Full));
    }
}
