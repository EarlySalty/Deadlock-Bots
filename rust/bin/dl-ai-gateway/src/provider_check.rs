use anyhow::{Context, Result};
use sqlx::PgPool;

const DOWN_SQL: &str = "SELECT provider, count(*)::bigint AS failures, \
     string_agg(DISTINCT coalesce(error_code, 'unknown'), ',' ORDER BY coalesce(error_code, 'unknown')) AS codes \
     FROM public.llm_usage \
     WHERE project = 'Deadlock-Bots' AND service = $1 \
       AND attempt_state IN ('succeeded', 'failed') \
       AND finished_at >= now() - interval '24 hours' \
     GROUP BY provider \
     HAVING count(*) FILTER (WHERE attempt_state = 'succeeded') = 0 \
        AND max(finished_at) - min(finished_at) >= interval '12 hours' \
     ORDER BY provider";

#[derive(Debug, PartialEq, Eq)]
pub struct ProviderDown {
    pub provider: String,
    pub failures: i64,
    pub codes: String,
}

pub async fn failing_providers(pool: &PgPool, service: &str) -> Result<Vec<ProviderDown>> {
    let rows: Vec<(Option<String>, i64, Option<String>)> = sqlx::query_as(DOWN_SQL)
        .bind(service)
        .fetch_all(pool)
        .await
        .context("reading provider outcomes from llm_usage")?;
    Ok(rows
        .into_iter()
        .map(|(provider, failures, codes)| ProviderDown {
            provider: provider.unwrap_or_else(|| "unknown".into()),
            failures,
            codes: codes.unwrap_or_default(),
        })
        .collect())
}

pub fn report(down: &[ProviderDown]) -> Result<()> {
    for provider in down {
        eprintln!(
            "AI_GATEWAY_PROVIDER_DOWN provider={} failures_24h={} codes={}",
            provider.provider, provider.failures, provider.codes
        );
    }
    anyhow::ensure!(
        down.is_empty(),
        "{} KI-Anbieter ohne einen erfolgreichen Aufruf seit 24 Stunden: {}",
        down.len(),
        down.iter()
            .map(|provider| format!("{} ({})", provider.provider, provider.codes))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("AI_GATEWAY_PROVIDERS_OK");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_fails_only_when_a_provider_is_down() {
        assert!(report(&[]).is_ok());
        let down = [ProviderDown {
            provider: "anthropic_api".into(),
            failures: 874,
            codes: "billing".into(),
        }];
        let error = report(&down).expect_err("down provider must fail the check");
        assert!(error.to_string().contains("anthropic_api (billing)"));
    }
}
