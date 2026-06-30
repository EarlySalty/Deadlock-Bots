//! Settings-Verwaltung über die Bestands-Tabelle `tierlist_settings(k, v, updated_at)`.
//! Semantik exakt wie tierlist_public.py (_ensure_default_settings/_get_settings).

use serde::Serialize;
use serde_json::Value;
use sqlx::PgPool;

use crate::error::Result;
use crate::util::{coerce_float, coerce_int, now_utc, parse_unix_or_iso, py_round2};

pub const REFRESH_DEFAULT_SECONDS: i64 = 8 * 60 * 60;
pub const SNAPSHOT_RETENTION_PER_BUCKET: i64 = 30;
pub const SESSION_COOKIE: &str = "master_dash_session";

/// (Bucket-Name, min_average_badge, max_average_badge) — Reihenfolge wie Python.
pub const BUCKETS: &[(&str, i64, i64)] = &[
    ("all", 0, 116),
    ("phantom_plus", 80, 116),
    ("eternus", 100, 116),
];

pub const TIER_ORDER: &[(&str, &str)] = &[
    ("S+", "Overpowered"),
    ("S", "Meta-Defining"),
    ("A", "Strong Picks"),
    ("B", "Viable"),
    ("C", "Situational"),
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Thresholds {
    pub s_plus_min: f64,
    pub s_min: f64,
    pub a_min: f64,
    pub b_min: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            s_plus_min: 52.0,
            s_min: 50.0,
            a_min: 48.0,
            b_min: 46.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    pub thresholds: Thresholds,
    pub refresh_interval_seconds: i64,
    pub patch_override_unix: Option<i64>,
    pub description_text: String,
    pub min_matches: i64,
}

/// Default-Werte als (k, v)-Paare — exakt wie Pythons DEFAULT_SETTINGS.
fn default_rows() -> Vec<(&'static str, String)> {
    // json.dumps(DEFAULT_THRESHOLDS, separators=(",",":"), sort_keys=True)
    let thresholds_json = r#"{"a_min":48.0,"b_min":46.0,"s_min":50.0,"s_plus_min":52.0}"#;
    vec![
        ("thresholds_json", thresholds_json.to_string()),
        (
            "refresh_interval_seconds",
            REFRESH_DEFAULT_SECONDS.to_string(),
        ),
        ("patch_override_unix", String::new()),
        ("description_text", String::new()),
        ("min_matches", "500".to_string()),
    ]
}

pub async fn ensure_defaults(pool: &PgPool) -> Result<()> {
    let now = now_utc();
    for (key, value) in default_rows() {
        sqlx::query!(
            r#"
            INSERT INTO tierlist.tierlist_settings (k, v, updated_at)
            VALUES ($1, $2, $3)
            ON CONFLICT (k) DO NOTHING
            "#,
            key,
            value,
            now,
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn read_settings(pool: &PgPool) -> Result<Settings> {
    ensure_defaults(pool).await?;
    let rows = sqlx::query!(
        r#"
        SELECT k, v
        FROM tierlist.tierlist_settings
        "#
    )
    .fetch_all(pool)
    .await?;

    let mut raw = std::collections::HashMap::new();
    for row in rows {
        raw.insert(row.k, row.v);
    }

    let thresholds = raw
        .get("thresholds_json")
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .and_then(|v| normalize_thresholds(&v).ok())
        .unwrap_or_default();

    let refresh_interval = raw
        .get("refresh_interval_seconds")
        .and_then(|s| coerce_int(Some(&Value::String(s.clone())), None))
        .filter(|v| *v > 0)
        .unwrap_or(REFRESH_DEFAULT_SECONDS);

    let min_matches = raw
        .get("min_matches")
        .and_then(|s| coerce_int(Some(&Value::String(s.clone())), None))
        .filter(|v| *v >= 0)
        .unwrap_or(500);

    let patch_override_unix = parse_unix_or_iso(
        raw.get("patch_override_unix")
            .map(|s| Value::String(s.clone()))
            .as_ref(),
    );

    Ok(Settings {
        thresholds,
        refresh_interval_seconds: refresh_interval,
        patch_override_unix,
        description_text: raw.get("description_text").cloned().unwrap_or_default(),
        min_matches,
    })
}

pub async fn set_setting(pool: &PgPool, key: &str, value: &str) -> Result<()> {
    let now = now_utc();
    sqlx::query!(
        r#"
        INSERT INTO tierlist.tierlist_settings (k, v, updated_at)
        VALUES ($1, $2, $3)
        ON CONFLICT (k) DO UPDATE
           SET v = EXCLUDED.v,
               updated_at = EXCLUDED.updated_at
        "#,
        key,
        value,
        now,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Wie Python `_normalize_thresholds` — inkl. Alias-Schlüssel und
/// Absteigend-Validierung. Fehlertexte sind Vertrag (gehen 1:1 in die API).
pub fn normalize_thresholds(payload: &Value) -> std::result::Result<Thresholds, String> {
    let empty = serde_json::Map::new();
    let data = payload.as_object().unwrap_or(&empty);
    let pick = |keys: [&str; 3], default: f64| -> Option<f64> {
        let value = keys.iter().find_map(|k| data.get(*k));
        coerce_float(value, Some(default))
    };
    let defaults = Thresholds::default();
    let s_plus_min = pick(["s_plus_min", "sPlusMin", "S+"], defaults.s_plus_min);
    let s_min = pick(["s_min", "sMin", "S"], defaults.s_min);
    let a_min = pick(["a_min", "aMin", "A"], defaults.a_min);
    let b_min = pick(["b_min", "bMin", "B"], defaults.b_min);
    let (Some(s_plus_min), Some(s_min), Some(a_min), Some(b_min)) =
        (s_plus_min, s_min, a_min, b_min)
    else {
        return Err("thresholds must contain numeric values".to_string());
    };
    if !(s_plus_min >= s_min && s_min >= a_min && a_min >= b_min) {
        return Err("thresholds must be descending (S+ >= S >= A >= B)".to_string());
    }
    Ok(Thresholds {
        s_plus_min: py_round2(s_plus_min),
        s_min: py_round2(s_min),
        a_min: py_round2(a_min),
        b_min: py_round2(b_min),
    })
}

pub fn tier_for_winrate(winrate: f64, t: &Thresholds) -> &'static str {
    if winrate >= t.s_plus_min {
        "S+"
    } else if winrate >= t.s_min {
        "S"
    } else if winrate >= t.a_min {
        "A"
    } else if winrate >= t.b_min {
        "B"
    } else {
        "C"
    }
}

pub fn tier_bounds(tier: &str, t: &Thresholds) -> (Option<f64>, Option<f64>) {
    match tier {
        "S+" => (Some(t.s_plus_min), None),
        "S" => (Some(t.s_min), Some(t.s_plus_min)),
        "A" => (Some(t.a_min), Some(t.s_min)),
        "B" => (Some(t.b_min), Some(t.a_min)),
        _ => (None, Some(t.b_min)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn thresholds_aliase_und_validierung() {
        let t = normalize_thresholds(&json!({"S+": 55, "S": 51, "A": 49, "B": 40})).expect("ok");
        assert_eq!(t.s_plus_min, 55.0);
        assert_eq!(t.b_min, 40.0);

        let err =
            normalize_thresholds(&json!({"s_plus_min": 40, "s_min": 50})).expect_err("aufsteigend");
        assert!(err.contains("descending"));

        // Nicht-numerische Werte fallen wie in Python auf den Default zurück
        let t = normalize_thresholds(&json!({"s_plus_min": "abc"})).expect("Default greift");
        assert_eq!(t.s_plus_min, 52.0);
    }

    #[test]
    fn tier_zuordnung() {
        let t = Thresholds::default();
        assert_eq!(tier_for_winrate(52.0, &t), "S+");
        assert_eq!(tier_for_winrate(51.99, &t), "S");
        assert_eq!(tier_for_winrate(45.0, &t), "C");
        assert_eq!(tier_bounds("C", &t), (None, Some(46.0)));
        assert_eq!(tier_bounds("S+", &t), (Some(52.0), None));
    }
}

#[cfg(all(test, feature = "testing"))]
mod pg_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires CENTRAL_TEST_DSN or DEADLOCK_CENTRAL_DSN"]
    async fn settings_defaults_and_upsert_roundtrip(
    ) -> std::result::Result<(), Box<dyn std::error::Error>> {
        let db = dl_central_db::testing::test_pool().await?;
        let pool = db.pool();

        let defaults = read_settings(pool).await?;
        assert_eq!(defaults.min_matches, 500);
        assert_eq!(defaults.refresh_interval_seconds, REFRESH_DEFAULT_SECONDS);

        set_setting(pool, "min_matches", "25").await?;
        set_setting(pool, "description_text", "Public notes").await?;
        let updated = read_settings(pool).await?;
        assert_eq!(updated.min_matches, 25);
        assert_eq!(updated.description_text, "Public notes");

        Ok(())
    }
}
