//! Fail-closed aggregate check. No network, dependencies or production credentials.
use std::{collections::BTreeMap, env, process::ExitCode};

const REQUIRED: [&str; 9] = [
    "actions",
    "format",
    "rust",
    "python",
    "secrets",
    "dependencies",
    "semgrep",
    "trivy",
    "codeql",
];

fn evaluate(results: &BTreeMap<String, String>) -> Result<(), String> {
    if results.len() != REQUIRED.len()
        || results.keys().any(|job| !REQUIRED.contains(&job.as_str()))
    {
        return Err("required job set is missing or unexpected".into());
    }
    for job in REQUIRED {
        match results.get(job).map(String::as_str) {
            Some("success") => {}
            result => return Err(format!("{job}: expected success, received {result:?}")),
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let results = env::vars()
        .filter_map(|(key, value)| {
            key.strip_prefix("CI_RESULT_")
                .map(|job| (job.to_owned(), value))
        })
        .collect();
    match evaluate(&results) {
        Ok(()) => {
            println!("Required PR Gate: every mandatory job succeeded");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("Required PR Gate BLOCKED: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn green() -> BTreeMap<String, String> {
        REQUIRED
            .into_iter()
            .map(|job| (job.to_owned(), "success".to_owned()))
            .collect()
    }

    #[test]
    fn all_success_passes() {
        assert!(evaluate(&green()).is_ok());
    }

    #[test]
    fn every_non_success_and_missing_result_blocks_for_every_job() {
        for job in REQUIRED {
            for status in [
                "failure",
                "cancelled",
                "skipped",
                "neutral",
                "timed_out",
                "",
                "unknown",
            ] {
                let mut results = green();
                results.insert(job.to_owned(), status.to_owned());
                assert!(evaluate(&results).is_err(), "accepted {job}={status}");
            }
            let mut results = green();
            results.remove(job);
            assert!(evaluate(&results).is_err(), "accepted missing {job}");
        }
    }

    #[test]
    fn empty_and_unexpected_job_sets_block() {
        assert!(evaluate(&BTreeMap::new()).is_err());
        let mut results = green();
        results.insert("unreviewed_new_check".into(), "success".into());
        assert!(evaluate(&results).is_err());
    }

    #[test]
    fn workflow_contract_matches_evaluator() {
        let workflow = include_str!("../workflows/required-pr-gate.yml");
        let jobs = workflow.split_once("\njobs:\n").expect("jobs mapping").1;
        let mut actual: Vec<&str> = jobs
            .lines()
            .filter_map(|line| {
                let key = line.strip_prefix("  ")?.strip_suffix(':')?;
                (!key.contains([' ', ':']) && key != "required").then_some(key)
            })
            .collect();
        actual.sort_unstable();
        let mut expected = REQUIRED.to_vec();
        expected.sort_unstable();
        assert_eq!(actual, expected, "every new job must also join the gate");
        let needs = format!("    needs: [{}]", REQUIRED.join(", "));
        assert!(
            jobs.lines().any(|line| line == needs),
            "aggregate needs must be exact"
        );
        for job in REQUIRED {
            assert!(jobs.contains(&format!("CI_RESULT_{job}: ${{{{ needs.{job}.result }}}}")));
        }
        assert!(jobs.contains("name: Required PR Gate\n    if: ${{ always() }}"));
        assert!(workflow.contains("  pull_request:\n"));
        assert!(!workflow.contains("paths:"));
        assert!(!workflow.contains("paths-ignore:"));
        assert!(!workflow.contains("continue-on-error:"));
    }
}
