use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    body::Bytes,
    extract::{ConnectInfo, Query, State},
    http::HeaderMap,
    response::Response,
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{authorize, error_body, request_id, respond, success_body, SharedBroker};

#[derive(Debug, Clone)]
pub struct InviteDestination {
    pub guild_id: u64,
    pub channel_id: u64,
    pub streamer_login: String,
    pub streamer_twitch_user_id: String,
    pub invite_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PersonalInvite {
    pub invite_url: String,
    pub personal: bool,
}

#[derive(Debug, Clone)]
pub struct QualifiedQuery {
    pub since: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
    pub after: Option<(DateTime<Utc>, i64)>,
    pub limit: u16,
}

#[async_trait::async_trait]
pub trait TwitchInvitePort: Send + Sync {
    fn guild_id(&self) -> u64;
    async fn destination(&self, streamer_id: &str) -> Result<Option<InviteDestination>, String>;
    async fn personal_invite(
        &self,
        destination: &InviteDestination,
        inviter_id: &str,
    ) -> Result<PersonalInvite, String>;
    async fn qualified_invites(&self, query: &QualifiedQuery) -> Result<Value, String>;
}

#[derive(Clone)]
struct InviteState {
    broker: SharedBroker,
    port: Arc<dyn TwitchInvitePort>,
}

pub fn router(broker: SharedBroker, port: Arc<dyn TwitchInvitePort>) -> Router {
    Router::new()
        .route(
            "/internal/master/v1/twitch/personal-invite",
            post(personal_invite),
        )
        .route(
            "/internal/master/v1/twitch/qualified-invites",
            get(qualified_invites),
        )
        .with_state(InviteState { broker, port })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersonalRequest {
    streamer_login: String,
    streamer_twitch_user_id: String,
    inviter_twitch_user_id: String,
}

fn valid_twitch_id(value: &str) -> bool {
    !value.starts_with('0')
        && !value.is_empty()
        && value.bytes().all(|value| value.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|value| value > 0)
}

fn failure(rid: &str, status: u16, code: &str) -> Response {
    respond(
        status,
        error_body(rid, None, code, "Einladungsanfrage nicht verfügbar"),
    )
}

async fn personal_invite(
    State(state): State<InviteState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = authorize(&state.broker, &peer, &headers, &rid) {
        return response;
    }
    if body.len() > 4096 {
        return failure(&rid, 413, "payload_too_large");
    }
    let Ok(request) = serde_json::from_slice::<PersonalRequest>(&body) else {
        return failure(&rid, 400, "bad_request");
    };
    if !valid_twitch_id(&request.streamer_twitch_user_id)
        || !valid_twitch_id(&request.inviter_twitch_user_id)
        || request.streamer_login.is_empty()
        || request.streamer_login.len() > 25
        || !request
            .streamer_login
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    {
        return failure(&rid, 400, "bad_request");
    }
    let destination = match state
        .port
        .destination(&request.streamer_twitch_user_id)
        .await
    {
        Ok(Some(destination)) => destination,
        Ok(None) => return failure(&rid, 404, "not_found"),
        Err(error) => {
            tracing::warn!(%error, "Twitch-Einladungszuordnung nicht verfügbar");
            return failure(&rid, 503, "unavailable");
        }
    };
    if destination.guild_id == 0
        || destination.guild_id != state.port.guild_id()
        || destination.channel_id == 0
        || destination.streamer_login != request.streamer_login
        || destination.streamer_twitch_user_id != request.streamer_twitch_user_id
        || !state.broker.guild_allowlist.permits(destination.guild_id)
        || !state
            .broker
            .channel_allowlist
            .permits(destination.channel_id)
    {
        return failure(&rid, 403, "forbidden");
    }
    match state
        .port
        .personal_invite(&destination, &request.inviter_twitch_user_id)
        .await
    {
        Ok(result) => respond(200, success_body(&rid, None, json!(result))),
        Err(error) => {
            tracing::warn!(%error, "Persönlicher Twitch-Invite nicht verfügbar");
            failure(&rid, 503, "unavailable")
        }
    }
}

fn parse_query(params: &HashMap<String, String>) -> Option<QualifiedQuery> {
    let timestamp = |value: &str| {
        DateTime::parse_from_rfc3339(value)
            .ok()
            .map(|value| value.with_timezone(&Utc))
    };
    let since = timestamp(params.get("since")?)?;
    let until = match params.get("until") {
        Some(value) => Some(timestamp(value)?),
        None => None,
    };
    if until.is_some_and(|until| until < since || until > Utc::now()) {
        return None;
    }
    let after = match (params.get("after_updated_at"), params.get("after_join_id")) {
        (None, None) => None,
        (Some(updated_at), Some(join_id)) => {
            let updated_at = timestamp(updated_at)?;
            let join_id = join_id.parse::<i64>().ok().filter(|value| *value >= 0)?;
            if until.is_none() || updated_at < since || updated_at > until? {
                return None;
            }
            Some((updated_at, join_id))
        }
        _ => return None,
    };
    let limit = params
        .get("limit")
        .map_or(Some(500), |value| value.parse::<u16>().ok())?;
    (1..=1000).contains(&limit).then_some(QualifiedQuery {
        since,
        until,
        after,
        limit,
    })
}

async fn qualified_invites(
    State(state): State<InviteState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let rid = request_id(&headers);
    if let Err(response) = authorize(&state.broker, &peer, &headers, &rid) {
        return response;
    }
    let guild_id = state.port.guild_id();
    if guild_id == 0 || !state.broker.guild_allowlist.permits(guild_id) {
        return failure(&rid, 403, "forbidden");
    }
    let Some(query) = parse_query(&params) else {
        return failure(&rid, 400, "bad_request");
    };
    match state.port.qualified_invites(&query).await {
        Ok(result) => respond(200, success_body(&rid, None, result)),
        Err(error) => {
            tracing::warn!(%error, "Qualifizierte Einladungen nicht verfügbar");
            failure(&rid, 503, "unavailable")
        }
    }
}

#[cfg(test)]
#[path = "twitch_invites_tests.rs"]
mod http_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_canonical_positive_platform_ids() {
        assert!(valid_twitch_id("123456"));
        for invalid in ["", "0", "0123", "-1", "hello", "18446744073709551616"] {
            assert!(!valid_twitch_id(invalid));
        }
    }

    #[test]
    fn since_and_pagination_are_strict() {
        let mut params = HashMap::new();
        assert!(parse_query(&params).is_none());
        params.insert("since".into(), "2026-01-01T00:00:00Z".into());
        assert!(parse_query(&params).is_some());
        params.insert("limit".into(), "0".into());
        assert!(parse_query(&params).is_none());
        params.insert("limit".into(), "1".into());
        params.insert("after_join_id".into(), "1".into());
        assert!(parse_query(&params).is_none());
        params.insert("after_updated_at".into(), "2026-01-01T12:00:00Z".into());
        assert!(parse_query(&params).is_none());
        params.insert("until".into(), "2026-01-02T00:00:00Z".into());
        assert!(parse_query(&params).is_some());
    }
}
