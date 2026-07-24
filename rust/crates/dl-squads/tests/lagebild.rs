use dl_squads::lagebild::{
    finalize_lagebild_text, lagebild_messages, ScrimLagebildEvidence, ScrimLagebildInput,
};

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
