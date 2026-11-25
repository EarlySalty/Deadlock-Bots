use std::collections::HashSet;
use std::time::Duration;

use serenity::model::id::{GuildId, RoleId, UserId};

use super::{BOOSTER_ROLE_ID, SUPPORTER_ROLE_ID};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemberSnapshot {
    pub id: u64,
    pub has_role: bool,
    pub booster: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RolePlan {
    pub grants: Vec<u64>,
    pub revokes: Vec<u64>,
    pub revokes_held: usize,
}

pub fn plan_roles(members: &[MemberSnapshot], active: &HashSet<u64>, max_revokes: u64) -> RolePlan {
    let mut grants: Vec<u64> = members
        .iter()
        .filter(|member| !member.has_role && active.contains(&member.id))
        .map(|member| member.id)
        .collect();
    let mut revokes: Vec<u64> = members
        .iter()
        .filter(|member| member.has_role && !active.contains(&member.id))
        .map(|member| member.id)
        .collect();
    grants.sort_unstable();
    revokes.sort_unstable();
    let candidates = revokes.len();
    let revokes: Vec<u64> = revokes
        .into_iter()
        .zip(0..max_revokes)
        .map(|(revoke, _)| revoke)
        .collect();
    let revokes_held = candidates - revokes.len();
    RolePlan {
        grants,
        revokes,
        revokes_held,
    }
}

pub async fn fetch_members(
    http: &serenity::http::Http,
    guild_id: u64,
    timeout: Duration,
) -> Result<Vec<MemberSnapshot>, String> {
    let supporter_role = RoleId::new(SUPPORTER_ROLE_ID);
    let booster_role = RoleId::new(BOOSTER_ROLE_ID);
    let members =
        crate::discord_pages::fetch_all_guild_members(http, GuildId::new(guild_id), timeout)
            .await?;
    Ok(members
        .into_iter()
        .filter(|member| !member.user.bot)
        .map(|member| MemberSnapshot {
            id: member.user.id.get(),
            has_role: member.roles.contains(&supporter_role),
            booster: member.premium_since.is_some() || member.roles.contains(&booster_role),
        })
        .collect())
}

pub async fn apply_roles(
    http: &serenity::http::Http,
    guild_id: u64,
    users: &[u64],
    grant: bool,
    timeout: Duration,
) -> Result<(), String> {
    let guild = GuildId::new(guild_id);
    let role = RoleId::new(SUPPORTER_ROLE_ID);
    let mut failed = 0_usize;
    let mut timed_out = 0_usize;
    let mut first_error = None;
    for user_id in users {
        let operation = async {
            if grant {
                http.add_member_role(
                    guild,
                    UserId::new(*user_id),
                    role,
                    Some(super::GRANT_REASON),
                )
                .await
            } else {
                http.remove_member_role(
                    guild,
                    UserId::new(*user_id),
                    role,
                    Some(super::REVOKE_REASON),
                )
                .await
            }
        };
        let error = match tokio::time::timeout(timeout, operation).await {
            Ok(Ok(())) => continue,
            Ok(Err(error)) => error.to_string(),
            Err(_) => {
                timed_out += 1;
                "Zeitlimit überschritten".to_string()
            }
        };
        tracing::debug!(user_id, grant, %error, "Server-Unterstützer: Rollenänderung fehlgeschlagen");
        failed += 1;
        first_error.get_or_insert(error);
    }
    match first_error {
        None => Ok(()),
        Some(error) => Err(format!(
            "{}: {failed} von {} fehlgeschlagen, davon {timed_out} mit Zeitlimit; erster Fehler: {error}",
            if grant { "Rollenvergabe" } else { "Rollenentzug" },
            users.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discord_pages::member_fixture;

    async fn discord(app: axum::Router) -> (serenity::http::Http, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let address = listener.local_addr().expect("address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        let http = serenity::http::HttpBuilder::new("test")
            .proxy(format!("http://{address}"))
            .ratelimiter_disabled(true)
            .build();
        (http, server)
    }

    fn limits() -> Duration {
        Duration::from_millis(150)
    }

    #[tokio::test]
    async fn hanging_member_snapshot_times_out_and_later_cycles_retry() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/v10/guilds/{guild}/members",
            axum::routing::get(
                move |axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| {
                    let seen = seen.clone();
                    async move {
                        if query.contains_key("after") {
                            return axum::Json(serde_json::json!([]));
                        }
                        if seen.fetch_add(1, Ordering::SeqCst) < 2 {
                            std::future::pending::<()>().await;
                        }
                        axum::Json(serde_json::json!([member_fixture(7, &[])]))
                    }
                },
            ),
        );
        let (http, server) = discord(app).await;
        for _ in 0..2 {
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                fetch_members(&http, 1, limits()),
            )
            .await
            .expect("cycle completed");
            assert!(result
                .expect_err("incomplete snapshot rejected")
                .contains("Zeitlimit"));
        }
        assert_eq!(
            fetch_members(&http, 1, limits())
                .await
                .expect("later snapshot")
                .iter()
                .map(|member| member.id)
                .collect::<Vec<_>>(),
            vec![7]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        server.abort();
    }

    #[tokio::test]
    async fn failed_and_empty_snapshots_preserve_boost_rows_until_a_complete_tick() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let db = dl_central_db::testing::test_pool().await.expect("database");
        super::super::store::sync_boost_rows(db.pool(), &[99])
            .await
            .expect("boost source");
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/v10/guilds/{guild}/members",
            axum::routing::get(
                move |axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| {
                    let seen = seen.clone();
                    async move {
                        match seen.fetch_add(1, Ordering::SeqCst) {
                            0 => std::future::pending::<axum::Json<serde_json::Value>>().await,
                            1 => axum::Json(serde_json::json!([])),
                            _ if query.contains_key("after") => axum::Json(serde_json::json!([])),
                            _ => axum::Json(serde_json::json!([member_fixture(2, &[])])),
                        }
                    }
                },
            ),
        );
        let (http, server) = discord(app).await;
        let sync = supporter_sync(&db, http);
        for _ in 0..2 {
            let result = tokio::time::timeout(std::time::Duration::from_secs(2), sync.tick())
                .await
                .expect("tick completed");
            assert!(result.is_err());
            assert!(super::super::store::active_supporters(db.pool())
                .await
                .expect("active")
                .contains(&99));
        }
        sync.tick().await.expect("later complete tick");
        assert!(!super::super::store::active_supporters(db.pool())
            .await
            .expect("active")
            .contains(&99));
        server.abort();
    }

    fn supporter_sync(
        db: &dl_central_db::TestDb,
        http: serenity::http::Http,
    ) -> super::super::SupporterSync {
        let serversync = crate::serversync::ServerSyncService::new(
            db.pool().clone(),
            dl_discord::DiscordAdapter::new("test"),
            "test".to_string(),
            1,
            std::sync::Arc::new(crate::serversync::FixedFundingSource::standard()),
        )
        .expect("serversync");
        let mut config =
            super::super::SupporterConfig::new(&Default::default(), "http://localhost", |_| None);
        config.discord_timeout = limits();
        super::super::SupporterSync::new(
            db.pool().clone(),
            std::sync::Arc::new(http),
            1,
            serversync,
            config,
        )
    }

    #[tokio::test]
    async fn tick_ohne_aktiven_datenschutz_trigger_fragt_discord_nicht_und_schreibt_nichts() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let db = dl_central_db::testing::test_pool().await.expect("database");
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/v10/guilds/{guild}/members",
            axum::routing::get(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!([]))
                }
            }),
        );
        let (http, server) = discord(app).await;
        let sync = supporter_sync(&db, http);
        sqlx::query(
            "ALTER TABLE core.supporter_sources DISABLE TRIGGER supporter_sources_privacy_gate",
        )
        .execute(db.pool())
        .await
        .expect("disable trigger");
        assert!(matches!(
            sync.tick().await,
            Err(super::super::store::StoreError::GateDisabled)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM core.supporter_sources")
            .fetch_one(db.pool())
            .await
            .expect("rows");
        assert_eq!(rows, 0);
        server.abort();
    }

    #[tokio::test]
    async fn hanging_and_failed_roles_allow_other_members_and_later_cycles() {
        use std::sync::{Arc, Mutex};
        let calls = Arc::new(Mutex::new(Vec::new()));
        let seen = calls.clone();
        let handler =
            move |axum::extract::Path((_, user, _)): axum::extract::Path<(u64, u64, u64)>| {
                let seen = seen.clone();
                async move {
                    seen.lock().expect("calls").push(user);
                    match user {
                        1 => std::future::pending::<axum::http::StatusCode>().await,
                        2 => axum::http::StatusCode::FORBIDDEN,
                        _ => axum::http::StatusCode::NO_CONTENT,
                    }
                }
            };
        let app = axum::Router::new().route(
            "/api/v10/guilds/{guild}/members/{user}/roles/{role}",
            axum::routing::put(handler.clone()).delete(handler),
        );
        let (http, server) = discord(app).await;
        for grant in [true, false, true] {
            let outcome = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                apply_roles(&http, 1, &[1, 2, 3], grant, limits()),
            )
            .await
            .expect("cycle completed");
            let action = if grant {
                "Rollenvergabe"
            } else {
                "Rollenentzug"
            };
            assert_eq!(
                outcome
                    .expect_err("failures are reported")
                    .split("; erster Fehler")
                    .next(),
                Some(format!("{action}: 2 von 3 fehlgeschlagen, davon 1 mit Zeitlimit").as_str())
            );
        }
        assert_eq!(
            *calls.lock().expect("calls"),
            vec![1, 2, 3, 1, 2, 3, 1, 2, 3]
        );
        server.abort();
    }

    async fn grants_through_rate_limiter(hold_routes: bool) -> (Result<(), String>, usize) {
        let sink = crate::discord_pages::connection_sink()
            .await
            .expect("connection sink");
        let limiter = serenity::http::Ratelimiter::new(sink.client.clone(), "test");
        let routes = limiter.routes();
        let guard = if hold_routes {
            Some(routes.write().await)
        } else {
            None
        };
        let mut http = serenity::http::HttpBuilder::new("test").build();
        http.ratelimiter = Some(limiter);
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            apply_roles(&http, 1, &[1, 2], true, limits()),
        )
        .await
        .expect("limiter waits completed");
        drop(guard);
        sink.server.abort();
        let connections = sink.connections.load(std::sync::atomic::Ordering::SeqCst);
        (outcome, connections)
    }

    #[tokio::test]
    async fn rate_limiter_waits_are_bounded() {
        let (outcome, connections) = grants_through_rate_limiter(true).await;
        assert!(outcome
            .expect_err("bounded waits are failures")
            .starts_with("Rollenvergabe: 2 von 2 fehlgeschlagen, davon 2 mit Zeitlimit"));
        assert_eq!(connections, 0);
        let (_, connections) = grants_through_rate_limiter(false).await;
        assert!(
            connections > 0,
            "the sink sees requests the limiter lets out"
        );
    }

    #[tokio::test]
    async fn tick_endet_an_der_frist_aus_dem_abgleichsintervall() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let db = dl_central_db::testing::test_pool().await.expect("database");
        sqlx::query(
            "INSERT INTO core.supporter_sources
                 (discord_id, source, source_ref, monthly_cents, valid_until)
             VALUES (5, 'stripe', 'sub_frist', 300, NULL)",
        )
        .execute(db.pool())
        .await
        .expect("supporter source");
        let grants = Arc::new(AtomicUsize::new(0));
        let seen = grants.clone();
        let app = axum::Router::new()
            .route(
                "/api/v10/guilds/{guild}/members",
                axum::routing::get(
                    |axum::extract::Query(query): axum::extract::Query<
                        std::collections::HashMap<String, String>,
                    >| async move {
                        if query.contains_key("after") {
                            axum::Json(serde_json::json!([]))
                        } else {
                            axum::Json(serde_json::json!([member_fixture(5, &[])]))
                        }
                    },
                ),
            )
            .route(
                "/api/v10/guilds/{guild}/members/{user}/roles/{role}",
                axum::routing::put(move || {
                    let seen = seen.clone();
                    async move {
                        seen.fetch_add(1, Ordering::SeqCst);
                        std::future::pending::<axum::http::StatusCode>().await
                    }
                }),
            );
        let (http, server) = discord(app).await;
        let interval = Duration::from_secs(2);
        let outer_guard = Duration::from_secs(10);
        let mut sync = supporter_sync(&db, http);
        sync.config.role_grant_apply = true;
        sync.config.interval = interval;
        sync.config.discord_timeout = outer_guard * 2;
        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(outer_guard, sync.tick_within_deadline())
            .await
            .expect("tick returned before the outer guard");
        let elapsed = started.elapsed();
        server.abort();
        assert!(
            matches!(outcome, Err(super::super::store::StoreError::TickDeadline(limit)) if limit == interval),
            "{outcome:?}"
        );
        assert!(elapsed >= interval);
        assert_eq!(grants.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ein_tick_mit_uebernahme_nimmt_die_tabellensperre_so_oft_wie_eingeplant() {
        let db = dl_central_db::testing::test_pool().await.expect("database");
        let app = axum::Router::new().route(
            "/api/v10/guilds/{guild}/members",
            axum::routing::get(
                |axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| async move {
                    if query.contains_key("after") {
                        axum::Json(serde_json::json!([]))
                    } else {
                        axum::Json(serde_json::json!([member_fixture(
                            8,
                            &[SUPPORTER_ROLE_ID.to_string().as_str()]
                        )]))
                    }
                },
            ),
        );
        let (http, server) = discord(app).await;
        let mut sync = supporter_sync(&db, http);
        sync.config.role_sync_apply = true;
        let (outcome, taken) = super::super::store::TABLE_LOCKS_TAKEN
            .scope(std::cell::Cell::new(0), async {
                let outcome = sync.tick().await;
                let taken = super::super::store::TABLE_LOCKS_TAKEN.with(std::cell::Cell::get);
                (outcome, taken)
            })
            .await;
        server.abort();
        outcome.expect("tick");
        assert!(super::super::store::manual_import_done(db.pool())
            .await
            .expect("import marker"));
        assert_eq!(taken, super::super::store::TABLE_LOCKS_PER_TICK);
    }

    fn member(id: u64, has_role: bool) -> MemberSnapshot {
        MemberSnapshot {
            id,
            has_role,
            booster: false,
        }
    }

    #[test]
    fn plan_vergibt_entzieht_und_deckelt_entzug() {
        let members = vec![
            member(1, false),
            member(2, true),
            member(3, true),
            member(4, true),
            member(5, true),
            member(6, false),
        ];
        let active: HashSet<u64> = [1, 2].into_iter().collect();
        let plan = plan_roles(&members, &active, 2);
        assert_eq!(plan.grants, vec![1]);
        assert_eq!(plan.revokes, vec![3, 4]);
        assert_eq!(plan.revokes_held, 1);
    }

    #[test]
    fn aktive_ausserhalb_des_servers_werden_ignoriert() {
        let members = vec![member(1, true)];
        let active: HashSet<u64> = [1, 99].into_iter().collect();
        assert_eq!(plan_roles(&members, &active, 10), RolePlan::default());
    }
}
