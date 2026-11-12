use std::collections::HashSet;

use serenity::model::id::{GuildId, RoleId};

use super::{BOOSTER_ROLE_ID, SUPPORTER_ROLE_ID};

const MEMBER_PAGE_SIZE: u64 = 1000;
const MEMBER_PAGE_SIZE_USIZE: usize = MEMBER_PAGE_SIZE as usize;

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
    let cap = usize::try_from(max_revokes).unwrap_or(usize::MAX);
    let revokes_held = revokes.len().saturating_sub(cap);
    revokes.truncate(cap);
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
    let guild = GuildId::new(guild_id);
    let supporter_role = RoleId::new(SUPPORTER_ROLE_ID);
    let booster_role = RoleId::new(BOOSTER_ROLE_ID);
    let mut after = None;
    let mut members = Vec::new();
    loop {
        let page = http
            .get_guild_members(guild, Some(MEMBER_PAGE_SIZE), after)
            .await
            .map_err(|error| error.to_string())?;
        let full_page = page.len() >= MEMBER_PAGE_SIZE_USIZE;
        let Some(cursor) = next_cursor(page.iter().map(|member| member.user.id.get()), after)?
        else {
            break;
        };
        members.extend(
            page.into_iter()
                .filter(|member| !member.user.bot)
                .map(|member| MemberSnapshot {
                    id: member.user.id.get(),
                    has_role: member.roles.contains(&supporter_role),
                    booster: member.premium_since.is_some() || member.roles.contains(&booster_role),
                }),
        );
        if !full_page {
            break;
        }
        after = Some(cursor);
    }
    Ok(members)
}

fn next_cursor(
    page_ids: impl Iterator<Item = u64>,
    previous: Option<u64>,
) -> Result<Option<u64>, String> {
    let Some(cursor) = page_ids.max() else {
        return Ok(None);
    };
    if previous.is_some_and(|previous| cursor <= previous) {
        return Err("Mitgliederliste blättert nicht weiter".to_string());
    }
    Ok(Some(cursor))
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

    #[test]
    fn blaettern_bricht_ab_wenn_der_cursor_nicht_vorankommt() {
        assert_eq!(next_cursor([3, 9, 5].into_iter(), None), Ok(Some(9)));
        assert_eq!(next_cursor([12, 10].into_iter(), Some(9)), Ok(Some(12)));
        assert_eq!(next_cursor(std::iter::empty(), Some(9)), Ok(None));
        assert!(next_cursor([4, 9].into_iter(), Some(9)).is_err());
    }
}
