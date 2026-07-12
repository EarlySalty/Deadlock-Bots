use std::{collections::HashMap, time::Duration};

use reqwest::Client;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Postgres, Row, Transaction};
use thiserror::Error;

pub const ITEMS_ENDPOINT: &str =
    "https://api.deadlock-api.com/v1/assets/items/by-type/upgrade?language=english";
pub const HEROES_ENDPOINT: &str = "https://api.deadlock-api.com/v1/assets/heroes?language=english";

const SOURCE: &str = "deadlock_assets_api";
const STATE_KIND: &str = "assets_api";

#[derive(Debug, Error)]
pub enum ApiIngestError {
    #[error("Deadlock API HTTP client could not be built: {0}")]
    Client(#[source] reqwest::Error),
    #[error("GET {url} failed: {source}")]
    Fetch {
        url: &'static str,
        #[source]
        source: reqwest::Error,
    },
    #[error("GET {url} returned HTTP {status}")]
    Http {
        url: &'static str,
        status: reqwest::StatusCode,
    },
    #[error("Deadlock API {0} payload must be an array")]
    InvalidPayload(&'static str),
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct IngestCounts {
    pub fetched: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeadlockApiIngestSummary {
    pub items: IngestCounts,
    pub heroes: IngestCounts,
}

#[derive(Debug, Clone)]
struct NormalizedBatch {
    fetched: usize,
    failed: usize,
    records: Vec<AssetRecord>,
}

#[derive(Debug, Clone)]
struct AssetRecord {
    entity_type: &'static str,
    external_id: String,
    canonical_name: String,
    content_hash: String,
    payload: Value,
    catalog: CatalogRecord,
}

#[derive(Debug, Clone)]
enum CatalogRecord {
    Item(ItemCatalog),
    Hero(HeroCatalog),
}

#[derive(Debug, Clone)]
struct ItemCatalog {
    item_id: i64,
    name: String,
    slot_type: String,
    tier: i64,
    defense_kind: Vec<String>,
    damage_axis: String,
    properties: Value,
}

#[derive(Debug, Clone)]
struct HeroCatalog {
    hero_id: i64,
    name: String,
    base_health: i64,
    archetype: String,
    stats: Value,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
struct EntityKey {
    entity_type: String,
    external_id: String,
}

impl From<&AssetRecord> for EntityKey {
    fn from(record: &AssetRecord) -> Self {
        Self {
            entity_type: record.entity_type.to_string(),
            external_id: record.external_id.clone(),
        }
    }
}

struct UpdatePlan {
    changed: Vec<AssetRecord>,
    unchanged: usize,
}

pub async fn ingest_deadlock_api(
    pool: &PgPool,
) -> Result<DeadlockApiIngestSummary, ApiIngestError> {
    let run_id = begin_run(pool).await?;
    let result = ingest_deadlock_api_inner(pool).await;

    match result {
        Ok(summary) => {
            finish_run(pool, run_id, "completed", &serde_json::to_value(&summary)?).await?;
            tracing::info!(
                items_fetched = summary.items.fetched,
                items_updated = summary.items.updated,
                items_unchanged = summary.items.unchanged,
                items_failed = summary.items.failed,
                heroes_fetched = summary.heroes.fetched,
                heroes_updated = summary.heroes.updated,
                heroes_unchanged = summary.heroes.unchanged,
                heroes_failed = summary.heroes.failed,
                "Deadlock assets ingest completed"
            );
            Ok(summary)
        }
        Err(error) => {
            let summary = json!({"fetched": 0, "updated": 0, "unchanged": 0, "failed": 1});
            if let Err(finish_error) = finish_run(pool, run_id, "failed", &summary).await {
                tracing::error!(%finish_error, "Deadlock assets failed run could not be recorded");
            }
            tracing::error!(%error, fetched = 0, updated = 0, unchanged = 0, failed = 1, "Deadlock assets ingest failed");
            Err(error)
        }
    }
}

async fn ingest_deadlock_api_inner(
    pool: &PgPool,
) -> Result<DeadlockApiIngestSummary, ApiIngestError> {
    let client = Client::builder()
        .user_agent("deadlock-bots-dl-brain/0.1")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(ApiIngestError::Client)?;
    let (items_payload, heroes_payload) = tokio::try_join!(
        fetch_json(&client, ITEMS_ENDPOINT),
        fetch_json(&client, HEROES_ENDPOINT),
    )?;
    let items = normalize_items(&items_payload)?;
    let heroes = normalize_heroes(&heroes_payload)?;

    let mut transaction = pool.begin().await?;
    let existing = existing_hashes(&mut transaction).await?;
    let item_plan = partition_updates(items.records, &existing);
    let hero_plan = partition_updates(heroes.records, &existing);

    let mut summary = DeadlockApiIngestSummary {
        items: IngestCounts {
            fetched: items.fetched,
            unchanged: item_plan.unchanged,
            failed: items.failed,
            ..IngestCounts::default()
        },
        heroes: IngestCounts {
            fetched: heroes.fetched,
            unchanged: hero_plan.unchanged,
            failed: heroes.failed,
            ..IngestCounts::default()
        },
    };

    persist_records(&mut transaction, item_plan.changed, &mut summary.items).await?;
    persist_records(&mut transaction, hero_plan.changed, &mut summary.heroes).await?;
    transaction.commit().await?;
    Ok(summary)
}

async fn fetch_json(client: &Client, url: &'static str) -> Result<Value, ApiIngestError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|source| ApiIngestError::Fetch { url, source })?;
    if !response.status().is_success() {
        return Err(ApiIngestError::Http {
            url,
            status: response.status(),
        });
    }
    response
        .json()
        .await
        .map_err(|source| ApiIngestError::Fetch { url, source })
}

fn normalize_items(payload: &Value) -> Result<NormalizedBatch, ApiIngestError> {
    let items = payload
        .as_array()
        .ok_or(ApiIngestError::InvalidPayload("items"))?;
    let mut records = Vec::new();
    let mut failed = 0;

    for item in items {
        let slot_type = text(item, "item_slot_type");
        let current_shop_item = text(item, "type").as_deref() == Some("upgrade")
            && matches!(slot_type.as_deref(), Some("weapon" | "vitality" | "spirit"))
            && item.get("shopable").and_then(Value::as_bool) == Some(true)
            && item.get("disabled").and_then(Value::as_bool) != Some(true);
        if !current_shop_item {
            continue;
        }

        let Some(item_id) = integer(item, "id") else {
            failed += 1;
            continue;
        };
        let Some(name) = text(item, "name") else {
            failed += 1;
            continue;
        };
        let Some(slot_type) = slot_type else {
            failed += 1;
            continue;
        };
        let Some(tier) = integer(item, "item_tier").or_else(|| integer(item, "tier")) else {
            failed += 1;
            continue;
        };
        let Some(cost) = integer(item, "cost") else {
            failed += 1;
            continue;
        };
        let properties = item.get("properties").cloned().unwrap_or_else(|| json!({}));
        let (defense_kind, damage_axis) = classify_properties(&properties, &slot_type);
        let normalized = json!({
            "id": item_id,
            "class_name": text(item, "class_name"),
            "name": name,
            "slot_type": slot_type,
            "tier": tier,
            "cost": cost,
            "is_active_item": item.get("is_active_item").and_then(Value::as_bool).unwrap_or(false),
            "description": text(item, "description"),
            "properties": properties,
            "defense_kind": defense_kind,
            "damage_axis": damage_axis,
        });
        records.push(AssetRecord {
            entity_type: "item",
            external_id: item_id.to_string(),
            canonical_name: name.clone(),
            content_hash: content_hash(&normalized)?,
            payload: normalized,
            catalog: CatalogRecord::Item(ItemCatalog {
                item_id,
                name,
                slot_type,
                tier,
                defense_kind,
                damage_axis,
                properties,
            }),
        });
    }

    Ok(NormalizedBatch {
        fetched: items.len(),
        failed,
        records,
    })
}

fn normalize_heroes(payload: &Value) -> Result<NormalizedBatch, ApiIngestError> {
    let heroes = payload
        .as_array()
        .ok_or(ApiIngestError::InvalidPayload("heroes"))?;
    let mut records = Vec::new();
    let mut failed = 0;

    for hero in heroes {
        let current_hero = hero.get("disabled").and_then(Value::as_bool) != Some(true)
            && hero.get("in_development").and_then(Value::as_bool) != Some(true)
            && hero.get("player_selectable").and_then(Value::as_bool) == Some(true);
        if !current_hero {
            continue;
        }

        let Some(hero_id) = integer(hero, "id") else {
            failed += 1;
            continue;
        };
        let Some(name) = text(hero, "name") else {
            failed += 1;
            continue;
        };
        let Some(starting_stats) = hero
            .get("starting_stats")
            .filter(|value| value.is_object())
            .cloned()
        else {
            failed += 1;
            continue;
        };
        let Some(base_health) = starting_stats
            .get("max_health")
            .and_then(|stat| stat.get("value"))
            .and_then(value_i64)
        else {
            failed += 1;
            continue;
        };
        let archetype = derive_archetype(base_health).to_string();
        let normalized = json!({
            "id": hero_id,
            "class_name": text(hero, "class_name"),
            "name": name,
            "description": text(hero, "description"),
            "hero_type": text(hero, "hero_type"),
            "base_health": base_health,
            "archetype": archetype,
            "starting_stats": starting_stats,
        });
        records.push(AssetRecord {
            entity_type: "hero",
            external_id: hero_id.to_string(),
            canonical_name: name.clone(),
            content_hash: content_hash(&normalized)?,
            payload: normalized.clone(),
            catalog: CatalogRecord::Hero(HeroCatalog {
                hero_id,
                name,
                base_health,
                archetype,
                stats: normalized,
            }),
        });
    }

    Ok(NormalizedBatch {
        fetched: heroes.len(),
        failed,
        records,
    })
}

fn partition_updates(
    records: Vec<AssetRecord>,
    existing: &HashMap<EntityKey, String>,
) -> UpdatePlan {
    let mut changed = Vec::new();
    let mut unchanged = 0;
    for record in records {
        if existing.get(&EntityKey::from(&record)) == Some(&record.content_hash) {
            unchanged += 1;
        } else {
            changed.push(record);
        }
    }
    UpdatePlan { changed, unchanged }
}

async fn existing_hashes(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<HashMap<EntityKey, String>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT entity_type, entity_name, content_hash
         FROM brain.current_entity_state
         WHERE state_kind = $1 AND entity_type IN ('item', 'hero')",
    )
    .bind(STATE_KIND)
    .fetch_all(&mut **transaction)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                EntityKey {
                    entity_type: row.try_get("entity_type")?,
                    external_id: row.try_get("entity_name")?,
                },
                row.try_get("content_hash")?,
            ))
        })
        .collect()
}

async fn persist_records(
    transaction: &mut Transaction<'_, Postgres>,
    records: Vec<AssetRecord>,
    counts: &mut IngestCounts,
) -> Result<(), ApiIngestError> {
    for record in records {
        let payload = serde_json::to_string(&record.payload)?;
        let snapshot_id: i64 = sqlx::query_scalar(
            "WITH inserted AS (
               INSERT INTO brain.entity_snapshots(
                 source, entity_type, external_id, canonical_name, payload_hash, payload, fetched_at
               ) VALUES($1, $2, $3, $4, $5, $6::jsonb, now())
               ON CONFLICT(source, entity_type, external_id, payload_hash) DO NOTHING
               RETURNING id
             )
             SELECT id FROM inserted
             UNION ALL
             SELECT id FROM brain.entity_snapshots
              WHERE source=$1 AND entity_type=$2 AND external_id=$3 AND payload_hash=$5
             LIMIT 1",
        )
        .bind(SOURCE)
        .bind(record.entity_type)
        .bind(&record.external_id)
        .bind(&record.canonical_name)
        .bind(&record.content_hash)
        .bind(&payload)
        .fetch_one(&mut **transaction)
        .await?;

        let changed = sqlx::query(
            "INSERT INTO brain.current_entity_state(
               entity_type, entity_name, state_kind, winning_snapshot_id, source,
               source_priority, observed_at, content_hash, payload, updated_at
             ) VALUES($1, $2, $3, $4, $5, 0, now(), $6, $7::jsonb, now())
             ON CONFLICT(entity_type, entity_name, state_kind) DO UPDATE SET
               winning_snapshot_id=excluded.winning_snapshot_id,
               source=excluded.source,
               source_priority=excluded.source_priority,
               observed_at=excluded.observed_at,
               content_hash=excluded.content_hash,
               payload=excluded.payload,
               updated_at=excluded.updated_at
             WHERE brain.current_entity_state.content_hash IS DISTINCT FROM excluded.content_hash",
        )
        .bind(record.entity_type)
        .bind(&record.external_id)
        .bind(STATE_KIND)
        .bind(snapshot_id)
        .bind(SOURCE)
        .bind(&record.content_hash)
        .bind(&payload)
        .execute(&mut **transaction)
        .await?
        .rows_affected();

        if changed == 0 {
            counts.unchanged += 1;
            continue;
        }
        persist_catalog(transaction, &record.catalog).await?;
        counts.updated += 1;
    }
    Ok(())
}

async fn persist_catalog(
    transaction: &mut Transaction<'_, Postgres>,
    catalog: &CatalogRecord,
) -> Result<(), ApiIngestError> {
    match catalog {
        CatalogRecord::Item(item) => {
            sqlx::query(
                "INSERT INTO brain.item_catalog(
                   item_id, name, slot_type, tier, defense_kind, damage_axis, properties, updated_at
                 ) VALUES($1, $2, $3, $4, $5::jsonb, $6, $7::jsonb, now())
                 ON CONFLICT(item_id) DO UPDATE SET
                   name=excluded.name,
                   slot_type=excluded.slot_type,
                   tier=excluded.tier,
                   defense_kind=excluded.defense_kind,
                   damage_axis=excluded.damage_axis,
                   properties=excluded.properties,
                   updated_at=excluded.updated_at",
            )
            .bind(item.item_id)
            .bind(&item.name)
            .bind(&item.slot_type)
            .bind(item.tier)
            .bind(serde_json::to_string(&item.defense_kind)?)
            .bind(&item.damage_axis)
            .bind(serde_json::to_string(&item.properties)?)
            .execute(&mut **transaction)
            .await?;
        }
        CatalogRecord::Hero(hero) => {
            sqlx::query(
                "INSERT INTO brain.hero_catalog(
                   hero_id, name, base_health, archetype, stats, updated_at
                 ) VALUES($1, $2, $3, $4, $5::jsonb, now())
                 ON CONFLICT(hero_id) DO UPDATE SET
                   name=excluded.name,
                   base_health=excluded.base_health,
                   archetype=excluded.archetype,
                   stats=excluded.stats,
                   updated_at=excluded.updated_at",
            )
            .bind(hero.hero_id)
            .bind(&hero.name)
            .bind(hero.base_health)
            .bind(&hero.archetype)
            .bind(serde_json::to_string(&hero.stats)?)
            .execute(&mut **transaction)
            .await?;
        }
    }
    Ok(())
}

async fn begin_run(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO brain.source_runs(source, status, started_at)
         VALUES($1, 'running', now()) RETURNING id",
    )
    .bind(SOURCE)
    .fetch_one(pool)
    .await
}

async fn finish_run(
    pool: &PgPool,
    run_id: i64,
    status: &str,
    summary: &Value,
) -> Result<(), ApiIngestError> {
    sqlx::query(
        "UPDATE brain.source_runs
         SET status=$2, finished_at=now(), summary=$3::jsonb
         WHERE id=$1",
    )
    .bind(run_id)
    .bind(status)
    .bind(serde_json::to_string(summary)?)
    .execute(pool)
    .await?;
    Ok(())
}

fn content_hash(payload: &Value) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(payload)?)
    ))
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(value_i64)
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
        .or_else(|| value.as_str().and_then(|raw| raw.parse().ok()))
}

fn derive_archetype(base_health: i64) -> &'static str {
    if base_health >= 850 {
        "tank"
    } else if base_health >= 760 {
        "bruiser"
    } else {
        "squishy"
    }
}

fn classify_properties(properties: &Value, slot_type: &str) -> (Vec<String>, String) {
    let mut defense = std::collections::BTreeSet::new();
    let mut weapon = false;
    let mut spirit = false;
    let Some(properties) = properties.as_object() else {
        return (Vec::new(), "utility".to_string());
    };
    for (name, property) in properties {
        if !active_property(property.get("value")) {
            continue;
        }
        let property_type = property
            .get("provided_property_type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if property_type.contains("ARMOR_DAMAGE_RESIST") || property_type.contains("TECH_RESIST") {
            defense.insert("percent_resist".to_string());
        }
        if property_type.contains("BARRIER_HEALTH") {
            defense.insert("flat_shield".to_string());
        }
        if property_type.contains("HEALTH_MAX") || property_type.contains("BASE_HEALTH_PERCENT") {
            defense.insert("flat_health".to_string());
        }
        if property_type.contains("HEALTH_REGEN") {
            defense.insert("regen".to_string());
        }
        spirit |= property_type.contains("TECH_POWER");
        weapon |= property_type.contains("WEAPON_POWER")
            || property_type.contains("WEAPON_DAMAGE_INCREASE")
            || (slot_type == "weapon" && name == "HeadShotBonusDamage");
    }
    let axis = match (weapon, spirit) {
        (true, true) => "hybrid",
        (true, false) => "weapon",
        (false, true) => "spirit",
        (false, false) => "utility",
    };
    (defense.into_iter().collect(), axis.to_string())
}

fn active_property(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Number(number)) => number.as_f64().is_some_and(|value| value != 0.0),
        Some(Value::String(value)) => value
            .trim()
            .parse::<f64>()
            .map_or(!value.trim().is_empty(), |value| value != 0.0),
        Some(Value::Bool(value)) => *value,
        Some(Value::Array(value)) => !value.is_empty(),
        Some(Value::Object(value)) => !value.is_empty(),
        Some(Value::Null) | None => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;

    use super::{normalize_heroes, normalize_items, partition_updates, EntityKey};

    fn fixture(path: &str) -> Value {
        serde_json::from_str(match path {
            "items" => include_str!("../tests/fixtures/deadlock_api_items.json"),
            "heroes" => include_str!("../tests/fixtures/deadlock_api_heroes.json"),
            _ => panic!("unknown fixture"),
        })
        .expect("fixture JSON must parse")
    }

    #[test]
    fn normalizes_current_shop_items_and_hero_base_stats() {
        let items = normalize_items(&fixture("items")).expect("items normalize");
        assert_eq!(items.fetched, 4);
        assert_eq!(items.failed, 1);
        assert_eq!(items.records.len(), 1);
        assert_eq!(items.records[0].external_id, "100");
        assert_eq!(items.records[0].payload["cost"], 1_600);
        assert_eq!(items.records[0].payload["tier"], 2);

        let heroes = normalize_heroes(&fixture("heroes")).expect("heroes normalize");
        assert_eq!(heroes.fetched, 3);
        assert_eq!(heroes.failed, 1);
        assert_eq!(heroes.records.len(), 1);
        assert_eq!(heroes.records[0].external_id, "1");
        assert_eq!(heroes.records[0].payload["base_health"], 830);
        assert_eq!(
            heroes.records[0].payload["starting_stats"]["stamina"]["value"],
            3
        );
    }

    #[test]
    fn content_hash_ignores_version_and_commit_metadata() {
        let original = fixture("items");
        let mut changed_metadata = original.clone();
        let item = changed_metadata[0]
            .as_object_mut()
            .expect("first item object");
        item.insert("commit_sha".into(), Value::String("different".into()));
        item.insert("version".into(), Value::String("999999".into()));

        let first = normalize_items(&original).expect("first normalization");
        let second = normalize_items(&changed_metadata).expect("second normalization");
        assert_eq!(
            first.records[0].content_hash,
            second.records[0].content_hash
        );
    }

    #[test]
    fn identical_second_run_plans_zero_updates() {
        let records = normalize_items(&fixture("items"))
            .expect("items normalize")
            .records;
        let first = partition_updates(records.clone(), &HashMap::new());
        assert_eq!(first.changed.len(), 1);
        assert_eq!(first.unchanged, 0);

        let existing = records
            .iter()
            .map(|record| (EntityKey::from(record), record.content_hash.clone()))
            .collect();
        let second = partition_updates(records, &existing);
        assert_eq!(second.changed.len(), 0);
        assert_eq!(second.unchanged, 1);
    }
}
