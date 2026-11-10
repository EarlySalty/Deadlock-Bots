use super::*;
use axum::{response::IntoResponse, Router};
use dl_access_gate::{GateClient, GateError, Origin};
use dl_core::runtime_config::StartOptions;
use std::sync::Mutex as StdMutex;

const CACHE_MS: u64 = 60_000;
const LOOKUP_MS: u64 = 2_000;
const DECISION_MS: u64 = 6_000;
const RELOAD_MS: u64 = 50;
const PURGE_SECS: u64 = 21_600;
const RETENTION_DAYS: u64 = 180;

const VIEW: u64 = 1 << 10;
const BAN: u64 = 1 << 2;
const EVENTS: u64 = 1 << 33;

const POLICY: &str = r#"
schema_version = 1

[connectors.brain]
enabled = true
shadow = true

[connectors.off]
enabled = false
shadow = true

[areas.moderation]
channels = ["200", "201", "203"]

[capabilities."discord.event.write"]
permissions = ["manage_events", "create_events"]

[capabilities."discord.moderation.ban"]
area = "moderation"
permissions = ["ban_members"]

[capabilities."discord.moderation.read"]
area = "moderation"
permissions = ["ban_members"]
confirm = false
shadow = false

[capabilities."knowledge.public.read"]
public = true

[capabilities."owner.thing"]
owner_only = true
"#;

type Members = Arc<StdMutex<HashMap<u64, Value>>>;

struct Fixture {
    client: GateClient,
    addr: std::net::SocketAddr,
    members: Members,
    dir: tempfile::TempDir,
}

fn roles() -> Value {
    json!([
        {"id":"1","permissions":"0","position":0},
        {"id":"3","permissions":VIEW.to_string(),"position":1},
        {"id":"5","permissions":EVENTS.to_string(),"position":10},
        {"id":"6","permissions":BAN.to_string(),"position":20},
        {"id":"7","permissions":(ADMIN | BAN).to_string(),"position":30},
        {"id":"8","permissions":BAN.to_string(),"position":30}
    ])
}

fn restricted(id: &str, extra: Option<&str>) -> Value {
    let mut overwrites = vec![
        json!({"id":"1","type":0,"deny":VIEW.to_string(),"allow":"0"}),
        json!({"id":"6","type":0,"deny":"0","allow":VIEW.to_string()}),
        json!({"id":"7","type":0,"deny":"0","allow":VIEW.to_string()}),
    ];
    if let Some(user) = extra {
        overwrites.push(json!({"id":user,"type":1,"deny":"0","allow":VIEW.to_string()}));
    }
    json!({"id":id,"guild_id":"1","type":0,"permission_overwrites":overwrites})
}

fn mock_answer(path: &str, members: &Members) -> Response {
    let found = |value: Value| axum::Json(value).into_response();
    match path {
        "/guilds/1/roles" => found(roles()),
        "/guilds/1/members/104" => StatusCode::FORBIDDEN.into_response(),
        "/channels/200" => {
            found(json!({"id":"200","guild_id":"1","type":0,"permission_overwrites":[]}))
        }
        "/channels/201" => found(restricted("201", None)),
        "/channels/203" => found(restricted("203", Some("100"))),
        "/channels/202" => found(json!({"id":"202","type":1,"recipients":[{"id":"102"}]})),
        _ => path
            .strip_prefix("/guilds/1/members/")
            .and_then(|user| user.parse::<u64>().ok())
            .and_then(|user| members.lock().ok()?.get(&user).cloned())
            .map_or_else(|| StatusCode::NOT_FOUND.into_response(), found),
    }
}

fn base_members() -> HashMap<u64, Value> {
    [
        (100, vec!["3"]),
        (101, vec!["3", "5"]),
        (102, vec!["3", "6"]),
        (103, vec!["3", "7"]),
        (106, vec!["3", "8"]),
    ]
    .into_iter()
    .map(|(id, roles)| (id, json!({"user":{"id":id.to_string()},"roles":roles})))
    .collect()
}

async fn serve(router: Router) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test listener");
    let addr = listener.local_addr().expect("test address");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("test server");
    });
    addr
}

async fn fixture(policy: Option<&str>, recorder: bool, cache_ms: u64) -> Fixture {
    let members: Members = Arc::new(StdMutex::new(base_members()));
    let mock_members = members.clone();
    let mock = Router::new().fallback(move |request: axum::extract::Request| {
        let members = mock_members.clone();
        async move { mock_answer(request.uri().path(), &members) }
    });
    let mock_addr = serve(mock).await;
    let dir = tempfile::tempdir().expect("test dir");
    let file = dir.path().join("access-gate.toml");
    if let Some(policy) = policy {
        std::fs::write(&file, policy).expect("test policy");
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql:///synthetic")
        .expect("test pool");
    let mut state = McpState::from_config(
        "bot-token".into(),
        Some("general".into()),
        &StartOptions::default(),
    )
    .expect("test state")
    .with_public_source(
        dl_discord::DiscordAdapter::new("synthetic-bot"),
        pool.clone(),
        1,
    );
    state.public_policy.mcp_verified_role_id = Some(3);
    state.discord_api = format!("http://{mock_addr}");
    let options = StartOptions {
        access_gate_cache_ttl_ms: Some(cache_ms),
        access_gate_lookup_timeout_ms: Some(LOOKUP_MS),
        access_gate_decision_timeout_ms: Some(DECISION_MS),
        access_gate_reload_ms: Some(RELOAD_MS),
        access_gate_purge_every_secs: Some(PURGE_SECS),
        access_gate_retention_days: Some(RETENTION_DAYS),
        ..StartOptions::default()
    };
    let settings = GateSettings::from_options(file, Some(999), recorder.then_some(pool), &options)
        .expect("gate settings");
    let state = state
        .with_gate(Some("gate-token".into()), settings)
        .expect("gate state");
    let addr = serve(super::super::router(Arc::new(state))).await;
    let client = GateClient::new(
        &format!("http://{addr}"),
        "gate-token",
        Duration::from_secs(10),
    )
    .expect("gate client");
    Fixture {
        client,
        addr,
        members,
        dir,
    }
}

fn request(user: Option<&str>, capability: &str, class: ActionClass) -> AuthorizeRequest {
    AuthorizeRequest {
        caller: "brain".into(),
        origin: Origin::DiscordMessage,
        user_id: user.map(str::to_owned),
        channel_id: None,
        dm: false,
        capability: capability.into(),
        class,
        internal: false,
        target_user_id: None,
        target_role_id: None,
        args_hash: "0".repeat(64),
        args_redacted: Some(json!({"name":"Turnier","api_token":"hidden"})),
    }
}

async fn reason(fixture: &Fixture, request: &AuthorizeRequest) -> AuthorizeResponse {
    fixture
        .client
        .authorize(request)
        .await
        .expect("authorize answer")
}

fn checked(answer: &AuthorizeResponse, expected: Reason) {
    assert_eq!(answer.reason, expected, "{answer:?}");
    assert_eq!(answer.allowed, expected == Reason::Allowed, "{answer:?}");
    assert_eq!(answer.decision_id.len(), 32);
}

#[tokio::test]
async fn token_and_request_shape_are_enforced() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let wrong = GateClient::new(
        &format!("http://{}", fixture.addr),
        "general",
        Duration::from_secs(5),
    )
    .expect("client");
    let request = request(Some("101"), "discord.event.write", ActionClass::Write);
    assert!(matches!(
        wrong.authorize(&request).await,
        Err(GateError::Status(401))
    ));
    assert!(!wrong.permits(&request).await);
    let raw = reqwest::Client::new();
    for path in [
        dl_access_gate::ACCESS_PATH,
        dl_access_gate::AUTHORIZE_PATH,
        dl_access_gate::OUTCOME_PATH,
    ] {
        let status = raw
            .post(format!("http://{}{path}", fixture.addr))
            .json(&json!({}))
            .send()
            .await
            .expect("answer")
            .status();
        assert_eq!(status.as_u16(), 401, "{path}");
    }
    let malformed = AuthorizeRequest {
        args_hash: "XYZ".into(),
        ..request.clone()
    };
    assert!(matches!(
        fixture.client.authorize(&malformed).await,
        Err(GateError::Status(400))
    ));
    let answer = reason(&fixture, &request).await;
    let outcome = fixture
        .client
        .outcome(&OutcomeRequest {
            decision_id: answer.decision_id,
            caller: "brain".into(),
            status: dl_access_gate::OutcomeStatus::Ok,
            detail: Some("x".repeat(5000)),
        })
        .await
        .expect("outcome answer");
    assert!(outcome.queued);
}

#[tokio::test]
async fn decision_table() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let write = |user| request(Some(user), "discord.event.write", ActionClass::Write);
    checked(&reason(&fixture, &write("100")).await, Reason::MissingRight);
    let streamer = reason(&fixture, &write("101")).await;
    checked(&streamer, Reason::Allowed);
    assert!(streamer.confirm_required && streamer.shadow && streamer.place_ok);
    checked(&reason(&fixture, &write("104")).await, Reason::LookupFailed);
    checked(&reason(&fixture, &write("105")).await, Reason::NotMember);
    checked(
        &reason(
            &fixture,
            &AuthorizeRequest {
                origin: Origin::None,
                ..write("103")
            },
        )
        .await,
        Reason::NoIdentity,
    );
    checked(
        &reason(
            &fixture,
            &request(None, "knowledge.public.read", ActionClass::Read),
        )
        .await,
        Reason::Allowed,
    );
    checked(
        &reason(
            &fixture,
            &AuthorizeRequest {
                caller: "nobody".into(),
                ..write("101")
            },
        )
        .await,
        Reason::UnknownCaller,
    );
    checked(
        &reason(
            &fixture,
            &AuthorizeRequest {
                caller: "off".into(),
                ..write("101")
            },
        )
        .await,
        Reason::ConnectorDisabled,
    );
    let unmapped = |user| request(Some(user), "discord.unknown.thing", ActionClass::Write);
    checked(&reason(&fixture, &unmapped("106")).await, Reason::Unmapped);
    checked(&reason(&fixture, &unmapped("103")).await, Reason::Allowed);
    let owner_thing = |user| request(Some(user), "owner.thing", ActionClass::Destructive);
    checked(
        &reason(&fixture, &owner_thing("103")).await,
        Reason::OwnerOnly,
    );
    checked(
        &reason(&fixture, &owner_thing("999")).await,
        Reason::Allowed,
    );
}

#[tokio::test]
async fn hierarchy_refuses_equal_or_higher_targets() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let ban = |user: &str, target: Option<&str>, role: Option<&str>| AuthorizeRequest {
        target_user_id: target.map(str::to_owned),
        target_role_id: role.map(str::to_owned),
        ..request(
            Some(user),
            "discord.moderation.ban",
            ActionClass::Destructive,
        )
    };
    checked(
        &reason(&fixture, &ban("102", Some("103"), None)).await,
        Reason::Hierarchy,
    );
    checked(
        &reason(&fixture, &ban("102", Some("100"), None)).await,
        Reason::Allowed,
    );
    checked(
        &reason(&fixture, &ban("102", Some("105"), None)).await,
        Reason::Allowed,
    );
    checked(
        &reason(&fixture, &ban("102", Some("104"), None)).await,
        Reason::TargetLookupFailed,
    );
    checked(
        &reason(&fixture, &ban("103", Some("106"), None)).await,
        Reason::Hierarchy,
    );
    checked(
        &reason(&fixture, &ban("103", Some("999"), None)).await,
        Reason::Hierarchy,
    );
    checked(
        &reason(&fixture, &ban("102", None, Some("7"))).await,
        Reason::Hierarchy,
    );
    checked(
        &reason(&fixture, &ban("102", None, Some("6"))).await,
        Reason::Hierarchy,
    );
    checked(
        &reason(&fixture, &ban("102", None, Some("5"))).await,
        Reason::Allowed,
    );
    checked(
        &reason(&fixture, &ban("102", None, Some("77"))).await,
        Reason::TargetLookupFailed,
    );
    checked(
        &reason(&fixture, &ban("999", Some("103"), None)).await,
        Reason::Allowed,
    );
}

#[tokio::test]
async fn place_ok_follows_channel_visibility() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let internal = |channel: &str, dm: bool| AuthorizeRequest {
        channel_id: Some(channel.into()),
        dm,
        internal: true,
        ..request(Some("102"), "discord.moderation.read", ActionClass::Read)
    };
    for (channel, dm, expected) in [
        ("200", false, false),
        ("201", false, true),
        ("203", false, false),
        ("202", true, true),
        ("201", true, false),
    ] {
        let answer = reason(&fixture, &internal(channel, dm)).await;
        checked(&answer, Reason::Allowed);
        assert_eq!(answer.place_ok, expected, "{channel} {dm}");
        assert!(!answer.confirm_required);
        assert!(answer.shadow);
    }
    let unbound = AuthorizeRequest {
        capability: "discord.event.write".into(),
        ..internal("201", false)
    };
    let answer = reason(
        &fixture,
        &AuthorizeRequest {
            user_id: Some("103".into()),
            ..unbound
        },
    )
    .await;
    checked(&answer, Reason::Allowed);
    assert!(!answer.place_ok);
}

#[tokio::test]
async fn access_answers_match_roles_and_fail_closed() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let access = |user: &str, channel: Option<&str>| AccessRequest {
        user_id: user.into(),
        channel_id: channel.map(str::to_owned),
    };
    let moderator = fixture
        .client
        .access(&access("103", Some("201")))
        .await
        .expect("access");
    assert!(moderator.member && !moderator.owner);
    assert!(moderator.permissions.contains(&"administrator".to_owned()));
    assert_eq!(moderator.top_role_position, 30);
    assert!(moderator
        .channel_permissions
        .is_some_and(|names| names.contains(&"view_channel".to_owned())));
    let base = fixture
        .client
        .access(&access("100", Some("201")))
        .await
        .expect("access");
    assert_eq!(base.permissions, ["view_channel"]);
    assert_eq!(base.channel_permissions, Some(Vec::new()));
    assert_eq!(base.role_ids, ["3"]);
    for (user, status) in [
        ("104", MemberStatus::LookupFailed),
        ("105", MemberStatus::NotMember),
    ] {
        let answer = fixture
            .client
            .access(&access(user, None))
            .await
            .expect("access");
        assert_eq!(answer, AccessResponse::no_rights(status));
    }
}

#[tokio::test]
async fn role_removal_is_visible_after_the_cache_time() {
    let cache_ms = 300;
    let fixture = fixture(Some(POLICY), true, cache_ms).await;
    let write = request(Some("101"), "discord.event.write", ActionClass::Write);
    checked(&reason(&fixture, &write).await, Reason::Allowed);
    fixture
        .members
        .lock()
        .expect("members")
        .insert(101, json!({"user":{"id":"101"},"roles":["3"]}));
    checked(&reason(&fixture, &write).await, Reason::Allowed);
    tokio::time::sleep(Duration::from_millis(cache_ms + 100)).await;
    checked(&reason(&fixture, &write).await, Reason::MissingRight);
}

#[tokio::test]
async fn missing_file_or_recorder_never_allows() {
    let write = request(Some("101"), "discord.event.write", ActionClass::Write);
    let no_file = fixture(None, true, CACHE_MS).await;
    checked(
        &reason(&no_file, &write).await,
        Reason::AccessFileUnavailable,
    );
    let no_record = fixture(Some(POLICY), false, CACHE_MS).await;
    checked(&reason(&no_record, &write).await, Reason::RecordUnavailable);
    let mut refused = request(Some("100"), "discord.event.write", ActionClass::Write);
    refused.args_redacted = None;
    checked(&reason(&no_record, &refused).await, Reason::MissingRight);
}

#[tokio::test]
async fn malformed_file_keeps_the_last_good_one() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let file = fixture.dir.path().join("access-gate.toml");
    let write = request(Some("101"), "discord.event.write", ActionClass::Write);
    std::fs::write(
        &file,
        format!("{POLICY}\n[capabilities.\"x\"]\npublic = true\nowner_only = true\n"),
    )
    .expect("bad policy");
    tokio::time::sleep(Duration::from_millis(200)).await;
    checked(&reason(&fixture, &write).await, Reason::Allowed);
    std::fs::write(
        &file,
        POLICY.replace(
            "[connectors.brain]\nenabled = true",
            "[connectors.brain]\nenabled = false",
        ),
    )
    .expect("changed policy");
    tokio::time::sleep(Duration::from_millis(200)).await;
    checked(&reason(&fixture, &write).await, Reason::ConnectorDisabled);
    std::fs::remove_file(&file).expect("remove policy");
    tokio::time::sleep(Duration::from_millis(200)).await;
    checked(&reason(&fixture, &write).await, Reason::ConnectorDisabled);
}

#[test]
fn shipped_access_file_is_valid() {
    let text = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../config/access-gate.toml"
    ));
    let parsed = policy::parse(text).expect("shipped access file");
    assert!(parsed.connectors.values().all(|connector| connector.shadow));
    let event = parsed
        .capabilities
        .get("discord.event.write")
        .expect("event capability");
    assert_eq!(event.mask, EVENTS | (1 << 44));
    for bad in [
        "schema_version = 2",
        "schema_version = 1\nunknown = 1",
        "schema_version = 1\n[capabilities.\"a.b\"]\npermissions = [\"no_such_bit\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\narea = \"missing\"",
        "schema_version = 1\n[capabilities.\"a.b\"]\nroles = [\"abc\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\npublic = true\nroles = [\"5\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\npermissions = [\"administrator\"]",
    ] {
        assert!(policy::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn redaction_hides_secret_keys_and_bounds_size() {
    let value = json!({"name":"x","nested":[{"Api_Key":"s","note":"ok"}],"token":"s"});
    assert_eq!(
        record::bounded(Some(&value)),
        Some(
            json!({"name":"x","nested":[{"Api_Key":"[redacted]","note":"ok"}],"token":"[redacted]"})
        )
    );
    let big = json!({"text":"x".repeat(5000)});
    assert_eq!(
        record::bounded(Some(&big)).and_then(|value| value.get("truncated").cloned()),
        Some(json!(true))
    );
}

#[test]
fn gate_settings_refuse_missing_zero_and_inverted_values() {
    let complete = StartOptions {
        access_gate_cache_ttl_ms: Some(CACHE_MS),
        access_gate_lookup_timeout_ms: Some(LOOKUP_MS),
        access_gate_decision_timeout_ms: Some(DECISION_MS),
        access_gate_reload_ms: Some(RELOAD_MS),
        access_gate_purge_every_secs: Some(PURGE_SECS),
        access_gate_retention_days: Some(RETENTION_DAYS),
        ..StartOptions::default()
    };
    let build = |options: &StartOptions| {
        GateSettings::from_options(PathBuf::from("access-gate.toml"), None, None, options)
    };
    assert!(build(&complete).is_ok());
    let missing = StartOptions {
        access_gate_reload_ms: None,
        ..complete.clone()
    };
    assert!(build(&missing).is_err());
    let zero = StartOptions {
        access_gate_retention_days: Some(0),
        ..complete.clone()
    };
    assert!(build(&zero).is_err());
    let inverted = StartOptions {
        access_gate_decision_timeout_ms: Some(LOOKUP_MS - 1),
        ..complete
    };
    assert!(build(&inverted).is_err());
}
