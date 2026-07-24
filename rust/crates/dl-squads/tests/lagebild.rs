use dl_ai::ChatProviderError;
use dl_squads::lagebild::{
    finalize_lagebild_text, generate_due_lagebilder, lagebild_messages, ScrimLagebildEvidence,
    ScrimLagebildInput,
};
use sqlx::Row;

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

    assert_eq!(generate_due_lagebilder(db.pool(), None, 1).await?, 1);
    assert_eq!(generate_due_lagebilder(db.pool(), None, 1).await?, 0);
    sqlx::query(
        "UPDATE scrim.lagebild_snapshots SET generated_at = now() - interval '16 minutes' WHERE team_id = 1",
    )
    .execute(db.pool())
    .await?;
    assert_eq!(generate_due_lagebilder(db.pool(), None, 1).await?, 1);
    Ok(())
}

#[tokio::test]
async fn lagebild_match_history_enthaelt_nur_abgeschlossene_oder_abgerufene_matches(
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
    sqlx::query("UPDATE scrim.matches SET result_json = '{}'::jsonb WHERE id = 12")
        .execute(db.pool())
        .await?;
    sqlx::query(
        r#"
        INSERT INTO scrim.match_result_refs(
            match_id, steam_match_id, source_user_id, source_display_name,
            fetch_status, entered_at, updated_at
        )
        VALUES(12, 222, '42', 'Coach', 'pending', now(), now())
        "#,
    )
    .execute(db.pool())
    .await?;

    let provider = dl_ai::MockChatProvider::single("Die Lage ist nachvollziehbar.");
    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), 1).await?,
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
    assert!(!prompt.contains("Match 10:"));
    assert!(!prompt.contains("Match 12:"));

    let snapshot = sqlx::query(
        "SELECT status, error FROM scrim.lagebild_snapshots WHERE team_id = 1 ORDER BY id DESC LIMIT 1",
    )
    .fetch_one(db.pool())
    .await?;
    assert_eq!(snapshot.get::<String, _>("status"), "ok");
    assert_eq!(snapshot.get::<Option<String>, _>("error"), None);
    Ok(())
}

#[tokio::test]
async fn lagebild_timeout_bleibt_im_ledger_und_snapshot_sichtbar(
) -> Result<(), Box<dyn std::error::Error>> {
    let db = dl_central_db::testing::test_pool().await?;
    sqlx::query("INSERT INTO scrim.teams(id, name, created_at) VALUES(1, 'A', now())")
        .execute(db.pool())
        .await?;
    let provider = dl_ai::MockChatProvider::new(vec![Err(ChatProviderError::Timeout)]);

    assert_eq!(
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), 1).await?,
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
        generate_due_lagebilder(db.pool(), Some(provider.as_ref()), 1)
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
