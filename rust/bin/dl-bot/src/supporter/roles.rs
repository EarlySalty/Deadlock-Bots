use std::collections::HashSet;

use dl_bridges::http::HttpLimits;
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
    limits: HttpLimits,
) -> Result<Vec<MemberSnapshot>, String> {
    let supporter_role = RoleId::new(SUPPORTER_ROLE_ID);
    let booster_role = RoleId::new(BOOSTER_ROLE_ID);
    let members = tokio::time::timeout(
        limits.request,
        crate::guild_members::fetch_all(http, GuildId::new(guild_id)),
    )
    .await
    .map_err(|_| "Zeitlimit für die Mitgliederliste überschritten".to_string())??;
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
    limits: HttpLimits,
) {
    let guild = GuildId::new(guild_id);
    let role = RoleId::new(SUPPORTER_ROLE_ID);
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
        match tokio::time::timeout(limits.request, operation).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(user_id, grant, %error, "Server-Unterstützer: Rollenänderung fehlgeschlagen")
            }
            Err(error) => {
                tracing::warn!(user_id, grant, %error, "Server-Unterstützer: Zeitlimit für Rollenänderung überschritten")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn limits() -> HttpLimits {
        HttpLimits {
            connect: std::time::Duration::from_millis(50),
            request: std::time::Duration::from_millis(150),
        }
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
            axum::routing::get(move || {
                let seen = seen.clone();
                async move {
                    if seen.fetch_add(1, Ordering::SeqCst) < 2 {
                        std::future::pending::<()>().await;
                    }
                    axum::Json(serde_json::json!([]))
                }
            }),
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
        assert!(fetch_members(&http, 1, limits())
            .await
            .expect("later snapshot")
            .is_empty());
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
        let app = axum::Router::new().route("/api/v10/guilds/{guild}/members", axum::routing::get(move || {
            let seen = seen.clone();
            async move {
                match seen.fetch_add(1, Ordering::SeqCst) {
                    0 => std::future::pending::<axum::Json<serde_json::Value>>().await,
                    1 => axum::Json(serde_json::json!([])),
                    _ => axum::Json(serde_json::json!([{
                        "user": {"id": "2", "username": "member", "discriminator": "0001", "avatar": null},
                        "roles": [], "joined_at": null, "deaf": false, "mute": false, "flags": 0
                    }])),
                }
            }
        }));
        let (http, server) = discord(app).await;
        let serversync = crate::serversync::ServerSyncService::new(
            db.pool().clone(),
            dl_discord::DiscordAdapter::new("test"),
            "test".to_string(),
            1,
            limits(),
            Arc::new(crate::serversync::FixedFundingSource::standard()),
        )
        .expect("serversync");
        let sync = super::super::SupporterSync::new(
            db.pool().clone(),
            Arc::new(http),
            1,
            serversync,
            super::super::SupporterConfig::new(&Default::default(), "http://localhost", |_| None),
            limits(),
        );
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
            tokio::time::timeout(
                std::time::Duration::from_secs(2),
                apply_roles(&http, 1, &[1, 2, 3], grant, limits()),
            )
            .await
            .expect("cycle completed");
        }
        assert_eq!(
            *calls.lock().expect("calls"),
            vec![1, 2, 3, 1, 2, 3, 1, 2, 3]
        );
        server.abort();
    }

    #[tokio::test]
    async fn rate_limiter_waits_are_bounded_and_later_cycles_proceed() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let app = axum::Router::new().route(
            "/api/v10/guilds/{guild}/members/{user}/roles/{role}",
            axum::routing::put(move || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let (mut http, server) = discord(app).await;
        let limiter = serenity::http::Ratelimiter::new(reqwest::Client::new(), "test");
        let routes = limiter.routes();
        let guard = routes.write().await;
        http.ratelimiter = Some(limiter);
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            apply_roles(&http, 1, &[1, 2], true, limits()),
        )
        .await
        .expect("limiter waits completed");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        drop(guard);
        http.ratelimiter = None;
        apply_roles(&http, 1, &[1, 2], true, limits()).await;
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        server.abort();
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
