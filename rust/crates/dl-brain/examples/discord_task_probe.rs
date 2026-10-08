use dl_brain::{
    brain_api::BrainApiAnswerer, AiAnswerer, BrainOutcome, DiscordAnswerCapability,
    DiscordAnswerTask,
};
use std::{collections::BTreeSet, time::Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = std::env::var("DISCORD_BRAIN_PROBE_ENDPOINT")?;
    let timeout_ms = std::env::var("DISCORD_BRAIN_PROBE_TIMEOUT_MS")?.parse()?;
    let token = std::env::var("DISCORD_BRAIN_CLIENT_TOKEN")?;
    let channel_id = std::env::var("DISCORD_BRAIN_PROBE_CHANNEL_ID")?.parse()?;
    let brain = BrainApiAnswerer::new(
        &endpoint,
        &token,
        Duration::from_millis(timeout_ms),
        "discord-synthetic-probe".into(),
        BTreeSet::from(["bot.public".into()]),
    )?;
    let question =
        "Wie erstelle ich einen Sprachkanal für meine Gruppe auf dem deutschen Deadlock-Discord?";
    for capability in [
        DiscordAnswerCapability::Concierge,
        DiscordAnswerCapability::Faq,
    ] {
        let task = DiscordAnswerTask {
            capability,
            channel_id,
        };
        let outcome = brain.answer_discord_task(question, 42, &task).await?;
        let BrainOutcome::Answer(answer) = outcome else {
            return Err("Keine Antwort auf die synthetische Frage".into());
        };
        println!(
            "{}",
            serde_json::json!({"capability": capability, "question": question, "answer": answer})
        );
    }
    Ok(())
}
