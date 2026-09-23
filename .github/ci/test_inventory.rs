//! Reject missing tests and substring collisions in the explicitly external test list.
use std::{
    collections::BTreeSet,
    io::{self, Read},
};

const EXTERNAL: &str = include_str!("external-rust-tests.txt");

fn exceptions() -> Vec<&'static str> {
    EXTERNAL
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
}

fn validate(inventory: &str) -> Result<usize, String> {
    let tests: Vec<&str> = inventory
        .lines()
        .filter_map(|line| line.strip_suffix(": test"))
        .collect();
    let external = exceptions();
    let unique: BTreeSet<_> = external.iter().collect();
    if external.is_empty() || unique.len() != external.len() {
        return Err("external test list is empty or duplicated".into());
    }
    for name in &external {
        // libtest's --skip is a substring filter, not an exact-name filter.
        let matches: Vec<_> = tests.iter().filter(|test| test.contains(name)).collect();
        if matches.len() != 1 || matches[0].rsplit("::").next() != Some(name) {
            return Err(format!(
                "external exception {name} must match exactly one existing test"
            ));
        }
    }
    if tests.len() <= external.len() {
        return Err("no mandatory Rust tests remain".into());
    }
    Ok(tests.len() - external.len())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut inventory = String::new();
    io::stdin().read_to_string(&mut inventory)?;
    let count = validate(&inventory)?;
    println!("Rust test inventory: {count} mandatory tests; {} explicitly external tests not counted as passed", exceptions().len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn inventory() -> String {
        let mut lines: Vec<String> = exceptions()
            .iter()
            .map(|name| format!("tests::{name}: test"))
            .collect();
        lines.push("tests::hermetic_contract: test".into());
        lines.join("\n")
    }
    #[test]
    fn existing_external_tests_leave_a_mandatory_test() {
        assert_eq!(validate(&inventory()), Ok(1));
    }
    #[test]
    fn absent_or_renamed_tests_cannot_disappear_silently() {
        for name in exceptions() {
            assert!(validate(&inventory().replace(&format!("tests::{name}: test"), "")).is_err());
        }
        assert!(validate("").is_err());
    }
    #[test]
    fn substring_collisions_cannot_expand_the_exclusion() {
        for name in exceptions() {
            let extra = format!("{}\ntests::new_{name}_regression: test", inventory());
            assert!(validate(&extra).is_err());
            let renamed = inventory().replace(
                &format!("tests::{name}: test"),
                &format!("tests::{name}_new: test"),
            );
            assert!(validate(&renamed).is_err());
        }
    }
    #[test]
    fn a_suite_containing_only_external_tests_is_not_success() {
        assert!(validate(&inventory().replace("tests::hermetic_contract: test", "")).is_err());
    }
}
