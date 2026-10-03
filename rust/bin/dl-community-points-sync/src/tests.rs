//! Cursor- und Seitenlogik gegen einen Fake-Server mit der Seitensemantik
//! aus Paket B (`updated_since` exklusiv, `next_updated_since` = letzte Zeile
//! oder der uebergebene Wert, `null` ohne Zeilen und ohne Cursor).

use super::*;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const TOKEN: &str = "test-token";

#[derive(Default)]
struct FakeState {
    /// (updated_at als Cursor-Text, Zeile)
    rows: Vec<(String, Value)>,
    page_size: usize,
    requests: Vec<HashMap<String, String>>,
    /// Liefert has_more=true, ohne den Cursor zu bewegen.
    stuck: bool,
}

type Shared = Arc<Mutex<FakeState>>;

async fn page_handler(
    State(state): State<Shared>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    if headers
        .get("X-Internal-Token")
        .and_then(|v| v.to_str().ok())
        != Some(TOKEN)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut state = state
        .lock()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    state.requests.push(query.clone());
    let since = query.get("updated_since").cloned();
    if state.stuck {
        return Ok(Json(
            json!({"rows": [], "next_updated_since": since, "has_more": true}),
        ));
    }
    let since_ts = since
        .as_deref()
        .map(|s| timestamp(s).ok_or(StatusCode::BAD_REQUEST));
    let since_ts = since_ts.transpose()?;
    let mut matching: Vec<&(String, Value)> = state
        .rows
        .iter()
        .filter(|(ts, _)| since_ts.is_none_or(|since| timestamp(ts).is_some_and(|t| t > since)))
        .collect();
    matching.sort_by_key(|(ts, _)| timestamp(ts));
    let has_more = matching.len() > state.page_size;
    matching.truncate(state.page_size);
    let next = matching.last().map(|(ts, _)| ts.clone()).or(since);
    let rows: Vec<Value> = matching.iter().map(|(_, row)| row.clone()).collect();
    Ok(Json(
        json!({"rows": rows, "next_updated_since": next, "has_more": has_more}),
    ))
}

async fn fake_server(state: Shared) -> reqwest::Url {
    let app = Router::new()
        .route(VIEWERS_PATH, get(page_handler))
        .route(STREAMERS_PATH, get(page_handler))
        .route(SUGGESTION_OUTCOMES_PATH, get(page_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    validate_base_url(&format!("http://{addr}/")).expect("loopback url")
}

fn viewer_json(viewer: &str, updated_at: &str) -> Value {
    json!({
        "twitch_user_id": viewer,
        "twitch_login": format!("login{viewer}"),
        "channel_twitch_user_id": "456",
        "day": "2026-10-01",
        "watch_minutes": 95,
        "chat_messages": 12,
        "points_watch": 19,
        "points_chat": 12,
        "points_discovery": 10,
        "updated_at": updated_at,
    })
}

fn stamp(i: usize) -> String {
    // Mikrosekunden wie bei Paket B, Cursor muss exakt zurueckgehen.
    format!("2026-10-01T20:00:00.{:06}Z", i + 1)
}

#[derive(Default)]
struct MemorySink {
    state: Mutex<(Option<String>, Vec<ViewerDailyRow>, usize)>,
}

impl PageSink<ViewerDailyRow> for MemorySink {
    async fn cursor(&self) -> anyhow::Result<Option<String>> {
        Ok(self.state.lock().expect("lock").0.clone())
    }
    async fn apply(&self, rows: &[ViewerDailyRow], next: Option<&str>) -> anyhow::Result<u64> {
        let mut state = self.state.lock().expect("lock");
        state.1.extend_from_slice(rows);
        state.2 += 1;
        if let Some(next) = next {
            state.0 = Some(next.to_string());
        }
        Ok(rows.len() as u64)
    }
}

async fn run(state: &Shared, sink: &MemorySink) -> anyhow::Result<SourceSummary> {
    let base = fake_server(state.clone()).await;
    let url = endpoint(&base, VIEWERS_PATH)?;
    sync_source::<ViewerWire, _, _>(&http_client()?, &url, TOKEN, sink, viewer_row).await
}

fn shared(rows: Vec<(String, Value)>, page_size: usize) -> Shared {
    Arc::new(Mutex::new(FakeState {
        rows,
        page_size,
        ..FakeState::default()
    }))
}

#[tokio::test]
async fn liest_alle_seiten_ohne_verlust_und_doppel() {
    let rows = (0..7)
        .map(|i| (stamp(i), viewer_json(&format!("{}", 100 + i), &stamp(i))))
        .collect();
    let state = shared(rows, 3);
    let sink = MemorySink::default();
    let summary = run(&state, &sink).await.expect("sync");
    assert_eq!(summary.pages, 3);
    assert_eq!(summary.fetched, 7);
    assert_eq!(summary.skipped, 0);
    assert_eq!(summary.cursor.as_deref(), Some(stamp(6).as_str()));
    let stored = sink.state.lock().expect("lock");
    let ids: Vec<&str> = stored.1.iter().map(|r| r.twitch_user_id.as_str()).collect();
    assert_eq!(ids, vec!["100", "101", "102", "103", "104", "105", "106"]);
    assert_eq!(stored.0.as_deref(), Some(stamp(6).as_str()));
    let requests = &state.lock().expect("lock").requests;
    assert_eq!(requests[0].get("updated_since"), None);
    assert_eq!(requests[0].get("limit").map(String::as_str), Some("1000"));
    // Cursor exakt so zurueck, wie er kam (Mikrosekunden, Z).
    assert_eq!(
        requests[1].get("updated_since").map(String::as_str),
        Some(stamp(2).as_str())
    );
    assert_eq!(
        requests[2].get("updated_since").map(String::as_str),
        Some(stamp(5).as_str())
    );
}

#[tokio::test]
async fn setzt_beim_gespeicherten_cursor_fort() {
    let rows = (0..4)
        .map(|i| (stamp(i), viewer_json(&format!("{}", 200 + i), &stamp(i))))
        .collect();
    let state = shared(rows, 10);
    let sink = MemorySink::default();
    sink.state.lock().expect("lock").0 = Some(stamp(1));
    let summary = run(&state, &sink).await.expect("sync");
    assert_eq!(summary.pages, 1);
    let stored = sink.state.lock().expect("lock");
    let ids: Vec<&str> = stored.1.iter().map(|r| r.twitch_user_id.as_str()).collect();
    assert_eq!(ids, vec!["202", "203"]);
    assert_eq!(
        state.lock().expect("lock").requests[0]
            .get("updated_since")
            .map(String::as_str),
        Some(stamp(1).as_str())
    );
}

#[tokio::test]
async fn leere_quelle_laesst_cursor_unveraendert() {
    // Ohne Cursor: next_updated_since = null.
    let state = shared(vec![], 10);
    let sink = MemorySink::default();
    let summary = run(&state, &sink).await.expect("sync");
    assert_eq!(summary.pages, 1);
    assert_eq!(summary.cursor, None);
    assert_eq!(sink.state.lock().expect("lock").0, None);

    // Mit Cursor: Quelle gibt ihn unveraendert zurueck.
    let sink = MemorySink::default();
    sink.state.lock().expect("lock").0 = Some(stamp(4));
    let summary = run(&state, &sink).await.expect("sync");
    assert_eq!(summary.cursor.as_deref(), Some(stamp(4).as_str()));
    assert_eq!(summary.fetched, 0);
}

#[tokio::test]
async fn ungueltige_zeilen_werden_uebersprungen_cursor_laeuft_weiter() {
    let mut bad = viewer_json("0123", &stamp(1));
    bad["twitch_user_id"] = json!("0123");
    let mut negative = viewer_json("301", &stamp(2));
    negative["watch_minutes"] = json!(-1);
    let rows = vec![
        (stamp(0), viewer_json("300", &stamp(0))),
        (stamp(1), bad),
        (stamp(2), negative),
    ];
    let state = shared(rows, 10);
    let sink = MemorySink::default();
    let summary = run(&state, &sink).await.expect("sync");
    assert_eq!(summary.fetched, 3);
    assert_eq!(summary.skipped, 2);
    assert_eq!(summary.cursor.as_deref(), Some(stamp(2).as_str()));
    assert_eq!(sink.state.lock().expect("lock").1.len(), 1);
}

#[tokio::test]
async fn has_more_ohne_fortschritt_bricht_ab() {
    let state = shared(vec![], 10);
    state.lock().expect("lock").stuck = true;
    let sink = MemorySink::default();
    sink.state.lock().expect("lock").0 = Some(stamp(0));
    assert!(run(&state, &sink).await.is_err());
    assert_eq!(sink.state.lock().expect("lock").2, 0, "nichts gespeichert");
}

#[tokio::test]
async fn falsches_token_schreibt_nichts() {
    let state = shared(vec![(stamp(0), viewer_json("300", &stamp(0)))], 10);
    let base = fake_server(state.clone()).await;
    let url = endpoint(&base, VIEWERS_PATH).expect("url");
    let sink = MemorySink::default();
    let result = sync_source::<ViewerWire, _, _>(
        &http_client().expect("client"),
        &url,
        "falsch",
        &sink,
        viewer_row,
    )
    .await;
    assert!(result.is_err());
    assert_eq!(sink.state.lock().expect("lock").2, 0);
}

#[test]
fn basis_url_nur_numerisches_loopback() {
    for ok in [
        "http://127.0.0.1:8776",
        "http://127.0.0.1:8776/",
        "https://[::1]:8776/",
    ] {
        assert!(validate_base_url(ok).is_ok(), "{ok}");
    }
    for bad in [
        "http://localhost:8776",
        "http://10.0.0.1:8776",
        "http://user:pw@127.0.0.1:8776",
        "http://127.0.0.1:8776/pfad",
        "http://127.0.0.1:8776/?a=1",
        "ftp://127.0.0.1/",
        "kein-url",
    ] {
        assert!(validate_base_url(bad).is_err(), "{bad}");
    }
    let base = validate_base_url("http://127.0.0.1:8776").expect("ok");
    assert_eq!(
        endpoint(&base, STREAMERS_PATH).expect("join").as_str(),
        "http://127.0.0.1:8776/internal/twitch/v1/community-points/streamers"
    );
}

#[test]
fn zeilen_aus_dem_vertrags_json() {
    let viewer: ViewerWire =
        serde_json::from_value(viewer_json("123", "2026-10-01T20:00:00Z")).expect("viewer json");
    let row = viewer_row(&viewer).expect("gueltig");
    assert_eq!(row.twitch_user_id, "123");
    assert_eq!(row.channel_twitch_user_id, "456");
    assert_eq!(row.day.to_string(), "2026-10-01");
    assert_eq!(
        (row.points_watch, row.points_chat, row.points_discovery),
        (19, 12, 10)
    );

    let streamer: StreamerWire = serde_json::from_value(json!({
        "streamer_twitch_user_id": "456",
        "streamer_login": "name",
        "discord_user_id": "789",
        "day": "2026-10-01",
        "viewer_minutes": 4200,
        "unique_viewers": 61,
        "raids_to_partners": 1,
        "updated_at": "2026-10-01T22:00:00Z"
    }))
    .expect("streamer json");
    let row = streamer_row(&streamer).expect("gueltig");
    assert_eq!(row.discord_user_id, Some(789));
    assert_eq!(row.raids_to_partners, 1);

    let mut ohne_discord = streamer.clone();
    ohne_discord.discord_user_id = None;
    assert_eq!(
        streamer_row(&ohne_discord)
            .expect("gueltig")
            .discord_user_id,
        None
    );
    let mut kaputte_discord = streamer.clone();
    kaputte_discord.discord_user_id = Some("abc".into());
    assert_eq!(
        streamer_row(&kaputte_discord)
            .expect("gueltig")
            .discord_user_id,
        None
    );
    let mut ohne_login = streamer.clone();
    ohne_login.streamer_login = "  ".into();
    assert!(streamer_row(&ohne_login).is_none());
    let mut kaputter_tag = streamer;
    kaputter_tag.day = "01.10.2026".into();
    assert!(streamer_row(&kaputter_tag).is_none());
}

/// Ende-zu-Ende gegen die zentrale Test-DB: Seiten landen idempotent in der
/// Tabelle, Cursor wird gespeichert, ein zweiter Lauf schreibt nichts neu.
#[cfg(feature = "testing")]
#[tokio::test]
async fn ende_zu_ende_in_die_zentrale_db() {
    if std::env::var("CENTRAL_TEST_DSN").is_err() && std::env::var("DATABASE_URL").is_err() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = dl_central_db::testing::test_pool().await.expect("test db");
    let pool = db.pool();
    let rows = (0..5)
        .map(|i| (stamp(i), viewer_json(&format!("{}", 500 + i), &stamp(i))))
        .collect();
    let state = shared(rows, 2);
    let base = fake_server(state.clone()).await;
    let url = endpoint(&base, VIEWERS_PATH).expect("url");
    let client = http_client().expect("client");

    let first =
        sync_source::<ViewerWire, _, _>(&client, &url, TOKEN, &ViewerDbSink(pool), viewer_row)
            .await
            .expect("sync");
    assert_eq!((first.pages, first.written), (3, 5));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM community_points.twitch_viewer_daily")
            .fetch_one(pool)
            .await
            .expect("count");
    assert_eq!(count, 5);
    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS).await.expect("cursor"),
        Some(stamp(4))
    );

    let second =
        sync_source::<ViewerWire, _, _>(&client, &url, TOKEN, &ViewerDbSink(pool), viewer_row)
            .await
            .expect("sync");
    assert_eq!((second.pages, second.fetched, second.written), (1, 0, 0));
    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS).await.expect("cursor"),
        Some(stamp(4))
    );
}

fn outcome(active: bool, suggested_at: &str, partner_since: Option<&str>) -> SuggestionOutcomeWire {
    serde_json::from_value(json!({
        "twitch_user_id": "456",
        "twitch_login": "partner_a",
        "suggested_by_discord_id": "1001",
        "suggested_at": suggested_at,
        "suggestion_count": 2,
        "is_first_eligible": true,
        "candidate_status": "approved",
        "is_partner_active": active,
        "partner_since": partner_since,
        "updated_at": "2026-10-20T10:00:00Z",
    }))
    .expect("vertrags-json")
}

#[test]
fn outcome_berechtigung_und_urspruengliche_privacyfelder_bleiben_gebunden() {
    let mut wire = outcome(true, "2026-10-02T12:00:00Z", None);
    wire.is_first_eligible = false;
    assert!(suggestion_event(&wire).is_none());
    wire.is_first_eligible = true;
    wire.submitted_at = Some("2026-10-02T12:00:00.123456Z".into());
    wire.privacy_epoch = Some(4);
    let event = suggestion_event(&wire).expect("Eigene Zuordnung");
    assert_eq!(event.privacy_epoch, Some(4));
    assert_eq!(
        event
            .submitted_at
            .expect("Herkunft")
            .timestamp_subsec_micros(),
        123456
    );
    wire.submitted_at = Some("keine Zeit".into());
    assert!(suggestion_event(&wire).is_none());
    wire.submitted_at = None;
    wire.privacy_epoch = Some(-1);
    assert!(suggestion_event(&wire).is_none());
    let mut value = serde_json::to_value(json!({
        "twitch_user_id":"456", "suggested_by_discord_id":"42",
        "is_partner_active":true, "updated_at":"2026-10-02T12:00:00Z"
    }))
    .expect("JSON");
    assert!(serde_json::from_value::<SuggestionOutcomeWire>(value.clone()).is_err());
    value["is_first_eligible"] = json!(true);
    assert!(serde_json::from_value::<SuggestionOutcomeWire>(value).is_ok());
}

#[test]
fn vorschlag_bringt_punkte_nur_als_neuer_aktiver_partner() {
    let event = suggestion_event(&outcome(
        true,
        "2026-10-02T12:00:00Z",
        Some("2026-10-15T12:00:00Z"),
    ))
    .expect("punkte");
    assert_eq!(event.event.recipient, LedgerRecipient::Member(1001));
    assert_eq!(event.event.source, SOURCE_STREAMER_SUGGESTION);
    assert_eq!(event.event.reference, "streamer_suggestion:456");
    assert_eq!(event.event.points, 150);
    assert_eq!(
        event.event.occurred_at.to_rfc3339(),
        "2026-10-15T12:00:00+00:00"
    );

    // Noch kein Partner: keine Punkte.
    assert!(suggestion_event(&outcome(false, "2026-10-02T12:00:00Z", None)).is_none());
    // Schon vor dem Vorschlag Partner (Altpartner): keine Punkte.
    assert!(suggestion_event(&outcome(
        true,
        "2026-10-02T12:00:00Z",
        Some("2026-09-01T12:00:00Z")
    ))
    .is_none());
    // Ohne Partnerzeit zählt der Zeitpunkt der Zeile.
    let ohne = suggestion_event(&outcome(true, "2026-10-02T12:00:00Z", None)).expect("punkte");
    assert_eq!(
        ohne.event.occurred_at.to_rfc3339(),
        "2026-10-20T10:00:00+00:00"
    );
    // Kaputte IDs: keine Punkte.
    let mut kaputt = outcome(true, "2026-10-02T12:00:00Z", None);
    kaputt.suggested_by_discord_id = "abc".into();
    assert!(suggestion_event(&kaputt).is_none());
    let mut kaputt = outcome(true, "2026-10-02T12:00:00Z", None);
    kaputt.twitch_user_id = "0".into();
    assert!(suggestion_event(&kaputt).is_none());
}

/// Ergebnisse landen genau einmal je Kanal im Ledger, auch wenn eine Zeile
/// (Partner, Pause, wieder Partner) erneut kommt; Widerspruch bucht nichts.
#[cfg(feature = "testing")]
#[tokio::test]
async fn vorschlags_punkte_einmal_je_kanal_in_die_zentrale_db() {
    if std::env::var("CENTRAL_TEST_DSN").is_err() && std::env::var("DATABASE_URL").is_err() {
        eprintln!("skipping: CENTRAL_TEST_DSN or DATABASE_URL is required");
        return;
    }
    let db = dl_central_db::testing::test_pool().await.expect("test db");
    let pool = db.pool();
    sqlx::query("INSERT INTO core.user_privacy (user_id, opted_out) VALUES (1002, TRUE)")
        .execute(pool)
        .await
        .expect("privacy");
    let row = |id: &str, discord: &str, stamp: &str, since: &str| {
        let mut value = serde_json::to_value(json!({
            "twitch_user_id": id,
            "twitch_login": "x",
            "suggested_by_discord_id": discord,
            "suggested_at": "2026-10-02T12:00:00Z",
            "suggestion_count": 1,
            "is_first_eligible": true,
            "candidate_status": "approved",
            "is_partner_active": true,
            "partner_since": since,
            "updated_at": stamp,
        }))
        .expect("json");
        value["updated_at"] = json!(stamp);
        (stamp.to_string(), value)
    };
    let state = shared(
        vec![
            row("456", "1001", &stamp(0), "2026-10-10T12:00:00Z"),
            row("789", "1002", &stamp(1), "2026-10-11T12:00:00Z"),
            row("456", "1001", &stamp(2), "2026-10-18T12:00:00Z"),
        ],
        2,
    );
    let base = fake_server(state.clone()).await;
    let url = endpoint(&base, SUGGESTION_OUTCOMES_PATH).expect("url");
    let client = http_client().expect("client");
    let summary = sync_source::<SuggestionOutcomeWire, _, _>(
        &client,
        &url,
        TOKEN,
        &SuggestionLedgerSink(pool),
        suggestion_event,
    )
    .await
    .expect("sync");
    assert_eq!((summary.fetched, summary.written), (3, 1));
    let rows: Vec<(i64, String, i32)> = sqlx::query_as(
        "SELECT discord_id, ref, points FROM community_points.ledger
          WHERE source = 'streamer_suggestion'",
    )
    .fetch_all(pool)
    .await
    .expect("ledger");
    assert_eq!(
        rows,
        vec![(1001, "streamer_suggestion:456".to_string(), 150)]
    );
    assert_eq!(
        load_cursor(pool, CURSOR_SUGGESTION_OUTCOMES)
            .await
            .expect("cursor"),
        Some(stamp(2))
    );
}

#[cfg(feature = "testing")]
#[derive(Default)]
struct ActivityFixture {
    queries: Vec<HashMap<String, String>>,
    mismatch: bool,
    empty: bool,
    unavailable: bool,
}
#[cfg(feature = "testing")]
async fn activity_handler(
    State(state): State<Arc<Mutex<ActivityFixture>>>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    if headers
        .get("X-Internal-Token")
        .and_then(|v| v.to_str().ok())
        != Some(TOKEN)
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let mut state = state.lock().expect("Fixture");
    state.queries.push(query.clone());
    if state.unavailable {
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let computed_at = Utc::now().to_rfc3339();
    let mut row = viewer_json(&query["twitch_user_id"], &computed_at);
    row["day"] = json!(query["day"]);
    row["watch_minutes"] = json!(2);
    row["points_watch"] = json!(0);
    row["chat_messages"] = json!(1);
    row["points_chat"] = json!(1);
    row["points_discovery"] = json!(0);
    Ok(Json(
        json!({ "twitch_user_id": if state.mismatch {"999"} else {&query["twitch_user_id"]},
        "day":query["day"], "activity_since":query["activity_since"], "computed_at":computed_at,
        "rows":if state.empty {vec![]} else {vec![row]} }),
    ))
}

#[cfg(feature = "testing")]
#[tokio::test]
async fn consent_rohdaten_bindung_nulltag_fehler_ohne_cursorfortschritt() {
    let db = dl_central_db::testing::test_pool()
        .await
        .expect("Echte Wegwerf-DB");
    let pool = db.pool();
    sqlx::query("INSERT INTO core.user_privacy(user_id, opted_out, reason, updated_at) VALUES (42, false, 'user_opt_in', now())")
        .execute(pool).await.expect("Bewusste Einwilligung");
    sqlx::query("INSERT INTO community_points.twitch_viewer_privacy_blocks(subject_hash) VALUES (sha256(convert_to('community-points:twitch-viewer-privacy:v1:111', 'UTF8')))")
        .execute(pool).await.expect("Gelöschte Vorgeschichte");
    dl_central_db::platform_connections::upsert_twitch_connection(
        pool,
        42,
        &dl_central_db::platform_connections::TwitchConnection {
            twitch_user_id: "111".into(),
            twitch_login: "viewer".into(),
            verified: true,
        },
    )
    .await
    .expect("Bewusste neue Verknüpfung");
    let fixture = Arc::new(Mutex::new(ActivityFixture::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Listener");
    let address = listener.local_addr().unwrap();
    let app = Router::new()
        .route(VIEWER_ACTIVITY_PATH, get(activity_handler))
        .with_state(fixture.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("Testserver");
    });
    let client = http_client().expect("Client");
    let sink = ConsentViewerDbSink {
        pool,
        client: &client,
        activity_url: reqwest::Url::parse(&format!("http://{address}{VIEWER_ACTIVITY_PATH}"))
            .unwrap(),
        token: TOKEN,
    };
    let today = berlin_day(Utc::now());
    let old_wire: ViewerWire = serde_json::from_value(viewer_json("111", &Utc::now().to_rfc3339()))
        .expect("Viewer-Testdaten entsprechen dem Wirevertrag");
    let mut old = viewer_row(&old_wire).unwrap();
    old.day = today;
    assert_eq!(
        sink.apply(&[old.clone()], Some("bound-cursor"))
            .await
            .expect("Rohdaten statt Altwerte"),
        1
    );
    let values: (i32, i32) = sqlx::query_as("SELECT watch_minutes, points_discovery FROM community_points.twitch_viewer_daily WHERE twitch_user_id = '111'")
        .fetch_one(pool).await.expect("Gespeicherte neue Werte");
    assert_eq!(values, (2, 0));
    let consent = load_viewer_consents(pool).await.unwrap().remove(0);
    let queries = fixture.lock().unwrap().queries.clone();
    assert_eq!(queries[0]["twitch_user_id"], "111");
    assert_eq!(
        timestamp(&queries[0]["activity_since"]),
        Some(consent.activity_since)
    );
    fixture.lock().unwrap().mismatch = true;
    assert!(sink
        .apply(&[old.clone()], Some("falsche-bindung"))
        .await
        .is_err());
    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS).await.unwrap().as_deref(),
        Some("bound-cursor")
    );
    fixture.lock().unwrap().mismatch = false;
    fixture.lock().unwrap().unavailable = true;
    assert!(sink.apply(&[old], Some("remoteausfall")).await.is_err());
    assert_eq!(
        load_cursor(pool, CURSOR_VIEWERS).await.unwrap().as_deref(),
        Some("bound-cursor")
    );
    fixture.lock().unwrap().unavailable = false;
    fixture.lock().unwrap().empty = true;
    // Kein normaler Tagescursor nötig: neuer Nullstand wird unabhängig ersetzt.
    assert_eq!(
        sink.refresh_current_day()
            .await
            .expect("Bestätigter Nulltag"),
        0
    );
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM community_points.twitch_viewer_daily WHERE twitch_user_id = '111'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 0);
    assert!(fetch_viewer_activity(
        &client,
        &sink.activity_url,
        "falsches-fixture-token",
        &consent,
        today
    )
    .await
    .is_err());
    server.abort();
}
