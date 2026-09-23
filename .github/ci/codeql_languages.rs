//! Select CodeQL languages from tracked files, not generated artifacts or obsolete paths.
use std::{collections::BTreeSet, path::Path, process::Command};

fn languages<'a>(files: impl Iterator<Item = &'a str>) -> BTreeSet<&'static str> {
    let mut result = BTreeSet::new();
    for file in files {
        let extension = Path::new(file).extension().and_then(|value| value.to_str());
        match extension {
            Some("rs") => {
                result.insert("rust");
            }
            Some("py") => {
                result.insert("python");
            }
            Some("js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx") => {
                result.insert("javascript-typescript");
            }
            Some("yml" | "yaml") if file.starts_with(".github/workflows/") => {
                result.insert("actions");
            }
            _ => {}
        }
    }
    result
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = Command::new("git").args(["ls-files", "-z"]).output()?;
    if !output.status.success() {
        return Err("git file inventory failed".into());
    }
    let text = String::from_utf8(output.stdout)?;
    let found = languages(text.split('\0'));
    if found.is_empty() {
        return Err("no analyzable tracked language found".into());
    }
    let rows: Vec<String> = found
        .into_iter()
        .map(|language| format!("{{\"language\":\"{language}\",\"build\":\"none\"}}"))
        .collect();
    println!("matrix={{\"include\":[{}]}}", rows.join(","));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn detects_real_python_without_a_legacy_cogs_directory() {
        let found = languages(
            [
                "scripts/export_infisical_env.py",
                ".tasks/task/test_report.py",
                "rust/bin/dl-bot/src/main.rs",
                "service/static/operating-config.js",
                ".github/workflows/codeql.yml",
            ]
            .into_iter(),
        );
        assert_eq!(
            found,
            ["actions", "javascript-typescript", "python", "rust"]
                .into_iter()
                .collect()
        );
    }
    #[test]
    fn unrelated_yaml_and_docs_do_not_create_toolchains() {
        assert!(
            languages(["README.md", "config/example.yaml", "rust/Cargo.toml"].into_iter())
                .is_empty()
        );
    }
}
