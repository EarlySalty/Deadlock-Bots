use super::{discord_call, DiscordHttpError, McpState};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};

pub(crate) const VIEW: u64 = 1 << 10;
pub(crate) const HISTORY: u64 = 1 << 16;
pub(crate) const ADMIN: u64 = 1 << 3;
const SEND: u64 = 1 << 11;
const SEND_DEPENDENT: u64 = (1 << 12) | (1 << 14) | (1 << 15) | (1 << 17);

#[derive(Clone, Debug)]
pub(crate) struct Access {
    pub user_id: Option<u64>,
    pub guild: String,
    pub roles: Vec<String>,
    pub base: u64,
    pub timed_out: bool,
    pub top_position: i64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct RoleInfo {
    pub bits: u64,
    pub position: Option<i64>,
}

#[derive(Clone, Debug)]
pub(crate) struct GuildRoles {
    pub guild: u64,
    pub verified: u64,
    pub everyone: u64,
    pub roles: BTreeMap<String, RoleInfo>,
}

impl GuildRoles {
    pub fn target_rank<'a>(&self, roles: impl IntoIterator<Item = &'a String>) -> i64 {
        roles
            .into_iter()
            .map(|role| {
                self.roles
                    .get(role)
                    .and_then(|info| info.position)
                    .unwrap_or(i64::MAX)
            })
            .max()
            .unwrap_or_default()
    }
}

pub(crate) enum MemberLookup {
    Found(Value),
    Missing,
}

pub(crate) fn guild_source(st: &McpState) -> Result<(u64, u64)> {
    let (_, _, guild_id) = st.public_source.as_ref().context("Gateway-Quelle fehlt")?;
    let verified = st
        .public_policy
        .mcp_verified_role_id
        .filter(|id| *id != 0 && *id != *guild_id)
        .context("Mitgliederrolle ist nicht konfiguriert")?;
    Ok((*guild_id, verified))
}

pub(crate) fn parse_roles(guild: u64, verified: u64, raw: &Value) -> Result<GuildRoles> {
    let mut roles = BTreeMap::new();
    for role in raw.as_array().context("Rollenformat ist unbekannt")? {
        let id = role["id"].as_str().context("Rollen-ID fehlt")?.to_owned();
        let bits = role["permissions"]
            .as_str()
            .context("Rollenrechte fehlen")?
            .parse::<u64>()?;
        let position = role["position"].as_i64();
        if roles.insert(id, RoleInfo { bits, position }).is_some() {
            bail!("Rollen-Zulassung ist uneindeutig");
        }
    }
    let everyone = roles
        .get(&guild.to_string())
        .context("everyone-Rolle fehlt")?
        .bits;
    let verified_bits = roles
        .get(&verified.to_string())
        .context("Mitgliederrolle ist unbekannt")?
        .bits;
    if verified_bits & ADMIN != 0 {
        bail!("Mitgliederrolle ist nicht eindeutig zulässig");
    }
    Ok(GuildRoles {
        guild,
        verified,
        everyone,
        roles,
    })
}

pub(crate) async fn guild_roles(st: &McpState) -> Result<GuildRoles> {
    let (guild, verified) = guild_source(st)?;
    let raw = discord_call(
        st,
        "GET",
        &format!("/guilds/{guild}/roles"),
        &[],
        None,
        None,
    )
    .await?;
    parse_roles(guild, verified, &raw)
}

pub(crate) async fn fetch_member(st: &McpState, guild: u64, user: u64) -> Result<MemberLookup> {
    match discord_call(
        st,
        "GET",
        &format!("/guilds/{guild}/members/{user}"),
        &[],
        None,
        None,
    )
    .await
    {
        Ok(member) if member["user"]["id"].as_str() == Some(user.to_string().as_str()) => {
            Ok(MemberLookup::Found(member))
        }
        Ok(_) => bail!("Mitglied ist uneindeutig"),
        Err(error)
            if error
                .downcast_ref::<DiscordHttpError>()
                .is_some_and(|error| error.status == 404) =>
        {
            Ok(MemberLookup::Missing)
        }
        Err(error) => Err(error),
    }
}

fn timed_out(member: Option<&Value>) -> Result<bool> {
    match member.and_then(|member| member.get("communication_disabled_until")) {
        None | Some(Value::Null) => Ok(false),
        Some(until) => Ok(chrono::DateTime::parse_from_rfc3339(
            until.as_str().context("Timeout-Zeitpunkt ist ungültig")?,
        )
        .context("Timeout-Zeitpunkt ist ungültig")?
            > chrono::Utc::now()),
    }
}

pub(crate) fn member_access(
    roles: &GuildRoles,
    requested_user: Option<u64>,
    member: Option<&Value>,
) -> Result<Access> {
    let member_roles = member
        .and_then(|member| member["roles"].as_array())
        .and_then(|roles| {
            roles
                .iter()
                .map(|role| role.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
        });
    if requested_user.is_some() && member_roles.is_none() {
        bail!("Mitgliedsrollen sind unbekannt");
    }
    let user_id = member_roles.as_ref().and(requested_user);
    let timed_out = timed_out(member.filter(|_| user_id.is_some()))?;
    let member_roles = member_roles.unwrap_or_else(|| vec![roles.verified.to_string()]);
    let mut base = roles.everyone;
    let mut top_position = 0;
    for role in &member_roles {
        role.parse::<u64>()?;
        let info = roles
            .roles
            .get(role)
            .context("Mitgliedsrolle ist unbekannt")?;
        base |= info.bits;
        top_position = top_position.max(info.position.unwrap_or_default());
    }
    Ok(Access {
        user_id,
        guild: roles.guild.to_string(),
        roles: member_roles,
        base,
        timed_out,
        top_position,
    })
}

pub(crate) async fn access(st: &McpState, requested_user: Option<u64>) -> Result<Access> {
    let (guild, _) = guild_source(st)?;
    let member = match requested_user {
        Some(user) => match fetch_member(st, guild, user)
            .await
            .context("Mitglied ist nicht abrufbar")?
        {
            MemberLookup::Found(member) => Some(member),
            MemberLookup::Missing => bail!("Mitglied ist nicht abrufbar"),
        },
        None => None,
    };
    let roles = guild_roles(st).await?;
    member_access(&roles, requested_user, member.as_ref())
}

pub(crate) fn effective_permissions(channel: &Value, access: &Access) -> Option<u64> {
    if access.base & ADMIN != 0 && access.user_id.is_some() {
        return Some(u64::MAX);
    }
    if access.base & ADMIN != 0 {
        return None;
    }
    let mut permissions = access.base;
    let mut role_deny = 0;
    let mut role_allow = 0;
    let mut member = None;
    let mut seen = HashSet::new();
    for overwrite in channel.get("permission_overwrites")?.as_array()? {
        let kind = overwrite.get("type")?.as_u64()?;
        let id = overwrite.get("id")?.as_str()?;
        if kind > 1 || !seen.insert((kind, id)) {
            return None;
        }
        let deny = overwrite.get("deny")?.as_str()?.parse::<u64>().ok()?;
        let allow = overwrite.get("allow")?.as_str()?.parse::<u64>().ok()?;
        if kind == 0 && id == access.guild {
            permissions = (permissions & !deny) | allow;
        } else if kind == 0 && access.roles.iter().any(|role| role == id) {
            role_deny |= deny;
            role_allow |= allow;
        } else if kind == 1 && access.user_id.is_some_and(|user| user.to_string() == id) {
            member = Some((deny, allow));
        }
    }
    permissions = (permissions & !role_deny) | role_allow;
    if let Some((deny, allow)) = member {
        permissions = (permissions & !deny) | allow;
    }
    permissions = implicit_denies(permissions);
    if access.timed_out {
        permissions &= VIEW | HISTORY;
    }
    Some(permissions)
}

fn implicit_denies(permissions: u64) -> u64 {
    if permissions & VIEW == 0 {
        0
    } else if permissions & SEND == 0 {
        permissions & !SEND_DEPENDENT
    } else {
        permissions
    }
}

pub(crate) fn role_can_view(channel: &Value, roles: &GuildRoles, role: Option<&str>) -> bool {
    let info = role.and_then(|role| roles.roles.get(role));
    if role.is_some() && info.is_none() {
        return true;
    }
    let bits = roles.everyone | info.map_or(0, |info| info.bits);
    if bits & ADMIN != 0 {
        return true;
    }
    let probe = Access {
        user_id: None,
        guild: roles.guild.to_string(),
        roles: role.map(str::to_owned).into_iter().collect(),
        base: bits,
        timed_out: false,
        top_position: 0,
    };
    effective_permissions(channel, &probe).is_none_or(|permissions| permissions & VIEW != 0)
}
