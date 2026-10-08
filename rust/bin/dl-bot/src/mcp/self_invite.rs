use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Sent,
    Pending,
    FriendshipMissing,
    AlreadyHasGame,
    Error,
    Unknown,
    Unavailable,
}

#[derive(Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Reply {
    status: Status,
    at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {}

#[derive(FromRow)]
struct Link {
    steam_id: String,
    steam_id64: Option<i64>,
    primary_account: bool,
    verified: bool,
}

#[derive(FromRow)]
struct Observation {
    code: Option<String>,
    transport_failed: bool,
    at: DateTime<Utc>,
}

#[derive(FromRow)]
struct Request {
    created_at: i64,
    invite_sent_at: Option<i64>,
    dispatch_task_id: Option<i64>,
    task_status: Option<String>,
    code: Option<String>,
    finished_at: Option<DateTime<Utc>>,
}

impl Reply {
    fn new(status: Status, at: Option<DateTime<Utc>>) -> Self {
        Self { status, at }
    }

    pub(super) fn unavailable() -> Self {
        Self::new(Status::Unavailable, None)
    }
}

fn gc_status(code: &str) -> Status {
    match code.parse::<i64>() {
        Ok(0) => Status::Sent,
        Ok(3) => Status::FriendshipMissing,
        Ok(5) => Status::AlreadyHasGame,
        Ok(1 | 4 | 6 | 7) => Status::Error,
        _ => Status::Unknown,
    }
}

pub(super) async fn read(pool: &PgPool, user: u64, guild: u64, args: &Value) -> Option<Reply> {
    args.as_object().filter(|args| args.is_empty())?;
    serde_json::from_value::<Arguments>(args.clone()).ok()?;
    let user = i64::try_from(user).ok().filter(|id| *id > 0)?;
    let guild = i64::try_from(guild).ok().filter(|id| *id > 0)?;
    Some(read_owned(pool, user, guild).await.unwrap_or_else(|error| {
        let kind = match &error {
            sqlx::Error::Io(_)
            | sqlx::Error::Tls(_)
            | sqlx::Error::PoolTimedOut
            | sqlx::Error::PoolClosed => "connection",
            sqlx::Error::Database(_) => "database",
            _ => "query",
        };
        let sqlstate = error
            .as_database_error()
            .and_then(|error| error.code())
            .filter(|code| {
                code.len() == 5 && code.bytes().all(|byte| byte.is_ascii_alphanumeric())
            });
        tracing::warn!(
            error_kind = kind,
            sqlstate = sqlstate.as_deref().unwrap_or("none"),
            "self_invite_status unavailable"
        );
        Reply::unavailable()
    }))
}

async fn read_owned(pool: &PgPool, user: i64, guild: i64) -> Result<Reply, sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    let links = sqlx::query_as::<_, Link>(
        "SELECT steam_id, steam_id64, primary_account, verified FROM core.steam_links
         WHERE discord_id = $1
         ORDER BY primary_account DESC, verified DESC, steam_id LIMIT 2",
    )
    .bind(user)
    .fetch_all(&mut *tx)
    .await?;
    let Some(link) = links.first().filter(|link| link.verified) else {
        return Ok(Reply::new(Status::Unknown, None));
    };
    if links.get(1).is_some_and(|other| {
        other.primary_account == link.primary_account && other.verified == link.verified
    }) {
        return Ok(Reply::new(Status::Unknown, None));
    }
    let Some(steam) = link.steam_id.parse::<i64>().ok().filter(|id| {
        (76_561_197_960_265_729..=76_561_202_255_233_023).contains(id)
            && id.to_string() == link.steam_id
            && link.steam_id64.is_none_or(|legacy| legacy == *id)
    }) else {
        return Ok(Reply::new(Status::Unknown, None));
    };
    let ambiguous: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM core.steam_links
         WHERE discord_id <> $1 AND (steam_id = $2 OR steam_id64 = $3))",
    )
    .bind(user)
    .bind(&link.steam_id)
    .bind(steam)
    .fetch_one(&mut *tx)
    .await?;
    if ambiguous {
        return Ok(Reply::new(Status::Unknown, None));
    }
    let event = sqlx::query_as::<_, Observation>(
        "SELECT CASE WHEN jsonb_typeof(detail->'code') = 'number' THEN detail->>'code' END AS code,
                decision = 'failed' AS transport_failed, occurred_at AS at FROM steam.bot_event_log
         WHERE (discord_id = $1 OR discord_id IS NULL) AND steam_id = $2 AND event_type = 'playtest_invite'
           AND (jsonb_typeof(detail->'code') = 'number' OR decision = 'failed')
         ORDER BY occurred_at DESC, id DESC LIMIT 1",
    )
    .bind(user)
    .bind(steam)
    .fetch_optional(&mut *tx)
    .await?;
    let request = sqlx::query_as::<_, Request>(
        "SELECT r.created_at, r.invite_sent_at, r.dispatch_task_id, t.status AS task_status,
                CASE WHEN jsonb_typeof(t.result #> '{data,response,code}') = 'number'
                     THEN t.result #>> '{data,response,code}' END AS code,
                CASE WHEN t.status IN ('PENDING', 'RUNNING') THEN t.updated_at
                     ELSE COALESCE(t.finished_at, t.updated_at) END AS finished_at
         FROM steam.invite_requests r
         LEFT JOIN steam.steam_tasks t ON t.id = r.dispatch_task_id
           AND t.type = 'AUTH_SEND_PLAYTEST_INVITE'
           AND (t.payload->>'steam_id' = $3
                OR (t.payload IS NULL AND t.result #>> '{data,steam_id64}' = $3))
           AND (t.result #>> '{data,steam_id64}' IS NULL OR t.result #>> '{data,steam_id64}' = $3)
         WHERE r.steam_id64 = $1 AND (r.target_discord_id = $2 OR r.target_discord_id IS NULL)",
    )
    .bind(steam)
    .bind(user)
    .bind(&link.steam_id)
    .fetch_optional(&mut *tx)
    .await?;
    let audit: Option<DateTime<Utc>> = sqlx::query_scalar(
        "SELECT invited_at FROM steam.beta_invite_audit
         WHERE steam_id64 = $1 AND discord_id IN ($2, 0)
           AND (guild_id = $3 OR guild_id IS NULL)
         ORDER BY invited_at DESC, id DESC LIMIT 1",
    )
    .bind(steam)
    .bind(user)
    .bind(guild)
    .fetch_optional(&mut *tx)
    .await?;
    let task_observation = request
        .as_ref()
        .filter(|r| matches!(r.task_status.as_deref(), Some("DONE" | "FAILED")))
        .and_then(|r| {
            Some(Observation {
                code: Some(r.code.clone()?),
                transport_failed: false,
                at: r.finished_at?,
            })
        });
    let observation = [event, task_observation]
        .into_iter()
        .flatten()
        .max_by_key(|o| o.at);
    let observed_reply =
        observation.map(|observation| {
            Reply::new(
                observation.code.as_deref().map(gc_status).unwrap_or(
                    if observation.transport_failed {
                        Status::Error
                    } else {
                        Status::Unknown
                    },
                ),
                Some(observation.at),
            )
        });
    let request_reply = request.map(|request| {
        let request_at = DateTime::from_timestamp(request.created_at, 0);
        let at = [
            request_at,
            DateTime::from_timestamp(request.invite_sent_at.unwrap_or(request.created_at), 0),
            request.finished_at,
        ]
        .into_iter()
        .flatten()
        .max();
        let current_task = request
            .finished_at
            .is_some_and(|task_at| request_at.is_some_and(|request_at| task_at >= request_at));
        let status = match request.task_status.as_deref().filter(|_| current_task) {
            Some("PENDING" | "RUNNING") => Status::Pending,
            Some("FAILED" | "DONE") if request.code.is_some() => {
                gc_status(request.code.as_deref().unwrap_or_default())
            }
            Some("FAILED") => Status::Error,
            Some("DONE") => Status::Unknown,
            None if request.invite_sent_at.is_some() || request.dispatch_task_id.is_some() => {
                Status::Unknown
            }
            None => Status::Pending,
            _ => Status::Unknown,
        };
        Reply::new(status, at)
    });
    let reply = [observed_reply, request_reply]
        .into_iter()
        .flatten()
        .max_by_key(|reply| reply.at)
        .unwrap_or_else(|| Reply::new(Status::Unknown, audit));
    tx.commit().await?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn scratch_postgres_belegt_status_quellen_und_fail_closed_ownership() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Wegwerf-Postgres");
        sqlx::raw_sql(
            "INSERT INTO core.users(discord_id) VALUES(42),(43);
             INSERT INTO core.steam_links(discord_id,steam_id,steam_id64,verified,primary_account)
             VALUES(42,'76561197960265839',NULL,true,true);",
        )
        .execute(db.pool())
        .await
        .expect("Synthetische Konten");
        let empty = json!({});
        let reply = read(db.pool(), 42, 1, &empty).await.expect("Ergebnis");
        assert_eq!(reply, Reply::new(Status::Unknown, None));
        assert_eq!(
            read(db.pool(), 43, 1, &empty)
                .await
                .expect("Unverknüpft")
                .status,
            Status::Unknown
        );
        assert!(read(db.pool(), 0, 1, &empty).await.is_none());
        assert!(read(db.pool(), 42, 1, &json!({"discord_id":43}))
            .await
            .is_none());
        sqlx::query("INSERT INTO steam.invite_requests(steam_id64,account_id,admin_id,target_discord_id,created_at) VALUES(76561197960265839,111,42,42,1700000000)")
            .execute(db.pool()).await.expect("Offene Bitte");
        assert_eq!(
            read(db.pool(), 42, 1, &empty).await.expect("Ausstehend"),
            Reply::new(Status::Pending, DateTime::from_timestamp(1700000000, 0))
        );
        sqlx::query("INSERT INTO steam.steam_tasks(id,type,payload,status,error,finished_at) VALUES(100,'AUTH_SEND_PLAYTEST_INVITE','{\"steam_id\":\"76561197960265839\"}','FAILED','private Rawfehlermeldung','2026-10-01T10:00:00Z')")
            .execute(db.pool()).await.expect("Fehlgeschlagener Auftrag");
        sqlx::query("UPDATE steam.invite_requests SET dispatch_task_id=100")
            .execute(db.pool())
            .await
            .expect("Auftragsanschluss");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Transportfehler")
                .status,
            Status::Error
        );
        for (code, expected) in [
            (5, Status::AlreadyHasGame),
            (3, Status::FriendshipMissing),
            (0, Status::Sent),
            (4, Status::Error),
            (999, Status::Unknown),
        ] {
            sqlx::query("UPDATE steam.steam_tasks SET result = jsonb_build_object('data',jsonb_build_object('response',jsonb_build_object('code',$1::int,'message','private Antwort'))) WHERE id=100")
                .bind(code).execute(db.pool()).await.expect("GC-Code");
            assert_eq!(
                read(db.pool(), 42, 1, &empty)
                    .await
                    .expect("GC-Ergebnis")
                    .status,
                expected
            );
        }
        for (code, expected) in [(0, Status::Sent), (5, Status::AlreadyHasGame)] {
            sqlx::query("UPDATE steam.steam_tasks SET payload=NULL, status=$1, result=jsonb_build_object('ok',$2::boolean,'data',jsonb_build_object('steam_id64','76561197960265839','response',jsonb_build_object('code',$3::int))) WHERE id=100")
                .bind(if code == 0 { "DONE" } else { "FAILED" })
                .bind(code == 0)
                .bind(code)
                .execute(db.pool()).await.expect("Produktiver Abschluss ohne Payload");
            assert_eq!(
                read(db.pool(), 42, 1, &empty)
                    .await
                    .expect("Ergebnisidentität")
                    .status,
                expected
            );
        }
        sqlx::query("UPDATE steam.steam_tasks SET result='{\"data\":{\"steam_id64\":\"76561197960265950\",\"response\":{\"code\":0}}}' WHERE id=100")
            .execute(db.pool()).await.expect("Fremdes abgeschlossenes Ergebnis");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Keine fremde Ergebnisidentität")
                .status,
            Status::Unknown
        );
        sqlx::query("UPDATE steam.steam_tasks SET payload='{\"steam_id\":\"76561197960265950\"}', result='{\"data\":{\"response\":{\"code\":0}}}' WHERE id=100")
            .execute(db.pool()).await.expect("Fremder Auftrag");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Kein fremdes Ergebnis")
                .status,
            Status::Unknown
        );
        sqlx::query("UPDATE steam.steam_tasks SET type='AUTH_SEND_FRIEND_REQUEST', payload='{\"steam_id\":\"76561197960265839\"}' WHERE id=100")
            .execute(db.pool()).await.expect("Fremder Auftragstyp");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Typbindung")
                .status,
            Status::Unknown
        );
        sqlx::query("UPDATE steam.steam_tasks SET type='AUTH_SEND_PLAYTEST_INVITE', result='{\"data\":{\"steam_id64\":\"76561197960265950\",\"response\":{\"code\":0}}}' WHERE id=100")
            .execute(db.pool()).await.expect("Fremde Ergebnisidentität");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Ergebnisbindung")
                .status,
            Status::Unknown
        );
        sqlx::query("UPDATE steam.steam_tasks SET payload='{\"steam_id\":\"76561197960265839\"}', result='{\"data\":{\"response\":{\"code\":5}}}', finished_at=NULL, updated_at='2026-10-01T10:00:00Z' WHERE id=100")
            .execute(db.pool()).await.expect("GC ohne Endmarker");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("GC-Aufzeichnungszeit")
                .status,
            Status::AlreadyHasGame
        );
        sqlx::raw_sql(
            "DELETE FROM steam.invite_requests;
             INSERT INTO steam.beta_invite_audit(guild_id,discord_id,discord_name,steam_id64,steam_profile,invited_at)
             VALUES(1,42,'privat',76561197960265839,'privat','2026-10-01T12:00:00Z');",
        ).execute(db.pool()).await.expect("Recovery-Audit");
        let recovery_at = "2026-10-01T12:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("Zeitpunkt");
        assert_eq!(
            read(db.pool(), 42, 1, &empty).await.expect("Audit allein"),
            Reply::new(Status::Unknown, Some(recovery_at))
        );
        sqlx::raw_sql(
            "INSERT INTO steam.bot_event_log(event_type,decision,discord_id,steam_id,detail,occurred_at)
             VALUES('playtest_invite','sent',42,76561197960265839,'{\"code\":5,\"message\":\"privat\"}','2026-10-01T10:00:00Z'),
                   ('playtest_invite','audit_recovered',42,76561197960265839,NULL,'2026-10-01T12:00:00Z');",
        ).execute(db.pool()).await.expect("GC und Recovery");
        let gc_at = "2026-10-01T10:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("Zeitpunkt");
        let reply = read(db.pool(), 42, 1, &empty)
            .await
            .expect("Code 5 bleibt erhalten");
        assert_eq!(reply, Reply::new(Status::AlreadyHasGame, Some(gc_at)));
        let projected = serde_json::to_value(reply).expect("Projektion");
        assert_eq!(projected.as_object().expect("Objekt").len(), 2);
        assert!(!projected.to_string().contains("privat"));
        assert!(!projected.to_string().contains("765611"));
        sqlx::query("UPDATE steam.bot_event_log SET discord_id=NULL WHERE detail->>'code'='5'")
            .execute(db.pool())
            .await
            .expect("GC ohne Discord-Zuordnung");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Kanonischer Eigentümer")
                .status,
            Status::AlreadyHasGame
        );
        sqlx::query("INSERT INTO steam.bot_event_log(event_type,decision,steam_id,reason,detail,occurred_at) VALUES('playtest_invite','failed',76561197960265839,'privater Fehler','{\"message\":\"privat\"}','2026-10-01T11:00:00Z')")
            .execute(db.pool()).await.expect("Transportfehler ohne GC");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Keine erfundene Ablehnung")
                .status,
            Status::Error
        );
        sqlx::query("DELETE FROM steam.bot_event_log WHERE decision='failed'")
            .execute(db.pool())
            .await
            .expect("Synthetischen Fehler entfernen");
        sqlx::query("INSERT INTO core.steam_links(discord_id,steam_id,verified,primary_account) VALUES(42,'76561197960265950',true,false)")
            .execute(db.pool()).await.expect("Zweitkonto");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Primäres Konto")
                .status,
            Status::AlreadyHasGame
        );
        sqlx::query("UPDATE core.steam_links SET primary_account=false WHERE discord_id=42")
            .execute(db.pool())
            .await
            .expect("Mehrdeutige Auswahl");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Mehrdeutig")
                .status,
            Status::Unknown
        );
        sqlx::query(
            "UPDATE core.steam_links SET primary_account=true WHERE steam_id='76561197960265839'",
        )
        .execute(db.pool())
        .await
        .expect("Primärwahl");
        assert!(sqlx::query("INSERT INTO core.steam_links(discord_id,steam_id,verified) VALUES(43,'76561197960265839',true)")
            .execute(db.pool()).await.is_err());
        sqlx::raw_sql("DROP INDEX core.uq_steam_links_steam_owner;
             ALTER TABLE core.steam_links DISABLE TRIGGER USER;
             INSERT INTO core.steam_links(discord_id,steam_id,verified) VALUES(43,'76561197960265839',true);
             ALTER TABLE core.steam_links ENABLE TRIGGER USER;")
            .execute(db.pool()).await.expect("Synthetischer Altbestand mit widersprüchlicher Ownership");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Widerspruch")
                .status,
            Status::Unknown
        );
        sqlx::query("DELETE FROM core.steam_links WHERE discord_id=43")
            .execute(db.pool())
            .await
            .expect("Synthetischer Widerspruch entfernt");
        sqlx::query("UPDATE core.steam_links SET verified=false WHERE primary_account")
            .execute(db.pool())
            .await
            .expect("Unverifiziert");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Kein fremdes Ersatzkonto")
                .status,
            Status::Unknown
        );
        sqlx::query("DROP TABLE steam.bot_event_log")
            .execute(db.pool())
            .await
            .expect("Synthetischer Lesefehler");
        sqlx::query("UPDATE core.steam_links SET verified=true WHERE primary_account")
            .execute(db.pool())
            .await
            .expect("Verifiziert");
        assert_eq!(
            read(db.pool(), 42, 1, &empty).await.expect("Lesefehler"),
            Reply::unavailable()
        );
    }

    #[tokio::test]
    async fn neuere_requestzustaende_verdrängen_historische_gc_belege() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Wegwerf-Postgres");
        sqlx::raw_sql(
            "INSERT INTO core.users(discord_id) VALUES(42);
             INSERT INTO core.steam_links(discord_id,steam_id,verified,primary_account)
             VALUES(42,'76561197960265839',true,true);
             INSERT INTO steam.bot_event_log(event_type,decision,discord_id,steam_id,detail,occurred_at)
             VALUES('playtest_invite','sent',42,76561197960265839,'{\"code\":0}','2026-10-01T10:00:00Z');
             INSERT INTO steam.beta_invite_audit(guild_id,discord_id,discord_name,steam_id64,steam_profile,invited_at)
             VALUES(1,42,'privat',76561197960265839,'privat','2026-10-03T12:00:00Z');
             INSERT INTO steam.invite_requests(steam_id64,account_id,admin_id,target_discord_id,created_at)
             VALUES(76561197960265839,111,42,42,1790938800);",
        )
        .execute(db.pool())
        .await
        .expect("Alte Beobachtung und neue Anfrage");
        let empty = json!({});
        let request_at = "2026-10-02T11:00:00Z".parse().expect("Anfragezeit");
        assert_eq!(
            read(db.pool(), 42, 1, &empty).await.expect("Neue Anfrage"),
            Reply::new(Status::Pending, Some(request_at))
        );
        sqlx::raw_sql(
            "INSERT INTO steam.steam_tasks(id,type,payload,status,updated_at,finished_at)
             VALUES(100,'AUTH_SEND_PLAYTEST_INVITE','{\"steam_id\":\"76561197960265839\"}',
                    'FAILED','2026-10-02T12:00:00Z','2026-10-02T12:00:00Z');
             UPDATE steam.invite_requests SET dispatch_task_id=100;",
        )
        .execute(db.pool())
        .await
        .expect("Neuer fehlgeschlagener Auftrag");
        let task_at = "2026-10-02T12:00:00Z".parse().expect("Auftragszeit");
        assert_eq!(
            read(db.pool(), 42, 1, &empty).await.expect("Neuer Fehler"),
            Reply::new(Status::Error, Some(task_at))
        );
        for status in ["PENDING", "RUNNING"] {
            sqlx::query("UPDATE steam.steam_tasks SET status=$1, finished_at=NULL WHERE id=100")
                .bind(status)
                .execute(db.pool())
                .await
                .expect("Aktueller offener Auftrag");
            assert_eq!(
                read(db.pool(), 42, 1, &empty)
                    .await
                    .expect("Neuer offener Auftrag"),
                Reply::new(Status::Pending, Some(task_at))
            );
        }
        sqlx::query("UPDATE steam.steam_tasks SET status='DONE', result='{\"data\":{\"response\":{\"code\":5}}}', finished_at='2026-10-01T10:00:00Z', updated_at='2026-10-01T10:00:00Z' WHERE id=100")
            .execute(db.pool()).await.expect("Alter zugeordneter GC-Auftrag");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Neue Anfrage bleibt maßgeblich"),
            Reply::new(Status::Unknown, Some(request_at))
        );
        sqlx::query("UPDATE steam.steam_tasks SET status='RUNNING', updated_at='2026-10-02T12:00:00Z' WHERE id=100")
            .execute(db.pool()).await.expect("Neuer Lauf mit altem Endmarker");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Aktuelle Auftragszeit"),
            Reply::new(Status::Pending, Some(task_at))
        );
        sqlx::query("UPDATE steam.bot_event_log SET detail='{\"code\":3}', occurred_at='2026-10-02T13:00:00Z'")
            .execute(db.pool()).await.expect("Neueste tatsächliche Beobachtung");
        assert_eq!(
            read(db.pool(), 42, 1, &empty)
                .await
                .expect("Neuere Beobachtung"),
            Reply::new(
                Status::FriendshipMissing,
                Some("2026-10-02T13:00:00Z".parse().expect("Beobachtungszeit"))
            )
        );
    }

    #[tokio::test]
    async fn loopback_mcp_verlangt_auth_und_echten_requester_ohne_fallback() {
        use super::super::{router, McpState};
        use axum::{http::StatusCode, response::IntoResponse, Json, Router};
        use std::sync::Arc;
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Wegwerf-Postgres");
        sqlx::raw_sql("INSERT INTO core.users(discord_id) VALUES(42),(43); INSERT INTO core.steam_links(discord_id,steam_id,verified,primary_account) VALUES(42,'76561197960265839',true,true); INSERT INTO steam.bot_event_log(event_type,decision,discord_id,steam_id,detail,occurred_at) VALUES('playtest_invite','sent',42,76561197960265839,'{\"code\":0}','2026-10-01T10:00:00Z');")
            .execute(db.pool()).await.expect("Synthetische Quelle");
        let mock = Router::new().fallback(|req: axum::extract::Request| async move {
            match req.uri().path() {
                "/guilds/1/members/42" => {
                    Json(json!({"user":{"id":"42"},"roles":["3"]})).into_response()
                }
                "/guilds/1/members/43" => {
                    Json(json!({"user":{"id":"43"},"roles":["3"]})).into_response()
                }
                "/guilds/1/roles" => {
                    Json(json!([{"id":"1","permissions":"0"},{"id":"3","permissions":"1024"}]))
                        .into_response()
                }
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        });
        let discord_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Loopback");
        let discord_addr = discord_listener.local_addr().expect("Adresse");
        let discord_task = tokio::spawn(async move {
            axum::serve(discord_listener, mock)
                .await
                .expect("Mock-Discord");
        });
        let config = dl_core::runtime_config::StartOptions {
            mcp_verified_role_id: Some(3),
            ..Default::default()
        };
        let mut state = McpState::from_config(
            "fixture-bot".into(),
            Some("fixture-internal".into()),
            &config,
        )
        .expect("State")
        .with_public_source(
            dl_discord::DiscordAdapter::new("fixture-bot"),
            db.pool().clone(),
            1,
        );
        state.public_token = Some("fixture-brain".into());
        state.discord_api = format!("http://{discord_addr}");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("Loopback");
        let addr = listener.local_addr().expect("Adresse");
        let task = tokio::spawn(async move {
            axum::serve(listener, router(Arc::new(state)))
                .await
                .expect("MCP");
        });
        let client = reqwest::Client::new();
        for (token, route, user, request, args, expected) in [
            (
                "falsch",
                "/mcp/public",
                Some("42"),
                Some("fixture"),
                json!({}),
                401,
            ),
            (
                "fixture-internal",
                "/mcp/public",
                Some("42"),
                Some("fixture"),
                json!({}),
                401,
            ),
            (
                "fixture-brain",
                "/mcp",
                Some("42"),
                Some("fixture"),
                json!({}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                None,
                Some("fixture"),
                json!({}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("0"),
                Some("fixture"),
                json!({}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("999"),
                Some("fixture"),
                json!({}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("42"),
                None,
                json!({}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("42"),
                Some(""),
                json!({}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("42"),
                Some("fixture"),
                json!({"user_id":43}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("42"),
                Some("fixture"),
                json!({"steam_code":"123456"}),
                403,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("42"),
                Some("fixture"),
                json!({}),
                200,
            ),
            (
                "fixture-brain",
                "/mcp/public",
                Some("43"),
                Some("fixture"),
                json!({}),
                200,
            ),
        ]
        .into_iter()
        .chain(
            [
                Value::Null,
                json!([]),
                json!("invalid"),
                json!(1),
                json!(true),
            ]
            .map(|args| {
                (
                    "fixture-brain",
                    "/mcp/public",
                    Some("42"),
                    Some("fixture"),
                    args,
                    403,
                )
            }),
        ) {
            let mut req = client
                .post(format!("http://{addr}{route}"))
                .bearer_auth(token);
            if let Some(user) = user {
                req = req.header("x-discord-user-id", user);
            }
            if let Some(request) = request {
                req = req.header("x-discord-request-id", request);
            }
            let response = req.json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"self_invite_status","arguments":args}})).send().await.expect("HTTP");
            assert_eq!(response.status().as_u16(), expected);
            if expected == 200 {
                let rpc: Value = response.json().await.expect("JSON-RPC");
                let reply: Reply = serde_json::from_str(
                    rpc["result"]["content"][0]["text"]
                        .as_str()
                        .expect("Projektion"),
                )
                .expect("Minimalvertrag");
                assert_eq!(
                    reply.status,
                    if user == Some("42") {
                        Status::Sent
                    } else {
                        Status::Unknown
                    }
                );
            }
        }
        for (token, user, request, expected) in [
            ("falsch", Some("42"), Some("fixture"), 401),
            ("fixture-internal", Some("42"), Some("fixture"), 401),
            ("fixture-brain", None, Some("fixture"), 403),
            ("fixture-brain", Some("0"), Some("fixture"), 403),
            ("fixture-brain", Some("invalid"), Some("fixture"), 403),
            ("fixture-brain", Some("999"), Some("fixture"), 403),
            ("fixture-brain", Some("42"), None, 403),
            ("fixture-brain", Some("42"), Some(""), 403),
            ("fixture-brain", Some("42"), Some("fixture"), 200),
            ("fixture-brain", Some("43"), Some("fixture"), 200),
        ] {
            let mut req = client
                .post(format!("http://{addr}/mcp/public"))
                .bearer_auth(token);
            if let Some(user) = user {
                req = req.header("x-discord-user-id", user);
            }
            if let Some(request) = request {
                req = req.header("x-discord-request-id", request);
            }
            let response = req
                .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"self_invite_status"}}))
                .send()
                .await
                .expect("HTTP ohne arguments");
            assert_eq!(response.status().as_u16(), expected);
            if expected == 200 {
                let rpc: Value = response.json().await.expect("JSON-RPC ohne arguments");
                assert_eq!(rpc["id"], 2);
                assert_eq!(rpc["result"]["isError"], false);
                let reply: Reply = serde_json::from_str(
                    rpc["result"]["content"][0]["text"]
                        .as_str()
                        .expect("Projektion ohne arguments"),
                )
                .expect("Minimalvertrag ohne arguments");
                assert_eq!(
                    reply,
                    if user == Some("42") {
                        Reply::new(
                            Status::Sent,
                            Some("2026-10-01T10:00:00Z".parse().expect("Beobachtungszeit")),
                        )
                    } else {
                        Reply::new(Status::Unknown, None)
                    }
                );
            }
        }
        task.abort();
        discord_task.abort();
    }

    #[test]
    fn strikter_minimalvertrag_und_gc_semantik() {
        for (code, status) in [
            ("0", Status::Sent),
            ("3", Status::FriendshipMissing),
            ("5", Status::AlreadyHasGame),
            ("1", Status::Error),
            ("4", Status::Error),
            ("6", Status::Error),
            ("7", Status::Error),
            ("999", Status::Unknown),
            ("Freitext", Status::Unknown),
        ] {
            assert_eq!(gc_status(code), status);
        }
        for value in [
            json!({"status":"other","at":null}),
            json!({"status":"sent","at":null,"steam_id":42}),
            json!({"status":"sent","at":"kein Zeitpunkt"}),
        ] {
            assert!(serde_json::from_value::<Reply>(value).is_err());
        }
        assert!(serde_json::from_value::<Arguments>(json!({"user_id":42})).is_err());
        assert!(serde_json::from_value::<Arguments>(json!({"steam_code":"123456"})).is_err());
    }
}
