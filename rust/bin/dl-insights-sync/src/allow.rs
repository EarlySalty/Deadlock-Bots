//! Klickt das Brave-Overlay „Allow remote debugging?“ weg.
//! Das Overlay sitzt in der Browser-Chrome, nicht in der Seite.
//! Immer ein neuer Tab im schon laufenden Fenster, nie ein zweites Fenster.
//! Tasten gehen über XTEST (ohne `--window`): Chromium ignoriert XSendEvent.

use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};

const INSPECT_URL: &str = "brave://inspect/#remote-debugging";
const INSPECT_TITLE: &str = "Inspect with Chrome Developer Tools";
/// Cancel trägt im Overlay den Fokusring. Return allein klickt Cancel.
pub const ALLOW_CONFIRM_KEYS: &[&str] = &["Tab", "Return"];

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
    let id = main_brave_window()?;
    raise_brave(&id)?;
    send_key("ctrl+t")?;
    std::thread::sleep(Duration::from_millis(250));
    send_key("ctrl+l")?;
    std::thread::sleep(Duration::from_millis(120));
    let typed = xdotool_cmd()
        .args(["type", "--delay", "8", "--clearmodifiers", INSPECT_URL])
        .status()
        .context("xdotool type")?;
    if !typed.success() {
        return Err(anyhow!("Inspect-URL konnte nicht eingetippt werden"));
    }
    send_key("Return")?;
    std::thread::sleep(Duration::from_millis(900));
    raise_brave(&id)?;
    tracing::info!(window = %id, "Inspect-Tab im bestehenden Fenster geöffnet");
    Ok(())
}

/// Allow sitzt am Fenster, nicht an einem bestimmten Tab.
/// Cancel ist der Default-Fokus (lila Ring). Tab setzt ihn auf Allow, Return
/// bestätigt. Return allein klickt Cancel und räumt das Overlay weg.
/// Brave-Downloadleiste: „Behalten“ / „Zulassen“ ist oft der Default.
pub fn confirm_download() -> Result<()> {
    confirm_allow(&["Return"])
}

pub fn confirm_allow(keys: &[&str]) -> Result<()> {
    let id = main_brave_window()?;
    raise_brave(&id)?;
    for key in keys {
        send_key(key)?;
        std::thread::sleep(Duration::from_millis(80));
    }
    Ok(())
}

fn raise_brave(id: &str) -> Result<()> {
    let _ = wmctrl_raise(id);
    xdotool(&["windowactivate", "--sync", id])?;
    let _ = xdotool(&["windowfocus", "--sync", id]);
    let _ = xdotool(&["windowraise", id]);
    std::thread::sleep(Duration::from_millis(150));
    Ok(())
}

fn wmctrl_raise(id: &str) -> Result<()> {
    let hex = window_id_hex(id)?;
    let status = wmctrl_cmd()
        .args(["-i", "-a", &hex])
        .status()
        .context("wmctrl fehlt")?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("wmctrl -i -a {hex} fehlgeschlagen"))
    }
}

fn window_id_hex(id: &str) -> Result<String> {
    let n: u64 = id.parse().context("Brave-Fenster-ID ist keine Zahl")?;
    Ok(format!("0x{n:x}"))
}

fn send_key(key: &str) -> Result<()> {
    let args = key_args(key);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    xdotool(&refs)
}

fn key_args(key: &str) -> Vec<String> {
    vec![
        "key".to_string(),
        "--clearmodifiers".to_string(),
        key.to_string(),
    ]
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

fn wmctrl_cmd() -> Command {
    let mut cmd = Command::new("wmctrl");
    cmd.env("DISPLAY", display());
    if let Ok(xauth) = std::env::var("XAUTHORITY") {
        cmd.env("XAUTHORITY", xauth);
    } else {
        cmd.env("XAUTHORITY", "/home/nathanael/.Xauthority");
    }
    cmd
}

fn xdotool(args: &[&str]) -> Result<()> {
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
    fn allow_bestaetigt_nicht_den_cancel_fokus() {
        assert_eq!(super::ALLOW_CONFIRM_KEYS, ["Tab", "Return"]);
        assert_ne!(super::ALLOW_CONFIRM_KEYS.first().copied(), Some("Return"));
    }

    #[test]
    fn tasten_gehen_ueber_xtest_nicht_xsend_event() {
        let args = super::key_args("Tab");
        assert!(!args.iter().any(|a| a == "--window"));
        assert_eq!(args, ["key", "--clearmodifiers", "Tab"]);
    }

    #[test]
    fn wmctrl_nimmt_hex_fenster_id() {
        assert_eq!(super::window_id_hex("6291459").expect("parse"), "0x600003");
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
