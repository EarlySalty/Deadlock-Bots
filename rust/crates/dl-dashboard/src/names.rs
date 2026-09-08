//! Namensauflösung User-ID → Anzeigename für die Analytics-Reads.
//!
//! Pythons `_resolve_display_names` liest den Bot-Member-Cache. dl-web hat
//! keinen Gateway, also geht das über den Master-Broker
//! (`GET /internal/master/v1/discord/resolve-names`, Bulk). Nicht gefundene
//! IDs werden zusätzlich aus der gespeicherten Namenshistorie aufgelöst.
//! Dadurch bleiben frühere Mitglieder auch nach einem Bot-Neustart erkennbar.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::Value;
use sqlx::PgPool;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// Löst mehrere User-IDs gebündelt zu Anzeigenamen auf.
#[async_trait::async_trait]
pub trait NameResolver: Send + Sync {
    /// Liefert nur die gefundenen Namen; fehlende IDs bleiben weg.
    async fn resolve(&self, user_ids: &[u64]) -> HashMap<u64, String>;
}

/// Hilfsfunktion: Map auffüllen wie `_resolve_display_names` (`User <id>`).
pub fn display_name_or_default(names: &HashMap<u64, String>, user_id: u64) -> String {
    names
        .get(&user_id)
        .cloned()
        .unwrap_or_else(|| format!("User {user_id}"))
}

/// Snowflakes bleiben auch oberhalb von JavaScripts Ganzzahlgrenze exakt.
pub fn discord_id_json(user_id: i64) -> Value {
    Value::String(user_id.to_string())
}

const STORED_NAMES: &str = r#"
    SELECT discord_id AS user_id,
           COALESCE(NULLIF(global_name, ''), NULLIF(username, '')) AS name,
           last_seen AS seen_at
      FROM core.users
    UNION ALL
    SELECT user_id, NULLIF(display_name, ''), occurred_at FROM activity.member_events
    UNION ALL
    SELECT user_id, NULLIF(display_name, ''), ended_at FROM activity.voice_session_log
    UNION ALL
    SELECT user_id, NULLIF(user_display_name, ''), last_played_together
      FROM activity.user_co_players
    UNION ALL
    SELECT co_player_id, NULLIF(co_player_display_name, ''), last_played_together
      FROM activity.user_co_players
"#;

/// Erst aktuelle Broker-Namen, dann der jüngste gespeicherte Name derselben ID.
pub async fn resolve_with_history(
    pool: &PgPool,
    resolver: &dyn NameResolver,
    user_ids: &[u64],
) -> HashMap<u64, String> {
    let mut names = resolver.resolve(user_ids).await;
    names.retain(|_, name| !name.trim().is_empty());
    let mut missing: Vec<i64> = user_ids
        .iter()
        .filter(|id| !names.contains_key(id))
        .filter_map(|id| i64::try_from(*id).ok())
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if missing.is_empty() {
        return names;
    }
    let query = format!(
        "SELECT DISTINCT ON (user_id) user_id, name FROM ({STORED_NAMES}) names \
         WHERE user_id = ANY($1) AND name IS NOT NULL \
         ORDER BY user_id, seen_at DESC NULLS LAST, name"
    );
    match sqlx::query_as::<_, (i64, String)>(&query)
        .bind(&missing)
        .fetch_all(pool)
        .await
    {
        Ok(rows) => {
            for (id, name) in rows {
                if let Ok(id) = u64::try_from(id) {
                    names.insert(id, name);
                }
            }
        }
        Err(err) => tracing::warn!(%err, "Gespeicherte Discord-Namen konnten nicht geladen werden"),
    }
    names
}

/// Namenssuche liefert Plattform-IDs zur expliziten Auswahl, niemals einen
/// willkürlichen ersten Treffer als Identität. Auch frühere Namen sind suchbar.
pub async fn search_users(
    pool: &PgPool,
    resolver: &dyn NameResolver,
    query: &str,
) -> Result<Vec<Value>, sqlx::Error> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "WITH names AS ({STORED_NAMES}), matches AS (\
           SELECT user_id, MAX(seen_at) AS seen_at FROM names \
           WHERE user_id::text = $1 OR strpos(lower(name), lower($1)) > 0 \
           GROUP BY user_id ORDER BY (user_id::text = $1) DESC, MAX(seen_at) DESC NULLS LAST, user_id LIMIT 20\
         ) SELECT matches.user_id, COALESCE(latest.name, 'User ' || matches.user_id::text) \
           FROM matches LEFT JOIN LATERAL (\
             SELECT name FROM names WHERE names.user_id = matches.user_id AND name IS NOT NULL \
             ORDER BY seen_at DESC NULLS LAST, name LIMIT 1\
           ) latest ON TRUE \
          ORDER BY (matches.user_id::text = $1) DESC, matches.seen_at DESC NULLS LAST, matches.user_id"
    );
    let rows = sqlx::query_as::<_, (i64, String)>(&sql)
        .bind(query)
        .fetch_all(pool)
        .await?;
    let ids: Vec<u64> = rows
        .iter()
        .filter_map(|(id, _)| u64::try_from(*id).ok())
        .collect();
    let current = resolver.resolve(&ids).await;
    Ok(rows
        .into_iter()
        .map(|(id, stored)| {
            let name = u64::try_from(id)
                .ok()
                .and_then(|id| current.get(&id))
                .filter(|name| !name.trim().is_empty())
                .cloned()
                .unwrap_or(stored);
            serde_json::json!({ "user_id": discord_id_json(id), "display_name": name })
        })
        .collect())
}

#[cfg(test)]
mod identity_tests {
    use super::*;

    #[test]
    fn snowflake_bleibt_beim_json_roundtrip_exakt() {
        let id = 1_411_350_229_747_241_010_i64;
        let encoded = serde_json::to_string(&discord_id_json(id)).expect("gültige Testdaten");
        assert_eq!(encoded, "\"1411350229747241010\"");
        assert_eq!(
            serde_json::from_str::<Value>(&encoded)
                .expect("gültige Testdaten")
                .as_str(),
            Some("1411350229747241010")
        );
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn historische_namen_bleiben_ohne_gateway_suchbar_und_aufloesbar() {
        struct EmptyResolver;
        #[async_trait::async_trait]
        impl NameResolver for EmptyResolver {
            async fn resolve(&self, _: &[u64]) -> HashMap<u64, String> {
                HashMap::new()
            }
        }
        let db = crate::db::test_pool().await.expect("Testdatenbank");
        sqlx::query(
            "INSERT INTO core.users(discord_id,username,global_name,last_seen) \
             VALUES(1411350229747241010,'philipp','Philipp','2026-09-01T00:00:00Z')",
        )
        .execute(db.pool())
        .await
        .expect("gültige Testdaten");
        sqlx::query(
            "INSERT INTO activity.member_events(id,user_id,guild_id,event_type,occurred_at,display_name) \
             VALUES(1,1411350229747241010,1,'join','2026-08-01T00:00:00Z','Alter Name')",
        ).execute(db.pool()).await.expect("gültige Testdaten");
        let names =
            resolve_with_history(db.pool(), &EmptyResolver, &[1_411_350_229_747_241_010]).await;
        assert_eq!(
            names.get(&1_411_350_229_747_241_010).map(String::as_str),
            Some("Philipp")
        );
        let found = search_users(db.pool(), &EmptyResolver, "alter name")
            .await
            .expect("gültige Testdaten");
        assert_eq!(
            found,
            vec![serde_json::json!({
                "user_id": "1411350229747241010", "display_name": "Philipp"
            })]
        );
        assert!(search_users(db.pool(), &EmptyResolver, "%")
            .await
            .expect("gültige Testdaten")
            .is_empty());
        assert!(search_users(db.pool(), &EmptyResolver, "")
            .await
            .expect("gültige Testdaten")
            .is_empty());
    }
}

/// Broker-gestützter Resolver.
#[derive(Clone)]
pub struct BrokerNameResolver {
    http: reqwest::Client,
    base: String,
}

impl BrokerNameResolver {
    pub fn new(base: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .build()
            .expect("reqwest-Client bauen");
        Self {
            http,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }
}

#[async_trait::async_trait]
impl NameResolver for BrokerNameResolver {
    async fn resolve(&self, user_ids: &[u64]) -> HashMap<u64, String> {
        if user_ids.is_empty() {
            return HashMap::new();
        }
        let ids = user_ids
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let url = format!("{}/internal/master/v1/discord/resolve-names", self.base);
        let response = match self.http.get(&url).query(&[("user_ids", ids)]).send().await {
            Ok(r) => r,
            Err(err) => {
                tracing::warn!(%err, "Broker resolve-names nicht erreichbar");
                return HashMap::new();
            }
        };
        if !response.status().is_success() {
            return HashMap::new();
        }
        let Ok(data) = response.json::<Value>().await else {
            return HashMap::new();
        };
        data.get("names")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(id, name)| {
                        Some((id.parse::<u64>().ok()?, name.as_str()?.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fuellt_fehlende_auf() {
        let mut names = HashMap::new();
        names.insert(42u64, "Nani".to_string());
        assert_eq!(display_name_or_default(&names, 42), "Nani");
        assert_eq!(display_name_or_default(&names, 99), "User 99");
    }
}
