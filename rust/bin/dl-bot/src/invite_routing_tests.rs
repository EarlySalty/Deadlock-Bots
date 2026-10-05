#[derive(Default)]
struct RecordingInviteEvents {
    calls: Mutex<Vec<(u64, u64, String, u64)>>,
    response: Option<String>,
    fail: bool,
}

struct ConcurrentCodeReplies {
    replies: RecordingBrainReplies,
    first_checks: AtomicUsize,
    barrier: tokio::sync::Barrier,
}

#[async_trait::async_trait]
impl BrainDirectReplyPort for ConcurrentCodeReplies {
    async fn is_human(&self, event: &dl_discord::MessageEvent) -> bool {
        self.replies.is_human(event).await
    }

    async fn can_reply(&self, event: &dl_discord::MessageEvent) -> bool {
        if event.content == "1313436779" && self.first_checks.fetch_add(1, Ordering::Relaxed) < 2 {
            self.barrier.wait().await;
        }
        self.replies.can_reply(event).await
    }

    async fn reply(
        &self,
        event: &dl_discord::MessageEvent,
        body: &Map<String, Value>,
    ) -> Result<u64, String> {
        self.replies.reply(event, body).await
    }
}

#[derive(Default)]
struct DelayedInviteEvents {
    invites: RecordingInviteEvents,
    started: tokio::sync::Notify,
    resume: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl dl_community::invite_lounge::InviteEventPort for DelayedInviteEvents {
    async fn invite(
        &self,
        bot_id: u64,
        guild_id: u64,
        code: &str,
        user_id: u64,
    ) -> Result<String, String> {
        let response = dl_community::invite_lounge::InviteEventPort::invite(
            &self.invites,
            bot_id,
            guild_id,
            code,
            user_id,
        )
        .await;
        if code == "1313436779" {
            self.started.notify_one();
            self.resume.notified().await;
        }
        response
    }
}

struct FailedOldInviteReply {
    replies: RecordingBrainReplies,
    fail_message_id: u64,
    revoke: bool,
    checks: AtomicUsize,
}

#[async_trait::async_trait]
impl BrainDirectReplyPort for FailedOldInviteReply {
    async fn is_human(&self, event: &dl_discord::MessageEvent) -> bool {
        self.replies.is_human(event).await
    }

    async fn can_reply(&self, event: &dl_discord::MessageEvent) -> bool {
        if self.revoke
            && event.message_id == self.fail_message_id
            && self.checks.fetch_add(1, Ordering::Relaxed) > 0
        {
            return false;
        }
        self.replies.can_reply(event).await
    }

    async fn reply(
        &self,
        event: &dl_discord::MessageEvent,
        body: &Map<String, Value>,
    ) -> Result<u64, String> {
        if event.message_id == self.fail_message_id {
            return Err("Alte Versandantwort konnte nicht zugestellt werden".into());
        }
        self.replies.reply(event, body).await
    }
}

#[async_trait::async_trait]
impl dl_community::invite_lounge::InviteEventPort for RecordingInviteEvents {
    async fn invite(
        &self,
        bot_id: u64,
        guild_id: u64,
        code: &str,
        user_id: u64,
    ) -> Result<String, String> {
        self.calls
            .lock()
            .await
            .push((bot_id, guild_id, code.into(), user_id));
        if self.fail {
            return Err("Versand fehlgeschlagen".into());
        }
        Ok(self
            .response
            .clone()
            .unwrap_or_else(|| "Einladung raus".into()))
    }
}

fn invite_test_handler() -> (
    BrainHandler,
    Arc<RecordingDiscordAnswerer>,
    Arc<RecordingInviteEvents>,
) {
    let (mut handler, answerer) = direct_test_handler();
    let invites = Arc::new(RecordingInviteEvents::default());
    handler.invites = invites.clone();
    (handler, answerer, invites)
}

#[tokio::test]
async fn invite_mention_mit_mejn_und_mein_umgeht_brain_in_jedem_kanal() {
    for channel in [1, 1426220969088774185] {
        for mention in ["<@42>", "<@!42>"] {
            for word in ["mejn", "mein"] {
                let (handler, answerer, invites) = invite_test_handler();
                let replies = RecordingBrainReplies::default();
                let mut event = test_message_event(
                    Some(1),
                    &format!("{mention} kannst du mich einladen {word} Code ist 1313436779"),
                );
                event.channel_id = channel;
                handler
                    .handle_message_event_with_replies(&event, 42, &replies)
                    .await;
                assert_eq!(
                    *invites.calls.lock().await,
                    vec![(42, 1, "1313436779".into(), 3)]
                );
                assert!(answerer.calls.lock().await.is_empty());
                assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
                let sent = replies.sent.lock().await;
                assert_eq!(sent.len(), 1);
                assert_eq!(sent[0].0, channel);
                assert_eq!(sent[0].2["content"], "Einladung raus");
            }
        }
    }
}

#[tokio::test]
async fn invite_ohne_code_fragt_einmal_und_nimmt_zahl_in_fortsetzung_an() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    let event = test_message_event(Some(1), "<@42> kannst du mich einladen?");
    handler
        .handle_message_event_with_replies(&event, 42, &replies)
        .await;
    let mut repeated = event.clone();
    repeated.message_id = 4;
    handler
        .handle_message_event_with_replies(&repeated, 42, &replies)
        .await;
    assert!(answerer.calls.lock().await.is_empty());
    assert!(invites.calls.lock().await.is_empty());
    let sent = replies.sent.lock().await;
    assert_eq!(sent.len(), 1);
    assert!(sent[0].2["content"]
        .as_str()
        .unwrap()
        .contains("Steam-Freundescode als Zahl"));
    drop(sent);
    let mut code = test_message_event(Some(1), "1313436779");
    code.message_id = 5;
    handler
        .handle_message_event_with_replies(&code, 42, &replies)
        .await;
    assert_eq!(invites.calls.lock().await.len(), 1);
    assert!(answerer.calls.lock().await.is_empty());
    assert_eq!(replies.sent.lock().await.len(), 2);
}

#[tokio::test]
async fn invite_parallele_code_fortsetzungen_starten_nur_einen_versand() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = ConcurrentCodeReplies {
        replies: RecordingBrainReplies::default(),
        first_checks: AtomicUsize::new(0),
        barrier: tokio::sync::Barrier::new(2),
    };
    handler
        .handle_message_event_with_replies(
            &test_message_event(Some(1), "<@42> Kannst du mich einladen?"),
            42,
            &replies,
        )
        .await;
    let mut first = test_message_event(Some(1), "1313436779");
    first.message_id = 4;
    let mut second = first.clone();
    second.message_id = 5;
    tokio::time::timeout(Duration::from_secs(2), async {
        tokio::join!(
            handler.handle_message_event_with_replies(&first, 42, &replies),
            handler.handle_message_event_with_replies(&second, 42, &replies),
        );
    })
    .await
    .expect("Beide Zahlennachrichten haben die Rechteprüfung erreicht");
    assert_eq!(
        *invites.calls.lock().await,
        vec![(42, 1, "1313436779".into(), 3)]
    );
    assert!(answerer.calls.lock().await.is_empty());
    assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
    assert_eq!(replies.replies.sent.lock().await.len(), 2);
}

#[tokio::test]
async fn invite_alte_versandantwort_loescht_keine_neuere_code_nachfrage() {
    for revoke in [false, true] {
        let (mut handler, answerer, _) = invite_test_handler();
        let invites = Arc::new(DelayedInviteEvents::default());
        handler.invites = invites.clone();
        let handler = Arc::new(handler);
        let replies = Arc::new(FailedOldInviteReply {
            replies: RecordingBrainReplies::default(),
            fail_message_id: 2,
            revoke,
            checks: AtomicUsize::new(0),
        });
        let slow_handler = handler.clone();
        let slow_replies = replies.clone();
        let slow = tokio::spawn(async move {
            slow_handler
                .handle_message_event_with_replies(
                    &test_message_event(Some(1), "<@42> Kannst du mich einladen? 1313436779"),
                    42,
                    slow_replies.as_ref(),
                )
                .await;
        });
        tokio::time::timeout(Duration::from_secs(2), invites.started.notified())
            .await
            .expect("Der erste Versand wartet auf seine Freigabe");
        let mut request = test_message_event(Some(1), "<@42> Kannst du mich einladen?");
        request.message_id = 4;
        handler
            .handle_message_event_with_replies(&request, 42, replies.as_ref())
            .await;
        invites.resume.notify_one();
        tokio::time::timeout(Duration::from_secs(2), slow)
            .await
            .expect("Die alte Versandantwort wurde abgeschlossen")
            .expect("Der alte Versandtask ist erfolgreich beendet");
        let mut code = test_message_event(Some(1), "1234567890");
        code.message_id = 5;
        handler
            .handle_message_event_with_replies(&code, 42, replies.as_ref())
            .await;
        assert_eq!(
            *invites.invites.calls.lock().await,
            vec![
                (42, 1, "1313436779".into(), 3),
                (42, 1, "1234567890".into(), 3),
            ]
        );
        assert!(answerer.calls.lock().await.is_empty());
        assert_eq!(answerer.legacy_calls.load(Ordering::Relaxed), 0);
        let sent = replies.replies.sent.lock().await;
        assert_eq!(sent.len(), 2);
        assert!(sent[0].2["content"]
            .as_str()
            .unwrap()
            .contains("Steam-Freundescode als Zahl"));
        assert_eq!(sent[1].2["content"], "Einladung raus");
    }
}

#[tokio::test]
async fn invite_direkte_bitte_in_laufendem_gespraech_umgeht_brain() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    let first = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
    handler
        .handle_message_event_with_replies(&first, 42, &replies)
        .await;
    let mut request =
        test_message_event(Some(1), "kannst du mich einladen mejn Code ist 1313436779");
    request.message_id = 4;
    handler
        .handle_message_event_with_replies(&request, 42, &replies)
        .await;
    assert_eq!(answerer.calls.lock().await.len(), 1);
    assert_eq!(invites.calls.lock().await.len(), 1);
    assert_eq!(replies.sent.lock().await[1].2["content"], "Einladung raus");
}

#[tokio::test]
async fn invite_wissensfrage_bleibt_beim_brain() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    let event = test_message_event(Some(1), "<@42> Welche Lanes gibt es?");
    handler
        .handle_message_event_with_replies(&event, 42, &replies)
        .await;
    assert_eq!(answerer.calls.lock().await.len(), 1);
    assert!(invites.calls.lock().await.is_empty());
    assert_eq!(
        replies.sent.lock().await[0].2["content"],
        "Antwort: Welche Lanes gibt es?"
    );
}

#[tokio::test]
async fn invite_ohne_ansprache_ausserhalb_lounge_startet_keinen_versand() {
    for content in [
        "Kannst du mich einladen? 1313436779",
        "Kann mich jemand einladen? 1313436779",
        "<@42> ich lade dich ein 1313436779",
    ] {
        let (handler, _, invites) = invite_test_handler();
        handler
            .handle_message_event_with_replies(
                &test_message_event(Some(1), content),
                42,
                &RecordingBrainReplies::default(),
            )
            .await;
        assert!(invites.calls.lock().await.is_empty(), "{content}");
    }
}

#[tokio::test]
async fn invite_handler_antworten_bleiben_erhalten() {
    for response in [
        "Schon eingeladen",
        "📨 Freundschaftsanfrage ist raus. Nimm sie an.",
        "🎮 Ihr seid schon Steam-Freunde, Einladung ist raus.",
    ] {
        let (mut handler, answerer, _) = invite_test_handler();
        handler.invites = Arc::new(RecordingInviteEvents {
            response: Some(response.into()),
            ..Default::default()
        });
        let replies = RecordingBrainReplies::default();
        handler
            .handle_message_event_with_replies(
                &test_message_event(Some(1), "<@42> Kannst du mich einladen? 1313436779"),
                42,
                &replies,
            )
            .await;
        assert!(answerer.calls.lock().await.is_empty());
        assert_eq!(
            replies.sent.lock().await[0].2["content"],
            dl_community::invite_lounge::lounge_response(response)
        );
    }
}

#[tokio::test]
async fn invite_lounge_bleibt_beim_watcher_ohne_brain_antwort() {
    for content in [
        "<@42> Kannst du mich einladen? 1313436779",
        "<@42> Kannst du mich einladen?",
        "Kann mich jemand einladen?",
    ] {
        let (handler, answerer, invites) = invite_test_handler();
        let replies = RecordingBrainReplies::default();
        let event = guide_test_event(content);
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
        assert!(answerer.calls.lock().await.is_empty(), "{content}");
        assert!(invites.calls.lock().await.is_empty());
        assert!(replies.sent.lock().await.is_empty());
    }
}

#[tokio::test]
async fn invite_nachfrage_bleibt_auf_nutzer_und_kanal_begrenzt() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    let request = test_message_event(Some(1), "<@42> Kannst du mich einladen?");
    handler
        .handle_message_event_with_replies(&request, 42, &replies)
        .await;
    for other_user in [true, false] {
        let mut event = test_message_event(Some(1), "1313436779");
        event.message_id = 4;
        if other_user {
            event.author_id = 4;
        } else {
            event.channel_id = 4;
        }
        handler
            .handle_message_event_with_replies(&event, 42, &replies)
            .await;
    }
    assert_eq!(replies.sent.lock().await.len(), 1);
    assert!(answerer.calls.lock().await.is_empty());
    assert!(invites.calls.lock().await.is_empty());
}

#[tokio::test]
async fn invite_fehlender_zahlencode_im_profillink_fragt_nach() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    let event = test_message_event(
        Some(1),
        "<@42> Kannst du mich einladen? https://steamcommunity.com/id/1313436779",
    );
    handler
        .handle_message_event_with_replies(&event, 42, &replies)
        .await;
    assert!(answerer.calls.lock().await.is_empty());
    assert!(invites.calls.lock().await.is_empty());
    assert!(replies.sent.lock().await[0].2["content"]
        .as_str()
        .unwrap()
        .contains("Steam-Freundescode als Zahl"));
}

#[tokio::test]
async fn invite_versandfehler_antwortet_ohne_brain() {
    let (mut handler, answerer, _) = invite_test_handler();
    let invites = Arc::new(RecordingInviteEvents {
        fail: true,
        ..Default::default()
    });
    handler.invites = invites.clone();
    let replies = RecordingBrainReplies::default();
    let event = test_message_event(Some(1), "<@42> Kannst du mich einladen? 1313436779");
    handler
        .handle_message_event_with_replies(&event, 42, &replies)
        .await;
    assert!(answerer.calls.lock().await.is_empty());
    assert_eq!(invites.calls.lock().await.len(), 1);
    assert!(replies.sent.lock().await[0].2["content"]
        .as_str()
        .unwrap()
        .starts_with("Die Einladung hat gerade nicht geklappt."));
}

#[tokio::test]
async fn invite_fehlende_schreibrechte_starten_keinen_versand() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies {
        deny: true,
        ..Default::default()
    };
    let event = test_message_event(Some(1), "<@42> Kannst du mich einladen? 1313436779");
    handler
        .handle_message_event_with_replies(&event, 42, &replies)
        .await;
    assert!(answerer.calls.lock().await.is_empty());
    assert!(invites.calls.lock().await.is_empty());
    assert!(replies.sent.lock().await.is_empty());
}

#[tokio::test]
async fn invite_abgelaufene_fortsetzung_startet_keinen_versand() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    let event = test_message_event(Some(1), "Kannst du mich einladen? 1313436779");
    handler.conversations.lock().await.record(
        &event,
        1001,
        Instant::now() - BRAIN_CONVERSATION_TTL,
    );
    handler
        .handle_message_event_with_replies(&event, 42, &replies)
        .await;
    assert!(answerer.calls.lock().await.is_empty());
    assert!(invites.calls.lock().await.is_empty());
    assert!(replies.sent.lock().await.is_empty());
}

#[tokio::test]
async fn invite_eventschleife_routet_mention_und_code_fortsetzung() {
    let (handler, answerer, invites) = invite_test_handler();
    handler.adapter.bot_user_id_cell().set(42).expect("Testbot");
    let handler = Arc::new(handler);
    let replies = Arc::new(RecordingBrainReplies::default());
    let dispatcher = dl_discord::Dispatcher::new();
    let listener = spawn_brain_command_with_replies(handler, &dispatcher, replies.clone());
    dispatcher.publish_message(test_message_event(
        Some(1),
        "<@42> kannst du mich einladen?",
    ));
    tokio::time::timeout(Duration::from_secs(2), replies.delivered.notified())
        .await
        .expect("Code-Nachfrage wurde zugestellt");
    let mut code = test_message_event(Some(1), "1313436779");
    code.message_id = 4;
    dispatcher.publish_message(code);
    tokio::time::timeout(Duration::from_secs(2), replies.delivered.notified())
        .await
        .expect("Invite-Antwort wurde zugestellt");
    assert_eq!(
        *invites.calls.lock().await,
        vec![(42, 1, "1313436779".into(), 3)]
    );
    assert!(answerer.calls.lock().await.is_empty());
    assert_eq!(replies.sent.lock().await.len(), 2);
    listener.abort();
    assert!(listener.await.expect_err("Listenerabbruch").is_cancelled());
}

#[tokio::test]
async fn invite_angebot_waehrend_code_nachfrage_startet_keinen_versand() {
    let (handler, answerer, invites) = invite_test_handler();
    let replies = RecordingBrainReplies::default();
    handler
        .handle_message_event_with_replies(
            &test_message_event(Some(1), "<@42> Kannst du mich einladen?"),
            42,
            &replies,
        )
        .await;
    let mut offer = test_message_event(Some(1), "ich lade dich ein 1313436779");
    offer.message_id = 4;
    handler
        .handle_message_event_with_replies(&offer, 42, &replies)
        .await;
    assert!(invites.calls.lock().await.is_empty());
    assert_eq!(answerer.calls.lock().await.len(), 1);
}
