use super::*;

#[test]
fn login_aus_name_handle_und_links() {
    let cases = [
        ("dach_lock", "dach_lock"),
        ("  Dach_Lock  ", "dach_lock"),
        ("@SomeStreamer", "somestreamer"),
        ("twitch.tv/someone", "someone"),
        ("https://www.twitch.tv/SomeOne", "someone"),
        ("http://twitch.tv/someone/videos", "someone"),
        ("HTTPS://M.TWITCH.TV/someone?sr=a", "someone"),
        ("www.twitch.tv/someone#chat", "someone"),
        ("https://twitch.tv/someone/", "someone"),
    ];
    for (raw, expected) in cases {
        assert_eq!(parse_twitch_login(raw).as_deref(), Some(expected), "{raw}");
    }
}

#[test]
fn ungueltige_eingaben_sind_kein_kanal() {
    for raw in [
        "",
        "   ",
        "@",
        "twitch.tv/",
        "https://twitch.tv/",
        "https://twitch.tv/directory/game/Deadlock",
        "https://www.twitch.tv/videos/123",
        "https://clips.twitch.tv/SomeClipSlug",
        "https://youtube.com/@someone",
        "someone.tv",
        "name mit leerzeichen",
        "ümlaut",
        "a_very_long_login_name_over_25",
        "javascript:alert(1)",
    ] {
        assert_eq!(parse_twitch_login(raw), None, "{raw}");
    }
}

#[test]
fn grund_wird_bereinigt_und_begrenzt() {
    assert_eq!(
        normalize_reason("  spielt jeden Abend\nDeadlock  ").as_deref(),
        Some("spielt jeden Abend Deadlock")
    );
    assert_eq!(normalize_reason(""), None);
    assert_eq!(normalize_reason("  a "), None);
    let long = "ä".repeat(MAX_REASON_CHARS + 50);
    assert_eq!(
        normalize_reason(&long).map(|r| r.chars().count()),
        Some(MAX_REASON_CHARS)
    );
}

#[test]
fn idempotency_key_folgt_dem_vertrag() {
    assert_eq!(idempotency_key(42), "discord-suggest-42");
    let request = ForwardRequest {
        id: 7,
        discord_id: 123_456_789_012_345_678,
        twitch_login: "someone".into(),
        submitted_at: chrono::DateTime::parse_from_rfc3339("2026-10-03T10:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
        privacy_epoch: 0,
        reason: "passt".into(),
    };
    assert_eq!(
        request.payload(),
        json!({
            "twitch_login": "someone",
            "suggested_by_discord_id": "123456789012345678",
            "reason": "passt",
            "idempotency_key": "discord-suggest-7",
            "submitted_at": "2026-10-03T10:00:00+00:00",
            "privacy_epoch": 0,
        })
    );
}

#[test]
fn antworten_des_twitch_bots_werden_eingeordnet() {
    for (status, expected) in [
        ("created", SuggestionStatus::Created),
        ("already_known", SuggestionStatus::AlreadyKnown),
        ("already_partner", SuggestionStatus::AlreadyPartner),
        ("blocked", SuggestionStatus::Blocked),
        ("not_found", SuggestionStatus::NotFound),
    ] {
        assert_eq!(
            classify_response(200, &json!({"status": status, "twitch_user_id": "123"})),
            ForwardResult::Answered {
                status: expected,
                twitch_user_id: Some("123".into())
            }
        );
    }
    assert_eq!(
        classify_response(200, &json!({"status": "not_found", "twitch_user_id": null})),
        ForwardResult::Answered {
            status: SuggestionStatus::NotFound,
            twitch_user_id: None
        }
    );
    assert_eq!(
        classify_response(200, &json!({"status": "created", "twitch_user_id": "0x1"})),
        ForwardResult::Answered {
            status: SuggestionStatus::Created,
            twitch_user_id: None
        }
    );
    for body in [json!({"status": "pending"}), json!({}), Value::Null] {
        assert!(matches!(
            classify_response(200, &body),
            ForwardResult::Retry(_)
        ));
    }
    for code in [400, 409, 422] {
        assert!(matches!(
            classify_response(code, &json!({"error": "bad_request"})),
            ForwardResult::Permanent(_)
        ));
    }
    for code in [401, 403, 404, 429, 500, 503] {
        assert!(matches!(
            classify_response(code, &Value::Null),
            ForwardResult::Retry(_)
        ));
    }
}

fn all_texts() -> Vec<String> {
    let mut texts: Vec<String> = [
        SuggestionStatus::Pending,
        SuggestionStatus::Created,
        SuggestionStatus::AlreadyKnown,
        SuggestionStatus::AlreadyPartner,
        SuggestionStatus::Blocked,
        SuggestionStatus::NotFound,
        SuggestionStatus::Rejected,
    ]
    .into_iter()
    .map(|status| status_text(status, "some_one"))
    .collect();
    for outcome in [
        SubmitOutcome::InvalidChannel,
        SubmitOutcome::ReasonMissing,
        SubmitOutcome::OptedOut,
        SubmitOutcome::RateLimited,
        SubmitOutcome::Failed,
        SubmitOutcome::AlreadyByYou {
            login: "x".into(),
            status: SuggestionStatus::Created,
        },
    ] {
        texts.push(outcome_text(&outcome));
    }
    texts.push(PANEL_HINT.to_string());
    texts
}

#[test]
fn texte_in_nutzersprache_ohne_gedankenstriche() {
    for text in all_texts() {
        assert!(!text.contains('–') && !text.contains('—'), "{text}");
        assert!(!text.contains(" - "), "{text}");
        for fachwort in [
            "Scout", "Kandidat", "Status", "API", "Token", "Login", "Fehler",
        ] {
            assert!(!text.contains(fachwort), "{fachwort} in {text}");
        }
        assert!(
            !text.contains("ae ") && !text.contains("ue ") && !text.contains("oe "),
            "{text}"
        );
        assert!(text.encode_utf16().count() <= 2000);
    }
}

#[test]
fn jeder_status_hat_seine_antwort() {
    let created = status_text(SuggestionStatus::Created, "some_one");
    assert!(created.starts_with("Danke"));
    assert!(created.contains("schauen uns den Kanal **some\\_one** an"));
    assert!(created.contains("150 Punkte"));
    let pending = status_text(SuggestionStatus::Pending, "x");
    assert!(pending.contains("schauen uns den Kanal"));
    assert!(!pending.contains("Punkte"));
    assert!(status_text(SuggestionStatus::AlreadyKnown, "x").contains("kennen wir schon"));
    assert!(status_text(SuggestionStatus::AlreadyPartner, "x").contains("schon Partner"));
    assert!(status_text(SuggestionStatus::NotFound, "x").contains("finden wir nicht"));
    let blocked = status_text(SuggestionStatus::Blocked, "geheim");
    assert_eq!(blocked, TEXT_BLOCKED);
    assert!(!blocked.contains("geheim"), "blocked ohne Details");
    // Eigener Doppelvorschlag zeigt endgültige Stände, sonst einen Dank.
    assert_eq!(
        outcome_text(&SubmitOutcome::AlreadyByYou {
            login: "x".into(),
            status: SuggestionStatus::NotFound
        }),
        status_text(SuggestionStatus::NotFound, "x")
    );
    assert!(outcome_text(&SubmitOutcome::AlreadyByYou {
        login: "x".into(),
        status: SuggestionStatus::Pending
    })
    .contains("hast du schon vorgeschlagen"));
    assert!(TEXT_RATE_LIMITED.contains(&DAILY_LIMIT.to_string()));
}

#[test]
fn modal_und_button_halten_discord_grenzen() {
    let spec = modal();
    assert_eq!(spec.custom_id, MODAL_CUSTOM_ID);
    assert!(spec.title.chars().count() <= 45);
    assert_eq!(spec.fields.len(), 2);
    for field in &spec.fields {
        assert!(field.label.chars().count() <= 45, "{}", field.label);
        assert!(field.placeholder.chars().count() <= 100);
        assert!(field.required);
    }
    assert_eq!(spec.fields[0].custom_id, FIELD_CHANNEL);
    assert_eq!(spec.fields[1].custom_id, FIELD_REASON);
    assert_eq!(usize::from(spec.fields[1].max_length), MAX_REASON_CHARS);
    let button = panel_button();
    assert_eq!(button["custom_id"], OPEN_CUSTOM_ID);
    assert_eq!(button["label"], BUTTON_LABEL);
}

#[test]
fn clip_panel_traegt_den_knopf() {
    let components = crate::clips::interface_components();
    let buttons = components[0]["components"].as_array().expect("buttons");
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[0]["custom_id"], "clip_submit_btn_v1");
    assert_eq!(buttons[1], panel_button());
    assert_eq!(PANEL_CHANNEL_ID, crate::clips::SUBMIT_CHANNEL_ID);
}

#[test]
fn kanal_passt_zum_concierge() {
    assert_eq!(
        crate::concierge_community::STREAMER_SUGGEST_CHANNEL_ID,
        PANEL_CHANNEL_ID
    );
    assert!(crate::concierge_community::ANSWER_STREAMER_VORSCHLAGEN
        .contains(&format!("<#{PANEL_CHANNEL_ID}>")));
}

#[tokio::test]
async fn button_oeffnet_modal_und_ist_registriert() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://postgres@localhost/streamer_suggest_test")
        .expect("lazy pool");
    let service = StreamerSuggestions::new(pool, None);
    let mut router = InteractionRouter::new();
    register(&mut router, service.clone());
    assert!(router.resolve_component(OPEN_CUSTOM_ID).is_some());
    assert!(router.resolve_component(MODAL_CUSTOM_ID).is_some());
    let handler = SuggestHandler { service };
    let reply = handler
        .handle(BridgeInteraction {
            custom_id: OPEN_CUSTOM_ID.into(),
            user_id: 42,
            ..BridgeInteraction::default()
        })
        .await;
    assert_eq!(
        reply.modal.map(|m| m.custom_id).as_deref(),
        Some(MODAL_CUSTOM_ID)
    );
    // Ungültige Eingabe wird vor jedem DB-Zugriff beantwortet.
    let mut options = std::collections::HashMap::new();
    options.insert(FIELD_CHANNEL.to_string(), json!("https://youtube.com/x"));
    options.insert(FIELD_REASON.to_string(), json!("passt gut"));
    let reply = handler
        .handle(BridgeInteraction {
            custom_id: MODAL_CUSTOM_ID.into(),
            user_id: 42,
            options,
            ..BridgeInteraction::default()
        })
        .await;
    assert!(reply.ephemeral);
    assert_eq!(reply.content.as_deref(), Some(TEXT_INVALID_CHANNEL));
}

#[cfg(feature = "testing")]
mod db {
    use super::*;
    use std::sync::Mutex;

    const MEMBER: u64 = 123_456_789_012_345_678;

    /// Antwortet der Reihe nach mit den hinterlegten Ergebnissen, danach `created`.
    #[derive(Default)]
    struct FakeForwarder {
        script: Mutex<Vec<ForwardResult>>,
        seen: Mutex<Vec<ForwardRequest>>,
    }

    impl FakeForwarder {
        fn with(script: Vec<ForwardResult>) -> Arc<Self> {
            Arc::new(Self {
                script: Mutex::new(script),
                seen: Mutex::new(Vec::new()),
            })
        }
        fn seen(&self) -> Vec<ForwardRequest> {
            self.seen.lock().expect("lock").clone()
        }
    }

    #[async_trait]
    impl SuggestionForwarder for FakeForwarder {
        async fn forward(&self, request: &ForwardRequest) -> ForwardResult {
            self.seen.lock().expect("lock").push(request.clone());
            let mut script = self.script.lock().expect("lock");
            if script.is_empty() {
                ForwardResult::Answered {
                    status: SuggestionStatus::Created,
                    twitch_user_id: Some("456".into()),
                }
            } else {
                script.remove(0)
            }
        }
    }

    async fn setup(
        forwarder: Option<Arc<FakeForwarder>>,
    ) -> (dl_central_db::testing::TestDb, Arc<StreamerSuggestions>) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let service = StreamerSuggestions::new(
            db.pool().clone(),
            forwarder.map(|f| f as Arc<dyn SuggestionForwarder>),
        );
        (db, service)
    }

    async fn row(pool: &PgPool, id: i64) -> (String, Option<String>, bool, i32) {
        let row = sqlx::query(
            "SELECT status, twitch_user_id, forwarded_at IS NOT NULL AS forwarded, forward_attempts
               FROM community.streamer_suggestions WHERE id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("row");
        (
            row.get("status"),
            row.get("twitch_user_id"),
            row.get("forwarded"),
            row.get("forward_attempts"),
        )
    }

    fn saved_id(outcome: &SubmitOutcome) -> i64 {
        match outcome {
            SubmitOutcome::Saved { id, .. } => *id,
            other => panic!("erwartet Saved, bekam {other:?}"),
        }
    }

    #[tokio::test]
    async fn vorschlag_wird_gespeichert_und_weitergegeben() {
        let forwarder = FakeForwarder::with(Vec::new());
        let (db, service) = setup(Some(forwarder.clone())).await;
        let outcome = service
            .submit(
                MEMBER,
                "https://www.twitch.tv/Some_One",
                "  spielt jeden Abend  ",
            )
            .await;
        let id = saved_id(&outcome);
        assert_eq!(
            outcome,
            SubmitOutcome::Saved {
                id,
                login: "some_one".into(),
                status: SuggestionStatus::Created
            }
        );
        let seen = forwarder.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].twitch_login, "some_one");
        assert_eq!(seen[0].reason, "spielt jeden Abend");
        assert_eq!(
            seen[0].payload()["idempotency_key"],
            format!("discord-suggest-{id}")
        );
        assert_eq!(
            row(db.pool(), id).await,
            ("created".into(), Some("456".into()), true, 1)
        );
        // Abgeschlossenes wird nicht erneut gesendet.
        sqlx::query(
            "UPDATE community.streamer_suggestions SET last_attempt_at = now() - interval '1 day'",
        )
        .execute(db.pool())
        .await
        .expect("age");
        assert_eq!(service.forward_pending().await.expect("retry"), 0);
        assert_eq!(forwarder.seen().len(), 1);
    }

    #[tokio::test]
    async fn ein_vorschlag_je_mitglied_und_kanal() {
        let forwarder = FakeForwarder::with(vec![ForwardResult::Answered {
            status: SuggestionStatus::NotFound,
            twitch_user_id: None,
        }]);
        let (db, service) = setup(Some(forwarder.clone())).await;
        let first = service.submit(MEMBER, "ghost", "passt gut").await;
        assert!(matches!(
            first,
            SubmitOutcome::Saved {
                status: SuggestionStatus::NotFound,
                ..
            }
        ));
        let again = service.submit(MEMBER, "twitch.tv/GHOST", "nochmal").await;
        assert_eq!(
            again,
            SubmitOutcome::AlreadyByYou {
                login: "ghost".into(),
                status: SuggestionStatus::NotFound
            }
        );
        assert_eq!(forwarder.seen().len(), 1);
        // Ein anderes Mitglied darf denselben Kanal vorschlagen.
        let other = service.submit(MEMBER + 1, "ghost", "passt gut").await;
        assert!(matches!(other, SubmitOutcome::Saved { .. }));
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM community.streamer_suggestions")
            .fetch_one(db.pool())
            .await
            .expect("count");
        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn hoechstens_drei_vorschlaege_in_24_stunden() {
        let (db, service) = setup(Some(FakeForwarder::with(Vec::new()))).await;
        for login in ["eins", "zwei", "drei"] {
            assert!(matches!(
                service.submit(MEMBER, login, "passt gut").await,
                SubmitOutcome::Saved { .. }
            ));
        }
        assert_eq!(
            service.submit(MEMBER, "vier", "passt gut").await,
            SubmitOutcome::RateLimited
        );
        // Doppelvorschlag zählt nicht gegen das Limit und bleibt beantwortbar.
        assert!(matches!(
            service.submit(MEMBER, "eins", "passt gut").await,
            SubmitOutcome::AlreadyByYou { .. }
        ));
        // Andere Mitglieder sind nicht betroffen.
        assert!(matches!(
            service.submit(MEMBER + 1, "vier", "passt gut").await,
            SubmitOutcome::Saved { .. }
        ));
        // Nach 24 Stunden geht es wieder.
        sqlx::query(
            "UPDATE community.streamer_suggestions
                SET created_at = now() - interval '25 hours'
              WHERE twitch_login = 'eins'",
        )
        .execute(db.pool())
        .await
        .expect("age");
        assert!(matches!(
            service.submit(MEMBER, "vier", "passt gut").await,
            SubmitOutcome::Saved { .. }
        ));
    }

    #[tokio::test]
    async fn widerspruch_verhindert_speichern_und_weitergabe() {
        let forwarder = FakeForwarder::with(vec![ForwardResult::Retry("weg".into())]);
        let (db, service) = setup(Some(forwarder.clone())).await;
        let id = saved_id(&service.submit(MEMBER, "someone", "passt gut").await);
        assert_eq!(row(db.pool(), id).await.0, "pending");
        sqlx::query("INSERT INTO core.user_privacy(user_id, opted_out) VALUES ($1, TRUE)")
            .bind(MEMBER as i64)
            .execute(db.pool())
            .await
            .expect("opt-out");
        assert_eq!(
            service.submit(MEMBER, "other", "passt gut").await,
            SubmitOutcome::OptedOut
        );
        sqlx::query(
            "UPDATE community.streamer_suggestions SET last_attempt_at = now() - interval '1 day'",
        )
        .execute(db.pool())
        .await
        .expect("age");
        assert_eq!(service.forward_pending().await.expect("retry"), 0);
        assert_eq!(forwarder.seen().len(), 1, "kein Versand nach Widerspruch");
    }

    #[tokio::test]
    async fn fehlgeschlagene_weitergabe_wird_nachgeholt() {
        let forwarder = FakeForwarder::with(vec![
            ForwardResult::Retry("Twitch-Bot nicht erreichbar".into()),
            ForwardResult::Retry("503".into()),
            ForwardResult::Answered {
                status: SuggestionStatus::AlreadyKnown,
                twitch_user_id: Some("789".into()),
            },
        ]);
        let (db, service) = setup(Some(forwarder.clone())).await;
        let outcome = service.submit(MEMBER, "someone", "passt gut").await;
        let id = saved_id(&outcome);
        assert!(matches!(
            outcome,
            SubmitOutcome::Saved {
                status: SuggestionStatus::Pending,
                ..
            }
        ));
        assert_eq!(
            outcome_text(&outcome),
            status_text(SuggestionStatus::Pending, "someone")
        );
        assert_eq!(row(db.pool(), id).await, ("pending".into(), None, false, 1));
        // Noch nicht fällig: nichts passiert.
        assert_eq!(service.forward_pending().await.expect("retry"), 0);
        assert_eq!(forwarder.seen().len(), 1);
        for (expected_done, expected_seen) in [(0, 2), (1, 3)] {
            sqlx::query("UPDATE community.streamer_suggestions SET last_attempt_at = now() - interval '7 hours'")
                .execute(db.pool())
                .await
                .expect("age");
            assert_eq!(
                service.forward_pending().await.expect("retry"),
                expected_done
            );
            assert_eq!(forwarder.seen().len(), expected_seen);
        }
        assert_eq!(
            row(db.pool(), id).await,
            ("already_known".into(), Some("789".into()), true, 3)
        );
        // Alle Versuche mit demselben Schlüssel.
        assert!(forwarder.seen().iter().all(|r| r.id == id));
    }

    #[tokio::test]
    async fn endgueltige_ablehnung_und_ohne_weitergabe() {
        let forwarder = FakeForwarder::with(vec![ForwardResult::Permanent("HTTP 409".into())]);
        let (db, service) = setup(Some(forwarder)).await;
        let id = saved_id(&service.submit(MEMBER, "someone", "passt gut").await);
        assert_eq!(row(db.pool(), id).await, ("rejected".into(), None, true, 1));

        // Ohne Twitch-Anbindung wird gespeichert und später nachgeholt.
        let offline = StreamerSuggestions::new(db.pool().clone(), None);
        let outcome = offline.submit(MEMBER, "later", "passt gut").await;
        let later = saved_id(&outcome);
        assert_eq!(
            row(db.pool(), later).await,
            ("pending".into(), None, false, 0)
        );
        assert_eq!(offline.forward_pending().await.expect("retry"), 0);
        let forwarder = FakeForwarder::with(Vec::new());
        let online = StreamerSuggestions::new(db.pool().clone(), Some(forwarder.clone()));
        sqlx::query("UPDATE community.streamer_suggestions SET last_attempt_at = now() - interval '6 minutes' WHERE id = $1")
            .bind(later)
            .execute(db.pool())
            .await
            .expect("age");
        assert_eq!(online.forward_pending().await.expect("retry"), 1);
        assert_eq!(row(db.pool(), later).await.0, "created");
    }

    #[tokio::test]
    async fn erster_vorschlagender_je_kanal_ist_abfragbar() {
        let (db, service) = setup(Some(FakeForwarder::with(vec![
            ForwardResult::Answered {
                status: SuggestionStatus::Created,
                twitch_user_id: Some("456".into()),
            },
            ForwardResult::Answered {
                status: SuggestionStatus::AlreadyKnown,
                twitch_user_id: Some("456".into()),
            },
        ])))
        .await;
        service.submit(MEMBER, "someone", "passt gut").await;
        service.submit(MEMBER + 1, "someone", "passt auch").await;
        let first: i64 = sqlx::query_scalar(
            "SELECT DISTINCT ON (twitch_user_id) discord_id
               FROM community.streamer_suggestions
              WHERE twitch_user_id = '456' AND status IN ('created', 'already_known')
              ORDER BY twitch_user_id, created_at, id",
        )
        .fetch_one(db.pool())
        .await
        .expect("first");
        assert_eq!(first, MEMBER as i64);
    }
    async fn wait_locked(pool: &PgPool, fragment: &str) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            let waiting: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = current_database()
                 AND wait_event_type = 'Lock' AND query LIKE $1 AND pid <> pg_backend_pid())",
            )
            .bind(format!("%{fragment}%"))
            .fetch_one(pool)
            .await
            .expect("Beobachtbare DB-Sperre");
            if waiting {
                return;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "Erwartete Sperre fehlt: {fragment}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[derive(Default)]
    struct HoldingForwarder {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
        seen: Mutex<Vec<ForwardRequest>>,
    }
    #[async_trait]
    impl SuggestionForwarder for HoldingForwarder {
        async fn forward(&self, request: &ForwardRequest) -> ForwardResult {
            self.seen.lock().unwrap().push(request.clone());
            self.entered.notify_one();
            self.release.notified().await;
            ForwardResult::Answered {
                status: SuggestionStatus::Created,
                twitch_user_id: Some("456".into()),
            }
        }
    }

    #[tokio::test]
    async fn scout_schreiben_vor_erasure_wird_geloescht_und_nicht_wiederholt() {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("Echte Wegwerf-DB");
        let forwarder = Arc::new(HoldingForwarder::default());
        let service = StreamerSuggestions::new(db.pool().clone(), Some(forwarder.clone()));
        let submitting = service.clone();
        let writer =
            tokio::spawn(
                async move { submitting.submit(MEMBER, "someone", "eigener Grund").await },
            );
        forwarder.entered.notified().await;
        let pool = db.pool().clone();
        let erasing = tokio::spawn(async move {
            crate::privacy::delete_user_data(
                &pool,
                MEMBER as i64,
                "test".into(),
                chrono::Utc::now().timestamp(),
            )
            .await
        });
        wait_locked(db.pool(), "pg_advisory_xact_lock($1)").await;
        forwarder.release.notify_one();
        let outcome = writer.await.expect("Schreibaufgabe");
        assert!(matches!(
            outcome,
            SubmitOutcome::Saved {
                status: SuggestionStatus::Created,
                ..
            }
        ));
        erasing.await.expect("Erasureaufgabe").expect("Erasure");
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM community.streamer_suggestions WHERE discord_id = $1",
        )
        .bind(MEMBER as i64)
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 0);
        let request = forwarder.seen.lock().unwrap()[0].clone();
        assert_eq!(
            service.forward_one(&request).await,
            SuggestionStatus::Rejected
        );
        assert_eq!(forwarder.seen.lock().unwrap().len(), 1);
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM community.scout_privacy_outbox WHERE discord_id = $1 AND action = 'erase'")
            .bind(MEMBER as i64).fetch_one(db.pool()).await.unwrap();
        assert_eq!(pending, 1);
    }

    #[tokio::test]
    async fn scout_erasure_vor_schreiben_blockiert_wartenden_vorschlag() {
        let (db, offline) = setup(None).await;
        let old = saved_id(&offline.submit(MEMBER, "someone", "eigener Grund").await);
        let mut held = db.pool().begin().await.expect("Zeilensperre");
        sqlx::query("SELECT id FROM community.streamer_suggestions WHERE id = $1 FOR UPDATE")
            .bind(old)
            .fetch_one(&mut *held)
            .await
            .unwrap();
        let pool = db.pool().clone();
        let erasing = tokio::spawn(async move {
            crate::privacy::delete_user_data(
                &pool,
                MEMBER as i64,
                "test".into(),
                chrono::Utc::now().timestamp(),
            )
            .await
        });
        wait_locked(db.pool(), "DELETE FROM community.streamer_suggestions").await;
        let forwarder = FakeForwarder::with(Vec::new());
        let service = StreamerSuggestions::new(db.pool().clone(), Some(forwarder.clone()));
        let writer =
            tokio::spawn(async move { service.submit(MEMBER, "neuerkanal", "neuer Grund").await });
        wait_locked(db.pool(), "pg_advisory_xact_lock($1)").await;
        held.commit().await.unwrap();
        erasing.await.expect("Erasureaufgabe").expect("Erasure");
        assert_eq!(
            writer.await.expect("Schreibaufgabe"),
            SubmitOutcome::OptedOut
        );
        assert!(forwarder.seen().is_empty());
    }
}
