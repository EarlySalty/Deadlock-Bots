//! Klickt das Brave-Overlay „Allow remote debugging?“ weg.
//! Das Overlay sitzt in der Browser-Chrome, nicht in der Seite.
//! Immer ein neuer Tab im schon laufenden Fenster, nie ein zweites Fenster.

use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

const INSPECT_URL: &str = "brave://inspect/#remote-debugging";
const INSPECT_TITLE: &str = "Inspect with Chrome Developer Tools";

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
    ":10.0".to_string()
}

/// Öffnet immer einen neuen Tab im bestehenden Brave-Fenster.
pub fn open_inspect_tab() -> Result<()> {
    let disp = display();
    let id = main_brave_window()?;
    xdotool(&disp, &["windowactivate", "--sync", &id])?;
    std::thread::sleep(Duration::from_millis(150));
    xdotool(&disp, &["key", "--window", &id, "ctrl+t"])?;
    std::thread::sleep(Duration::from_millis(250));
    xdotool(&disp, &["key", "--window", &id, "ctrl+l"])?;
    std::thread::sleep(Duration::from_millis(120));
    let typed = xdotool_cmd()
        .args([
            "type",
            "--window",
            &id,
            "--delay",
            "8",
            "--clearmodifiers",
            INSPECT_URL,
        ])
        .status()
        .context("xdotool type")?;
    if !typed.success() {
        return Err(anyhow!("Inspect-URL konnte nicht eingetippt werden"));
    }
    xdotool(&disp, &["key", "--window", &id, "Return"])?;
    std::thread::sleep(Duration::from_millis(900));
    xdotool(&disp, &["windowactivate", "--sync", &id])?;
    tracing::info!(window = %id, "Inspect-Tab im bestehenden Fenster geöffnet");
    Ok(())
}

/// Allow sitzt am Fenster, nicht an einem bestimmten Tab.
/// Allow ist oft der Default (Return reicht). Nach einem Klick ist
/// Cancel umrandet, dann Tab und Return.
/// Brave-Downloadleiste: „Behalten“ / „Zulassen“ ist oft der Default.
pub fn confirm_download() -> Result<()> {
    confirm_allow(&["Return"])
}

pub fn confirm_allow(keys: &[&str]) -> Result<()> {
    let disp = display();
    let id = main_brave_window()?;
    xdotool(&disp, &["windowactivate", "--sync", &id])?;
    std::thread::sleep(Duration::from_millis(120));
    for key in keys {
        xdotool(&disp, &["key", "--window", &id, key])?;
        std::thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}

fn main_brave_window() -> Result<String> {
    let ids = brave_window_ids();
    if ids.is_empty() {
        return Err(anyhow!("kein Brave-Fenster unter DISPLAY={}", display()));
    }
    let named: Vec<(String, String)> = ids
        .into_iter()
        .map(|id| {
            let name = window_name(&id).unwrap_or_default();
            (id, name)
        })
        .collect();
    if let Some((id, _)) = named.iter().find(|(_, name)| name.contains(INSPECT_TITLE)) {
        return Ok(id.clone());
    }
    if let Some((id, _)) = named.iter().find(|(_, name)| !name.contains("New Tab")) {
        return Ok(id.clone());
    }
    Ok(named[0].0.clone())
}

fn brave_window_ids() -> Vec<String> {
    let out = xdotool_cmd()
        .args(["search", "--class", "Brave-browser"])
        .output();
    let Ok(out) = out else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    String::from_utf8(out.stdout)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn window_name(id: &str) -> Option<String> {
    let out = xdotool_cmd().args(["getwindowname", id]).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn xdotool_cmd() -> Command {
    let mut cmd = Command::new("/usr/bin/xdotool");
    cmd.env("DISPLAY", display());
    if let Ok(xauth) = std::env::var("XAUTHORITY") {
        cmd.env("XAUTHORITY", xauth);
    } else {
        cmd.env("XAUTHORITY", "/home/nathanael/.Xauthority");
    }
    cmd
}

fn xdotool(display: &str, args: &[&str]) -> Result<()> {
    let _ = display;
    let status = xdotool_cmd().args(args).status().context("xdotool fehlt")?;
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

    #[test]
    fn main_window_bevorzugt_inspect_vor_new_tab() {
        let named: [(&str, &str); 2] = [
            ("1", "New Tab - Brave"),
            ("2", "Inspect with Chrome Developer Tools - Brave"),
        ];
        let inspect = named
            .iter()
            .find(|(_, name)| name.contains(super::INSPECT_TITLE))
            .map(|(id, _)| *id);
        assert_eq!(inspect, Some("2"));
    }
}
