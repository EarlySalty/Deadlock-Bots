use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn snapshot(role_id: u64, fail_last_page: bool) -> Result<RoleMembers, PortError> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let member = |id: u64, roles: &[&str]| {
        json!({
            "user": {"id": id.to_string(), "username": "test", "discriminator": "0", "avatar": null},
            "roles": roles, "joined_at": "2026-10-08T00:00:00Z", "deaf": false, "mute": false, "flags": 0
        })
    };
    let pages = [
        json!([
            {"id":"1","name":"@everyone","color":0,"colors":{"primary_color":0,"secondary_color":null,"tertiary_color":null},"hoist":false,"position":0,"permissions":"0","managed":false,"mentionable":false},
            {"id":"2","name":"Streamer","color":0,"colors":{"primary_color":0,"secondary_color":null,"tertiary_color":null},"hoist":false,"position":1,"permissions":"0","managed":false,"mentionable":false}
        ]),
        json!([member(10, &["2"]), member(11, &["3"])]),
        json!([member(12, &["2"]), member(13, &[])]),
        json!([]),
    ];
    let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = requests.clone();
    let server = tokio::spawn(async move {
        for (index, page) in pages.into_iter().enumerate() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).await.unwrap();
                request.push(byte[0]);
            }
            captured
                .lock()
                .unwrap()
                .push(String::from_utf8(request).unwrap());
            let status = if fail_last_page && index == 3 {
                "403 Forbidden"
            } else {
                "200 OK"
            };
            let body = if fail_last_page && index == 3 {
                json!({"code":50013,"message":"Missing Permissions"}).to_string()
            } else {
                page.to_string()
            };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
    });
    let mut adapter = DiscordAdapter::new("test-only");
    Arc::get_mut(&mut adapter).unwrap().http = Arc::new(
        serenity::http::HttpBuilder::new("test-only")
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build(),
    );
    let result = adapter.live_role_members(1, role_id).await;
    server.abort();
    let _ = server.await;
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4, "REST decoding failed: {result:?}");
    assert!(requests[0].contains("/guilds/1/roles"));
    assert!(!requests[1].contains("after="));
    assert!(requests[2].contains("after=11"));
    assert!(requests[3].contains("after=13"));
    result
}

#[tokio::test]
async fn complete_role_snapshot_reads_every_rest_page_without_gateway_cache() {
    let result = snapshot(2, false).await.unwrap();
    assert_eq!(result.role_id, 2);
    assert_eq!(
        result
            .members
            .iter()
            .map(|member| member.user_id)
            .collect::<Vec<_>>(),
        vec![10, 12]
    );
}

#[tokio::test]
async fn everyone_snapshot_includes_all_members_without_explicit_everyone_role() {
    let result = snapshot(1, false).await.unwrap();
    assert_eq!(result.role_id, 1);
    assert_eq!(result.name, "@everyone");
    assert_eq!(
        result
            .members
            .iter()
            .map(|member| member.user_id)
            .collect::<Vec<_>>(),
        vec![10, 11, 12, 13]
    );
}

#[tokio::test]
async fn later_rest_failure_never_returns_partial_role_holders() {
    assert!(snapshot(2, true).await.is_err());
}

#[tokio::test]
async fn later_rest_failure_never_returns_partial_everyone_members() {
    assert!(snapshot(1, true).await.is_err());
}
