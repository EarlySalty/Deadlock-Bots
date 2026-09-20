//! Fester Dienstvertrag zum vorhandenen Steam-Client. Keine freien Zielpfade.
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    pub revision: String,
    pub patch: Patch,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    pub friends: Option<Friends>,
    pub core: Option<Core>,
    pub rank: Option<Rank>,
    pub accounts: Option<Vec<Account>>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Friends {
    pub slot_limit: Option<i16>,
    pub slot_reserve: Option<i16>,
    pub remove_timeout_secs: Option<u64>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Core {
    pub presence_interval_secs: Option<u64>,
    pub presence_chunk_size: Option<usize>,
    pub presence_chunk_delay_ms: Option<u64>,
}
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Rank {
    pub batch_size: Option<usize>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub id: i16,
    pub presence_enabled: Option<bool>,
    pub catalog_maintenance_enabled: Option<bool>,
}

#[derive(Deserialize, Serialize)]
pub struct Saved {
    pub revision: String,
    pub saved_fingerprint: String,
    pub editable: Editable,
    pub active: Vec<Active>,
    pub restart_required: Option<bool>,
}
#[derive(Deserialize, Serialize)]
pub struct Editable {
    pub friends: SavedFriends,
    pub core: SavedCore,
    pub rank: SavedRank,
    pub accounts: Vec<SavedAccount>,
}
#[derive(Deserialize, Serialize)]
pub struct SavedFriends {
    pub slot_limit: i16,
    pub slot_reserve: i16,
    pub remove_timeout_secs: u64,
}
#[derive(Deserialize, Serialize)]
pub struct SavedCore {
    pub presence_interval_secs: u64,
    pub presence_chunk_size: usize,
    pub presence_chunk_delay_ms: u64,
}
#[derive(Deserialize, Serialize)]
pub struct SavedRank {
    pub batch_size: usize,
}
#[derive(Deserialize, Serialize)]
pub struct SavedAccount {
    pub id: i16,
    pub presence_enabled: bool,
    pub catalog_maintenance_enabled: bool,
}
#[derive(Deserialize, Serialize)]
pub struct Active {
    pub service: String,
    pub config_fingerprint: Option<String>,
    pub state: ActiveState,
    pub observed_at: Option<i64>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveState {
    Current,
    RestartRequired,
    Unknown,
}

#[derive(Debug)]
pub enum Error {
    Unavailable,
    Conflict,
    Invalid,
    Upstream,
}

impl super::steam::SteamBotClient {
    pub async fn operating_config(&self, save: Option<&SaveRequest>) -> Result<Saved, Error> {
        let token = self
            .token
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or(Error::Unavailable)?;
        let url = format!("{}/internal/config", self.base_url);
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Unavailable)?;
        let mut request = match save {
            Some(body) => http.patch(url).json(body),
            None => http.get(url),
        };
        request = request
            .header("x-internal-token", token)
            .timeout(std::time::Duration::from_secs(10));
        let response = request.send().await.map_err(|_| Error::Unavailable)?;
        match response.status().as_u16() {
            200 => {}
            409 => return Err(Error::Conflict),
            400 | 422 => return Err(Error::Invalid),
            401 | 403 | 503 => return Err(Error::Unavailable),
            _ => return Err(Error::Upstream),
        }
        // Den Dienstvertrag neu serialisieren, niemals beliebige Upstreamfelder
        // oder Fehlermeldungen (möglicherweise mit internen Details) weitergeben.
        let mut response = response;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Upstream)? {
            if bytes.len() + chunk.len() > 65536 {
                return Err(Error::Upstream);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| Error::Upstream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        http::{HeaderMap, StatusCode},
        routing::get,
        Json, Router,
    };
    use serde_json::json;

    #[test]
    fn requests_reject_fields_outside_the_operating_contract() {
        for raw in [
            json!({"revision":"x", "patch":{"model":"other"}}),
            json!({"revision":"x", "patch":{"accounts":[{"id":1,"steam_id":"other"}]}}),
            json!({"revision":"x", "patch":{"friends":{"slot_limit":1,"url":"http://elsewhere"}}}),
        ] {
            assert!(serde_json::from_value::<SaveRequest>(raw).is_err());
        }
    }

    #[tokio::test]
    async fn proxy_sends_existing_token_and_filters_upstream_fields() {
        let server = Router::new().route("/internal/config", get(|headers: HeaderMap| async move {
            assert_eq!(headers["x-internal-token"], "synthetic-token");
            Json(json!({
                "revision":"a".repeat(64), "saved_fingerprint":"b".repeat(64),
                "editable": {"friends":{"slot_limit":305,"slot_reserve":2,"remove_timeout_secs":25},"core":{"presence_interval_secs":60,"presence_chunk_size":50,"presence_chunk_delay_ms":250},"rank":{"batch_size":10},"accounts":[{"id":1,"presence_enabled":true,"catalog_maintenance_enabled":false}]},
                "active":[{"service":"steam-core-1","config_fingerprint":null,"state":"unknown","observed_at":null}],"restart_required":null,
                "not_for_browser":"synthetic-sensitive-field"
            }))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("isolierte Testdaten gültig");
        let address = listener.local_addr().expect("isolierte Testdaten gültig");
        let task = tokio::spawn(async move {
            axum::serve(listener, server)
                .await
                .expect("isolierte Testdaten gültig");
        });
        let client = crate::steam::SteamBotClient::new(
            format!("http://{address}"),
            Some("synthetic-token".into()),
        );
        let response = serde_json::to_value(
            client
                .operating_config(None)
                .await
                .expect("isolierte Testdaten gültig"),
        )
        .expect("isolierte Testdaten gültig");
        assert!(response.get("not_for_browser").is_none());
        assert_eq!(response["active"][0]["state"], "unknown");
        task.abort();
    }

    #[tokio::test]
    async fn redirects_are_not_followed_and_conflicts_remain_conflicts() {
        let server = Router::new()
            .route(
                "/internal/config",
                get(|| async {
                    (
                        StatusCode::TEMPORARY_REDIRECT,
                        [("location", "/unexpected")],
                    )
                })
                .patch(|| async { StatusCode::CONFLICT }),
            )
            .route(
                "/unexpected",
                get(|| async {
                    panic!("Diensttoken darf keinem Redirect folgen");
                    #[allow(unreachable_code)]
                    StatusCode::OK
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("isolierte Testdaten gültig");
        let address = listener.local_addr().expect("isolierte Testdaten gültig");
        let task = tokio::spawn(async move {
            axum::serve(listener, server)
                .await
                .expect("isolierte Testdaten gültig");
        });
        let client = crate::steam::SteamBotClient::new(
            format!("http://{address}"),
            Some("synthetic-token".into()),
        );
        assert!(matches!(
            client.operating_config(None).await,
            Err(Error::Upstream)
        ));
        let request = SaveRequest {
            revision: "a".repeat(64),
            patch: Patch::default(),
        };
        assert!(matches!(
            client.operating_config(Some(&request)).await,
            Err(Error::Conflict)
        ));
        task.abort();
    }
}
