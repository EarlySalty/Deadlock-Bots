//! Community-Punkte-Sync (Paket C). Logik in `lib.rs`; hier nur Bootstrap
//! wie bei `dl-twitch-invite-sync`: TOML-Konfiguration, Token aus dem
//! privaten Infisical-Snapshot über FD3, wie beim Hauptbot.

use dl_central_db::community_points::{import_clip_contest_ledger, import_qualified_join_ledger};
use dl_community_points_sync::{
    endpoint, http_client, streamer_row, suggestion_event, sync_source, validate_base_url,
    viewer_row, StreamerDbSink, StreamerWire, SuggestionLedgerSink, SuggestionOutcomeWire,
    ViewerDbSink, ViewerWire, DEFAULT_TWITCH_API_URL, STREAMERS_PATH, SUGGESTION_OUTCOMES_PATH,
    VIEWERS_PATH,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = dl_core::config::process_bot_config()?.snapshot();
    dl_core::token_snapshot::load(dl_core::config::process_bot_config()?.source())
        .map_err(|error| anyhow::anyhow!(error))?;
    let base = config
        .runtime
        .bridges
        .twitch_api_url
        .clone()
        .unwrap_or_else(|| DEFAULT_TWITCH_API_URL.to_string());
    let token = dl_core::runtime_config::secret_value("TWITCH_INTERNAL_API_TOKEN")
        .ok_or_else(|| anyhow::anyhow!("TWITCH_INTERNAL_API_TOKEN fehlt im Infisical-Bootstrap"))?;
    let base = validate_base_url(&base)?;
    let central_dsn = dl_core::token_snapshot::value("DEADLOCK_CENTRAL_DSN")
        .ok_or_else(|| anyhow::anyhow!("Zentraler Datenbankzugang fehlt im privaten Snapshot"))?;
    let pool = dl_central_db::connect_pool(central_dsn).await?;
    let client = http_client()?;

    let viewers = sync_source::<ViewerWire, _, _>(
        &client,
        &endpoint(&base, VIEWERS_PATH)?,
        &token,
        &ViewerDbSink(&pool),
        viewer_row,
    )
    .await?;
    println!(
        "Zuschauer: {} Seiten, {} Zeilen geholt, {} übersprungen, {} geschrieben, Cursor {}",
        viewers.pages,
        viewers.fetched,
        viewers.skipped,
        viewers.written,
        viewers.cursor.as_deref().unwrap_or("-")
    );

    let streamers = sync_source::<StreamerWire, _, _>(
        &client,
        &endpoint(&base, STREAMERS_PATH)?,
        &token,
        &StreamerDbSink(&pool),
        streamer_row,
    )
    .await?;
    println!(
        "Streamer: {} Seiten, {} Zeilen geholt, {} übersprungen, {} geschrieben, Cursor {}",
        streamers.pages,
        streamers.fetched,
        streamers.skipped,
        streamers.written,
        streamers.cursor.as_deref().unwrap_or("-")
    );

    // Ein Fehler hier (z. B. Twitch-Bot noch ohne Paket F) stoppt die
    // übrigen Ledger-Importe nicht.
    match sync_source::<SuggestionOutcomeWire, _, _>(
        &client,
        &endpoint(&base, SUGGESTION_OUTCOMES_PATH)?,
        &token,
        &SuggestionLedgerSink(&pool),
        suggestion_event,
    )
    .await
    {
        Ok(outcomes) => println!(
            "Streamer-Vorschläge: {} Seiten, {} Zeilen geholt, {} ohne Punkte, {} neu gebucht",
            outcomes.pages, outcomes.fetched, outcomes.skipped, outcomes.written
        ),
        Err(error) => eprintln!("Streamer-Vorschläge nicht synchronisiert: {error:#}"),
    }

    let joins = import_qualified_join_ledger(&pool).await?;
    println!("Ledger: {joins} neue qualifizierte Beitritte");
    let clips = import_clip_contest_ledger(&pool).await?;
    if clips.tables_missing {
        println!("Ledger: Clip-Contest-Tabellen fehlen noch, kein Import");
    } else {
        println!(
            "Ledger: {} neue Clip-Plätze, {} neue Clip-Stimmen",
            clips.places, clips.votes
        );
    }
    Ok(())
}
