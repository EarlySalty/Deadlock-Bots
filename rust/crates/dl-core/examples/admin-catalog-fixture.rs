//! Synthetischer API-Vertrag für Browser-Smoke-Tests; liest keine Betriebsdatei.
fn main() {
    let config = dl_core::bot_config::BotConfig::parse("schema_version=1\n[llm.fireworks]\nmodel='accounts/fireworks/models/deepseek-v4-flash-0731'\n[llm.use_cases.faq]\nmodel='accounts/fireworks/models/deepseek-v4-flash-0731'\n[runtime.community]\nconcierge_proactive=false\nconcierge_pate_channel_id=1547199955133927464\n").expect("synthetische Config");
    println!(
        "{}",
        serde_json::json!({
            "revision":"a".repeat(64), "saved_fingerprint":"b".repeat(64),
            "options":dl_core::operating_config::OperatingOptions::from(&config),
            "catalog":dl_core::admin_settings::catalog(&config),
            "services":[{"name":"Discord-Web","restart_required":false},{"name":"Discord-Bot","restart_required":false}]
        })
    );
}
