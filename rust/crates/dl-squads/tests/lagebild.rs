use dl_ai::ChatProviderError;
use dl_squads::lagebild::{
    correct_team_lagebild, finalize_lagebild_text, generate_due_lagebilder, generate_lagebild,
    lagebild_messages, plan_lagebild, refresh_team_lagebild, render_lagebild_report,
    ChannelHistory, ChannelHistoryBatch, ChannelHistoryError, ChannelHistoryMessage,
    CorrectionActor, LagebildError, LagebildPlan, LagebildPrioritaet, LagebildReport,
    ScrimLagebildEvidence, ScrimLagebildInput,
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::sync::{Arc, Mutex};

#[test]
fn lagebild_text_haengt_relevante_evidenzen_ans_ende() {
    let evidence = ScrimLagebildEvidence::discord_message(
        "Terminabfrage Team A",
        100,
        9001,
        Some("2026-07-24T18:00:00Z".to_string()),
    );

    let text = finalize_lagebild_text("Die Lage wirkt aktuell okay.", &[evidence]);

    assert!(text.ends_with(
        "Evidenzen:\n- [Terminabfrage Team A](https://discord.com/channels/1289721245281292288/100/9001)"
    ));
    assert!(!text.contains('—'));
    assert!(!text.contains("Ampel"));
}

#[test]
fn lagebild_prompt_benennt_begrenzte_datenlage_und_schliesst_dms_aus() {
    let input = ScrimLagebildInput {
        team_id: 1,
        team_name: "Team A".to_string(),
        generated_for: "weekly".to_string(),
        data_limited: true,
        has_operational_data: true,
        facts: vec!["Nur Teamstamm und eine offene Terminabfrage sind vorhanden.".to_string()],
        corrections: vec!["Coach: A2 ist wieder verfügbar.".to_string()],
        evidences: Vec::new(),
    };

    let messages = lagebild_messages(&input);
    let joined = messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(joined.contains("Keine DMs"));
    assert!(joined.contains("begrenzte Datenlage"));
    assert!(joined.contains("A2 ist wieder verfügbar"));
}

#[tokio::test]
async fn fehler_lagebild_wird_nach_kurzem_backoff_erneut_versucht(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    insert_terminabfrage(db.pool(), 1).await?;

    assert_eq!(generate_due_lagebilder(db.pool(), None, None, 1).await?, 1);
    assert_eq!(
        latest_scrim_ai_verdict(db.pool(), "lagebild_generate").await?,
        "error"
    );
    assert_eq!(generate_due_lagebilder(db.pool(), None, None, 1).await?, 0);
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(2, 'B', now())")
        .execute(db.pool())
        .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.lagebild_snapshots(
            team_id, generated_at, generated_for, source, status, lagebild_text, data_summary, error
        )
        VALUES(2, now() - interval '16 minutes', 'weekly', 'ai', 'error', 'Alter Fehler', '{}'::jsonb, 'AI-Provider fehlt')
        "#,
    )
    .execute(db.pool())
    .await?;
    assert_eq!(generate_due_lagebilder(db.pool(), None, None, 1).await?, 1);
    Ok(())
}

#[tokio::test]
async fn lagebild_match_history_nutzt_nur_final_ausgewaehlte_result_refs(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    for (id, name) in [(1, "A"), (2, "B")] {
        sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES($1, $2, now())")
            .bind(id)
            .bind(name)
            .execute(db.pool())
            .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO scrim.matches(id, team_a_id, team_b_id, status, lobby_state, created_at, updated_at)
        VALUES
            (10, 1, 2, 'scheduled', 'draft', now(), now()),
            (11, 1, 2, 'completed', 'finished', now(), now()),
            (12, 1, 2, 'scheduled', 'result_requested', now(), now())
        "#,
    )
    .execute(db.pool())
    .await?;
    sqlx::query(
        "UPDATE scrim.matches SET result_json = '{\"legacy_raw\":true}'::jsonb WHERE id = 12",
    )
    .execute(db.pool())
    .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.match_result_refs(
            match_id, steam_match_id, source_user_id, source_display_name,
            fetch_status, winner_team_id, normalized_result_json, validation_status, entered_at, updated_at
        )
        VALUES
            (11, 111, '42', 'Coach', 'fetched', 1, '{"selected":true}'::jsonb, 'valid', now(), now()),
            (12, 222, '42', 'Coach', 'pending', NULL, '{}'::jsonb, 'unvalidated', now(), now())
        "#,
    )
    .execute(db.pool())
    .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.match_result_selections(
            match_id, result_ref_id, selected_by_user_id, selected_by_display_name, selection_reason
        )
        SELECT 11, id, '42', 'Coach', 'contract_test'
          FROM scrim.match_result_refs
         WHERE match_id = 11
        "#,
    )
    .execute(db.pool())
    .await?;

    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Die Lage ist nachvollziehbar.","risiken":[],"naechster_schritt":"Naechsten Scrim planen.","prioritaet":"keine"}"#,
    );
    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), None, 1).await?,
        1
    );
    let request = provider.requests().pop().ok_or("missing AI request")?;
    let prompt = request
        .0
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("Match 11:"));
    assert!(prompt.contains("Steam-Match 111"));
    assert!(!prompt.contains("Match 10:"));
    assert!(!prompt.contains("Match 12:"));
    assert!(!prompt.contains("legacy_raw"));
    assert!(!prompt.contains("selected"));

    let snapshot = sqlx::query(
        "SELECT status, error FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(snapshot.get::<String, _>("status"), "ok");
    assert_eq!(snapshot.get::<Option<String>, _>("error"), None);
    assert_eq!(
        latest_scrim_ai_verdict(db.pool(), "lagebild_generate").await?,
        "yes"
    );
    let run_state: String = sqlx::query_scalar(
        "SELECT state FROM scrim.ai_runs WHERE run_kind = 'lagebild_generate' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(run_state, "succeeded");
    Ok(())
}

#[tokio::test]
async fn lagebild_timeout_bleibt_im_ledger_und_snapshot_sichtbar(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    insert_terminabfrage(db.pool(), 1).await?;
    let provider = dl_ai::MockChatProvider::new(vec![Err(ChatProviderError::Timeout)]);

    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), None, 1).await?,
        1
    );
    let decision: String = sqlx::query_scalar(
        "SELECT decision FROM bot.ai_decision_ledger WHERE source = 'scrim.lagebild.generate' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(decision, "timeout");
    let error: Option<String> = sqlx::query_scalar(
        "SELECT error FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert!(error.is_some());
    assert_eq!(
        latest_scrim_ai_verdict(db.pool(), "lagebild_generate").await?,
        "timeout"
    );
    let run = sqlx::query(
        "SELECT state, last_error_code FROM scrim.ai_runs WHERE run_kind = 'lagebild_generate' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(run.get::<String, _>("state"), "failed");
    assert_eq!(
        run.get::<Option<String>, _>("last_error_code"),
        Some("err_ai_timeout".to_string())
    );
    Ok(())
}

#[tokio::test]
async fn lagebild_korrektur_persistiert_no_unsure_timeout_und_fehlerstatus(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;

    let no_provider = dl_ai::MockChatProvider::single(r#"{"reply":"Passt so","lagebild":null}"#);
    let no_receipt = correct_team_lagebild(
        db.pool(),
        Some(no_provider.as_ref()),
        None,
        1,
        "Passt so?",
        correction_actor(),
    )
    .await?;
    assert_eq!(no_receipt.verdict, "no");
    assert_eq!(latest_snapshot_status(db.pool()).await?, "ok");
    assert_eq!(
        latest_scrim_ai_verdict(db.pool(), "lagebild_correction").await?,
        "no"
    );

    let invalid_provider = dl_ai::MockChatProvider::single("kein json");
    let unsure_receipt = correct_team_lagebild(
        db.pool(),
        Some(invalid_provider.as_ref()),
        None,
        1,
        "Bitte korrigieren",
        correction_actor(),
    )
    .await?;
    assert_eq!(unsure_receipt.verdict, "unsure");
    assert_eq!(latest_snapshot_status(db.pool()).await?, "error");
    assert_eq!(
        latest_scrim_ai_verdict(db.pool(), "lagebild_correction").await?,
        "unsure"
    );
    let unsure_state: String = sqlx::query_scalar(
        "SELECT state FROM scrim.ai_runs WHERE run_kind = 'lagebild_correction' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(unsure_state, "uncertain");

    let timeout_provider = dl_ai::MockChatProvider::new(vec![Err(ChatProviderError::Timeout)]);
    let timeout_receipt = correct_team_lagebild(
        db.pool(),
        Some(timeout_provider.as_ref()),
        None,
        1,
        "Bitte nochmal",
        correction_actor(),
    )
    .await?;
    assert_eq!(timeout_receipt.verdict, "timeout");
    assert_eq!(latest_snapshot_status(db.pool()).await?, "error");
    assert_eq!(
        latest_scrim_ai_verdict(db.pool(), "lagebild_correction").await?,
        "timeout"
    );
    let timeout_error: Option<String> = sqlx::query_scalar(
        "SELECT last_error_code FROM scrim.ai_runs WHERE run_kind = 'lagebild_correction' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(timeout_error, Some("err_ai_timeout".to_string()));
    let correction_actor: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT author_user_id, author_display_name
           FROM scrim.lagebild_corrections
          WHERE role = 'user'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(
        correction_actor,
        (Some("42".to_string()), Some("Coach".to_string()))
    );
    Ok(())
}

#[tokio::test]
async fn lagebild_ai_ledger_schreibt_actor_nur_pseudonymisiert(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;

    let actor = correction_actor();
    refresh_team_lagebild(db.pool(), None, None, 1, actor.clone()).await?;
    assert_ledger_actor_private(db.pool(), "scrim.lagebild.refresh").await?;

    let provider = dl_ai::MockChatProvider::single(r#"{"reply":"Erledigt","lagebild":null}"#);
    correct_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        None,
        1,
        "Bitte korrigieren",
        actor,
    )
    .await?;
    assert_ledger_actor_private(db.pool(), "scrim.lagebild.correction").await?;

    let correction_actor: (Option<String>, Option<String>) = sqlx::query_as(
        "SELECT author_user_id, author_display_name
           FROM scrim.lagebild_corrections
          WHERE role = 'user'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(
        correction_actor,
        (Some("42".to_string()), Some("Coach".to_string()))
    );
    Ok(())
}

#[tokio::test]
async fn lagebild_correction_rohtext_bleibt_aus_append_only_decision_logs(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    let sensitive_marker = "DL_PRIVACY_MARKER_20260725_GEHEIM";
    let message = format!("Bitte intern korrigieren: {sensitive_marker}");
    let provider = dl_ai::MockChatProvider::single(r#"{"reply":"Gespeichert","lagebild":null}"#);

    correct_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        None,
        1,
        &message,
        correction_actor(),
    )
    .await?;

    let correction = sqlx::query(
        "SELECT id::bigint AS id, message
           FROM scrim.lagebild_corrections
          WHERE role = 'user'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    let user_correction_id = correction.get::<i64, _>("id");
    assert_eq!(correction.get::<String, _>("message"), message);

    let expected_summary = format!(
        "team=1 correction_ref=scrim.lagebild_corrections:{user_correction_id} chars={} hash={}",
        message.chars().count(),
        test_stable_short_hash(&message)
    );
    let ledger = sqlx::query(
        "SELECT input_summary, payload
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.correction'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    let ledger_summary = ledger.get::<String, _>("input_summary");
    let ledger_payload = ledger.get::<serde_json::Value, _>("payload");
    assert_eq!(ledger_summary, expected_summary);
    assert!(ledger_summary.contains(&format!(
        "correction_ref=scrim.lagebild_corrections:{user_correction_id}"
    )));
    assert!(ledger_summary.contains(&format!("hash={}", test_stable_short_hash(&message))));
    assert!(!ledger_summary.contains(sensitive_marker));
    assert!(!json_contains_substring(&ledger_payload, sensitive_marker));

    let decision_data: serde_json::Value = sqlx::query_scalar(
        "SELECT decision.decision_data
           FROM scrim.ai_decision_refs decision
           JOIN scrim.ai_runs run ON run.id = decision.run_id
          WHERE run.run_kind = 'lagebild_correction'
          ORDER BY decision.id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(
        decision_data
            .get("input_summary")
            .and_then(serde_json::Value::as_str),
        Some(expected_summary.as_str())
    );
    assert!(!json_contains_substring(&decision_data, sensitive_marker));
    Ok(())
}

#[tokio::test]
async fn lagebild_snapshot_wird_ohne_entscheidungslog_zurueckgerollt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    sqlx::query(
        "ALTER TABLE bot.ai_decision_ledger ADD CONSTRAINT reject_scrim_lagebild_test CHECK (source <> 'scrim.lagebild.generate')",
    )
    .execute(db.pool())
    .await?;
    let provider = dl_ai::MockChatProvider::single("Die Lage wirkt aktuell okay.");

    assert!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), None, 1)
            .await
            .is_err()
    );
    let snapshot_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM scrim.lagebild_snapshots WHERE team_id = 1")
            .fetch_one(db.pool())
            .await?;
    assert_eq!(snapshot_count, 0);
    Ok(())
}

#[tokio::test]
async fn teamkanal_chat_loest_ohne_strukturdaten_einen_ai_call_aus(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let mut bot_message = channel_message(9000, "Scrim-Bot", "Interner Bottext");
    bot_message.is_bot = true;
    let history = FakeChannelHistory::ok(vec![
        bot_message,
        channel_message(9001, "Orga", "Wir können am Donnerstag um 20 Uhr spielen."),
    ]);
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Das Team stimmt einen Termin für Donnerstag ab.","risiken":[],"naechster_schritt":"Den Termin für Donnerstag bestätigen.","prioritaet":"mittel"}"#,
    );

    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?,
        1
    );

    let requests = provider.requests();
    assert_eq!(requests.len(), 1);
    let prompt = requests[0]
        .0
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("Wir können am Donnerstag um 20 Uhr spielen."));
    assert!(prompt.contains("Orga"));
    assert!(!prompt.contains("Interner Bottext"));
    assert!(prompt.contains("nicht wörtlich"));
    Ok(())
}

#[tokio::test]
async fn nach_dem_abruf_entstandene_nachricht_wird_im_naechsten_lauf_gelesen(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let mut bereits_gelesen = channel_message(9001, "Orga", "Die Abstimmung für Donnerstag läuft.");
    bereits_gelesen.timestamp = chrono::Utc::now() - chrono::Duration::minutes(2);
    let nach_dem_abruf = channel_message(9002, "Orga", "Donnerstag um 20 Uhr passt.");
    let history = MessageAfterFetchHistory::new(bereits_gelesen, nach_dem_abruf);
    let provider = dl_ai::MockChatProvider::new(vec![
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Die Terminabstimmung läuft.","risiken":[],"naechster_schritt":"Rückmeldungen sammeln.","prioritaet":"mittel"}"#,
        )),
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Donnerstag um 20 Uhr passt.","risiken":[],"naechster_schritt":"Den Termin bestätigen.","prioritaet":"mittel"}"#,
        )),
    ]);

    refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;
    refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;

    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let second_prompt = requests[1]
        .0
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        second_prompt.contains("Donnerstag um 20 Uhr passt."),
        "Nachricht aus dem Fenster zwischen Abruf und Snapshot fehlt im zweiten Lauf"
    );
    Ok(())
}

#[tokio::test]
async fn leerer_erfolgreicher_abruf_speichert_einen_neuen_lesepunkt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    insert_terminabfrage(db.pool(), 1).await?;
    let history = FakeChannelHistory::ok(Vec::new());
    let provider = dl_ai::MockChatProvider::new(vec![
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Die Terminabfrage läuft.","risiken":[],"naechster_schritt":"Rückmeldungen sammeln.","prioritaet":"mittel"}"#,
        )),
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Die Terminabfrage läuft weiter.","risiken":[],"naechster_schritt":"Offene Rückmeldungen prüfen.","prioritaet":"mittel"}"#,
        )),
    ]);

    let first = refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;
    let first_summary: serde_json::Value = sqlx::query_scalar(
        "SELECT data_summary
           FROM scrim.lagebild_snapshots
          WHERE id = $1",
    )
    .bind(first.snapshot_id)
    .fetch_one(db.pool())
    .await?;
    let first_read_at = first_summary["channel_history_read_at"]
        .as_str()
        .ok_or("Lesepunkt fehlt nach leerem erfolgreichen Abruf")?
        .parse::<chrono::DateTime<chrono::Utc>>()?;
    refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;

    assert_eq!(history.since_calls(), vec![None, Some(first_read_at)]);
    Ok(())
}

#[tokio::test]
async fn ai_fehler_rueckt_den_teamkanal_wasserstand_nicht_vor(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let history = FakeChannelHistory::ok(vec![channel_message(
        9002,
        "Orga",
        "Wir können am Donnerstag um 20 Uhr spielen.",
    )]);
    let provider = dl_ai::MockChatProvider::new(vec![
        Err(ChatProviderError::Timeout),
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Das Team stimmt einen Termin für Donnerstag ab.","risiken":[],"naechster_schritt":"Den Termin für Donnerstag bestätigen.","prioritaet":"mittel"}"#,
        )),
    ]);

    let first = refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;
    let second = refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;

    assert_eq!(first.verdict, "timeout");
    assert_eq!(second.verdict, "yes");
    assert_eq!(history.since_calls(), vec![None, None]);
    Ok(())
}

#[tokio::test]
async fn teamkanal_abruf_fehler_rueckt_den_wasserstand_nicht_vor(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    insert_terminabfrage(db.pool(), 1).await?;
    let message = channel_message(9002, "Orga", "Wir können am Donnerstag um 20 Uhr spielen.");
    let failed_history = FakeChannelHistory::error("Discord REST antwortete mit Status 503");
    let loaded_history = FakeChannelHistory::ok(vec![message]);
    let provider = dl_ai::MockChatProvider::new(vec![
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Die Terminabfrage läuft.","risiken":[],"naechster_schritt":"Rückmeldungen sammeln.","prioritaet":"mittel"}"#,
        )),
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Das Team stimmt einen Termin für Donnerstag ab.","risiken":[],"naechster_schritt":"Den Termin für Donnerstag bestätigen.","prioritaet":"mittel"}"#,
        )),
    ]);

    let first = refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&failed_history),
        1,
        correction_actor(),
    )
    .await?;
    let first_snapshot = sqlx::query(
        "SELECT status, data_summary
           FROM scrim.lagebild_snapshots
          WHERE id = $1",
    )
    .bind(first.snapshot_id)
    .fetch_one(db.pool())
    .await?;
    assert_eq!(first_snapshot.get::<String, _>("status"), "ok");
    assert_eq!(
        first_snapshot.get::<serde_json::Value, _>("data_summary")["channel_history_state"],
        "failed"
    );
    let second = refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&loaded_history),
        1,
        correction_actor(),
    )
    .await?;

    assert_eq!(first.verdict, "yes");
    assert_eq!(second.verdict, "yes");
    assert_eq!(latest_snapshot_status(db.pool()).await?, "ok");
    assert_eq!(loaded_history.since_calls(), vec![None]);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    let second_prompt = requests[1]
        .0
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(second_prompt.contains("Wir können am Donnerstag um 20 Uhr spielen."));
    Ok(())
}

#[tokio::test]
async fn alter_snapshot_ohne_chat_ladezustand_rueckt_den_wasserstand_nicht_vor(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let history = FakeChannelHistory::ok(vec![channel_message(
        9002,
        "Orga",
        "Wir können am Donnerstag um 20 Uhr spielen.",
    )]);
    sqlx::query(
        r#"
        INSERT INTO scrim.lagebild_snapshots(
            team_id, generated_for, source, status, lagebild_text, data_summary
        )
        VALUES(1, 'weekly', 'ai', 'ok', 'Altes Lagebild', '{}'::jsonb)
        "#,
    )
    .execute(db.pool())
    .await?;
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Das Team stimmt einen Termin für Donnerstag ab.","risiken":[],"naechster_schritt":"Den Termin für Donnerstag bestätigen.","prioritaet":"mittel"}"#,
    );

    refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;

    assert_eq!(history.since_calls(), vec![None]);
    assert_eq!(provider.requests().len(), 1);
    Ok(())
}

#[tokio::test]
async fn nicht_angefragter_teamkanal_rueckt_den_wasserstand_nicht_vor(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    insert_terminabfrage(db.pool(), 1).await?;
    let provider = dl_ai::MockChatProvider::new(vec![
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Die Terminabfrage läuft.","risiken":[],"naechster_schritt":"Rückmeldungen sammeln.","prioritaet":"mittel"}"#,
        )),
        Ok(dl_ai::ChatResponse::text(
            r#"{"lage":"Das Team stimmt einen Termin für Donnerstag ab.","risiken":[],"naechster_schritt":"Den Termin für Donnerstag bestätigen.","prioritaet":"mittel"}"#,
        )),
    ]);

    let first = refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        None,
        1,
        correction_actor(),
    )
    .await?;
    let first_summary: serde_json::Value = sqlx::query_scalar(
        "SELECT data_summary
           FROM scrim.lagebild_snapshots
          WHERE id = $1",
    )
    .bind(first.snapshot_id)
    .fetch_one(db.pool())
    .await?;
    assert_eq!(first_summary["channel_history_state"], "not_requested");
    let history = FakeChannelHistory::ok(vec![channel_message(
        9002,
        "Orga",
        "Wir können am Donnerstag um 20 Uhr spielen.",
    )]);
    refresh_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;

    assert_eq!(history.since_calls(), vec![None]);
    assert_eq!(provider.requests().len(), 2);
    Ok(())
}

#[tokio::test]
async fn teamkanal_rohtext_und_autor_bleiben_aus_geschriebenen_spalten(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let raw_text = "DL_CHAT_PRIVACY_MARKER_20260728";
    let author = "DL_CHAT_AUTHOR_MARKER_20260728";
    let history = FakeChannelHistory::ok(vec![channel_message(9002, author, raw_text)]);
    let provider = dl_ai::MockChatProvider::single(format!(
        r#"{{"lage":"{raw_text}","risiken":[],"naechster_schritt":"{author} soll die Abstimmung abschließen.","prioritaet":"mittel"}}"#
    ));

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;
    assert_eq!(provider.requests().len(), 1);

    let written: String = sqlx::query_scalar(
        r#"
        SELECT concat_ws(
            E'\n',
            snapshot.lagebild_text,
            snapshot.data_summary::text,
            snapshot.error,
            COALESCE((
                SELECT string_agg(
                    concat_ws('|', evidence.label, evidence.url, evidence.reference_id, evidence.payload::text),
                    E'\n'
                )
                  FROM scrim.lagebild_evidences evidence
                 WHERE evidence.snapshot_id = snapshot.id
            ), ''),
            COALESCE((
                SELECT string_agg(concat_ws('|', ledger.input_summary, ledger.payload::text), E'\n')
                  FROM bot.ai_decision_ledger ledger
                 WHERE ledger.source LIKE 'scrim.lagebild.%'
            ), ''),
            COALESCE((
                SELECT string_agg(decision.decision_data::text, E'\n')
                  FROM scrim.ai_decision_refs decision
            ), '')
        )
          FROM scrim.lagebild_snapshots snapshot
         WHERE snapshot.team_id = 1
         ORDER BY snapshot.id DESC
         LIMIT 1
        "#,
    )
    .fetch_one(db.pool())
    .await?;
    assert!(!written.contains(raw_text));
    assert!(!written.contains(author));
    Ok(())
}

#[tokio::test]
async fn kurzes_alltagswort_aus_teamkanal_wird_nicht_abgelehnt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let history = FakeChannelHistory::ok(vec![channel_message(9002, "Zed", "passt")]);
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Der Termin passt.","risiken":[],"naechster_schritt":"Den Termin bestätigen.","prioritaet":"mittel"}"#,
    );

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;

    assert_eq!(latest_snapshot_status(db.pool()).await?, "ok");
    Ok(())
}

#[tokio::test]
async fn laengere_teamkanal_passage_in_ai_antwort_wird_abgelehnt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let raw_text = "Wir können am Donnerstag um 20 Uhr spielen.";
    let history = FakeChannelHistory::ok(vec![channel_message(9002, "Zed", raw_text)]);
    let provider = dl_ai::MockChatProvider::single(format!(
        r#"{{"lage":"{raw_text}","risiken":[],"naechster_schritt":"Den Termin bestätigen.","prioritaet":"mittel"}}"#
    ));

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;

    let reason: String = sqlx::query_scalar(
        "SELECT reason
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.generate'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(latest_snapshot_status(db.pool()).await?, "error");
    assert_eq!(reason, "ai_response_copied_chat");
    Ok(())
}

/// Der Schutz muss auch greifen, wenn die AI nur einen Ausschnitt einer langen
/// Nachricht übernimmt — nicht erst, wenn sie die ganze Nachricht kopiert.
#[tokio::test]
async fn teilzitat_aus_langer_teamkanal_nachricht_wird_abgelehnt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let raw_text = "Also ich muss diese Woche leider absagen weil meine Schicht verlegt wurde und ich erst spät zu Hause bin";
    let ausschnitt = "diese Woche leider absagen";
    assert!(raw_text.contains(ausschnitt));
    let history = FakeChannelHistory::ok(vec![channel_message(9002, "Zed", raw_text)]);
    let provider = dl_ai::MockChatProvider::single(format!(
        r#"{{"lage":"Ein Spieler schrieb {ausschnitt}.","risiken":[],"naechster_schritt":"Ersatz suchen.","prioritaet":"hoch"}}"#
    ));

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;

    let reason: String = sqlx::query_scalar(
        "SELECT reason
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.generate'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(latest_snapshot_status(db.pool()).await?, "error");
    assert_eq!(reason, "ai_response_copied_chat");
    Ok(())
}

#[tokio::test]
async fn kurzer_teamkanal_autorenname_in_ai_antwort_wird_abgelehnt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let history = FakeChannelHistory::ok(vec![channel_message(
        9002,
        "Max",
        "Wir stimmen den Termin ab.",
    )]);
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Max stimmt den Termin ab.","risiken":[],"naechster_schritt":"Den Termin bestätigen.","prioritaet":"mittel"}"#,
    );

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;

    let reason: String = sqlx::query_scalar(
        "SELECT reason
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.generate'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(latest_snapshot_status(db.pool()).await?, "error");
    assert_eq!(reason, "ai_response_copied_chat");
    Ok(())
}

#[tokio::test]
async fn kurzer_teamkanal_autorenname_als_teilstring_wird_nicht_abgelehnt(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let history = FakeChannelHistory::ok(vec![channel_message(
        9002,
        "Ari",
        "Wir stimmen den Termin ab.",
    )]);
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Die Planung bleibt variabel.","risiken":[],"naechster_schritt":"Den Termin bestätigen.","prioritaet":"mittel"}"#,
    );

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;

    assert_eq!(latest_snapshot_status(db.pool()).await?, "ok");
    Ok(())
}

#[tokio::test]
async fn ungueltiges_json_und_zitatschutz_haben_unterschiedliche_ledger_gruende(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES
             (1, 'Team 1', 100, now()),
             (2, 'Team 2', 200, now())",
    )
    .execute(db.pool())
    .await?;
    let raw_text = "Wir können am Donnerstag um 20 Uhr spielen.";
    let history = FakeChannelHistory::ok(vec![channel_message(9002, "Zed", raw_text)]);
    let invalid_provider = dl_ai::MockChatProvider::single("kein JSON");
    let copied_provider = dl_ai::MockChatProvider::single(format!(
        r#"{{"lage":"{raw_text}","risiken":[],"naechster_schritt":"Den Termin bestätigen.","prioritaet":"mittel"}}"#
    ));

    refresh_team_lagebild(
        db.pool(),
        Some(invalid_provider.as_ref()),
        Some(&history),
        1,
        correction_actor(),
    )
    .await?;
    refresh_team_lagebild(
        db.pool(),
        Some(copied_provider.as_ref()),
        Some(&history),
        2,
        correction_actor(),
    )
    .await?;

    let reasons: Vec<(i64, String)> = sqlx::query_as(
        "SELECT (payload ->> 'team_id')::bigint, reason
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.refresh'
          ORDER BY (payload ->> 'team_id')::bigint",
    )
    .fetch_all(db.pool())
    .await?;
    assert_eq!(
        reasons,
        vec![
            (1, "ai_response_invalid".to_string()),
            (2, "ai_response_copied_chat".to_string()),
        ]
    );
    Ok(())
}

#[tokio::test]
async fn teamkanal_abruf_error_bekommt_eine_eigene_ledger_entscheidung(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let history = FakeChannelHistory::error("Discord REST antwortete mit Status 503");
    let provider = dl_ai::MockChatProvider::single("darf nicht aufgerufen werden");

    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?,
        1
    );

    assert!(provider.requests().is_empty());
    let decision: Option<(String, String)> = sqlx::query_as(
        "SELECT decision, reason
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.channel_history'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_optional(db.pool())
    .await?;
    assert_eq!(
        decision,
        Some((
            "error".to_string(),
            "channel_history_fetch_failed".to_string()
        ))
    );
    Ok(())
}

#[tokio::test]
async fn teamkanal_deckelung_auf_200_nachrichten_ist_im_ledger_sichtbar(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query(
        "INSERT INTO scrim.teams(id, name, discord_channel_id, created_at)
         VALUES(1, 'Team 1', 100, now())",
    )
    .execute(db.pool())
    .await?;
    let messages = (1..=200)
        .map(|id| channel_message(id, "Orga", &format!("Abstimmung {id}")))
        .collect();
    let history = FakeChannelHistory::truncated(messages);
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Im Teamkanal laufen mehrere Abstimmungen.","risiken":[],"naechster_schritt":"Die jüngste Abstimmung abschließen.","prioritaet":"mittel"}"#,
    );

    generate_due_lagebilder(db.pool(), Some(provider.as_ref()), Some(&history), 1).await?;

    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT payload
           FROM bot.ai_decision_ledger
          WHERE source = 'scrim.lagebild.generate'
          ORDER BY id DESC
          LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(payload["channel_history_truncated"], true);
    let request = provider.requests().pop().ok_or("missing AI request")?;
    let prompt = request
        .0
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt.contains("auf 200 Nachrichten begrenzt"));
    Ok(())
}

#[derive(Clone)]
struct FakeChannelHistory {
    response: Result<ChannelHistoryBatch, ChannelHistoryError>,
    since_calls: Arc<Mutex<Vec<Option<chrono::DateTime<chrono::Utc>>>>>,
}

impl FakeChannelHistory {
    fn ok(messages: Vec<ChannelHistoryMessage>) -> Self {
        Self {
            response: Ok(ChannelHistoryBatch {
                messages,
                truncated: false,
            }),
            since_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn truncated(messages: Vec<ChannelHistoryMessage>) -> Self {
        Self {
            response: Ok(ChannelHistoryBatch {
                messages,
                truncated: true,
            }),
            since_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn error(message: &str) -> Self {
        Self {
            response: Err(ChannelHistoryError(message.to_string())),
            since_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn since_calls(&self) -> Vec<Option<chrono::DateTime<chrono::Utc>>> {
        self.since_calls.lock().expect("since calls").clone()
    }
}

#[async_trait::async_trait]
impl ChannelHistory for FakeChannelHistory {
    async fn recent_messages(
        &self,
        _channel_id: u64,
        since: Option<chrono::DateTime<chrono::Utc>>,
        _limit: usize,
    ) -> Result<ChannelHistoryBatch, ChannelHistoryError> {
        self.since_calls.lock().expect("since calls").push(since);
        self.response.clone().map(|mut batch| {
            if let Some(since) = since {
                batch.messages.retain(|message| message.timestamp > since);
            }
            batch
        })
    }
}

#[derive(Clone)]
struct MessageAfterFetchHistory {
    first_message: ChannelHistoryMessage,
    late_message: Arc<Mutex<ChannelHistoryMessage>>,
    call_count: Arc<Mutex<usize>>,
}

impl MessageAfterFetchHistory {
    fn new(first_message: ChannelHistoryMessage, late_message: ChannelHistoryMessage) -> Self {
        Self {
            first_message,
            late_message: Arc::new(Mutex::new(late_message)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }
}

#[async_trait::async_trait]
impl ChannelHistory for MessageAfterFetchHistory {
    async fn recent_messages(
        &self,
        _channel_id: u64,
        since: Option<chrono::DateTime<chrono::Utc>>,
        _limit: usize,
    ) -> Result<ChannelHistoryBatch, ChannelHistoryError> {
        let mut call_count = self.call_count.lock().expect("call count");
        let first_call = *call_count == 0;
        *call_count += 1;
        drop(call_count);
        let mut messages = vec![self.first_message.clone()];
        if first_call {
            // Der erste Abruf ist zusammengestellt; die Nachricht entsteht erst danach.
            self.late_message.lock().expect("late message").timestamp = chrono::Utc::now();
        } else {
            messages.push(self.late_message.lock().expect("late message").clone());
        }
        if let Some(since) = since {
            messages.retain(|message| message.timestamp > since);
        }
        Ok(ChannelHistoryBatch {
            messages,
            truncated: false,
        })
    }
}

fn channel_message(id: u64, author: &str, content: &str) -> ChannelHistoryMessage {
    ChannelHistoryMessage {
        id,
        timestamp: chrono::Utc::now(),
        author_display_name: author.to_string(),
        content: content.to_string(),
        is_bot: false,
    }
}

async fn latest_scrim_ai_verdict(
    pool: &sqlx::PgPool,
    run_kind: &str,
) -> Result<String, sqlx::Error> {
    sqlx::query_scalar(
        r#"
        SELECT decision_data ->> 'verdict'
          FROM scrim.ai_decision_refs decision
          JOIN scrim.ai_runs run ON run.id = decision.run_id
         WHERE run.run_kind = $1
         ORDER BY decision.id DESC
         LIMIT 1
        "#,
    )
    .bind(run_kind)
    .fetch_one(pool)
    .await
}

async fn latest_snapshot_status(pool: &sqlx::PgPool) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM scrim.lagebild_snapshots ORDER BY id DESC LIMIT 1")
        .fetch_one(pool)
        .await
}

async fn assert_ledger_actor_private(
    pool: &sqlx::PgPool,
    source: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let row = sqlx::query(
        "SELECT input_summary, payload
           FROM bot.ai_decision_ledger
          WHERE source = $1
          ORDER BY id DESC
          LIMIT 1",
    )
    .bind(source)
    .fetch_one(pool)
    .await?;
    let input_summary = row.get::<String, _>("input_summary");
    let payload = row.get::<serde_json::Value, _>("payload");
    assert!(!input_summary.contains("42"));
    assert!(!input_summary.contains("Coach"));
    assert!(payload.get("actor_discord_id").is_none());
    assert!(payload.get("actor_display_name").is_none());
    assert!(!json_contains_string(&payload, "42"));
    assert!(!json_contains_string(&payload, "Coach"));
    let actor_pseudonym = payload
        .get("actor_pseudonym")
        .and_then(serde_json::Value::as_str)
        .ok_or("actor_pseudonym fehlt")?;
    assert!(actor_pseudonym.starts_with("act_"));
    let expected_pseudonym: String = sqlx::query_scalar(
        "SELECT actor_pseudonym
           FROM scrim.audit_actor_pseudonyms
          WHERE actor_type = 'user'
            AND actor_ref = '42'",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(actor_pseudonym, expected_pseudonym);
    Ok(())
}

fn json_contains_string(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value == needle,
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_string(value, needle)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| json_contains_string(value, needle)),
        _ => false,
    }
}

fn json_contains_substring(value: &serde_json::Value, needle: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value.contains(needle),
        serde_json::Value::Array(values) => values
            .iter()
            .any(|value| json_contains_substring(value, needle)),
        serde_json::Value::Object(values) => values
            .values()
            .any(|value| json_contains_substring(value, needle)),
        _ => false,
    }
}

fn test_stable_short_hash(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    hex::encode(&digest[..8])
}

fn correction_actor() -> CorrectionActor {
    CorrectionActor {
        author_user_id: "42".to_string(),
        author_display_name: "Coach".to_string(),
        request_id: "bff:lagebild-test".to_string(),
        idempotency_key: "bff:lagebild-test-idem".to_string(),
    }
}

async fn insert_terminabfrage(pool: &sqlx::PgPool, team_id: i32) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO scrim.match_request_batches(
            id, template, deadline_at, status, created_by_user_id, created_by_display_name
        )
        VALUES($1, 'regular_scrim', now() + interval '2 days', 'open', '42', 'Coach')
        ON CONFLICT (id) DO NOTHING
        "#,
    )
    .bind(team_id)
    .execute(pool)
    .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.match_requests(id, batch_id, team_a_id, status, slot_options)
        VALUES($1, $1, $1, 'open', '[]'::jsonb)
        "#,
    )
    .bind(team_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn input_ohne_operative_daten() -> ScrimLagebildInput {
    ScrimLagebildInput {
        team_id: 7,
        team_name: "Team 7".to_string(),
        generated_for: "weekly".to_string(),
        data_limited: true,
        has_operational_data: false,
        facts: vec!["Team 7 hat 6 bekannte Mitglieder.".to_string()],
        corrections: Vec::new(),
        evidences: Vec::new(),
    }
}

#[test]
fn ohne_operative_daten_wird_die_ai_gar_nicht_erst_gefragt() {
    let plan = plan_lagebild(&input_ohne_operative_daten());

    let LagebildPlan::Deterministisch(report) = plan else {
        panic!("ohne Terminabfrage, Reminder und Match darf kein AI-Call geplant werden");
    };
    assert_eq!(report.prioritaet, LagebildPrioritaet::Mittel);
    assert!(report.naechster_schritt.contains("Terminabfrage"));
    assert!(report.lage.contains("keine"));
}

#[test]
fn mit_operativen_daten_fragt_das_lagebild_die_ai() {
    let mut input = input_ohne_operative_daten();
    input.has_operational_data = true;

    assert!(matches!(plan_lagebild(&input), LagebildPlan::AiFragen));
}

#[test]
fn report_wird_als_kurze_karte_ohne_markdown_gerendert() {
    let report = LagebildReport {
        lage: "**Team 7** trainiert regelmässig.".to_string(),
        risiken: vec![
            "Zwei Stammspieler haben nicht geantwortet.".to_string(),
            "*Ersatz* fehlt.".to_string(),
        ],
        naechster_schritt: "Fehlende Antworten erinnern.".to_string(),
        prioritaet: LagebildPrioritaet::Hoch,
    };
    let evidence = ScrimLagebildEvidence::discord_message("Terminabfrage 5", 100, 9001, None);

    let text = render_lagebild_report(&report, &[evidence]);

    assert!(!text.contains('*'));
    assert!(text.starts_with("Lage: Team 7 trainiert regelmässig."));
    assert!(
        text.contains("Risiken:\n- Zwei Stammspieler haben nicht geantwortet.\n- Ersatz fehlt.")
    );
    assert!(text.contains("Nächster Schritt: Fehlende Antworten erinnern."));
    assert!(text.contains("Priorität: hoch"));
    assert!(text.ends_with(
        "Evidenzen:\n- [Terminabfrage 5](https://discord.com/channels/1289721245281292288/100/9001)"
    ));
}

#[test]
fn prompt_verlangt_ein_json_report_mit_genau_einem_naechsten_schritt() {
    let mut input = input_ohne_operative_daten();
    input.has_operational_data = true;

    let joined = lagebild_messages(&input)
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(joined.contains("naechster_schritt"));
    assert!(joined.contains("prioritaet"));
    assert!(joined.contains("risiken"));
    assert!(joined.contains("Keine Markdown"));
    assert!(joined.contains("Keine DMs"));
}

#[tokio::test]
async fn ai_report_wird_geparst_gerendert_und_als_json_angefordert(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = input_ohne_operative_daten();
    input.has_operational_data = true;
    let provider = dl_ai::MockChatProvider::single(
        r#"{"lage":"Team 7 hat zwei offene Abfragen.","risiken":["Frist laeuft heute ab."],"naechster_schritt":"Fehlende Antworten erinnern.","prioritaet":"hoch"}"#,
    );

    let outcome = generate_lagebild(provider.as_ref(), &input).await?;

    assert_eq!(outcome.report.prioritaet, LagebildPrioritaet::Hoch);
    assert_eq!(
        outcome.report.naechster_schritt,
        "Fehlende Antworten erinnern."
    );
    assert!(outcome
        .text
        .starts_with("Lage: Team 7 hat zwei offene Abfragen."));
    let request = provider.requests().pop().ok_or("missing AI request")?;
    assert!(request.1.json_mode);
    Ok(())
}

#[tokio::test]
async fn freitext_statt_report_gilt_als_unklare_ai_antwort(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut input = input_ohne_operative_daten();
    input.has_operational_data = true;
    let provider = dl_ai::MockChatProvider::single("**Lagebild Team 7** Die Lage wirkt okay.");

    let error = generate_lagebild(provider.as_ref(), &input)
        .await
        .expect_err("Freitext darf kein gueltiges Lagebild sein");

    assert!(matches!(error, LagebildError::InvalidAi(_)));
    Ok(())
}

#[tokio::test]
async fn korrektur_ohne_naechsten_schritt_gilt_als_unklare_antwort(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    let provider = dl_ai::MockChatProvider::single(
        r#"{"reply":"Angepasst.","lagebild":{"lage":"Neue Lage.","risiken":[],"naechster_schritt":"   ","prioritaet":"mittel"}}"#,
    );

    let receipt = correct_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        None,
        1,
        "Bitte korrigieren",
        correction_actor(),
    )
    .await?;

    assert_eq!(receipt.verdict, "unsure");
    assert_eq!(latest_snapshot_status(db.pool()).await?, "error");
    Ok(())
}

#[tokio::test]
async fn korrektur_ohne_ueberarbeitung_laesst_altes_format_faellig(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.lagebild_snapshots(
            team_id, generated_at, generated_for, source, status, lagebild_text, data_summary
        )
        VALUES(1, now(), 'weekly', 'ai', 'ok', '**Altes Lagebild**', '{}'::jsonb)
        "#,
    )
    .execute(db.pool())
    .await?;
    let provider = dl_ai::MockChatProvider::single(r#"{"reply":"Passt so","lagebild":null}"#);

    correct_team_lagebild(
        db.pool(),
        Some(provider.as_ref()),
        None,
        1,
        "Passt das?",
        correction_actor(),
    )
    .await?;

    // Der alte Text steht weiter drin, also muss der Snapshot fällig bleiben.
    assert_eq!(generate_due_lagebilder(db.pool(), None, None, 5).await?, 1);
    Ok(())
}

#[tokio::test]
async fn lagebild_im_alten_format_wird_sofort_neu_erzeugt() -> Result<(), Box<dyn std::error::Error>>
{
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.lagebild_snapshots(
            team_id, generated_at, generated_for, source, status, lagebild_text, data_summary
        )
        VALUES(1, now(), 'weekly', 'ai', 'ok', '**Lagebild Team 1** Die Lage wirkt okay.', '{}'::jsonb)
        "#,
    )
    .execute(db.pool())
    .await?;

    assert_eq!(generate_due_lagebilder(db.pool(), None, None, 5).await?, 1);
    assert_eq!(generate_due_lagebilder(db.pool(), None, None, 5).await?, 0);
    let text: String = sqlx::query_scalar(
        "SELECT lagebild_text FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert!(!text.contains('*'));
    assert!(text.contains("Nächster Schritt: "));
    Ok(())
}

#[tokio::test]
async fn team_ohne_operative_daten_bekommt_snapshot_ohne_ai_und_sichtbare_entscheidung(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'Team 1', now())")
        .execute(db.pool())
        .await?;
    let provider = dl_ai::MockChatProvider::single("darf nicht aufgerufen werden");

    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), None, 1).await?,
        1
    );

    assert!(provider.requests().is_empty());
    let snapshot = sqlx::query(
        "SELECT status, lagebild_text, model FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(snapshot.get::<String, _>("status"), "ok");
    assert_eq!(snapshot.get::<Option<String>, _>("model"), None);
    let text = snapshot.get::<String, _>("lagebild_text");
    assert!(text.contains("Nächster Schritt: "));
    assert!(text.contains("Terminabfrage"));
    let decision = sqlx::query(
        "SELECT decision, reason FROM bot.ai_decision_ledger WHERE source = 'scrim.lagebild.generate' ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(decision.get::<String, _>("decision"), "no");
    assert_eq!(
        decision.get::<String, _>("reason"),
        "keine_operativen_daten"
    );
    let summary: serde_json::Value = sqlx::query_scalar(
        "SELECT data_summary FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(summary["prioritaet"], "mittel");
    assert!(summary["naechster_schritt"]
        .as_str()
        .unwrap_or_default()
        .contains("Terminabfrage"));
    Ok(())
}
