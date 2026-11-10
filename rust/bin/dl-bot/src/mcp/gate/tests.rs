use super::*;
use crate::mcp::access::{HISTORY, SEND, SEND_DEPENDENT, VIEW};
use axum::{response::IntoResponse, Router};
use dl_access_gate::{GateClient, GateError, Origin};
use dl_core::runtime_config::StartOptions;
use std::sync::Mutex as StdMutex;

const CACHE_MS: u64 = 60_000;
const LOOKUP_MS: u64 = 2_000;
const DECISION_MS: u64 = 6_000;
const RELOAD_MS: u64 = 500;
const SLOW_MS: u64 = 1_500;
const PURGE_SECS: u64 = 21_600;
const RETENTION_DAYS: u64 = 180;

const RETIRED_BITS: [u32; 1] = [47];

fn named(name: &str) -> u64 {
    bits::bit(name).unwrap_or_else(|| panic!("known permission {name}"))
}

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
    db: dl_central_db::testing::TestDb,
}

fn roles() -> Value {
    let ban = named("ban_members");
    json!([
        {"id":"1","permissions":"0","position":0},
        {"id":"3","permissions":VIEW.to_string(),"position":1},
        {"id":"5","permissions":named("manage_events").to_string(),"position":10},
        {"id":"6","permissions":ban.to_string(),"position":20},
        {"id":"7","permissions":(ADMIN | ban).to_string(),"position":30},
        {"id":"8","permissions":ban.to_string(),"position":30}
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
        (110, vec!["3", "6"]),
        (111, vec!["3"]),
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

fn options(cache_ms: u64, decision_ms: u64) -> StartOptions {
    StartOptions {
        access_gate_cache_ttl_ms: Some(cache_ms),
        access_gate_lookup_timeout_ms: Some(LOOKUP_MS),
        access_gate_decision_timeout_ms: Some(decision_ms),
        access_gate_reload_ms: Some(RELOAD_MS),
        access_gate_purge_every_secs: Some(PURGE_SECS),
        access_gate_retention_days: Some(RETENTION_DAYS),
        mcp_discord_timeout_ms: Some(10_000),
        mcp_public_facts_cache_secs: Some(60),
        ..StartOptions::default()
    }
}

async fn fixture(policy: Option<&str>, recorder: bool, cache_ms: u64) -> Fixture {
    fixture_with(policy, recorder, options(cache_ms, DECISION_MS)).await
}

async fn fixture_with(policy: Option<&str>, recorder: bool, options: StartOptions) -> Fixture {
    let members: Members = Arc::new(StdMutex::new(base_members()));
    let mock_members = members.clone();
    let mock = Router::new().fallback(move |request: axum::extract::Request| {
        let members = mock_members.clone();
        async move {
            let path = request.uri().path().to_owned();
            if path.ends_with("/members/110") || path.ends_with("/members/111") {
                tokio::time::sleep(Duration::from_millis(SLOW_MS)).await;
            }
            mock_answer(&path, &members)
        }
    });
    let mock_addr = serve(mock).await;
    let dir = tempfile::tempdir().expect("test dir");
    let file = dir.path().join("access-gate.toml");
    if let Some(policy) = policy {
        std::fs::write(&file, policy).expect("test policy");
    }
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("throwaway test database");
    let pool = db.pool().clone();
    let mut state = McpState::from_config("bot-token".into(), Some("general".into()), &options)
        .expect("test state")
        .with_public_source(
            dl_discord::DiscordAdapter::new("synthetic-bot"),
            pool.clone(),
            1,
        );
    state.public_policy.mcp_verified_role_id = Some(3);
    state.discord_api = format!("http://{mock_addr}");
    let settings = GateSettings::from_options(file, Some(999), recorder.then_some(pool), &options)
        .expect("valid gate settings");
    assert!(settings.is_some(), "gate settings");
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
        db,
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
        args_redacted: json!({"name":"Turnier","api_token":"hidden"})
            .as_object()
            .cloned(),
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
    assert_eq!(
        wrong.verdict(&request).await,
        dl_access_gate::Verdict::Refused
    );
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
    checked(&answer, Reason::Allowed);
    for (path, body) in [
        (dl_access_gate::ACCESS_PATH, json!({"user_id":"0"})),
        (dl_access_gate::ACCESS_PATH, json!({"user_id":"abc"})),
        (
            dl_access_gate::ACCESS_PATH,
            json!({"user_id":"9223372036854775808"}),
        ),
        (
            dl_access_gate::OUTCOME_PATH,
            json!({"decision_id":"XYZ","caller":"brain","status":"ok"}),
        ),
        (
            dl_access_gate::OUTCOME_PATH,
            json!({"decision_id":answer.decision_id,"caller":"Brain","status":"ok"}),
        ),
    ] {
        let status = raw
            .post(format!("http://{}{path}", fixture.addr))
            .bearer_auth("gate-token")
            .json(&body)
            .send()
            .await
            .expect("answer")
            .status();
        assert_eq!(status.as_u16(), 400, "{path} {body}");
    }
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

async fn stored(
    fixture: &Fixture,
    decision_id: &str,
) -> Option<(bool, String, Option<Value>, Option<String>)> {
    sqlx::query_as(
        "SELECT allowed, reason, args_redacted, outcome_detail FROM bot.access_gate_decisions WHERE decision_id = $1",
    )
    .bind(decision_id)
    .fetch_optional(fixture.db.pool())
    .await
    .expect("record query")
}

async fn stored_eventually(
    fixture: &Fixture,
    decision_id: &str,
    done: impl Fn(&(bool, String, Option<Value>, Option<String>)) -> bool,
) -> (bool, String, Option<Value>, Option<String>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(row) = stored(fixture, decision_id).await.filter(|row| done(row)) {
            return row;
        }
        assert!(
            Instant::now() < deadline,
            "record {decision_id} never arrived"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn allows_are_stored_before_the_answer_and_text_loses_nul() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let mut allowed = request(Some("101"), "discord.event.write", ActionClass::Write);
    allowed.args_redacted = json!({"na\u{0}me":"Tur\u{0}nier","list":["a\u{0}b"]})
        .as_object()
        .cloned();
    let answer = reason(&fixture, &allowed).await;
    checked(&answer, Reason::Allowed);
    let (is_allowed, why, args, _) = stored(&fixture, &answer.decision_id)
        .await
        .expect("allow row exists when the answer arrives");
    assert!(is_allowed);
    assert_eq!(why, "allowed");
    assert_eq!(args, Some(json!({"name":"Turnier","list":["ab"]})));
    fixture
        .client
        .outcome(&OutcomeRequest {
            decision_id: answer.decision_id.clone(),
            caller: "brain".into(),
            status: dl_access_gate::OutcomeStatus::Error,
            detail: Some(format!("a\u{0}{}", "x".repeat(5000))),
        })
        .await
        .expect("outcome answer");
    let (_, _, _, detail) =
        stored_eventually(&fixture, &answer.decision_id, |row| row.3.is_some()).await;
    let detail = detail.expect("detail");
    assert!(!detail.contains('\0'));
    assert_eq!(
        detail.chars().count(),
        dl_access_gate::MAX_OUTCOME_DETAIL_CHARS
    );
    let refused = reason(
        &fixture,
        &request(Some("100"), "discord.event.write", ActionClass::Write),
    )
    .await;
    checked(&refused, Reason::MissingRight);
    let (is_allowed, why, _, _) = stored_eventually(&fixture, &refused.decision_id, |_| true).await;
    assert!(!is_allowed);
    assert_eq!(why, "missing_right");
}

#[tokio::test]
async fn late_allow_insert_is_revoked_and_never_shows_as_allowed() {
    let fixture = fixture_with(Some(POLICY), true, options(CACHE_MS, LOOKUP_MS)).await;
    let mut lock = fixture.db.pool().begin().await.expect("lock transaction");
    sqlx::query("LOCK TABLE bot.access_gate_decisions IN SHARE MODE")
        .execute(&mut *lock)
        .await
        .expect("table lock");
    let answer = reason(
        &fixture,
        &request(Some("101"), "discord.event.write", ActionClass::Write),
    )
    .await;
    checked(&answer, Reason::RecordUnavailable);
    assert!(answer.confirm_required && answer.shadow && !answer.place_ok);
    assert!(stored(&fixture, &answer.decision_id).await.is_none());
    lock.commit().await.expect("release lock");
    let (is_allowed, why, _, _) =
        stored_eventually(&fixture, &answer.decision_id, |row| row.1 != "allowed").await;
    assert!(!is_allowed);
    assert_eq!(why, "record_unavailable");
}

#[tokio::test]
async fn every_known_reason_passes_the_record_checks() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    for (index, reason) in Reason::KNOWN.into_iter().enumerate() {
        sqlx::query(
            "INSERT INTO bot.access_gate_decisions (decision_id, caller, origin, dm, capability, class, internal, args_hash, allowed, reason, shadow, confirm_required, place_ok) VALUES ($1, 'brain', 'none', false, 'a.b', 'read', false, $2, $3, $4, true, false, false)",
        )
        .bind(format!("{index:032x}"))
        .bind("0".repeat(64))
        .bind(reason == Reason::Allowed)
        .bind(reason.as_str())
        .execute(fixture.db.pool())
        .await
        .unwrap_or_else(|error| panic!("{}: {error}", reason.as_str()));
    }
}

#[tokio::test]
async fn decision_timeout_refuses_with_confirmation() {
    let fixture = fixture_with(Some(POLICY), true, options(CACHE_MS, LOOKUP_MS)).await;
    let slow = AuthorizeRequest {
        target_user_id: Some("111".into()),
        ..request(Some("110"), "discord.moderation.read", ActionClass::Read)
    };
    let started = Instant::now();
    let answer = reason(&fixture, &slow).await;
    checked(&answer, Reason::LookupFailed);
    assert!(answer.confirm_required && answer.shadow && !answer.place_ok);
    assert!(started.elapsed() < Duration::from_millis(2 * SLOW_MS));
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
    let hidden = fixture
        .client
        .access(&access("101", Some("201")))
        .await
        .expect("access");
    assert!(hidden.permissions.contains(&"manage_events".to_owned()));
    assert_eq!(hidden.channel_permissions, Some(Vec::new()));
    let owner = fixture
        .client
        .access(&access("999", None))
        .await
        .expect("access");
    assert_eq!(
        owner,
        AccessResponse {
            owner: true,
            ..AccessResponse::no_rights(MemberStatus::NotMember)
        }
    );
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
    let cache_ms = 3_000;
    let fixture = fixture(Some(POLICY), true, cache_ms).await;
    let write = request(Some("101"), "discord.event.write", ActionClass::Write);
    checked(&reason(&fixture, &write).await, Reason::Allowed);
    fixture
        .members
        .lock()
        .expect("members")
        .insert(101, json!({"user":{"id":"101"},"roles":["3"]}));
    checked(&reason(&fixture, &write).await, Reason::Allowed);
    tokio::time::sleep(Duration::from_millis(cache_ms + 1_000)).await;
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

async fn eventually(fixture: &Fixture, request: &AuthorizeRequest, expected: Reason) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if reason(fixture, request).await.reason == expected {
            return;
        }
        assert!(Instant::now() < deadline, "never reached {expected:?}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn malformed_file_keeps_the_last_good_one() {
    let fixture = fixture(Some(POLICY), true, CACHE_MS).await;
    let file = fixture.dir.path().join("access-gate.toml");
    let write = request(Some("101"), "discord.event.write", ActionClass::Write);
    let settle = Duration::from_millis(RELOAD_MS * 4);
    std::fs::write(
        &file,
        format!(
            "{}\n[capabilities.\"x\"]\npublic = true\nowner_only = true\n",
            POLICY.replace(
                "[connectors.brain]\nenabled = true",
                "[connectors.brain]\nenabled = false",
            )
        ),
    )
    .expect("bad policy");
    tokio::time::sleep(settle).await;
    checked(&reason(&fixture, &write).await, Reason::Allowed);
    std::fs::write(
        &file,
        POLICY.replace(
            "[connectors.brain]\nenabled = true",
            "[connectors.brain]\nenabled = false",
        ),
    )
    .expect("changed policy");
    eventually(&fixture, &write, Reason::ConnectorDisabled).await;
    std::fs::remove_file(&file).expect("remove policy");
    tokio::time::sleep(settle).await;
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
    assert_eq!(event.mask, named("manage_events") | named("create_events"));
    for bad in [
        "schema_version = 2",
        "schema_version = 1\nunknown = 1",
        "schema_version = 1\n[capabilities.\"a.b\"]\npermissions = [\"no_such_bit\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\narea = \"missing\"",
        "schema_version = 1\n[capabilities.\"a.b\"]\nroles = [\"abc\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\nroles = [\" 5\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\nroles = [\"05\"]",
        "schema_version = 1\n[capabilities.\"a.b\"]\nroles = [\"9223372036854775808\"]",
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
        record::bounded(value.as_object()),
        Some(
            json!({"name":"x","nested":[{"Api_Key":"[redacted]","note":"ok"}],"token":"[redacted]"})
        )
    );
    let big = json!({"text":"x".repeat(5000)});
    assert_eq!(
        record::bounded(big.as_object()).and_then(|value| value.get("truncated").cloned()),
        Some(json!(true))
    );
}

#[test]
fn gate_settings_are_off_when_missing_and_an_error_when_invalid() {
    let complete = options(CACHE_MS, DECISION_MS);
    let build = |options: &StartOptions| {
        GateSettings::from_options(PathBuf::from("access-gate.toml"), None, None, options)
    };
    assert!(matches!(build(&complete), Ok(Some(_))));
    assert!(matches!(
        build(&StartOptions {
            access_gate_reload_ms: None,
            ..complete.clone()
        }),
        Ok(None)
    ));
    for broken in [
        StartOptions {
            access_gate_retention_days: Some(0),
            ..complete.clone()
        },
        StartOptions {
            access_gate_cache_ttl_ms: Some(1),
            ..complete.clone()
        },
        StartOptions {
            access_gate_decision_timeout_ms: Some(LOOKUP_MS - 1),
            ..complete.clone()
        },
    ] {
        assert!(build(&broken).is_err());
    }
}

#[test]
fn reused_gate_token_stops_the_start() {
    let state = || {
        McpState::from_config(
            "bot-token".into(),
            Some("general".into()),
            &options(CACHE_MS, DECISION_MS),
        )
        .expect("state")
    };
    assert!(state().with_gate(Some("general".into()), None).is_err());
    assert!(state().with_gate(Some(" ".into()), None).is_err());
    assert!(state().with_gate(Some("gate".into()), None).is_ok());
}

#[test]
fn wire_ids_and_names_follow_the_record_checks() {
    for good in ["1", "9223372036854775807"] {
        assert!(snowflake_id(good).is_some(), "{good}");
    }
    for bad in [
        "",
        "0",
        "00",
        "012",
        "+12",
        " 12",
        "12 ",
        "-1",
        "9223372036854775808",
        "1a",
    ] {
        assert!(snowflake_id(bad).is_none(), "{bad}");
    }
    let longest = "a".repeat(dl_access_gate::MAX_NAME_CHARS);
    assert!(name_ok(&longest) && name_ok("a.b-c_1"));
    for bad in [
        String::new(),
        format!("{longest}a"),
        "A".into(),
        "a b".into(),
        "\u{e4}".into(),
    ] {
        assert!(!name_ok(&bad), "{bad}");
    }
}

#[test]
fn every_discord_permission_bit_has_a_name() {
    let known = bits::all_known();
    let serenity = serenity::all::Permissions::all().bits();
    assert_eq!(serenity & !known, 0, "serenity knows a bit without a name");
    let highest = 63 - known.leading_zeros();
    for bit in (0..=highest).filter(|bit| !RETIRED_BITS.contains(bit)) {
        let names = bits::names(1 << bit);
        assert!(
            names.len() == 1 && !names[0].starts_with("bit_"),
            "bit {bit} has no name"
        );
        assert_eq!(bits::bit(&names[0]), Some(1 << bit));
    }
    let admin = permission_names(ADMIN);
    assert_eq!(admin, bits::names(known));
    assert!(admin.contains(&"set_voice_channel_status".to_owned()));
    assert_eq!(bits::bit("view_channel"), Some(VIEW));
    assert_eq!(bits::bit("send_messages"), Some(SEND));
    assert_eq!(bits::bit("read_message_history"), Some(HISTORY));
    assert_eq!(bits::bit("administrator"), Some(ADMIN));
    assert_eq!(
        [
            "send_tts_messages",
            "embed_links",
            "attach_files",
            "mention_everyone"
        ]
        .iter()
        .fold(0, |mask, name| mask | named(name)),
        SEND_DEPENDENT
    );
}

#[test]
fn unknown_overwrite_role_is_never_a_safe_place() {
    let policy = policy::parse(POLICY).expect("policy");
    let capability = policy.capabilities.get("discord.moderation.read");
    let roles = parse_roles(1, 3, &roles()).expect("roles");
    let safe = restricted("201", None);
    assert_eq!(
        decide::guild_place(&policy, capability, 201, &safe, &roles),
        Some(Vec::new())
    );
    assert!(!decide::unknown_overwrite_role(&safe, &roles));
    let mut unknown = restricted("201", None);
    if let Some(overwrites) = unknown["permission_overwrites"].as_array_mut() {
        overwrites.push(json!({"id":"99","type":0,"deny":"0","allow":VIEW.to_string()}));
    }
    assert!(decide::unknown_overwrite_role(&unknown, &roles));
    assert_eq!(
        decide::guild_place(&policy, capability, 201, &unknown, &roles),
        None
    );
}

fn probe(base: u64) -> Access {
    Access {
        user_id: Some(100),
        guild: "1".into(),
        roles: vec!["3".into()],
        base,
        timed_out: false,
        top_position: 1,
    }
}

#[test]
fn implicit_denies_follow_view_and_send() {
    let events = named("manage_events");
    let base = VIEW | SEND | SEND_DEPENDENT | events;
    let channel = |deny: u64| json!({"id":"9","guild_id":"1","permission_overwrites":[{"id":"1","type":0,"deny":deny.to_string(),"allow":"0"}]});
    assert_eq!(effective_permissions(&channel(0), &probe(base)), Some(base));
    assert_eq!(effective_permissions(&channel(VIEW), &probe(base)), Some(0));
    assert_eq!(
        effective_permissions(&channel(SEND), &probe(base)),
        Some(VIEW | events)
    );
    assert_eq!(
        effective_permissions(&channel(VIEW), &probe(base | ADMIN)),
        Some(u64::MAX)
    );
}
