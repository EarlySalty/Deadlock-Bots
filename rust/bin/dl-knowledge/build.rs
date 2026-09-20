use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    // Cargo's output directory is build plumbing, not application configuration.
    let output = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output directory"));
    let revision = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|result| result.status.success())
        .and_then(|result| String::from_utf8(result.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_default();
    fs::write(
        output.join("source_revision.rs"),
        format!("pub const SOURCE_REVISION: &str = {revision:?};\n"),
    )
    .expect("write build provenance");
    // Git worktrees use a .git indirection; track the actual reference locations.
    let mut references = vec!["HEAD".to_owned(), "packed-refs".to_owned()];
    if let Ok(result) = Command::new("git").args(["symbolic-ref", "HEAD"]).output() {
        if result.status.success() {
            references.push(String::from_utf8_lossy(&result.stdout).trim().to_owned());
        }
    }
    for reference in references {
        if let Ok(result) = Command::new("git")
            .args(["rev-parse", "--git-path", &reference])
            .output()
        {
            if result.status.success() {
                println!(
                    "cargo:rerun-if-changed={}",
                    String::from_utf8_lossy(&result.stdout).trim()
                );
            }
        }
    }
    println!("cargo:rerun-if-changed=build.rs");
}
