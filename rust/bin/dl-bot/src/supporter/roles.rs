use std::collections::HashSet;

use serenity::model::id::{GuildId, RoleId};

use super::{BOOSTER_ROLE_ID, SUPPORTER_ROLE_ID};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberSnapshot {
    pub id: u64,
    pub has_role: bool,
    pub booster: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RolePlan {
    pub grants: Vec<u64>,
    pub revokes: Vec<u64>,
    pub revokes_held: usize,
}

pub fn plan_roles(members: &[MemberSnapshot], active: &HashSet<u64>, max_revokes: u64) -> RolePlan {
    let mut grants: Vec<u64> = members
        .iter()
        .filter(|member| !member.has_role && active.contains(&member.id))
        .map(|member| member.id)
        .collect();
    let mut revokes: Vec<u64> = members
        .iter()
        .filter(|member| member.has_role && !active.contains(&member.id))
        .map(|member| member.id)
        .collect();
    grants.sort_unstable();
    revokes.sort_unstable();
    let candidates = revokes.len();
    let revokes: Vec<u64> = revokes
        .into_iter()
        .zip(0..max_revokes)
        .map(|(revoke, _)| revoke)
        .collect();
    let revokes_held = candidates - revokes.len();
    RolePlan {
        grants,
        revokes,
        revokes_held,
    }
}

pub async fn fetch_members(
    http: &serenity::http::Http,
    guild_id: u64,
) -> Result<Vec<MemberSnapshot>, String> {
    let supporter_role = RoleId::new(SUPPORTER_ROLE_ID);
    let booster_role = RoleId::new(BOOSTER_ROLE_ID);
    let members = crate::guild_members::fetch_all(http, GuildId::new(guild_id)).await?;
    Ok(members
        .into_iter()
        .filter(|member| !member.user.bot)
        .map(|member| MemberSnapshot {
            id: member.user.id.get(),
            has_role: member.roles.contains(&supporter_role),
            booster: member.premium_since.is_some() || member.roles.contains(&booster_role),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(id: u64, has_role: bool) -> MemberSnapshot {
        MemberSnapshot {
            id,
            has_role,
            booster: false,
        }
    }

    #[test]
    fn plan_vergibt_entzieht_und_deckelt_entzug() {
        let members = vec![
            member(1, false),
            member(2, true),
            member(3, true),
            member(4, true),
            member(5, true),
            member(6, false),
        ];
        let active: HashSet<u64> = [1, 2].into_iter().collect();
        let plan = plan_roles(&members, &active, 2);
        assert_eq!(plan.grants, vec![1]);
        assert_eq!(plan.revokes, vec![3, 4]);
        assert_eq!(plan.revokes_held, 1);
    }

    #[test]
    fn aktive_ausserhalb_des_servers_werden_ignoriert() {
        let members = vec![member(1, true)];
        let active: HashSet<u64> = [1, 99].into_iter().collect();
        assert_eq!(plan_roles(&members, &active, 10), RolePlan::default());
    }
}
