use dl_brain::{brain_api::BrainApiAnswerer, AiAnswerer, BrainOutcome};
use serde_json::json;
use std::{
    collections::BTreeSet,
    time::{Duration, Instant},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let endpoint = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Aufruf: qa_probe BRAIN-URL FRAGEN.json"))?;
    let questions = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Fragen-Datei fehlt"))?;
    let token = dl_core::runtime_config::secret_value("DISCORD_BRAIN_CLIENT_TOKEN")
        .ok_or_else(|| anyhow::anyhow!("Brain-Zugang nicht konfiguriert"))?;
    let brain = BrainApiAnswerer::new(
        &endpoint,
        &token,
        Duration::from_secs(100),
        "qa-probe".into(),
        BTreeSet::from(["bot.public".into()]),
    )?;
    let questions: Vec<serde_json::Value> =
        serde_json::from_slice(&tokio::fs::read(questions).await?)?;
    for item in questions {
        let question = item["question"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Frage fehlt"))?;
        let start = Instant::now();
        let (status, answer) = match brain.answer(question).await {
            Ok(BrainOutcome::Answer(text)) => ("answered", Some(text)),
            Ok(_) => ("no_answer", None),
            Err(_) => ("error", None),
        };
        println!(
            "{}",
            json!({"id":item["id"],"status":status,"answer":answer,"total_ms":start.elapsed().as_millis()})
        );
    }
    Ok(())
}
