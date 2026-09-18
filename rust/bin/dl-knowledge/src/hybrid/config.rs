use std::collections::BTreeMap;

use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub bm25_k: usize,
    pub dense_k: usize,
    pub fusion_k: usize,
    pub output_k: usize,
    pub rrf_k: f64,
    pub bm25_weight: f64,
    pub dense_weight: f64,
    pub rerank: bool,
    pub rerank_max_tokens: usize,
    pub rerank_batch_size: usize,
    pub timeout_ms: u64,
    pub metadata: Metadata,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bm25_k: 24,
            dense_k: 24,
            fusion_k: 12,
            output_k: 6,
            rrf_k: 60.0,
            bm25_weight: 1.0,
            dense_weight: 1.0,
            rerank: true,
            rerank_max_tokens: 256,
            rerank_batch_size: 4,
            timeout_ms: 3000,
            metadata: Metadata::default(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        for k in [self.bm25_k, self.dense_k, self.fusion_k, self.output_k] {
            ensure!(
                (1..=100).contains(&k),
                "Hybrid k muss zwischen 1 und 100 liegen"
            );
        }
        ensure!(
            self.output_k <= self.fusion_k,
            "output_k ist größer als fusion_k"
        );
        ensure!(
            self.rrf_k.is_finite() && self.rrf_k > 0.0 && self.rrf_k <= 10000.0,
            "Ungültiges RRF k"
        );
        for weight in [self.bm25_weight, self.dense_weight] {
            ensure!(
                weight.is_finite() && (0.0..=100.0).contains(&weight),
                "Ungültiges RRF-Gewicht"
            );
        }
        ensure!(
            self.bm25_weight + self.dense_weight > 0.0,
            "Beide Retrieval-Gewichte sind null"
        );
        ensure!(
            (32..=512).contains(&self.rerank_max_tokens),
            "Ungültiges Tokenlimit"
        );
        ensure!(
            (1..=16).contains(&self.rerank_batch_size),
            "Ungültige Reranker-Batchgröße"
        );
        ensure!(
            (1..=10000).contains(&self.timeout_ms),
            "Ungültiges Retrieval-Zeitbudget"
        );
        self.metadata.validate()
    }

    pub fn from_values(enabled: Option<&str>, options: Option<&str>) -> Result<Option<Self>> {
        match enabled.unwrap_or("false") {
            "false" | "0" => Ok(None),
            "true" | "1" => {
                let config: Self = options
                    .map(serde_json::from_str)
                    .transpose()
                    .context("Ungültige Hybrid-Optionen")?
                    .unwrap_or_default();
                config.validate()?;
                Ok(Some(config))
            }
            _ => anyhow::bail!("DL_KNOWLEDGE_HYBRID muss true, false, 1 oder 0 sein"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Metadata {
    pub stand_min: Option<String>,
    pub stand_exact: Option<String>,
    pub quellen: Vec<String>,
    pub recency_weight: f64,
    pub recency_half_life_days: f64,
    pub source_weights: BTreeMap<String, f64>,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            stand_min: None,
            stand_exact: None,
            quellen: Vec::new(),
            recency_weight: 0.0,
            recency_half_life_days: 30.0,
            source_weights: BTreeMap::new(),
        }
    }
}

impl Metadata {
    fn validate(&self) -> Result<()> {
        if let Some(min) = &self.stand_min {
            ensure!(
                day(min).is_some(),
                "stand_min benötigt ein gültiges ISO-Datum YYYY-MM-DD"
            );
        }
        ensure!(
            self.stand_exact
                .as_ref()
                .is_none_or(|value| !value.trim().is_empty()),
            "Leerer Stand-Filter"
        );
        ensure!(
            self.quellen.iter().all(|value| !value.trim().is_empty()),
            "Leerer Quellenfilter"
        );
        ensure!(
            self.recency_weight.is_finite() && (0.0..=10.0).contains(&self.recency_weight),
            "Ungültiges Aktualitätsgewicht"
        );
        ensure!(
            self.recency_half_life_days.is_finite() && self.recency_half_life_days > 0.0,
            "Ungültige Aktualitätshalbwertszeit"
        );
        ensure!(
            self.source_weights
                .iter()
                .all(|(key, weight)| !key.trim().is_empty()
                    && weight.is_finite()
                    && *weight > 0.0
                    && *weight <= 100.0),
            "Ungültige Quellenpriorität"
        );
        Ok(())
    }

    pub fn accepts(&self, stand: &str, quelle: &str) -> bool {
        self.stand_exact.as_ref().is_none_or(|value| value == stand)
            && self.stand_min.as_ref().is_none_or(|min| {
                day(stand)
                    .zip(day(min))
                    .is_some_and(|(actual, min)| actual >= min)
            })
            && (self.quellen.is_empty() || self.quellen.iter().any(|value| value == quelle))
    }

    pub fn multiplier(&self, stand: &str, quelle: &str, newest: Option<i64>) -> f64 {
        let recency = day(stand).zip(newest).map_or(0.0, |(date, newest)| {
            2.0_f64.powf(-((newest - date).max(0) as f64) / self.recency_half_life_days)
        });
        (1.0 + self.recency_weight * recency)
            * self.source_weights.get(quelle).copied().unwrap_or(1.0)
    }
}

pub fn day(value: &str) -> Option<i64> {
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return None;
    }
    let year: i64 = value[..4].parse().ok()?;
    let month: usize = value[5..7].parse().ok()?;
    let date: i64 = value[8..].parse().ok()?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year < 1 || !(1..=12).contains(&month) || !(1..=months[month - 1]).contains(&date) {
        return None;
    }
    let before = year - 1;
    Some(
        before * 365 + before / 4 - before / 100
            + before / 400
            + months[..month - 1].iter().sum::<i64>()
            + date,
    )
}
