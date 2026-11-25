use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FakeInvites {
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl TwitchInvitePort for FakeInvites {
    fn guild_id(&self) -> u64 {
        1
    }

    async fn destination(&self, streamer_id: &str) -> Result<Option<InviteDestination>, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok((streamer_id == "42").then(|| InviteDestination {
            guild_id: 1,
            channel_id: 2,
            streamer_login: "streamer".into(),
            streamer_twitch_user_id: "42".into(),
            invite_url: "https://discord.gg/ChannelCode".into(),
        }))
    }

    async fn personal_invite(
        &self,
        _: &InviteDestination,
        _: &str,
    ) -> Result<PersonalInvite, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(PersonalInvite {
            invite_url: "https://discord.gg/ViewerCode".into(),
            personal: true,
        })
    }

    async fn qualified_invites(&self, _: &QualifiedQuery) -> Result<Value, String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"invites": [{
            "join_id": "9", "streamer_login": "streamer", "inviter_twitch_user_id": "43",
            "joined_at": "2026-01-01T00:00:00Z", "status": "qualified",
            "qualified_at": "2026-01-15T00:00:00Z", "updated_at": "2026-01-15T00:00:00Z"
        }]}))
    }
}

fn fixture() -> (InviteState, Arc<FakeInvites>, HeaderMap) {
    let mut broker = crate::handlers::tests::test_state().expect("broker fixture");
    let inner = Arc::get_mut(&mut broker).expect("exclusive test state");
    inner.channel_allowlist = crate::Allowlist {
        enabled: true,
        ids: [2].into(),
    };
    inner.guild_allowlist = crate::Allowlist {
        enabled: true,
        ids: [1].into(),
    };
    let mut headers = HeaderMap::new();
    headers.insert(
        crate::TOKEN_HEADER,
        broker.token.parse().expect("fixture header"),
    );
    let port = Arc::new(FakeInvites {
        calls: AtomicUsize::new(0),
    });
    (
        InviteState {
            broker,
            port: port.clone(),
        },
        port,
        headers,
    )
}

fn peer(value: &str) -> ConnectInfo<SocketAddr> {
    ConnectInfo(value.parse().expect("fixture peer"))
}

fn payload() -> Bytes {
    Bytes::from(serde_json::to_vec(&json!({
        "streamer_login": "streamer", "streamer_twitch_user_id": "42", "inviter_twitch_user_id": "43"
    })).expect("fixture JSON"))
}

#[tokio::test]
async fn both_routes_require_loopback_and_internal_token_before_any_port_access() {
    for remote in [false, true] {
        let (state, port, headers) = fixture();
        let (address, headers, expected) = if remote {
            ("192.0.2.1:1234", headers, 403)
        } else {
            ("127.0.0.1:1234", HeaderMap::new(), 401)
        };
        let response = personal_invite(
            State(state.clone()),
            peer(address),
            headers.clone(),
            payload(),
        )
        .await;
        assert_eq!(response.status().as_u16(), expected);
        let response =
            qualified_invites(State(state), peer(address), headers, Query(HashMap::new())).await;
        assert_eq!(response.status().as_u16(), expected);
        assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn trusted_guild_and_channel_allowlists_cannot_be_bypassed() {
    for channel in [false, true] {
        let (mut state, port, headers) = fixture();
        let broker = Arc::get_mut(&mut state.broker).expect("exclusive state");
        if channel {
            broker.channel_allowlist.ids.clear();
        } else {
            broker.guild_allowlist.ids.clear();
        }
        let response =
            personal_invite(State(state), peer("127.0.0.1:1234"), headers, payload()).await;
        assert_eq!(response.status().as_u16(), 403);
        assert_eq!(port.calls.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn successful_personal_link_uses_standard_broker_envelope() {
    let (state, port, headers) = fixture();
    let response = personal_invite(State(state), peer("[::1]:1234"), headers, payload()).await;
    assert_eq!(response.status().as_u16(), 200);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("body");
    let value: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(value["ok"], true);
    assert_eq!(
        value["result"],
        json!({"invite_url": "https://discord.gg/ViewerCode", "personal": true})
    );
    assert_eq!(port.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn qualified_route_rejects_bad_since_and_does_not_add_discord_identifiers() {
    let (state, port, headers) = fixture();
    let response = qualified_invites(
        State(state.clone()),
        peer("127.0.0.1:1234"),
        headers.clone(),
        Query(HashMap::new()),
    )
    .await;
    assert_eq!(response.status().as_u16(), 400);
    assert_eq!(port.calls.load(Ordering::SeqCst), 0);
    let params = HashMap::from([("since".into(), "2026-01-01T00:00:00Z".into())]);
    let response =
        qualified_invites(State(state), peer("127.0.0.1:1234"), headers, Query(params)).await;
    assert_eq!(response.status().as_u16(), 200);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .expect("body");
    let value: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(value["result"]["invites"][0]["status"], "qualified");
    for field in [
        "user_id",
        "guild_id",
        "display_name",
        "content",
        "discord_user_id",
    ] {
        assert!(
            value["result"]["invites"][0].get(field).is_none(),
            "{field}"
        );
    }
}
