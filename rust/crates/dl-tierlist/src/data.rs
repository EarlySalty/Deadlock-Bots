//! Lese-Queries und Payload-Assemblierung — feldgenau wie tierlist_public.py.
//!
//! Alle Funktionen laufen innerhalb einer dl-db-Read-Closure auf einer
//! Connection (das Python-Original verteilt dieselben Queries auf mehrere
//! Aufrufe — semantisch identisch, hier nur in einem Rutsch).

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Map, Value};

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

pub fn hero_catalog(conn: &Connection) -> rusqlite::Result<Vec<HeroEntry>> {
    let mut stmt = conn.prepare(
        "SELECT hero_id, name FROM deadlock_heroes WHERE is_active = 1
         ORDER BY name COLLATE NOCASE ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let hero_id: i64 = row.get(0)?;
        let name: String = row.get(1)?;
        Ok((hero_id, name))
    })?;
    let mut heroes = Vec::new();
    for row in rows {
        let (hero_id, name) = row?;
        let slug = slugify_hero(&name);
        heroes.push(HeroEntry {
            hero_id,
            name,
            image_url: format!("/heroes/{slug}.png"),
            slug,
        });
    }
    Ok(heroes)
}

/// `GET /api/heroes` — Objekt mit str(hero_id) als Schlüssel.
pub fn heroes_payload(conn: &Connection) -> rusqlite::Result<Value> {
    let mut out = Map::new();
    for hero in hero_catalog(conn)? {
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

fn load_descriptions(conn: &Connection) -> rusqlite::Result<HashMap<i64, String>> {
    let mut stmt = conn.prepare("SELECT hero_id, description FROM tierlist_hero_meta")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Option<String>>(1)?.unwrap_or_default(),
        ))
    })?;
    rows.collect()
}

fn load_builds_by_hero(conn: &Connection) -> rusqlite::Result<HashMap<i64, Vec<Value>>> {
    let mut stmt = conn.prepare(
        "SELECT hb.hero_id, hb.build_id, hb.build_name, hb.author_name, hb.sort_order,
                COALESCE(v.upvotes, 0) AS upvotes, COALESCE(v.downvotes, 0) AS downvotes
           FROM deadlock_hero_builds hb
           LEFT JOIN tierlist_build_votes v ON v.build_id = hb.build_id
          WHERE hb.is_active = 1
          ORDER BY hb.hero_id ASC, hb.sort_order ASC, hb.build_id ASC",
    )?;
    struct BuildRow {
        hero_id: i64,
        build_id: i64,
        build_name: String,
        author_name: String,
        sort_order: i64,
        upvotes: i64,
        downvotes: i64,
    }
    let rows = stmt.query_map([], |row| {
        Ok(BuildRow {
            hero_id: row.get(0)?,
            build_id: row.get(1)?,
            build_name: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
            author_name: row.get::<_, Option<String>>(3)?.unwrap_or_default(),
            sort_order: row
                .get::<_, Option<i64>>(4)?
                .filter(|v| *v != 0)
                .unwrap_or(100),
            upvotes: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            downvotes: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
        })
    })?;

    let mut grouped: HashMap<i64, Vec<BuildRow>> = HashMap::new();
    for row in rows {
        let row = row?;
        grouped.entry(row.hero_id).or_default().push(row);
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

fn load_streamers_by_hero(conn: &Connection) -> rusqlite::Result<HashMap<i64, Vec<Value>>> {
    let mut stmt = conn.prepare(
        "SELECT id, hero_id, twitch_login, display_name, sort_order
           FROM tierlist_streamers
          WHERE is_active = 1
          ORDER BY hero_id ASC, sort_order ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let hero_id: i64 = row.get(1)?;
        Ok((
            hero_id,
            json!({
                "id": row.get::<_, i64>(0)?,
                "twitch_login": row.get::<_, String>(2)?,
                "display_name": row.get::<_, String>(3)?,
                "sort_order": row.get::<_, Option<i64>>(4)?.filter(|v| *v != 0).unwrap_or(100),
            }),
        ))
    })?;
    let mut out: HashMap<i64, Vec<Value>> = HashMap::new();
    for row in rows {
        let (hero_id, value) = row?;
        out.entry(hero_id).or_default().push(value);
    }
    Ok(out)
}

struct SnapshotMeta {
    id: i64,
    patch_id: String,
    patch_unix: i64,
    fetched_at: i64,
}

fn latest_snapshot(conn: &Connection, bucket: &str) -> rusqlite::Result<Option<SnapshotMeta>> {
    conn.query_row(
        "SELECT id, patch_id, patch_unix, fetched_at FROM tierlist_snapshots
          WHERE bucket = ?1 ORDER BY fetched_at DESC, id DESC LIMIT 1",
        [bucket],
        |row| {
            Ok(SnapshotMeta {
                id: row.get(0)?,
                patch_id: row.get(1)?,
                patch_unix: row.get(2)?,
                fetched_at: row.get(3)?,
            })
        },
    )
    .optional()
}

fn previous_snapshot_wr(
    conn: &Connection,
    bucket: &str,
    snapshot_id: i64,
) -> rusqlite::Result<HashMap<i64, f64>> {
    let prev_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM tierlist_snapshots
              WHERE bucket = ?1 AND id != ?2
              ORDER BY fetched_at DESC, id DESC LIMIT 1",
            rusqlite::params![bucket, snapshot_id],
            |row| row.get(0),
        )
        .optional()?;
    let Some(prev_id) = prev_id else {
        return Ok(HashMap::new());
    };
    let mut stmt = conn
        .prepare("SELECT hero_id, winrate FROM tierlist_snapshot_heroes WHERE snapshot_id = ?1")?;
    let rows = stmt.query_map([prev_id], |row| {
        Ok((row.get::<_, i64>(0)?, py_round2(row.get::<_, f64>(1)?)))
    })?;
    rows.collect()
}

/// `GET /api/tierlist` — kompletter Payload.
pub fn tierlist_payload(
    conn: &Connection,
    bucket: &str,
    settings: &Settings,
) -> rusqlite::Result<Value> {
    let thresholds = &settings.thresholds;
    let min_matches = settings.min_matches;
    let latest = latest_snapshot(conn, bucket)?;
    let heroes: HashMap<i64, HeroEntry> = hero_catalog(conn)?
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

    let descriptions = load_descriptions(conn)?;
    let builds_by_hero = load_builds_by_hero(conn)?;
    let streamers_by_hero = load_streamers_by_hero(conn)?;
    let previous_wr = previous_snapshot_wr(conn, bucket, latest.id)?;

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

    let mut stmt = conn.prepare(
        "SELECT hero_id, matches, wins, losses, winrate
           FROM tierlist_snapshot_heroes WHERE snapshot_id = ?1",
    )?;
    let rows = stmt.query_map([latest.id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, f64>(4)?,
        ))
    })?;
    for row in rows {
        let (hero_id, matches, winrate_raw) = row?;
        let Some(hero) = heroes.get(&hero_id) else {
            continue;
        };
        if matches < min_matches {
            continue;
        }
        let winrate = py_round2(winrate_raw);
        let tier = tier_for_winrate(winrate, thresholds);
        let wr_change = previous_wr
            .get(&hero_id)
            .map(|prior| py_round2(winrate - prior));
        let mut entry = hero.to_json();
        let obj = entry.as_object_mut().expect("hero json ist Objekt");
        obj.insert("wr".into(), json!(winrate));
        obj.insert("wr_change".into(), json!(wr_change));
        obj.insert("matches".into(), json!(matches));
        obj.insert("tier".into(), json!(tier));
        obj.insert(
            "description".into(),
            json!(descriptions.get(&hero_id).cloned().unwrap_or_default()),
        );
        obj.insert(
            "builds".into(),
            Value::Array(builds_by_hero.get(&hero_id).cloned().unwrap_or_default()),
        );
        obj.insert(
            "streamers".into(),
            Value::Array(streamers_by_hero.get(&hero_id).cloned().unwrap_or_default()),
        );
        tier_groups.entry(tier).or_default().push(TierHero {
            entry_json: entry,
            wr: winrate,
            matches,
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
pub fn history_payload(
    conn: &Connection,
    bucket: &str,
    settings: &Settings,
) -> rusqlite::Result<Value> {
    let thresholds = &settings.thresholds;
    let min_matches = settings.min_matches;
    let known: std::collections::HashSet<i64> =
        hero_catalog(conn)?.into_iter().map(|h| h.hero_id).collect();

    let mut stmt = conn.prepare(
        "SELECT id, patch_id, fetched_at FROM tierlist_snapshots
          WHERE bucket = ?1 ORDER BY fetched_at DESC, id DESC LIMIT ?2",
    )?;
    let snaps = stmt.query_map(
        rusqlite::params![bucket, SNAPSHOT_RETENTION_PER_BUCKET],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )?;
    let snaps: Vec<_> = snaps.collect::<Result<_, _>>()?;

    let mut hero_stmt = conn.prepare(
        "SELECT hero_id, matches, winrate FROM tierlist_snapshot_heroes WHERE snapshot_id = ?1",
    )?;
    let mut snapshots = Vec::new();
    for (snapshot_id, patch_id, fetched_at) in snaps {
        let rows = hero_stmt.query_map([snapshot_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        let mut heroes = Vec::new();
        for row in rows {
            let (hero_id, matches, winrate_raw) = row?;
            if !known.contains(&hero_id) || matches < min_matches {
                continue;
            }
            let wr = py_round2(winrate_raw);
            heroes.push((hero_id, wr));
        }
        // Python: sort key (-wr, hero_id)
        heroes.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        snapshots.push(json!({
            "snapshot_id": snapshot_id,
            "fetched_at": fetched_at,
            "patch_id": patch_id,
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
pub fn admin_hero_payload(conn: &Connection, hero_id: i64) -> rusqlite::Result<Option<Value>> {
    let hero: Option<(i64, String)> = conn
        .query_row(
            "SELECT hero_id, name FROM deadlock_heroes WHERE hero_id = ?1",
            [hero_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((hero_id, name)) = hero else {
        return Ok(None);
    };
    let description: Option<String> = conn
        .query_row(
            "SELECT description FROM tierlist_hero_meta WHERE hero_id = ?1",
            [hero_id],
            |row| row.get(0),
        )
        .optional()?;

    let mut stmt = conn.prepare(
        "SELECT build_id, build_name, author_name, is_active, sort_order
           FROM deadlock_hero_builds WHERE hero_id = ?1
          ORDER BY sort_order ASC, build_id ASC",
    )?;
    let builds: Vec<Value> = stmt
        .query_map([hero_id], |row| {
            Ok(json!({
                "build_id": row.get::<_, i64>(0)?,
                "build_name": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "author_name": row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                "is_active": row.get::<_, i64>(3)? != 0,
                "sort_order": row.get::<_, Option<i64>>(4)?.unwrap_or(100),
            }))
        })?
        .collect::<Result<_, _>>()?;

    let mut stmt = conn.prepare(
        "SELECT id, twitch_login, display_name, sort_order, is_active
           FROM tierlist_streamers WHERE hero_id = ?1
          ORDER BY sort_order ASC, id ASC",
    )?;
    let streamers: Vec<Value> = stmt
        .query_map([hero_id], |row| {
            Ok(json!({
                "id": row.get::<_, i64>(0)?,
                "twitch_login": row.get::<_, String>(1)?,
                "display_name": row.get::<_, String>(2)?,
                "sort_order": row.get::<_, Option<i64>>(3)?.unwrap_or(100),
                "is_active": row.get::<_, i64>(4)? != 0,
            }))
        })?
        .collect::<Result<_, _>>()?;

    Ok(Some(json!({
        "hero_id": hero_id,
        "name": name,
        "description": description.unwrap_or_default(),
        "builds_meta": builds,
        "streamers": streamers,
    })))
}
