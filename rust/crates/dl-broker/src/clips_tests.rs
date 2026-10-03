use super::*;
use serde_json::Value;
use std::sync::Mutex;

#[derive(Default)]
struct FakeClips {
    calls: Mutex<Vec<TwitchClipSubmission>>,
    fail: bool,
}

#[async_trait::async_trait]
impl ClipSubmitPort for FakeClips {
    async fn submit_twitch_clip(
        &self,
        submission: TwitchClipSubmission,
    ) -> Result<ClipSubmitOutcome, String> {
        if self.fail {
            return Err("db down".into());
        }
        let mut calls = self.calls.lock().expect("lock");
        let duplicate = calls.iter().any(|c| c.clip_url == submission.clip_url);
        calls.push(submission);
        Ok(if duplicate {
            ClipSubmitOutcome {
                status: ClipSubmitStatus::Duplicate,
                submission_id: Some(1),
                reason: None,
            }
        } else {
            ClipSubmitOutcome {
                status: ClipSubmitStatus::Accepted,
                submission_id: Some(1),
                reason: None,
            }
        })
    }
}

fn fixture(fail: bool) -> (ClipState, Arc<FakeClips>, HeaderMap) {
    let broker = crate::handlers::tests::test_state().expect("broker fixture");
    let mut headers = HeaderMap::new();
    headers.insert(
        crate::TOKEN_HEADER,
        broker.token.parse().expect("fixture header"),
    );
    let port = Arc::new(FakeClips {
        fail,
        ..FakeClips::default()
    });
    (
        ClipState {
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

fn payload(value: Value) -> Bytes {
    Bytes::from(serde_json::to_vec(&value).expect("fixture JSON"))
}

fn plan_payload() -> Value {
    json!({
        "source": "twitch",
        "clip_url": "https://clips.twitch.tv/Abc",
        "streamer_twitch_user_id": "456",
        "streamer_login": "Name",
        "submitted_by_twitch_user_id": "456",
        "title": " Ein Clip ",
        "idempotency_key": "twitch-clip-Abc"
    })
}

async fn body_json(response: Response) -> Value {
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .expect("body");
    serde_json::from_slice(&body).expect("JSON")
}

#[tokio::test]
async fn auth_kommt_vor_jedem_portzugriff() {
    for (address, with_token, expected) in [
        ("192.0.2.1:1234", true, 403),
        ("127.0.0.1:1234", false, 401),
    ] {
        let (state, port, headers) = fixture(false);
        let headers = if with_token {
            headers
        } else {
            HeaderMap::new()
        };
        let response = submit(
            State(state),
            peer(address),
            headers,
            payload(plan_payload()),
        )
        .await;
        assert_eq!(response.status().as_u16(), expected);
        assert!(port.calls.lock().expect("lock").is_empty());
    }
}

#[tokio::test]
async fn plan_payload_wird_angenommen_und_normalisiert() {
    let (state, port, headers) = fixture(false);
    let response = submit(
        State(state.clone()),
        peer("127.0.0.1:1"),
        headers.clone(),
        payload(plan_payload()),
    )
    .await;
    assert_eq!(response.status().as_u16(), 200);
    let value = body_json(response).await;
    assert_eq!(value["ok"], true);
    assert_eq!(value["idempotency_key"], "twitch-clip-Abc");
    assert_eq!(
        value["result"],
        json!({"status": "accepted", "submission_id": 1, "reason": null})
    );
    {
        let calls = port.calls.lock().expect("lock");
        assert_eq!(calls[0].streamer_login, "name");
        assert_eq!(calls[0].title.as_deref(), Some("Ein Clip"));
    }
    let response = submit(
        State(state),
        peer("[::1]:1"),
        headers,
        payload(plan_payload()),
    )
    .await;
    let value = body_json(response).await;
    assert_eq!(value["result"]["status"], "duplicate");
}

#[tokio::test]
async fn formfehler_sind_400_ohne_portzugriff() {
    let mut cases = Vec::new();
    for (field, bad) in [
        ("source", json!("discord")),
        ("streamer_twitch_user_id", json!("0123")),
        ("streamer_twitch_user_id", json!("abc")),
        ("streamer_login", json!("böse name")),
        ("idempotency_key", json!("")),
        ("idempotency_key", json!("mit leerzeichen")),
        ("submitted_by_twitch_user_id", json!("x")),
        ("title", json!("a".repeat(MAX_TITLE_CHARS + 1))),
        ("clip_url", json!("")),
        ("unbekannt", json!(1)),
    ] {
        let mut value = plan_payload();
        value[field] = bad;
        cases.push(value);
    }
    let mut missing = plan_payload();
    missing
        .as_object_mut()
        .expect("object")
        .remove("idempotency_key");
    cases.push(missing);
    for case in cases {
        let (state, port, headers) = fixture(false);
        let response = submit(
            State(state),
            peer("127.0.0.1:1"),
            headers,
            payload(case.clone()),
        )
        .await;
        assert_eq!(response.status().as_u16(), 400, "{case}");
        assert!(port.calls.lock().expect("lock").is_empty());
    }
    let (state, _, headers) = fixture(false);
    let big = Bytes::from(vec![b' '; MAX_BODY_BYTES + 1]);
    let response = submit(State(state), peer("127.0.0.1:1"), headers, big).await;
    assert_eq!(response.status().as_u16(), 413);
}

#[tokio::test]
async fn portfehler_wird_503() {
    let (state, _, headers) = fixture(true);
    let response = submit(
        State(state),
        peer("127.0.0.1:1"),
        headers,
        payload(plan_payload()),
    )
    .await;
    assert_eq!(response.status().as_u16(), 503);
}

#[test]
fn rejected_serialisiert_mit_grund() {
    let outcome = ClipSubmitOutcome {
        status: ClipSubmitStatus::Rejected,
        submission_id: None,
        reason: Some("not_partner".into()),
    };
    assert_eq!(
        serde_json::to_value(outcome).expect("json"),
        json!({"status": "rejected", "submission_id": null, "reason": "not_partner"})
    );
}

#[tokio::test]
async fn authentifizierte_clip_herkunft_bleibt_mikrosekundengenau_und_optional() {
    let (state, port, headers) = fixture(false);
    let mut body = plan_payload();
    body["submitted_by_twitch_user_id"] = json!("789");
    body["submitted_at"] = json!("2026-10-03T07:12:34.123456Z");
    let response = submit(
        State(state.clone()),
        peer("127.0.0.1:1"),
        headers.clone(),
        payload(body.clone()),
    )
    .await;
    assert_eq!(response.status().as_u16(), 200);
    {
        let calls = port.calls.lock().expect("Portaufruf");
        assert_eq!(calls[0].submitted_by_twitch_user_id.as_deref(), Some("789"));
        assert_eq!(
            calls[0]
                .submitted_at
                .expect("Herkunft")
                .timestamp_subsec_micros(),
            123456
        );
        assert_eq!(
            calls[0]
                .submitted_at
                .expect("Herkunft")
                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
            "2026-10-03T07:12:34.123456Z"
        );
    }
    body["submitted_at"] = json!("nicht-RFC3339");
    let response = submit(
        State(state.clone()),
        peer("127.0.0.1:1"),
        headers.clone(),
        payload(body),
    )
    .await;
    assert_eq!(response.status().as_u16(), 400);
    assert_eq!(
        port.calls.lock().expect("Keine ungültige Herkunft").len(),
        1
    );
    let response = submit(
        State(state),
        peer("127.0.0.1:1"),
        headers,
        payload(plan_payload()),
    )
    .await;
    assert_eq!(response.status().as_u16(), 200);
    assert!(port.calls.lock().expect("Altbestand")[1]
        .submitted_at
        .is_none());
}
