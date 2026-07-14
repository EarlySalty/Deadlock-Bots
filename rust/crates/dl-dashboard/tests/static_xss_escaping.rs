const ESCAPER: &str = r#"function esc(s){ return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;'); }"#;

#[test]
fn dynamic_dashboard_html_is_escaped() {
    let dashboard = include_str!("../../../../service/static/dashboard.html");
    let insights = include_str!("../../../../service/static/insights.html");
    let audit = include_str!("../../../../service/static/audit.html");

    for html in [dashboard, insights, audit] {
        assert!(html.contains(ESCAPER), "HTML escaper is missing");
    }

    for escaped_sink in [
        "${esc(sourceName)}",
        "${esc(runtime.account_name || '–')}",
        "${esc(cmd.command)}",
        "${esc(repo.name)}",
        "${esc(e.message)}",
    ] {
        assert!(dashboard.contains(escaped_sink), "missing {escaped_sink}");
    }

    for escaped_sink in [
        "${esc(r.invite_code ?? \"?\")}",
        "${esc(data.error ?? \"unbekannter Fehler\")}",
        "${esc(r.file)}",
        "${esc(r.import_kind)}",
        "${esc(err.message)}",
    ] {
        assert!(insights.contains(escaped_sink), "missing {escaped_sink}");
    }

    for escaped_sink in [
        "${esc(formatDate(entry.occurred_at))}",
        "${esc(entry.action_name)}",
        "${esc(displayId(entry.actor))}",
        "${esc(displayId(entry.target))}",
        "${esc(formatDetails(entry.changes, entry.options))}",
        "${esc(entry.reason ?? \"–\")}",
    ] {
        assert!(audit.contains(escaped_sink), "missing {escaped_sink}");
    }

    for action_type in [
        1, 10, 11, 12, 13, 14, 15, 20, 22, 23, 24, 25, 26, 27, 30, 31, 32, 40, 41, 42, 50, 51, 52,
        60, 61, 62, 72, 73, 74, 75, 80, 81, 82, 110, 111, 112, 143, 144, 145, 163, 164, 165, 166,
        167, 190, 191, 192, 193,
    ] {
        let option = format!(r#"<option value="{action_type}">"#);
        assert!(audit.contains(&option), "missing {option}");
    }
    assert!(audit.contains("Optionen:"));
}
