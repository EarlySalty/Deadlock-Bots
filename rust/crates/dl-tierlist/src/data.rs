//! Lese-Queries und Payload-Assemblierung — feldgenau wie tierlist_public.py.

use std::collections::HashMap;

use serde_json::{json, Map, Value};
use sqlx::PgPool;

use crate::error::Result;
use crate::settings::{
    tier_bounds, tier_for_winrate, Settings, SNAPSHOT_RETENTION_PER_BUCKET, TIER_ORDER,
};
use crate::util::py_round2;

pub fn slugify_hero(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .replace('&', "and")
        .replace(' ', "_")
}

#[derive(Debug, Clone)]
pub struct HeroEntry {
    pub hero_id: i64,
    pub name: String,
    pub slug: String,
    pub image_url: String,
}

impl HeroEntry {
    fn to_json(&self) -> Value {
        json!({
            "hero_id": self.hero_id,
            "name": self.name,
            "slug": self.slug,
            "image_url": self.image_url,
        })
    }
}

pub async fn hero_catalog(pool: &PgPool) -> Result<Vec<HeroEntry>> {
    let rows = sqlx::query!(
        r#"
        SELECT hero_id, name
        FROM tierlist.deadlock_heroes
        WHERE is_active = TRUE
        ORDER BY lower(name) ASC, name ASC
        "#
    )
    .fetch_all(pool)
    .await?;

    let heroes = rows
        .into_iter()
        .map(|row| {
            let slug = slugify_hero(&row.name);
            HeroEntry {
                hero_id: row.hero_id,
                name: row.name,
                image_url: format!("/heroes/{slug}.png"),
                slug,
            }
        })
        .collect();
    Ok(heroes)
}

/// `GET /api/heroes` — Objekt mit str(hero_id) als Schlüssel.
pub async fn heroes_payload(pool: &PgPool) -> Result<Value> {
    let mut out = Map::new();
    for hero in hero_catalog(pool).await? {
        out.insert(
            hero.hero_id.to_string(),
            json!({
                "name": hero.name,
                "slug": hero.slug,
                "image_url": hero.image_url,
            }),
        );
    }
    Ok(Value::Object(out))
}

async fn load_descriptions(pool: &PgPool) -> Result<HashMap<i64, String>> {
    let rows = sqlx::query!(
        r#"
        SELECT hero_id, description
        FROM tierlist.tierlist_hero_meta
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.hero_id, row.description))
        .collect())
}

async fn load_builds_by_hero(pool: &PgPool) -> Result<HashMap<i64, Vec<Value>>> {
    let rows = sqlx::query!(
        r#"
        SELECT hb.hero_id,
               hb.build_id,
               hb.build_name,
               hb.author_name,
               hb.sort_order::BIGINT AS "sort_order!",
               COALESCE(v.upvotes, 0)::BIGINT AS "upvotes!",
               COALESCE(v.downvotes, 0)::BIGINT AS "downvotes!"
          FROM tierlist.deadlock_hero_builds hb
          LEFT JOIN tierlist.tierlist_build_votes v ON v.build_id = hb.build_id
         WHERE hb.is_active = TRUE
         ORDER BY hb.hero_id ASC, hb.sort_order ASC, hb.build_id ASC
        "#
    )
    .fetch_all(pool)
    .await?;

    struct BuildRow {
        build_id: i64,
        build_name: String,
        author_name: String,
        sort_order: i64,
        upvotes: i64,
        downvotes: i64,
    }

    let mut grouped: HashMap<i64, Vec<BuildRow>> = HashMap::new();
    for row in rows {
        grouped.entry(row.hero_id).or_default().push(BuildRow {
            build_id: row.build_id,
            build_name: row.build_name,
            author_name: row.author_name,
            sort_order: if row.sort_order != 0 {
                row.sort_order
            } else {
                100
            },
            upvotes: row.upvotes,
            downvotes: row.downvotes,
        });
    }

    // Sortierung wie Python: (sort_order, -(up-down), -up, build_id)
    let mut out = HashMap::new();
    for (hero_id, mut items) in grouped {
        items.sort_by(|a, b| {
            (
                a.sort_order,
                -(a.upvotes - a.downvotes),
                -a.upvotes,
                a.build_id,
            )
                .cmp(&(
                    b.sort_order,
                    -(b.upvotes - b.downvotes),
                    -b.upvotes,
                    b.build_id,
                ))
        });
        out.insert(
            hero_id,
            items
                .into_iter()
                .map(|b| {
                    json!({
                        "build_id": b.build_id,
                        "build_name": b.build_name,
                        "author_name": b.author_name,
                        "sort_order": b.sort_order,
                        "upvotes": b.upvotes,
                        "downvotes": b.downvotes,
                    })
                })
                .collect(),
        );
    }
    Ok(out)
}

async fn load_streamers_by_hero(pool: &PgPool) -> Result<HashMap<i64, Vec<Value>>> {
    let rows = sqlx::query!(
        r#"
        SELECT id,
               hero_id,
               twitch_login,
               display_name,
               sort_order::BIGINT AS "sort_order!"
          FROM tierlist.tierlist_streamers
         WHERE is_active = TRUE
         ORDER BY hero_id ASC, sort_order ASC, id ASC
        "#
    )
    .fetch_all(pool)
    .await?;

    let mut out: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows {
        out.entry(row.hero_id).or_default().push(json!({
            "id": row.id,
            "twitch_login": row.twitch_login,
            "display_name": row.display_name,
            "sort_order": if row.sort_order != 0 { row.sort_order } else { 100 },
        }));
    }
    Ok(out)
}

struct SnapshotMeta {
    id: i64,
    patch_id: String,
    patch_unix: i64,
    fetched_at: i64,
}

async fn latest_snapshot(pool: &PgPool, bucket: &str) -> Result<Option<SnapshotMeta>> {
    let row = sqlx::query!(
        r#"
        SELECT id, patch_id, patch_at, fetched_at
          FROM tierlist.tierlist_snapshots
         WHERE bucket = $1
         ORDER BY fetched_at DESC, id DESC
         LIMIT 1
        "#,
        bucket,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|row| SnapshotMeta {
        id: row.id,
        patch_id: row.patch_id,
        patch_unix: row.patch_at.timestamp(),
        fetched_at: row.fetched_at.timestamp(),
    }))
}

async fn previous_snapshot_wr(
    pool: &PgPool,
    bucket: &str,
    snapshot_id: i64,
) -> Result<HashMap<i64, f64>> {
    let prev = sqlx::query!(
        r#"
        SELECT id
          FROM tierlist.tierlist_snapshots
         WHERE bucket = $1 AND id != $2
         ORDER BY fetched_at DESC, id DESC
         LIMIT 1
        "#,
        bucket,
        snapshot_id,
    )
    .fetch_optional(pool)
    .await?;
    let Some(prev) = prev else {
        return Ok(HashMap::new());
    };

    let rows = sqlx::query!(
        r#"
        SELECT hero_id, winrate
          FROM tierlist.tierlist_snapshot_heroes
         WHERE snapshot_id = $1
        "#,
        prev.id,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| (row.hero_id, py_round2(row.winrate)))
        .collect())
}

/// `GET /api/tierlist` — kompletter Payload.
pub async fn tierlist_payload(pool: &PgPool, bucket: &str, settings: &Settings) -> Result<Value> {
    let thresholds = &settings.thresholds;
    let min_matches = settings.min_matches;
    let latest = latest_snapshot(pool, bucket).await?;
    let heroes: HashMap<i64, HeroEntry> = hero_catalog(pool)
        .await?
        .into_iter()
        .map(|h| (h.hero_id, h))
        .collect();

    let tiers_empty = |thresholds| {
        TIER_ORDER
            .iter()
            .map(|(tier, title)| {
                let (min_wr, max_wr) = tier_bounds(tier, thresholds);
                json!({"key": tier, "title": title, "min_wr": min_wr, "max_wr": max_wr, "heroes": []})
            })
            .collect::<Vec<_>>()
    };

    let Some(latest) = latest else {
        return Ok(json!({
            "bucket": bucket,
            "patch_id": null,
            "patch_unix": null,
            "last_updated": null,
            "description": settings.description_text,
            "thresholds": thresholds,
            "min_matches": min_matches,
            "tiers": tiers_empty(thresholds),
        }));
    };

    let descriptions = load_descriptions(pool).await?;
    let builds_by_hero = load_builds_by_hero(pool).await?;
    let streamers_by_hero = load_streamers_by_hero(pool).await?;
    let previous_wr = previous_snapshot_wr(pool, bucket, latest.id).await?;

    struct TierHero {
        entry_json: Value,
        wr: f64,
        matches: i64,
        name: String,
    }
    let mut tier_groups: HashMap<&str, Vec<TierHero>> = TIER_ORDER
        .iter()
        .map(|(tier, _)| (*tier, Vec::new()))
        .collect();

    let rows = sqlx::query!(
        r#"
        SELECT hero_id, matches, winrate
          FROM tierlist.tierlist_snapshot_heroes
         WHERE snapshot_id = $1
        "#,
        latest.id,
    )
    .fetch_all(pool)
    .await?;

    for row in rows {
        let Some(hero) = heroes.get(&row.hero_id) else {
            continue;
        };
        if row.matches < min_matches {
            continue;
        }
        let winrate = py_round2(row.winrate);
        let tier = tier_for_winrate(winrate, thresholds);
        let wr_change = previous_wr
            .get(&row.hero_id)
            .map(|prior| py_round2(winrate - prior));
        let mut entry = hero.to_json();
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("wr".into(), json!(winrate));
            obj.insert("wr_change".into(), json!(wr_change));
            obj.insert("matches".into(), json!(row.matches));
            obj.insert("tier".into(), json!(tier));
            obj.insert(
                "description".into(),
                json!(descriptions.get(&row.hero_id).cloned().unwrap_or_default()),
            );
            obj.insert(
                "builds".into(),
                Value::Array(
                    builds_by_hero
                        .get(&row.hero_id)
                        .cloned()
                        .unwrap_or_default(),
                ),
            );
            obj.insert(
                "streamers".into(),
                Value::Array(
                    streamers_by_hero
                        .get(&row.hero_id)
                        .cloned()
                        .unwrap_or_default(),
                ),
            );
        }
        tier_groups.entry(tier).or_default().push(TierHero {
            entry_json: entry,
            wr: winrate,
            matches: row.matches,
            name: hero.name.clone(),
        });
    }

    let tiers = TIER_ORDER
        .iter()
        .map(|(tier, title)| {
            let mut group = tier_groups.remove(*tier).unwrap_or_default();
            // Python: sort key (-wr, -matches, name)
            group.sort_by(|a, b| {
                b.wr.partial_cmp(&a.wr)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.matches.cmp(&a.matches))
                    .then(a.name.cmp(&b.name))
            });
            let (min_wr, max_wr) = tier_bounds(tier, thresholds);
            json!({
                "key": tier,
                "title": title,
                "min_wr": min_wr,
                "max_wr": max_wr,
                "heroes": group.into_iter().map(|h| h.entry_json).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    Ok(json!({
        "bucket": bucket,
        "patch_id": latest.patch_id,
        "patch_unix": latest.patch_unix,
        "last_updated": latest.fetched_at,
        "description": settings.description_text,
        "thresholds": thresholds,
        "min_matches": min_matches,
        "tiers": tiers,
    }))
}

/// `GET /api/tierlist/history`.
pub async fn history_payload(pool: &PgPool, bucket: &str, settings: &Settings) -> Result<Value> {
    let thresholds = &settings.thresholds;
    let min_matches = settings.min_matches;
    let known: std::collections::HashSet<i64> = hero_catalog(pool)
        .await?
        .into_iter()
        .map(|h| h.hero_id)
        .collect();

    let snaps = sqlx::query!(
        r#"
        SELECT id, patch_id, fetched_at
          FROM tierlist.tierlist_snapshots
         WHERE bucket = $1
         ORDER BY fetched_at DESC, id DESC
         LIMIT $2
        "#,
        bucket,
        SNAPSHOT_RETENTION_PER_BUCKET,
    )
    .fetch_all(pool)
    .await?;

    let mut snapshots = Vec::new();
    for snap in snaps {
        let rows = sqlx::query!(
            r#"
            SELECT hero_id, matches, winrate
              FROM tierlist.tierlist_snapshot_heroes
             WHERE snapshot_id = $1
            "#,
            snap.id,
        )
        .fetch_all(pool)
        .await?;

        let mut heroes = Vec::new();
        for row in rows {
            if !known.contains(&row.hero_id) || row.matches < min_matches {
                continue;
            }
            let wr = py_round2(row.winrate);
            heroes.push((row.hero_id, wr));
        }
        // Python: sort key (-wr, hero_id)
        heroes.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        snapshots.push(json!({
            "snapshot_id": snap.id,
            "fetched_at": snap.fetched_at.timestamp(),
            "patch_id": snap.patch_id,
            "heroes": heroes
                .into_iter()
                .map(|(hero_id, wr)| json!({
                    "hero_id": hero_id,
                    "wr": wr,
                    "tier": tier_for_winrate(wr, thresholds),
                }))
                .collect::<Vec<_>>(),
        }));
    }

    Ok(json!({ "bucket": bucket, "snapshots": snapshots }))
}

/// Admin-Hero-Payload (GET + PUT-Antwort).
pub async fn admin_hero_payload(pool: &PgPool, hero_id: i64) -> Result<Option<Value>> {
    let hero = sqlx::query!(
        r#"
        SELECT hero_id, name
          FROM tierlist.deadlock_heroes
         WHERE hero_id = $1
        "#,
        hero_id,
    )
    .fetch_optional(pool)
    .await?;
    let Some(hero) = hero else {
        return Ok(None);
    };

    let description = sqlx::query!(
        r#"
        SELECT description
          FROM tierlist.tierlist_hero_meta
         WHERE hero_id = $1
        "#,
        hero_id,
    )
    .fetch_optional(pool)
    .await?
    .map(|row| row.description)
    .unwrap_or_default();

    let build_rows = sqlx::query!(
        r#"
        SELECT build_id,
               build_name,
               author_name,
               is_active,
               sort_order::BIGINT AS "sort_order!"
          FROM tierlist.deadlock_hero_builds
         WHERE hero_id = $1
         ORDER BY sort_order ASC, build_id ASC
        "#,
        hero_id,
    )
    .fetch_all(pool)
    .await?;
    let builds = build_rows
        .into_iter()
        .map(|row| {
            json!({
                "build_id": row.build_id,
                "build_name": row.build_name,
                "author_name": row.author_name,
                "is_active": row.is_active,
                "sort_order": row.sort_order,
            })
        })
        .collect::<Vec<_>>();

    let streamer_rows = sqlx::query!(
        r#"
        SELECT id,
               twitch_login,
               display_name,
               sort_order::BIGINT AS "sort_order!",
               is_active
          FROM tierlist.tierlist_streamers
         WHERE hero_id = $1
         ORDER BY sort_order ASC, id ASC
        "#,
        hero_id,
    )
    .fetch_all(pool)
    .await?;
    let streamers = streamer_rows
        .into_iter()
        .map(|row| {
            json!({
                "id": row.id,
                "twitch_login": row.twitch_login,
                "display_name": row.display_name,
                "sort_order": row.sort_order,
                "is_active": row.is_active,
            })
        })
        .collect::<Vec<_>>();

    Ok(Some(json!({
        "hero_id": hero.hero_id,
        "name": hero.name,
        "description": description,
        "builds_meta": builds,
        "streamers": streamers,
    })))
}

#[cfg(all(test, feature = "testing"))]
mod pg_tests {
    use super::*;
    use crate::settings;
    use chrono::{DateTime, Utc};

    fn ts(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .expect("gueltiger Test-Zeitpunkt")
            .with_timezone(&Utc)
    }

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn tierlist_payload_roundtrip_and_streamer_unique_constraint(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        sqlx::query!(
            r#"
            INSERT INTO tierlist.deadlock_heroes (hero_id, name, is_active)
            VALUES ($1, $2, TRUE)
            "#,
            1_i64,
            "Abrams",
        )
        .execute(pool)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO tierlist.deadlock_hero_builds
                (hero_id, build_id, build_name, author_name, is_active, sort_order)
            VALUES ($1, $2, $3, $4, TRUE, $5)
            "#,
            1_i64,
            10_001_i64,
            "Frontline",
            "Tester",
            0_i32,
        )
        .execute(pool)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_build_votes
                (build_id, upvotes, downvotes, updated_at)
            VALUES ($1, $2, $3, $4)
            "#,
            10_001_i64,
            3_i64,
            1_i64,
            ts("2026-01-01T00:00:00Z"),
        )
        .execute(pool)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_hero_meta (hero_id, description, updated_at)
            VALUES ($1, $2, $3)
            "#,
            1_i64,
            "Front hero",
            ts("2026-01-01T00:00:01Z"),
        )
        .execute(pool)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_streamers
                (hero_id, twitch_login, display_name, sort_order, is_active, created_at)
            VALUES ($1, $2, $3, $4, TRUE, $5)
            "#,
            1_i64,
            "abrams_main",
            "Abrams Main",
            0_i32,
            ts("2026-01-01T00:00:02Z"),
        )
        .execute(pool)
        .await?;

        let duplicate_streamer = sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_streamers
                (hero_id, twitch_login, display_name, sort_order, is_active, created_at)
            VALUES ($1, $2, $3, $4, TRUE, $5)
            "#,
            1_i64,
            "abrams_main",
            "Duplicate",
            50_i32,
            ts("2026-01-01T00:00:03Z"),
        )
        .execute(pool)
        .await;
        assert!(duplicate_streamer.is_err());

        let previous = sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_snapshots (bucket, patch_id, patch_at, fetched_at)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
            "all",
            "previous",
            ts("2025-12-31T00:00:00Z"),
            ts("2026-01-01T00:00:10Z"),
        )
        .fetch_one(pool)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_snapshot_heroes
                (snapshot_id, hero_id, matches, wins, losses, winrate)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            previous.id,
            1_i64,
            40_i64,
            20_i64,
            20_i64,
            51.0_f64,
        )
        .execute(pool)
        .await?;

        let latest = sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_snapshots (bucket, patch_id, patch_at, fetched_at)
            VALUES ($1, $2, $3, $4)
            RETURNING id
            "#,
            "all",
            "latest",
            ts("2026-01-01T00:00:00Z"),
            ts("2026-01-01T00:00:20Z"),
        )
        .fetch_one(pool)
        .await?;

        sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_snapshot_heroes
                (snapshot_id, hero_id, matches, wins, losses, winrate)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
            latest.id,
            1_i64,
            42_i64,
            23_i64,
            19_i64,
            53.0_f64,
        )
        .execute(pool)
        .await?;

        settings::set_setting(pool, "min_matches", "10").await?;
        let settings = settings::read_settings(pool).await?;
        let payload = tierlist_payload(pool, "all", &settings).await?;

        assert_eq!(payload["patch_id"], json!("latest"));
        assert_eq!(payload["patch_unix"], json!(1767225600_i64));
        assert_eq!(payload["last_updated"], json!(1767225620_i64));
        assert_eq!(payload["min_matches"], json!(10_i64));
        let heroes = payload["tiers"][0]["heroes"].as_array().expect("S+ heroes");
        assert_eq!(heroes.len(), 1);
        assert_eq!(heroes[0]["hero_id"], json!(1_i64));
        assert_eq!(heroes[0]["wr"], json!(53.0));
        assert_eq!(heroes[0]["wr_change"], json!(2.0));
        assert_eq!(heroes[0]["description"], json!("Front hero"));
        assert_eq!(heroes[0]["builds"][0]["sort_order"], json!(100_i64));
        assert_eq!(heroes[0]["builds"][0]["upvotes"], json!(3_i64));
        assert_eq!(heroes[0]["streamers"][0]["sort_order"], json!(100_i64));

        Ok(())
    }
}
