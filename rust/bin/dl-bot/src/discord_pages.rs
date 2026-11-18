pub const GUILD_MEMBER_PAGE_LIMIT: u64 = 1000;

pub fn discord_call_timeout() -> std::time::Duration {
    std::time::Duration::from_millis(dl_core::runtime_config::DISCORD_CALL_TIMEOUT_MS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalledCursor;

pub fn next_page_cursor(
    after: Option<u64>,
    ids: impl IntoIterator<Item = u64>,
) -> Result<Option<u64>, StalledCursor> {
    advance_after(after.as_ref(), ids.into_iter().max())
}

pub fn advance_after<T: Ord>(
    previous: Option<&T>,
    next: Option<T>,
) -> Result<Option<T>, StalledCursor> {
    match (previous, next) {
        (Some(previous), Some(next)) if next <= *previous => Err(StalledCursor),
        (_, next) => Ok(next),
    }
}

pub fn advance_before<T: Ord>(
    previous: Option<&T>,
    next: Option<T>,
) -> Result<Option<T>, StalledCursor> {
    match (previous, next) {
        (Some(previous), Some(next)) if next >= *previous => Err(StalledCursor),
        (_, next) => Ok(next),
    }
}

pub async fn fetch_all_guild_members(
    http: &serenity::http::Http,
    guild: serenity::all::GuildId,
    page_timeout: std::time::Duration,
) -> Result<Vec<serenity::all::Member>, String> {
    let mut members = Vec::new();
    let mut after = None;
    loop {
        let request = http.get_guild_members(guild, Some(GUILD_MEMBER_PAGE_LIMIT), after);
        let page = tokio::time::timeout(page_timeout, request)
            .await
            .map_err(|_| "Zeitlimit für eine Seite der Mitgliederliste überschritten".to_string())?
            .map_err(|error| error.to_string())?;
        let next = next_member_cursor(after, &page)?;
        members.extend(page);
        match next {
            Some(cursor) => after = Some(cursor),
            None if members.is_empty() => return Err(EMPTY_MEMBER_LIST.to_string()),
            None => return Ok(members),
        }
    }
}

pub const EMPTY_MEMBER_LIST: &str = "Mitgliederliste ist leer";

fn next_member_cursor(
    after: Option<u64>,
    page: &[serenity::all::Member],
) -> Result<Option<u64>, String> {
    next_page_cursor(after, page.iter().map(|member| member.user.id.get()))
        .map_err(|StalledCursor| "Mitgliederliste blättert nicht weiter".to_string())
}

#[cfg(test)]
pub(crate) fn member_fixture(id: u64, roles: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "user": {"id": id.to_string(), "username": "member", "discriminator": "0001", "avatar": null},
        "roles": roles, "joined_at": "2026-10-08T00:00:00Z", "deaf": false, "mute": false, "flags": 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use serenity::all::GuildId;
    use serenity::http::Http;

    fn member_page(ids: impl Iterator<Item = u64>) -> Vec<serenity::all::Member> {
        let rows: Vec<Value> = ids.map(member_json).collect();
        serde_json::from_value(Value::Array(rows)).expect("members")
    }

    #[test]
    fn mitgliederseiten_blaettern_bis_zur_leeren_seite_und_brechen_bei_stehendem_cursor_ab() {
        let full = member_page((1..=GUILD_MEMBER_PAGE_LIMIT).rev());
        assert_eq!(
            next_member_cursor(None, &full),
            Ok(Some(GUILD_MEMBER_PAGE_LIMIT))
        );
        assert_eq!(
            next_member_cursor(Some(7), &member_page([3, 9, 5].into_iter())),
            Ok(Some(9))
        );
        assert_eq!(next_member_cursor(None, &[]), Ok(None));
        assert_eq!(next_member_cursor(Some(9), &[]), Ok(None));
        assert!(next_member_cursor(Some(9), &member_page([3, 9].into_iter())).is_err());
        assert!(next_member_cursor(Some(GUILD_MEMBER_PAGE_LIMIT), &full).is_err());
    }

    async fn member_pages_server(
        pages: Vec<Option<Value>>,
    ) -> std::io::Result<(Http, tokio::task::JoinHandle<()>)> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            for page in pages {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    if socket.read_exact(&mut byte).await.is_err() {
                        return;
                    }
                    request.push(byte[0]);
                }
                let Some(page) = page else {
                    std::future::pending::<()>().await;
                    return;
                };
                let body = page.to_string();
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                if socket.write_all(response.as_bytes()).await.is_err() {
                    return;
                }
            }
        });
        let http = serenity::http::HttpBuilder::new("test-only")
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build();
        Ok((http, server))
    }

    fn member_json(id: u64) -> Value {
        member_fixture(id, &[])
    }

    #[tokio::test]
    async fn mitgliederliste_bricht_bei_haengender_oder_wiederholter_seite_ohne_teilstand_ab(
    ) -> std::io::Result<()> {
        let short = std::time::Duration::from_millis(200);
        let (http, server) = member_pages_server(vec![
            Some(json!([member_json(10), member_json(11)])),
            Some(json!([member_json(12)])),
            Some(json!([])),
        ])
        .await?;
        let members = fetch_all_guild_members(&http, GuildId::new(1), short).await;
        server.abort();
        assert_eq!(
            members.map(|page| page.iter().map(|m| m.user.id.get()).collect::<Vec<_>>()),
            Ok(vec![10, 11, 12])
        );

        let (http, server) =
            member_pages_server(vec![Some(json!([member_json(10), member_json(11)])), None])
                .await?;
        let hanging = fetch_all_guild_members(&http, GuildId::new(1), short).await;
        server.abort();
        assert_eq!(
            hanging.err().as_deref(),
            Some("Zeitlimit für eine Seite der Mitgliederliste überschritten")
        );

        let (http, server) = member_pages_server(vec![Some(json!([]))]).await?;
        let empty = fetch_all_guild_members(&http, GuildId::new(1), short).await;
        server.abort();
        assert_eq!(empty.err().as_deref(), Some(EMPTY_MEMBER_LIST));

        let (http, server) = member_pages_server(vec![
            Some(json!([member_json(10), member_json(11)])),
            Some(json!([member_json(10), member_json(11)])),
        ])
        .await?;
        let repeated = fetch_all_guild_members(&http, GuildId::new(1), short).await;
        server.abort();
        assert_eq!(
            repeated.err().as_deref(),
            Some("Mitgliederliste blättert nicht weiter")
        );
        Ok(())
    }

    #[tokio::test]
    async fn seitenlimit_beendet_den_ganzen_serenity_aufruf_mit_aktivem_ratelimiter(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let connections = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let accepted = connections.clone();
        let server = tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                accepted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                held.extend([socket]);
            }
        });
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!("http://{address}"))?)
            .build()?;
        let http = serenity::http::HttpBuilder::new("test-only")
            .client(client)
            .build();
        assert!(http.ratelimiter.is_some());
        let page_timeout = std::time::Duration::from_millis(300);
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            fetch_all_guild_members(&http, GuildId::new(1), page_timeout),
        )
        .await;
        server.abort();
        assert_eq!(
            result.map(|members| members.err()),
            Ok(Some(
                "Zeitlimit für eine Seite der Mitgliederliste überschritten".to_string()
            ))
        );
        assert!(started.elapsed() >= page_timeout);
        assert!(connections.load(std::sync::atomic::Ordering::SeqCst) >= 1);
        Ok(())
    }
}
