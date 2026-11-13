use dl_discord::member_pages::{MemberPager, MEMBER_PAGE_LIMIT};
use serenity::all::{GuildId, Member};
use serenity::http::Http;

pub async fn fetch_all(http: &Http, guild: GuildId) -> Result<Vec<Member>, String> {
    let mut pager = MemberPager::default();
    let mut members = Vec::new();
    while !pager.is_done() {
        let page = http
            .get_guild_members(guild, Some(u64::from(MEMBER_PAGE_LIMIT)), pager.after())
            .await
            .map_err(|error| error.to_string())?;
        pager
            .advance(
                page.iter().map(|member| Some(member.user.id.get())),
                page.len(),
            )
            .map_err(|error| error.to_string())?;
        members.extend(page);
    }
    Ok(members)
}
