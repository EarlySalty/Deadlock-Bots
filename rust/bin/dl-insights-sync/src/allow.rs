//! Klickt das Brave-Overlay „Allow remote debugging?“ weg.
//! Das Overlay sitzt in der Browser-Chrome, nicht in der Seite.
//! xdotool auf maximierten Fenstern ist unzuverlässig, Tastatur geht.

use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

const INSPECT_URL: &str = "brave://inspect/#remote-debugging";
const INSPECT_TITLE: &str = "Inspect with Chrome Developer Tools";
const DEFAULT_BRAVE_BIN: &str = "/usr/bin/brave-browser-stable";

pub fn display() -> String {
    if let Ok(d) = std::env::var("INSIGHTS_BRAVE_DISPLAY") {
        if !d.is_empty() {
            return d;
        }
    }
    if let Ok(d) = std::env::var("DISPLAY") {
        if !d.is_empty() {
            return d;
        }
    }
    ":10".to_string()
}

pub fn ensure_inspect_page() -> Result<()> {
    if inspect_window().is_some() {
        return Ok(());
    }
    let bin = std::env::var("INSIGHTS_BRAVE_BIN").unwrap_or_else(|_| DEFAULT_BRAVE_BIN.to_string());
    let disp = display();
    let xauth =
        std::env::var("XAUTHORITY").unwrap_or_else(|_| "/home/nathanael/.Xauthority".to_string());
    Command::new(&bin)
        .arg(INSPECT_URL)
        .env("DISPLAY", &disp)
        .env("XAUTHORITY", xauth)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("Inspect-Seite öffnen: {bin}"))?;
    let deadline = std::time::Instant::now() + Duration::from_secs(8);
    while std::time::Instant::now() < deadline {
        if inspect_window().is_some() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(anyhow!(
        "Inspect-Seite nicht sichtbar. Brave unter DISPLAY={disp} prüfen."
    ))
}

/// Allow ist oft der Default (Return reicht). Nach einem Klick ist
/// Cancel umrandet, dann Tab und Return.
pub fn confirm_allow(keys: &[&str]) -> Result<()> {
    let disp = display();
    let id = inspect_window().ok_or_else(|| anyhow!("kein Inspect-Fenster"))?;
    xdotool(&disp, &["windowactivate", "--sync", &id])?;
    std::thread::sleep(Duration::from_millis(120));
    for key in keys {
        xdotool(&disp, &["key", "--window", &id, key])?;
        std::thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}

fn inspect_window() -> Option<String> {
    let disp = display();
    let out = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", INSPECT_TITLE])
        .env("DISPLAY", disp)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout)
        .ok()?
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

fn xdotool(display: &str, args: &[&str]) -> Result<()> {
    let status = Command::new("xdotool")
        .args(args)
        .env("DISPLAY", display)
        .status()
        .context("xdotool fehlt")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("xdotool {args:?} fehlgeschlagen"))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn inspect_titel_ist_der_aus_der_seite() {
        assert!(super::INSPECT_TITLE.contains("Inspect"));
        assert!(super::INSPECT_URL.contains("remote-debugging"));
    }
}
