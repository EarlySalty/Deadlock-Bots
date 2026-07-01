//! DB-gestützter Speicher für delegierte OAuth-States (`oauth_states`).
//!
//! Vertrag 1:1 zu `service/db.py` (`create_state`/`validate_state`/
//! `consume_state`): ein State ist genau einmal einlösbar, läuft nach TTL ab
//! und trägt optionale Metadaten (kompaktes JSON oder roher String). Diese
//! Tabelle gehört weiterhin der gemeinsamen DB; wir legen sie nicht an.

use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;

use crate::db::{unix_to_utc, DashboardDbError, DashboardDbResult};

/// Ein delegierter OAuth-State, wie `validate_state` ihn zurückgibt.
#[derive(Debug, Clone, PartialEq)]
pub struct OAuthState {
    pub state: String,
    pub provider: String,
    pub flow_type: String,
    pub requesting_service: Option<String>,
    pub redirect_after: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub used: bool,
    pub metadata: Option<Value>,
}

/// Eingabe für [`OAuthStateStore::create`].
pub struct NewOAuthState<'a> {
    pub state: &'a str,
    pub provider: &'a str,
    pub flow_type: &'a str,
    pub redirect_after: &'a str,
    pub metadata: Option<&'a Value>,
    pub requesting_service: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("{0} darf nicht leer sein")]
    Empty(&'static str),
    #[error(transparent)]
    Db(#[from] DashboardDbError),
}

#[derive(Clone)]
pub struct OAuthStateStore {
    pool: PgPool,
    default_ttl_secs: i64,
}

impl OAuthStateStore {
    pub fn new(pool: PgPool, default_ttl_secs: i64) -> Self {
        Self {
            pool,
            default_ttl_secs: default_ttl_secs.max(1),
        }
    }

    /// Legt einen State an (oder überschreibt einen bestehenden gleicher ID,
    /// `used` wird dabei zurückgesetzt) und gibt ihn validiert zurück.
    pub async fn create(
        &self,
        new: NewOAuthState<'_>,
        now: i64,
    ) -> Result<Option<OAuthState>, StateError> {
        let state = new.state.trim().to_string();
        let provider = new.provider.trim().to_string();
        let flow_type = new.flow_type.trim().to_string();
        let redirect_after = new.redirect_after.trim().to_string();
        if state.is_empty() {
            return Err(StateError::Empty("state"));
        }
        if provider.is_empty() {
            return Err(StateError::Empty("provider"));
        }
        if flow_type.is_empty() {
            return Err(StateError::Empty("flow_type"));
        }
        if redirect_after.is_empty() {
            return Err(StateError::Empty("redirect_after"));
        }

        let metadata = encode_metadata(new.metadata);
        let requesting_service = derive_requesting_service(new.requesting_service, new.metadata);
        let expires_at = now + self.default_ttl_secs;

        let (s, p, f, rs, ra, md) = (
            state.clone(),
            provider,
            flow_type,
            requesting_service,
            redirect_after,
            metadata,
        );
        let created_at = unix_to_utc(now)?;
        let expires_at_utc = unix_to_utc(expires_at)?;
        sqlx::query!(
            r#"
            INSERT INTO bot.oauth_states(
                state, provider, flow_type, requesting_service,
                redirect_after, created_at, expires_at, used, metadata
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8::text::jsonb)
            ON CONFLICT(state) DO UPDATE SET
                provider = EXCLUDED.provider,
                flow_type = EXCLUDED.flow_type,
                requesting_service = EXCLUDED.requesting_service,
                redirect_after = EXCLUDED.redirect_after,
                created_at = EXCLUDED.created_at,
                expires_at = EXCLUDED.expires_at,
                used = FALSE,
                metadata = EXCLUDED.metadata
            "#,
            s,
            p,
            f,
            rs,
            ra,
            created_at,
            expires_at_utc,
            md,
        )
        .execute(&self.pool)
        .await
        .map_err(DashboardDbError::from)?;

        Ok(self.validate(&state, now).await?)
    }

    /// Gibt den State zurück, falls er existiert, ungenutzt und nicht
    /// abgelaufen ist.
    pub async fn validate(&self, state: &str, now: i64) -> DashboardDbResult<Option<OAuthState>> {
        let state = state.trim().to_string();
        if state.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query!(
            r#"
            SELECT state, provider, flow_type, requesting_service, redirect_after,
                   created_at, expires_at, used AS "used!", metadata::text AS "metadata?"
              FROM bot.oauth_states
             WHERE state = $1
            "#,
            state,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row
            .map(|row| RawState {
                state: row.state,
                provider: row.provider,
                flow_type: row.flow_type,
                requesting_service: row.requesting_service,
                redirect_after: row.redirect_after,
                created_at: row.created_at,
                expires_at: row.expires_at,
                used: row.used,
                metadata: row.metadata,
            })
            .and_then(|r| r.into_valid(now)))
    }

    /// Überschreibt die Metadaten eines States (für das Zurückschreiben des
    /// OAuth-Ergebnisses nach dem Callback — `_store_oauth_state_result`).
    pub async fn update_metadata(&self, state: &str, metadata: &Value) -> DashboardDbResult<()> {
        let state = state.trim().to_string();
        let json = serde_json::to_string(metadata).unwrap_or_else(|_| "null".to_string());
        sqlx::query!(
            r#"
            UPDATE bot.oauth_states
               SET metadata = $1::text::jsonb
             WHERE state = $2
            "#,
            json,
            state,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Markiert den State als eingelöst. `true`, wenn er vorher gültig war —
    /// atomar, sodass paralleles Einlösen nur einmal gelingt.
    pub async fn consume(&self, state: &str, now: i64) -> DashboardDbResult<bool> {
        let state = state.trim().to_string();
        if state.is_empty() {
            return Ok(false);
        }
        let now = unix_to_utc(now)?;
        let rows = sqlx::query!(
            r#"
            UPDATE bot.oauth_states
               SET used = TRUE
             WHERE state = $1
               AND used IS NOT TRUE
               AND expires_at > $2
            "#,
            state,
            now,
        )
        .execute(&self.pool)
        .await?;
        Ok(rows.rows_affected() > 0)
    }
}

struct RawState {
    state: String,
    provider: String,
    flow_type: String,
    requesting_service: Option<String>,
    redirect_after: String,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    used: bool,
    metadata: Option<String>,
}

impl RawState {
    fn into_valid(self, now: i64) -> Option<OAuthState> {
        let created_at = self.created_at.timestamp();
        let expires_at = self.expires_at.timestamp();
        if self.used || expires_at <= now {
            return None;
        }
        Some(OAuthState {
            state: self.state,
            provider: self.provider,
            flow_type: self.flow_type,
            requesting_service: self.requesting_service,
            redirect_after: self.redirect_after,
            created_at,
            expires_at,
            used: self.used,
            metadata: decode_metadata(self.metadata),
        })
    }
}

/// Kodiert Metadaten wie `_encode_oauth_state_metadata`: Strings werden
/// getrimmt, alles andere als kompaktes JSON serialisiert.
fn encode_metadata(metadata: Option<&Value>) -> Option<String> {
    match metadata {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                serde_json::to_string(trimmed).ok()
            }
        }
        Some(value) => serde_json::to_string(value).ok(),
    }
}

/// Liest Metadaten wie `_decode_oauth_state_metadata`: erst JSON versuchen,
/// sonst den rohen Text zurückgeben.
fn decode_metadata(raw: Option<String>) -> Option<Value> {
    let text = raw?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(value) => Some(value),
        Err(_) => Some(Value::String(trimmed.to_string())),
    }
}

fn derive_requesting_service(explicit: Option<&str>, metadata: Option<&Value>) -> Option<String> {
    if let Some(value) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(value.to_string());
    }
    metadata
        .and_then(|m| m.get("requesting_service"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(all(test, feature = "testing"))]
mod tests {
    use super::*;
    use serde_json::json;

    async fn store() -> (dl_central_db::TestDb, OAuthStateStore) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let store = OAuthStateStore::new(db.pool().clone(), 3600);
        (db, store)
    }

    fn new_state<'a>(id: &'a str, meta: Option<&'a Value>) -> NewOAuthState<'a> {
        NewOAuthState {
            state: id,
            provider: "discord",
            flow_type: "delegated:turnier",
            redirect_after: "https://x/cb",
            metadata: meta,
            requesting_service: None,
        }
    }

    #[tokio::test]
    async fn create_validate_consume_lebenszyklus() {
        let (_db, store) = store().await;
        let created = store
            .create(new_state("abc", None), 1000)
            .await
            .expect("create")
            .expect("validiert");
        assert_eq!(created.state, "abc");
        assert_eq!(created.expires_at, 1000 + 3600);
        assert!(!created.used);

        // Validierung vor dem Einlösen: gültig.
        assert!(store.validate("abc", 1000).await.expect("ok").is_some());

        // Einlösen gelingt genau einmal.
        assert!(store.consume("abc", 1000).await.expect("ok"));
        assert!(!store.consume("abc", 1000).await.expect("ok"));

        // Danach ist der State nicht mehr gültig.
        assert!(store.validate("abc", 1000).await.expect("ok").is_none());
    }

    #[tokio::test]
    async fn abgelaufener_state_ist_ungueltig() {
        let (_db, store) = store().await;
        store
            .create(new_state("exp", None), 1000)
            .await
            .expect("ok");
        // now nach Ablauf (1000 + 3600).
        assert!(store.validate("exp", 5000).await.expect("ok").is_none());
        assert!(!store.consume("exp", 5000).await.expect("ok"));
    }

    #[tokio::test]
    async fn metadata_json_und_requesting_service_aus_metadata() {
        let (_db, store) = store().await;
        let meta = json!({"requesting_service": "turnier", "redirect_uri": "https://x"});
        let created = store
            .create(new_state("m", Some(&meta)), 1000)
            .await
            .expect("ok")
            .expect("ok");
        assert_eq!(created.requesting_service.as_deref(), Some("turnier"));
        assert_eq!(
            created
                .metadata
                .as_ref()
                .and_then(|m| m.get("redirect_uri")),
            Some(&json!("https://x"))
        );
    }

    #[tokio::test]
    async fn leere_pflichtfelder_sind_fehler() {
        let (_db, store) = store().await;
        let res = store
            .create(
                NewOAuthState {
                    state: "  ",
                    provider: "discord",
                    flow_type: "f",
                    redirect_after: "https://x",
                    metadata: None,
                    requesting_service: None,
                },
                1000,
            )
            .await;
        assert!(matches!(res, Err(StateError::Empty("state"))));
    }

    #[tokio::test]
    async fn ueberschreiben_setzt_used_zurueck() {
        let (_db, store) = store().await;
        store.create(new_state("re", None), 1000).await.expect("ok");
        assert!(store.consume("re", 1000).await.expect("ok"));
        // Neu anlegen mit gleicher ID → wieder einlösbar.
        store.create(new_state("re", None), 2000).await.expect("ok");
        assert!(store.validate("re", 2000).await.expect("ok").is_some());
        assert!(store.consume("re", 2000).await.expect("ok"));
    }
}
