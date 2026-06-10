//! Build-Vote-Endpunkt inkl. In-Process-Rate-Limit (5 s pro Client-IP).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use dl_webcore::client_ip::client_ip;
use dl_webcore::envelope::error_message;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

use crate::util::now_ts;
use crate::SharedApp;

pub const VOTE_RATE_LIMIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct VoteRateLimiter {
    last_vote: HashMap<String, Instant>,
}

impl VoteRateLimiter {
    /// true = durchlassen (und merken), false = rate-limited.
    pub fn check_and_mark(&mut self, peer: &str, now: Instant) -> bool {
        if let Some(last) = self.last_vote.get(peer) {
            if now.duration_since(*last) < VOTE_RATE_LIMIT {
                return false;
            }
        }
        self.last_vote
            .retain(|_, ts| now.duration_since(*ts) < VOTE_RATE_LIMIT);
        self.last_vote.insert(peer.to_string(), now);
        true
    }
}

pub async fn handle_build_vote(
    State(app): State<SharedApp>,
    Path(build_id): Path<String>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> Response {
    let Ok(build_id) = build_id.trim().parse::<i64>() else {
        return error_message(
            StatusCode::BAD_REQUEST,
            "invalid_build_id",
            "Ungültige Build-ID.",
        );
    };

    let Some(Json(payload)) = body else {
        return error_message(
            StatusCode::BAD_REQUEST,
            "invalid_json",
            "Ungültiger JSON-Body.",
        );
    };
    let vote = payload
        .get("vote")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    if vote != "up" && vote != "down" {
        return error_message(
            StatusCode::BAD_REQUEST,
            "invalid_vote",
            "vote muss 'up' oder 'down' sein.",
        );
    }

    let exists = app
        .db
        .read(move |conn| {
            conn.query_row(
                "SELECT 1 FROM deadlock_hero_builds WHERE build_id = ?1 LIMIT 1",
                [build_id],
                |_| Ok(()),
            )
            .optional()
        })
        .await;
    match exists {
        Ok(Some(())) => {}
        Ok(None) => {
            return error_message(
                StatusCode::NOT_FOUND,
                "build_not_found",
                "Build nicht gefunden.",
            )
        }
        Err(err) => return internal(err),
    }

    let peer_key = {
        let ip = client_ip(&headers, Some(peer));
        if ip.is_empty() {
            "unknown".to_string()
        } else {
            ip
        }
    };
    {
        let mut limiter = app.votes.lock().expect("vote limiter mutex");
        if !limiter.check_and_mark(&peer_key, Instant::now()) {
            return error_message(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Bitte warte kurz, bevor du erneut abstimmst.",
            );
        }
    }

    let up = vote == "up";
    let result = app
        .db
        .write(move |conn| {
            let tx = conn.transaction()?;
            tx.execute(
                "INSERT INTO tierlist_build_votes(build_id, upvotes, downvotes, updated_at)
                 VALUES (?1, 0, 0, ?2) ON CONFLICT(build_id) DO NOTHING",
                (build_id, now_ts()),
            )?;
            let column = if up { "upvotes" } else { "downvotes" };
            tx.execute(
                &format!(
                    "UPDATE tierlist_build_votes SET {column} = {column} + 1, updated_at = ?1
                     WHERE build_id = ?2"
                ),
                (now_ts(), build_id),
            )?;
            let row = tx
                .query_row(
                    "SELECT upvotes, downvotes FROM tierlist_build_votes WHERE build_id = ?1",
                    [build_id],
                    |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
                )
                .optional()?;
            tx.commit()?;
            Ok(row.unwrap_or((0, 0)))
        })
        .await;

    match result {
        Ok((upvotes, downvotes)) => Json(json!({
            "ok": true,
            "build_id": build_id,
            "upvotes": upvotes,
            "downvotes": downvotes,
        }))
        .into_response(),
        Err(err) => internal(err),
    }
}

fn internal(err: impl std::fmt::Display) -> Response {
    tracing::error!(%err, "Vote-Endpunkt: DB-Fehler");
    error_message(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "Interner Serverfehler.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_blockt_und_vergisst() {
        let mut limiter = VoteRateLimiter::default();
        let t0 = Instant::now();
        assert!(limiter.check_and_mark("1.2.3.4", t0));
        assert!(!limiter.check_and_mark("1.2.3.4", t0 + Duration::from_secs(2)));
        // Andere IP ist unabhängig
        assert!(limiter.check_and_mark("5.6.7.8", t0 + Duration::from_secs(2)));
        // Nach Ablauf wieder erlaubt
        assert!(limiter.check_and_mark("1.2.3.4", t0 + Duration::from_secs(6)));
    }
}
