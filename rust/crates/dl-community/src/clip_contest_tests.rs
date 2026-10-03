use super::*;

fn clip(id: i64, created: i64, link: &str, submitter: Submitter) -> ContestClip {
    ContestClip {
        submission_id: id,
        created_at_ts: created,
        link: link.to_string(),
        credit: format!("@user{id}"),
        submitter,
    }
}

fn discord(id: i64, created: i64, user: u64) -> ContestClip {
    clip(
        id,
        created,
        &format!("https://clips.twitch.tv/Clip{id}"),
        Submitter::Discord(user),
    )
}

#[test]
fn twitch_url_formen() {
    let a = parse_twitch_clip_url("https://clips.twitch.tv/FunnyClip-abc_123").expect("clips");
    assert_eq!(a.slug, "FunnyClip-abc_123");
    assert_eq!(a.canonical, "https://clips.twitch.tv/FunnyClip-abc_123");
    let b = parse_twitch_clip_url(
        "https://www.twitch.tv/dach_lock/clip/FunnyClip-abc_123?filter=clips",
    )
    .expect("kanal");
    assert_eq!(b, a);
    assert!(parse_twitch_clip_url("https://twitch.tv/dach_lock/clip/FunnyClip-abc_123/").is_some());
    assert!(parse_twitch_clip_url("https://m.twitch.tv/dach_lock/clip/X").is_some());
    for invalid in [
        "http://clips.twitch.tv/X",
        "https://clips.twitch.tv/",
        "https://clips.twitch.tv/a/b",
        "https://twitch.tv/dach_lock/videos/123",
        "https://twitch.tv/dach_lock",
        "https://evil.example/clips.twitch.tv/X",
        "https://clips.twitch.tv.evil.example/X",
        "https://user@clips.twitch.tv/X",
        "https://clips.twitch.tv:8443/X",
        "https://clips.twitch.tv/X Y",
        "https://clips.twitch.tv/<script>",
        "https://www.youtube.com/watch?v=abc",
        "",
    ] {
        assert!(parse_twitch_clip_url(invalid).is_none(), "{invalid}");
    }
    let long = format!("https://clips.twitch.tv/{}", "a".repeat(MAX_CLIP_URL_LEN));
    assert!(parse_twitch_clip_url(&long).is_none());
}

#[test]
fn clip_schluessel_erkennt_gleichen_clip() {
    assert_eq!(
        clip_key("https://clips.twitch.tv/Abc"),
        clip_key("https://www.twitch.tv/kanal/clip/Abc?x=1")
    );
    assert_ne!(
        clip_key("https://clips.twitch.tv/Abc"),
        clip_key("https://clips.twitch.tv/abc")
    );
    assert_eq!(
        clip_key("https://YouTube.com/watch?v=1/"),
        clip_key("https://youtube.com/watch?v=1#t=3")
    );
}

#[test]
fn stimmzettel_sortiert_dedupliziert_und_kappt() {
    let mut clips: Vec<ContestClip> = (1..=30).rev().map(|i| discord(i, 1000 + i, 1)).collect();
    // Duplikat von Clip 3, aber später eingereicht → fliegt raus
    clips.push(clip(
        99,
        5000,
        "https://www.twitch.tv/x/clip/Clip3",
        Submitter::Discord(2),
    ));
    // ungültige URL → fliegt raus
    clips.push(clip(100, 0, "kein link", Submitter::Discord(3)));
    let ballot = select_ballot(&clips);
    assert_eq!(ballot.len(), MAX_BALLOT_ENTRIES);
    assert_eq!(ballot[0].submission_id, 1);
    assert_eq!(ballot[24].submission_id, 25);
    assert!(ballot
        .iter()
        .all(|c| c.submission_id != 99 && c.submission_id != 100));
}

#[test]
fn ranking_und_gleichstand() {
    let ballot = vec![
        discord(1, 100, 1),
        discord(2, 50, 2),
        discord(3, 75, 3),
        discord(4, 10, 4),
    ];
    // 1 und 2 gleich (2 Stimmen) → 2 ist früher eingereicht und gewinnt
    let votes = vec![1, 1, 2, 2, 3, 99];
    let top = rank_top3(&ballot, &votes);
    assert_eq!(
        top,
        vec![
            Placement {
                place: 1,
                submission_id: 2,
                votes: 2
            },
            Placement {
                place: 2,
                submission_id: 1,
                votes: 2
            },
            Placement {
                place: 3,
                submission_id: 3,
                votes: 1
            },
        ]
    );
    // Clips ohne Stimme bekommen keinen Platz
    let top = rank_top3(&ballot, &[4]);
    assert_eq!(top.len(), 1);
    assert_eq!(top[0].submission_id, 4);
    assert!(rank_top3(&ballot, &[]).is_empty());
    // gleiche Zeit → kleinere ID gewinnt
    let same = vec![discord(8, 1, 1), discord(7, 1, 2)];
    assert_eq!(rank_top3(&same, &[8, 7])[0].submission_id, 7);
}

#[test]
fn stimmrecht() {
    let now = 1_000_000;
    let own = discord(1, 0, 42);
    assert_eq!(
        check_vote(42, &[], &own, None, now),
        Err(VoteDenial::OwnClip)
    );
    assert_eq!(check_vote(43, &[], &own, None, now), Ok(()));
    let twitch = clip(
        2,
        0,
        "https://clips.twitch.tv/A",
        Submitter::Twitch {
            twitch_user_id: "456".into(),
            login: "streamer".into(),
        },
    );
    assert_eq!(
        check_vote(43, &["456".to_string()], &twitch, None, now),
        Err(VoteDenial::OwnClip)
    );
    assert_eq!(
        check_vote(43, &["457".to_string()], &twitch, None, now),
        Ok(())
    );
    let joined = now - MIN_MEMBERSHIP_DAYS * 86_400 + 60;
    assert_eq!(
        check_vote(43, &[], &own, Some(joined), now),
        Err(VoteDenial::MembershipTooNew {
            allowed_from_ts: now + 60
        })
    );
    assert_eq!(check_vote(43, &[], &own, Some(joined - 120), now), Ok(()));
}

#[test]
fn twitch_fensterzuordnung() {
    use chrono::TimeZone;
    // Mittwoch → laufendes Fenster
    let wed = Utc
        .with_ymd_and_hms(2026, 6, 10, 12, 0, 0)
        .single()
        .expect("ts");
    assert_eq!(twitch_target_window(wed), compute_week_window(wed));
    // Samstag 23:30 Berlin (21:30 UTC, Sommerzeit) → nächstes Fenster ab Sonntag
    let gap = Utc
        .with_ymd_and_hms(2026, 6, 13, 21, 30, 0)
        .single()
        .expect("ts");
    let (start, end) = twitch_target_window(gap);
    let (cur_start, cur_end) = compute_week_window(gap);
    assert!(gap.timestamp() > cur_end);
    assert!(start > cur_start);
    assert_eq!(
        chrono_tz::Europe::Berlin
            .timestamp_opt(start, 0)
            .single()
            .expect("start")
            .format("%Y-%m-%d %H:%M")
            .to_string(),
        "2026-06-14 00:00"
    );
    assert!(end > start && gap.timestamp() < start);
}

#[test]
fn posts_ohne_gedankenstriche_und_mit_grenzen() {
    let ballot: Vec<ContestClip> = (1..=25)
        .map(|i| {
            clip(
                i,
                i,
                &format!("https://example.com/{}", "x".repeat(350)),
                Submitter::Discord(i as u64),
            )
        })
        .collect();
    let body = voting_message(7, 0, 86_400, 200_000, &ballot);
    let embeds = body["embeds"].as_array().expect("embeds");
    let total: usize = embeds
        .iter()
        .map(|e| {
            ["description", "title"]
                .iter()
                .map(|k| e[*k].as_str().unwrap_or("").chars().count())
                .sum::<usize>()
                + e["footer"]["text"].as_str().unwrap_or("").chars().count()
        })
        .sum();
    assert!(total <= EMBED_TOTAL_BUDGET, "{total}");
    // kurze Links passen alle mit Link hinein
    let short: Vec<ContestClip> = (1..=25).map(|i| discord(i, i, 1)).collect();
    let short_body = voting_message(7, 0, 1, 2, &short);
    let text = serde_json::to_string(&short_body).expect("json");
    assert!(text.contains("https://clips.twitch.tv/Clip25"));
    assert!(!text.contains("Auswahlmenü."));
    assert!(embeds
        .iter()
        .all(|e| e["description"].as_str().unwrap_or("").chars().count() <= EMBED_DESCRIPTION_MAX));
    assert_eq!(
        embeds.last().expect("embed")["footer"]["text"],
        json!(voting_footer(7))
    );
    let options = body["components"][0]["components"][0]["options"]
        .as_array()
        .expect("options");
    assert_eq!(options.len(), 25);
    assert_eq!(
        body["components"][0]["components"][0]["custom_id"],
        json!("clip_vote_select_v1:7")
    );
    let results = vec![ResultEntry {
        place: 1,
        votes: 1,
        clip: ballot[0].clone(),
    }];
    let texts = [
        serde_json::to_string(&body).expect("json"),
        serde_json::to_string(&result_message(7, 0, 1, &results)).expect("json"),
        serde_json::to_string(&result_message(7, 0, 1, &[])).expect("json"),
        serde_json::to_string(&closed_voting_message(7, 0, 1, &ballot)).expect("json"),
        curator_text(0, 1, &results),
        denial_text(&VoteDenial::OwnClip),
        denial_text(&VoteDenial::MembershipTooNew { allowed_from_ts: 1 }),
    ];
    for text in texts {
        assert!(!text.contains('–') && !text.contains('—'), "{text}");
    }
    assert!(curator_text(0, 1, &results).contains("1 Stimme"));
}

#[test]
fn credit_wird_bereinigt() {
    let mut c = discord(1, 0, 1);
    c.credit = "[evil](https://x) **fett**\nzeile".to_string();
    assert_eq!(display_credit(&c), "evilhttps://x fett zeile");
    c.credit = "a".repeat(80);
    assert_eq!(display_credit(&c).chars().count(), 40);
    let t = clip(
        2,
        0,
        "https://clips.twitch.tv/A",
        Submitter::Twitch {
            twitch_user_id: "1".into(),
            login: "streamer_x".into(),
        },
    );
    assert_eq!(display_credit(&t), "streamerx");
}

#[cfg(feature = "testing")]
mod db {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakePort {
        posts: Mutex<Vec<(u64, serde_json::Map<String, Value>)>>,
        edits: Mutex<Vec<u64>>,
        curator: Mutex<Vec<String>>,
        footer_hit: Mutex<Option<(String, u64)>>,
        joined: Mutex<HashMap<u64, i64>>,
    }

    #[async_trait::async_trait]
    impl crate::clips::ClipPort for FakePort {
        async fn upsert_interface(
            &self,
            _: u64,
            _: Option<u64>,
            _: Value,
            _: Value,
        ) -> Result<u64, String> {
            Ok(1)
        }
        async fn send_dump(&self, _: u64, _: u64, _: String, _: String, _: String) {}
        async fn guild_name(&self, _: u64) -> String {
            "G".into()
        }
        async fn post_message(
            &self,
            channel_id: u64,
            body: serde_json::Map<String, Value>,
            _nonce: String,
        ) -> Result<u64, String> {
            let mut posts = self.posts.lock().expect("lock");
            posts.push((channel_id, body));
            Ok(1000 + posts.len() as u64)
        }
        async fn edit_message(
            &self,
            _: u64,
            message_id: u64,
            _: serde_json::Map<String, Value>,
        ) -> Result<(), String> {
            self.edits.lock().expect("lock").push(message_id);
            Ok(())
        }
        async fn find_message_by_footer(
            &self,
            _: u64,
            footer: &str,
        ) -> Result<Option<u64>, String> {
            Ok(self
                .footer_hit
                .lock()
                .expect("lock")
                .as_ref()
                .filter(|(f, _)| f == footer)
                .map(|(_, id)| *id))
        }
        async fn send_curator_text(&self, _: u64, _: u64, content: String) -> Result<(), String> {
            self.curator.lock().expect("lock").push(content);
            Ok(())
        }
        async fn member_joined_at(&self, _: u64, user_id: u64) -> Option<i64> {
            self.joined.lock().expect("lock").get(&user_id).copied()
        }
    }

    async fn setup() -> (
        dl_central_db::testing::TestDb,
        Arc<ClipSubmission>,
        Arc<FakePort>,
    ) {
        let db = dl_central_db::testing::test_pool()
            .await
            .expect("test_pool");
        let port = Arc::new(FakePort::default());
        let clips = ClipSubmission::new(db.pool().clone(), port.clone());
        (db, clips, port)
    }

    /// Abgelaufenes Fenster mit drei Discord-Clips (Nutzer 10, 11, 12) und einem
    /// Twitch-Clip; liefert window_id und Einsendungs-IDs.
    async fn ended_window(pool: &PgPool) -> (i64, Vec<i64>) {
        let now = Utc::now();
        let window_id: i64 = sqlx::query_scalar(
            "INSERT INTO clips.clip_windows(guild_id, start_at, end_at)
             VALUES (1, $1, $2) RETURNING id",
        )
        .bind(now - chrono::Duration::days(7))
        .bind(now - chrono::Duration::hours(1))
        .fetch_one(pool)
        .await
        .expect("window");
        let mut ids = Vec::new();
        for (offset, user) in [(60_i64, 10_i64), (50, 11), (40, 12)] {
            let id: i64 = sqlx::query_scalar(
                "INSERT INTO clips.clip_submissions(guild_id, user_id, link, credit, permission, created_at)
                 VALUES (1, $1, $2, $3, 'owner_or_permission', $4) RETURNING id",
            )
            .bind(user)
            .bind(format!("https://clips.twitch.tv/Clip{user}"))
            .bind(format!("@u{user}"))
            .bind(now - chrono::Duration::hours(offset))
            .fetch_one(pool)
            .await
            .expect("submission");
            sqlx::query("INSERT INTO clips.clip_window_submissions VALUES ($1, $2, $3)")
                .bind(window_id)
                .bind(id)
                .bind(user)
                .execute(pool)
                .await
                .expect("link");
            ids.push(id);
        }
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO clips.clip_submissions(guild_id, user_id, link, credit, permission,
                 created_at, source, streamer_twitch_user_id, streamer_login, idempotency_key)
             VALUES (1, NULL, 'https://clips.twitch.tv/Tw', 'streamer', 'twitch_partner_channel',
                     $1, 'twitch', '456', 'streamer', 'twitch-clip-Tw') RETURNING id",
        )
        .bind(now - chrono::Duration::hours(30))
        .fetch_one(pool)
        .await
        .expect("twitch submission");
        sqlx::query("INSERT INTO clips.clip_window_submissions VALUES ($1, $2, NULL)")
            .bind(window_id)
            .bind(id)
            .execute(pool)
            .await
            .expect("link");
        ids.push(id);
        (window_id, ids)
    }

    /// (place, submission_id, user_id, streamer_twitch_user_id, votes, source)
    type ResultTuple = (i16, Option<i64>, Option<i64>, Option<String>, i32, String);

    fn vote(user: u64, window_id: i64, submission_id: i64) -> BridgeInteraction {
        BridgeInteraction {
            custom_id: format!("{VOTE_SELECT_PREFIX}{window_id}"),
            values: vec![submission_id.to_string()],
            user_id: user,
            guild_id: 1,
            ..BridgeInteraction::default()
        }
    }

    #[tokio::test]
    async fn voting_ablauf_genau_einmal() {
        let (db, clips, port) = setup().await;
        let (window_id, ids) = ended_window(db.pool()).await;

        clips.process_contest(1).await;
        clips.process_contest(1).await;
        assert_eq!(
            port.posts.lock().expect("lock").len(),
            1,
            "genau ein Voting-Post"
        );
        let voting = clips
            .store
            .voting(window_id)
            .await
            .expect("q")
            .expect("voting");
        assert_eq!(voting.status, "open");
        assert_eq!(voting.message_id, Some(1001));
        let ballot = clips.store.ballot(window_id).await.expect("ballot");
        assert_eq!(
            ballot.iter().map(|c| c.submission_id).collect::<Vec<_>>(),
            vec![ids[0], ids[1], ids[2], ids[3]],
            "nach Einsendezeit"
        );

        let handler = VoteHandler {
            clips: clips.clone(),
        };
        // eigener Clip
        let reply = handler.handle(vote(10, window_id, ids[0])).await;
        assert!(reply.content.unwrap_or_default().contains("eigenen Clip"));
        // zu neues Mitglied
        port.joined
            .lock()
            .expect("lock")
            .insert(30, Utc::now().timestamp() - 3600);
        let reply = handler.handle(vote(30, window_id, ids[0])).await;
        assert!(reply.content.unwrap_or_default().contains("Tage"));
        // Stimmen: 20 → A, 21 → B, 22 → B, 23 → A dann geändert auf C
        for (user, id) in [(20, ids[0]), (21, ids[1]), (22, ids[1]), (23, ids[0])] {
            let reply = handler.handle(vote(user, window_id, id)).await;
            assert!(reply.content.unwrap_or_default().starts_with('✅'));
        }
        let reply = handler.handle(vote(23, window_id, ids[2])).await;
        assert!(reply.content.unwrap_or_default().starts_with('🔁'));
        let reply = handler.handle(vote(23, window_id, ids[2])).await;
        assert!(reply.content.unwrap_or_default().starts_with('👍'));
        // nicht auf dem Stimmzettel
        let reply = handler.handle(vote(24, window_id, 999_999)).await;
        assert!(reply.content.unwrap_or_default().contains("gibt es"));
        // Twitch-Clip mit 1 Stimme, Gleichstand mit C (C früher eingereicht → C vorne)
        handler.handle(vote(25, window_id, ids[3])).await;
        assert_eq!(
            clips.store.my_vote(window_id, 23).await.expect("q"),
            Some(ids[2])
        );

        // Ende erzwingen
        sqlx::query(
            "UPDATE clips.clip_votings SET voting_end_at = now() - interval '1 second',
                    voting_start_at = now() - interval '2 days' WHERE window_id = $1",
        )
        .bind(window_id)
        .execute(db.pool())
        .await
        .expect("end");
        let reply = handler.handle(vote(26, window_id, ids[0])).await;
        assert!(reply.content.unwrap_or_default().contains("beendet"));
        assert_eq!(
            clips
                .store
                .cast_vote(window_id, 26, ids[0], Utc::now())
                .await
                .expect("q"),
            VoteOutcome::Closed
        );

        clips.process_contest(1).await;
        clips.process_contest(1).await;
        let voting = clips
            .store
            .voting(window_id)
            .await
            .expect("q")
            .expect("voting");
        assert_eq!(voting.status, "closed");
        assert_eq!(
            port.posts.lock().expect("lock").len(),
            2,
            "genau ein Ergebnis-Post"
        );
        assert_eq!(
            port.curator.lock().expect("lock").len(),
            1,
            "genau eine Kurator-DM"
        );
        assert_eq!(*port.edits.lock().expect("lock"), vec![1001]);

        let rows: Vec<ResultTuple> = sqlx::query_as(
            "SELECT place, submission_id, user_id, streamer_twitch_user_id, votes, source
               FROM clips.clip_contest_results WHERE window_id = $1 ORDER BY place",
        )
        .bind(window_id)
        .fetch_all(db.pool())
        .await
        .expect("results");
        let expected: Vec<ResultTuple> = vec![
            (1, Some(ids[1]), Some(11), None, 2, "discord".into()),
            (2, Some(ids[0]), Some(10), None, 1, "discord".into()),
            (3, Some(ids[2]), Some(12), None, 1, "discord".into()),
        ];
        assert_eq!(rows, expected);
        let curator = port.curator.lock().expect("lock")[0].clone();
        assert!(curator.contains("https://clips.twitch.tv/Clip11"));
        assert!(curator.contains("Credit: @u10"));
        // Wählerstimmen bleiben für Paket C lesbar
        let voters: i64 =
            sqlx::query_scalar("SELECT count(*) FROM clips.clip_votes WHERE window_id = $1")
                .bind(window_id)
                .fetch_one(db.pool())
                .await
                .expect("votes");
        assert_eq!(voters, 5);
    }

    fn twitch_request(url: &str, key: &str) -> TwitchClipRequest {
        TwitchClipRequest {
            clip_url: url.to_string(),
            streamer_twitch_user_id: "456".into(),
            streamer_login: "streamer".into(),
            submitted_by_twitch_user_id: Some("456".into()),
            title: Some("  Toller Clip ".into()),
            idempotency_key: key.to_string(),
        }
    }

    #[tokio::test]
    async fn twitch_einsendung_idempotent_und_duplikatfrei() {
        let (db, clips, _port) = setup().await;
        let req = twitch_request("https://clips.twitch.tv/Wow-1", "twitch-clip-Wow-1");
        // kein Partner
        assert_eq!(
            clips.submit_twitch(1, &req).await.expect("submit"),
            TwitchSubmitOutcome::Rejected("not_partner")
        );
        sqlx::query(
            "INSERT INTO bot.twitch_streamer_invites(streamer_login, guild_id, twitch_user_id)
             VALUES ('streamer', 1, '456')",
        )
        .execute(db.pool())
        .await
        .expect("partner");
        let TwitchSubmitOutcome::Accepted(id) = clips.submit_twitch(1, &req).await.expect("submit")
        else {
            panic!("accepted erwartet");
        };
        // gleicher Aufruf erneut → gleiche Antwort
        assert_eq!(
            clips.submit_twitch(1, &req).await.expect("replay"),
            TwitchSubmitOutcome::Accepted(id)
        );
        // anderer Schlüssel, gleicher Clip in anderer URL-Form → Duplikat
        let dup = twitch_request("https://www.twitch.tv/streamer/clip/Wow-1", "anderer-key");
        assert_eq!(
            clips.submit_twitch(1, &dup).await.expect("dup"),
            TwitchSubmitOutcome::Duplicate(id)
        );
        // gleicher Schlüssel, anderer Clip → Konflikt
        let conflict = twitch_request("https://clips.twitch.tv/Other", "twitch-clip-Wow-1");
        assert_eq!(
            clips.submit_twitch(1, &conflict).await.expect("conflict"),
            TwitchSubmitOutcome::Rejected("idempotency_conflict")
        );
        let bad = twitch_request("https://youtube.com/watch?v=1", "k3");
        assert_eq!(
            clips.submit_twitch(1, &bad).await.expect("bad"),
            TwitchSubmitOutcome::Rejected("invalid_clip_url")
        );
        // gespeicherte Zeile: kein Discord-Einsender, Credit = Streamer-Login
        let row: (
            Option<i64>,
            String,
            String,
            Option<String>,
            String,
            Option<String>,
        ) = sqlx::query_as(
            "SELECT user_id, source, credit, title, link, streamer_twitch_user_id
               FROM clips.clip_submissions WHERE id = $1",
        )
        .bind(id)
        .fetch_one(db.pool())
        .await
        .expect("row");
        assert_eq!(
            row,
            (
                None,
                "twitch".into(),
                "streamer".into(),
                Some("Toller Clip".into()),
                "https://clips.twitch.tv/Wow-1".into(),
                Some("456".into())
            )
        );
        let (start, end) = twitch_target_window(Utc::now());
        let window_id: i64 = sqlx::query_scalar(
            "SELECT window_id FROM clips.clip_window_submissions WHERE submission_id = $1",
        )
        .bind(id)
        .fetch_one(db.pool())
        .await
        .expect("window link");
        let (w_start, w_end): (DateTime<Utc>, DateTime<Utc>) =
            sqlx::query_as("SELECT start_at, end_at FROM clips.clip_windows WHERE id = $1")
                .bind(window_id)
                .fetch_one(db.pool())
                .await
                .expect("window");
        assert_eq!((w_start.timestamp(), w_end.timestamp()), (start, end));
        // Dump enthält die Twitch-Einsendung
        let rows = clips.store.dump_rows(1, window_id, start, end).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_id, "twitch:streamer");
        // Login-Fallback, solange die Twitch-ID in der Partnertabelle fehlt
        sqlx::query(
            "INSERT INTO bot.twitch_streamer_invites(streamer_login, guild_id) VALUES ('Zweiter', 1)",
        )
        .execute(db.pool())
        .await
        .expect("partner2");
        let mut second = twitch_request("https://clips.twitch.tv/Zwei", "k4");
        second.streamer_twitch_user_id = "789".into();
        second.streamer_login = "zweiter".into();
        assert!(matches!(
            clips.submit_twitch(1, &second).await.expect("second"),
            TwitchSubmitOutcome::Accepted(_)
        ));
    }

    #[tokio::test]
    async fn neustart_nach_senden_postet_nicht_doppelt() {
        let (db, clips, port) = setup().await;
        let (window_id, _ids) = ended_window(db.pool()).await;
        let ballot = select_ballot(&clips.store.window_clips(window_id).await.expect("clips"));
        assert!(clips
            .store
            .create_voting(window_id, 1, VOTING_CHANNEL_ID, &ballot)
            .await
            .expect("create"));
        // zweites Anlegen ist ein No-op
        assert!(!clips
            .store
            .create_voting(window_id, 1, VOTING_CHANNEL_ID, &ballot)
            .await
            .expect("create2"));
        // Zustand wie nach Absturz: beansprucht, Zeiten geplant, Post im Kanal
        sqlx::query(
            "UPDATE clips.clip_votings SET status = 'publishing',
                    publish_claimed_at = now() - interval '1 hour',
                    voting_start_at = now() - interval '1 hour',
                    voting_end_at = now() + interval '47 hours'
              WHERE window_id = $1",
        )
        .bind(window_id)
        .execute(db.pool())
        .await
        .expect("crash state");
        *port.footer_hit.lock().expect("lock") = Some((voting_footer(window_id), 4242));
        clips.process_contest(1).await;
        assert!(port.posts.lock().expect("lock").is_empty());
        let voting = clips
            .store
            .voting(window_id)
            .await
            .expect("q")
            .expect("voting");
        assert_eq!(voting.status, "open");
        assert_eq!(voting.message_id, Some(4242));
    }

    #[tokio::test]
    async fn leeres_fenster_wird_uebersprungen_und_alte_fenster_bleiben_ruhig() {
        let (db, clips, port) = setup().await;
        let now = Utc::now();
        for (start_days, end_hours) in [(7_i64, 1_i64), (14, 24 * 7)] {
            sqlx::query(
                "INSERT INTO clips.clip_windows(guild_id, start_at, end_at) VALUES (1, $1, $2)",
            )
            .bind(now - chrono::Duration::days(start_days))
            .bind(now - chrono::Duration::hours(end_hours))
            .execute(db.pool())
            .await
            .expect("window");
        }
        clips.process_contest(1).await;
        let statuses: Vec<String> =
            sqlx::query_scalar("SELECT status FROM clips.clip_votings ORDER BY window_id")
                .fetch_all(db.pool())
                .await
                .expect("statuses");
        assert_eq!(statuses, vec!["skipped".to_string()]);
        assert!(port.posts.lock().expect("lock").is_empty());
    }
}
