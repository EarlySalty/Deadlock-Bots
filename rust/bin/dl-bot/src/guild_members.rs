use serenity::all::{GuildId, Member};
use serenity::http::Http;

const PAGE_LIMIT: u16 = 1000;

pub async fn fetch_all(http: &Http, guild: GuildId) -> Result<Vec<Member>, String> {
    let mut after = None;
    let mut members = Vec::new();
    loop {
        let page = http
            .get_guild_members(guild, Some(u64::from(PAGE_LIMIT)), after)
            .await
            .map_err(|error| error.to_string())?;
        let full_page = page.len() >= usize::from(PAGE_LIMIT);
        let Some(cursor) = next_cursor(page.iter().map(|member| member.user.id.get()), after)?
        else {
            break;
        };
        members.extend(page);
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
    use super::next_cursor;

    #[test]
    fn blaettern_bricht_ab_wenn_der_cursor_nicht_vorankommt() {
        assert_eq!(next_cursor([3_u64, 9, 5].into_iter(), None), Ok(Some(9)));
        assert_eq!(next_cursor([12_u64, 15].into_iter(), Some(9)), Ok(Some(15)));
        assert_eq!(next_cursor(std::iter::empty(), Some(9)), Ok(None));
        assert!(next_cursor([4_u64, 9].into_iter(), Some(9)).is_err());
    }
}
