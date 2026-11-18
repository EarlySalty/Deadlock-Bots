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

pub fn member_scan_pages(member_count: u64) -> u64 {
    member_count.div_ceil(GUILD_MEMBER_PAGE_LIMIT) + 1
}

pub fn member_scan_budget(
    member_count: u64,
    page_timeout: std::time::Duration,
) -> std::time::Duration {
    let pages = u32::try_from(member_scan_pages(member_count)).unwrap_or(u32::MAX);
    page_timeout.saturating_mul(pages)
}

pub async fn fetch_counted_guild_members(
    http: &serenity::http::Http,
    guild: serenity::all::GuildId,
    page_timeout: std::time::Duration,
    member_count: u64,
) -> Result<Vec<serenity::all::Member>, String> {
    let budget = member_scan_budget(member_count, page_timeout);
    tokio::time::timeout(budget, fetch_all_guild_members(http, guild, page_timeout))
        .await
        .map_err(|_| {
            format!(
                "Mitgliederliste mit {member_count} gemeldeten Mitgliedern nicht innerhalb von {} ms geladen",
                budget.as_millis()
            )
        })?
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
pub(crate) struct ConnectionSink {
    pub client: reqwest::Client,
    pub connections: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub server: tokio::task::JoinHandle<()>,
}

#[cfg(test)]
pub(crate) async fn connection_sink() -> Result<ConnectionSink, Box<dyn std::error::Error>> {
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
    Ok(ConnectionSink {
        client,
        connections,
        server,
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
        delayed_member_pages_server(
            pages
                .into_iter()
                .map(|page| (std::time::Duration::ZERO, page))
                .collect(),
        )
        .await
    }

    async fn delayed_member_pages_server(
        pages: Vec<(std::time::Duration, Option<Value>)>,
    ) -> std::io::Result<(Http, tokio::task::JoinHandle<()>)> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            for (delay, page) in pages {
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
                tokio::time::sleep(delay).await;
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

    #[test]
    fn mitgliederscan_budget_folgt_der_gemeldeten_mitgliederzahl() {
        let page = std::time::Duration::from_millis(100);
        for (count, pages) in [(0, 1), (1, 2), (999, 2), (1000, 2), (1001, 3), (2000, 3)] {
            assert_eq!(member_scan_pages(count), pages, "{count}");
            assert_eq!(
                member_scan_budget(count, page),
                page * u32::try_from(pages).expect("pages")
            );
        }
        assert_eq!(
            member_scan_budget(u64::MAX, page),
            page.saturating_mul(u32::MAX)
        );
    }

    #[tokio::test]
    async fn gezaehlte_mitgliederliste_endet_am_budget_der_gemeldeten_seiten() -> std::io::Result<()>
    {
        let page_timeout = std::time::Duration::from_millis(500);
        let slow = std::time::Duration::from_millis(400);
        let pages = || {
            vec![
                (slow, Some(json!([member_json(10), member_json(11)]))),
                (slow, Some(json!([member_json(12)]))),
                (slow, Some(json!([]))),
            ]
        };

        let (http, server) = delayed_member_pages_server(pages()).await?;
        let cut = fetch_counted_guild_members(&http, GuildId::new(1), page_timeout, 1).await;
        server.abort();
        assert_eq!(
            cut.err().as_deref(),
            Some(
                "Mitgliederliste mit 1 gemeldeten Mitgliedern nicht innerhalb von 1000 ms geladen"
            )
        );

        let (http, server) = delayed_member_pages_server(pages()).await?;
        let complete =
            fetch_counted_guild_members(&http, GuildId::new(1), page_timeout, 1001).await;
        server.abort();
        assert_eq!(
            complete.map(|members| members.iter().map(|m| m.user.id.get()).collect::<Vec<_>>()),
            Ok(vec![10, 11, 12])
        );
        Ok(())
    }

    #[tokio::test]
    async fn seitenlimit_beendet_eine_wartende_ratelimiter_seite_vor_dem_ersten_request(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let sink = connection_sink().await?;
        let http = serenity::http::HttpBuilder::new("test-only")
            .client(sink.client.clone())
            .build();
        let ratelimiter = http.ratelimiter.as_ref().ok_or("ratelimiter is off")?;
        let route = serenity::http::Route::GuildMembers {
            guild_id: GuildId::new(1),
        };
        let bucket = std::sync::Arc::clone(
            ratelimiter
                .routes()
                .write()
                .await
                .entry(route.ratelimiting_bucket())
                .or_default(),
        );
        let exhausted = reqwest::Response::from(
            axum::http::Response::builder()
                .header("x-ratelimit-limit", "1")
                .header("x-ratelimit-remaining", "0")
                .header("x-ratelimit-reset-after", "60")
                .body(String::new())?,
        );
        let request = serenity::http::Request::new(route, serenity::http::LightMethod::Get);
        bucket
            .lock()
            .await
            .post_hook(&exhausted, &request, &|_| {}, false)
            .await?;
        let page_timeout = std::time::Duration::from_millis(300);
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            fetch_all_guild_members(&http, GuildId::new(1), page_timeout),
        )
        .await;
        sink.server.abort();
        assert_eq!(
            result.map(|members| members.err()),
            Ok(Some(
                "Zeitlimit für eine Seite der Mitgliederliste überschritten".to_string()
            ))
        );
        assert!(started.elapsed() >= page_timeout);
        assert_eq!(
            sink.connections.load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        Ok(())
    }
}
