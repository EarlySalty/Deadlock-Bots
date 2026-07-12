const ESCAPER: &str = r#"function esc(s){ return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;'); }"#;

#[test]
fn dynamic_dashboard_html_is_escaped() {
    let dashboard = include_str!("../../../../service/static/dashboard.html");
    let insights = include_str!("../../../../service/static/insights.html");

    for html in [dashboard, insights] {
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
}
