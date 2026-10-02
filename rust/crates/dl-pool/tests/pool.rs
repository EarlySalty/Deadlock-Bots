use chrono::{Duration, Utc};
use dl_pool::*;
use sqlx::{PgPool, Row};

#[path = "../../../test-support/peer_database.rs"]
mod peer_database;

fn scope(user: i64) -> Scope {
    Scope {
        guild_id: 1289721245281292288,
        discord_id: user,
    }
}

fn preferences() -> Preferences {
    Preferences {
        modes: vec![Mode::Casual, Mode::Ranked],
        availability: vec![Availability {
            weekday: 4,
            start_minute: 1080,
            end_minute: 1440,
        }],
        ..Preferences::default()
    }
}

async fn profiles(pool: &PgPool) -> PoolStore {
    let store = PoolStore::new(pool.clone());
    for user in [100, 200, 300] {
        store
            .save_preferences(scope(user), &preferences(), true)
            .await
            .expect("Profil gespeichert");
    }
    store
}

fn snapshot(steam_id: &str) -> ApiSnapshot {
    let now = Utc::now();
    let mut heatmap = vec![0; 168];
    heatmap[4 * 24 + 18] = 2;
    ApiSnapshot {
        steam_id: steam_id.into(),
        rank_tier: Some(5),
        rank_subtier: Some(2),
        games_played: None,
        total_play_seconds: None,
        observed_games: 2,
        observed_play_seconds: 3600,
        window_start: now - Duration::days(30),
        window_end: now,
        fetched_at: now,
        heatmap,
    }
}

#[tokio::test]
async fn profile_zeitfenster_und_dm_default_bleiben_guildgebunden() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let profile = store
        .own_profile(scope(100))
        .await
        .expect("Profil lesbar")
        .expect("Profil vorhanden");
    assert!(!profile.dm_opt_in);
    assert_eq!(profile.modes, vec![Mode::Casual, Mode::Ranked]);
    assert_eq!(
        store.availability(scope(100)).await.expect("Zeitfenster"),
        preferences().availability
    );
    let other_guild = Scope {
        guild_id: 77,
        ..scope(100)
    };
    assert!(store
        .own_profile(other_guild)
        .await
        .expect("Andere Guild")
        .is_none());
    assert!(store
        .list_profiles(77, &PoolFilter::default())
        .await
        .expect("Anderer Pool")
        .is_empty());
    store.set_dm_opt_in(scope(100), true).await.expect("Opt-in");
    store
        .save_preferences(scope(100), &preferences(), true)
        .await
        .expect("Präferenzen geändert");
    assert!(
        store
            .own_profile(scope(100))
            .await
            .expect("Profil")
            .expect("Vorhanden")
            .dm_opt_in
    );
    store.delete_profile(scope(100)).await.expect("Gelöscht");
    store
        .save_preferences(scope(100), &preferences(), true)
        .await
        .expect("Neu angelegt");
    assert!(
        !store
            .own_profile(scope(100))
            .await
            .expect("Profil")
            .expect("Vorhanden")
            .dm_opt_in
    );
}

#[tokio::test]
async fn entwurf_und_fremde_bevorzugte_spieler_sind_nicht_oeffentlich() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    store
        .save_preferences(scope(300), &Preferences::default(), false)
        .await
        .expect("Entwurf");
    assert!(store
        .public_profile(scope(300))
        .await
        .expect("Öffentlich")
        .is_none());
    assert_eq!(
        store
            .list_profiles(scope(100).guild_id, &PoolFilter::default())
            .await
            .expect("Pool")
            .len(),
        2
    );
    let mut p = preferences();
    p.preferred_discord_ids = vec![200];
    store
        .save_preferences(scope(100), &p, true)
        .await
        .expect("Bevorzugter Spieler");
    assert_eq!(
        store.preferred_players(scope(100)).await.expect("Auswahl"),
        vec![200]
    );
    p.preferred_discord_ids = vec![300];
    assert!(matches!(
        store.save_preferences(scope(100), &p, true).await,
        Err(PoolError::NotFound)
    ));
    p.preferred_discord_ids = vec![999];
    assert!(matches!(
        store.save_preferences(scope(100), &p, true).await,
        Err(PoolError::NotFound)
    ));
    assert_eq!(
        store.preferred_players(scope(100)).await.expect("Rollback"),
        vec![200]
    );
}

#[tokio::test]
async fn steam_openid_link_braucht_keine_bot_freundschaft_und_keine_uebernahme() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    store
        .link_verified_steam(scope(100), "76561198000000100")
        .await
        .expect("Steam verknüpft");
    let row = sqlx::query("SELECT verified,is_steam_friend,friend_bot_account_id FROM core.steam_links WHERE discord_id=100")
        .fetch_one(&*db).await.expect("Link");
    assert!(row.get::<bool, _>("verified"));
    assert!(!row.get::<bool, _>("is_steam_friend"));
    assert!(row.get::<Option<i16>, _>("friend_bot_account_id").is_none());
    assert!(store
        .link_verified_steam(scope(200), "76561198000000100")
        .await
        .is_err());
    assert_eq!(
        store
            .steam_links_for_ingest(scope(100).guild_id)
            .await
            .expect("Ingest-Links")
            .len(),
        1
    );
    store
        .link_verified_steam(scope(100), "76561198000000400")
        .await
        .expect("Zweites Konto");
    assert!(matches!(
        store
            .store_api_snapshot(scope(100), &snapshot("76561198000000400"), &[])
            .await,
        Err(PoolError::UnverifiedSteamLink)
    ));
}

#[tokio::test]
async fn ingest_ist_atomar_optional_und_verwirft_alte_standbilder() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let snap = snapshot("76561198000000100");
    assert!(matches!(
        store.store_api_snapshot(scope(100), &snap, &[]).await,
        Err(PoolError::UnverifiedSteamLink)
    ));
    store
        .link_verified_steam(scope(100), &snap.steam_id)
        .await
        .expect("Link");
    let game = Match {
        match_id: 500,
        started_at: snap.window_end - Duration::hours(1),
        duration_seconds: Some(1800),
        mode: None,
    };
    store
        .store_api_snapshot(scope(100), &snap, std::slice::from_ref(&game))
        .await
        .expect("Ingest");
    store
        .store_api_snapshot(scope(100), &snap, &[game])
        .await
        .expect("Wiederholt");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pool.matches")
        .fetch_one(&*db)
        .await
        .expect("Matches");
    assert_eq!(count, 1);
    let public = store
        .public_profile(scope(100))
        .await
        .expect("Profil")
        .expect("Öffentlich");
    assert_eq!(public.rank_tier, Some(5));
    assert_eq!(public.total_play_seconds, None);
    assert_eq!(public.observed_play_seconds, Some(3600));
    assert_eq!(public.heatmap.expect("Heatmap")[4 * 24 + 18], 2);
    let mut stale = snap.clone();
    stale.fetched_at -= Duration::seconds(10);
    stale.window_end -= Duration::seconds(10);
    assert!(matches!(
        store.store_api_snapshot(scope(100), &stale, &[]).await,
        Err(PoolError::StaleSnapshot)
    ));
    let mut invalid = snap.clone();
    invalid.heatmap.pop();
    assert!(matches!(
        store.store_api_snapshot(scope(100), &invalid, &[]).await,
        Err(PoolError::Invalid(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pool.matches")
            .fetch_one(&*db)
            .await
            .expect("Unverändert"),
        1
    );
}

#[tokio::test]
async fn filter_verbindet_modus_rang_und_lokale_zeit() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let snap = snapshot("76561198000000100");
    store
        .link_verified_steam(scope(100), &snap.steam_id)
        .await
        .expect("Link");
    store
        .store_api_snapshot(scope(100), &snap, &[])
        .await
        .expect("Ingest");
    let mut filter = PoolFilter {
        mode: Some(Mode::Ranked),
        min_rank: Some(4),
        max_rank: Some(6),
        weekday: Some(4),
        minute: Some(1200),
        timezone: Some("Europe/Berlin".into()),
        ..PoolFilter::default()
    };
    assert_eq!(
        store
            .list_profiles(scope(100).guild_id, &filter)
            .await
            .expect("Filter")
            .len(),
        1
    );
    filter.minute = Some(1000);
    assert!(store
        .list_profiles(scope(100).guild_id, &filter)
        .await
        .expect("Außerhalb")
        .is_empty());
    filter.minute = Some(1200);
    filter.mode = Some(Mode::StreetBrawl);
    assert!(store
        .list_profiles(scope(100).guild_id, &filter)
        .await
        .expect("Modus")
        .is_empty());
}

#[tokio::test]
async fn mitspieler_paare_zeigen_nur_veroeffentlichte_poolprofile() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    store
        .link_verified_steam(scope(100), "76561198000000100")
        .await
        .expect("Erster Link");
    store
        .link_verified_steam(scope(200), "76561198000000200")
        .await
        .expect("Zweiter Link");
    let now = Utc::now();
    let pair = CoPlayer {
        discord_id: 100,
        target_discord_id: 200,
        games_together: 3,
        last_played_at: now,
        window_start: now - Duration::days(30),
        window_end: now,
        fetched_at: now,
    };
    store
        .store_co_player(scope(100).guild_id, &pair)
        .await
        .expect("Paar");
    store
        .store_co_player(scope(100).guild_id, &pair)
        .await
        .expect("Wiederholtes Paar");
    assert_eq!(
        store
            .recently_played_with(scope(200), 10)
            .await
            .expect("Mitspieler")
            .len(),
        1
    );
    store
        .save_preferences(scope(200), &Preferences::default(), false)
        .await
        .expect("Versteckt");
    assert!(store
        .recently_played_with(scope(100), 10)
        .await
        .expect("Versteckte Mitspieler")
        .is_empty());
    store.delete_profile(scope(200)).await.expect("Gelöscht");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pool.co_players")
            .fetch_one(&*db)
            .await
            .expect("Paar gelöscht"),
        0
    );
}

#[tokio::test]
async fn session_feedback_braucht_teilnahme_und_abgeschlossene_session() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let s = store
        .create_session(scope(100), &[200], Mode::Casual)
        .await
        .expect("Session");
    assert_eq!(s.expires_at - s.created_at, Duration::hours(1));
    assert!(store
        .participant_session(scope(300), s.session_id)
        .await
        .expect("Fremder Nutzer")
        .is_none());
    assert!(store
        .participants(scope(300), s.session_id)
        .await
        .expect("Fremde Teilnehmer")
        .is_empty());
    let feedback = Feedback {
        friendly: true,
        play_again: true,
        ..Feedback::default()
    };
    assert!(matches!(
        store
            .save_feedback(scope(100), s.session_id, feedback)
            .await,
        Err(PoolError::NotFound)
    ));
    store
        .attach_channel(s.guild_id, s.session_id, 999001)
        .await
        .expect("Kanal");
    store
        .record_join(
            scope(100),
            s.session_id,
            s.created_at + Duration::seconds(1),
        )
        .await
        .expect("Beitritt");
    store
        .finish_session(
            s.guild_id,
            s.session_id,
            SessionStatus::Ended,
            s.created_at + Duration::minutes(10),
        )
        .await
        .expect("Beendet");
    assert!(matches!(
        store
            .save_feedback(scope(200), s.session_id, feedback)
            .await,
        Err(PoolError::NotFound)
    ));
    assert!(matches!(
        store
            .save_feedback(scope(300), s.session_id, feedback)
            .await,
        Err(PoolError::NotFound)
    ));
    store
        .save_feedback(scope(100), s.session_id, feedback)
        .await
        .expect("Feedback");
    assert!(
        store
            .own_feedback(scope(100), s.session_id)
            .await
            .expect("Eigenes Feedback")
            .expect("Vorhanden")
            .friendly
    );
    assert!(store
        .own_feedback(scope(200), s.session_id)
        .await
        .expect("Fremdes Feedback")
        .is_none());
}

#[tokio::test]
async fn leere_session_verfaellt_nach_einer_stunde_auch_ohne_kanal() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let s = store
        .create_session(scope(100), &[200], Mode::Ranked)
        .await
        .expect("Session");
    assert!(matches!(
        store
            .finish_session(
                s.guild_id,
                s.session_id,
                SessionStatus::Expired,
                s.created_at + Duration::minutes(59)
            )
            .await,
        Err(PoolError::InvalidSessionState)
    ));
    assert_eq!(
        store
            .due_sessions(s.guild_id, s.expires_at)
            .await
            .expect("Fällig")
            .len(),
        1
    );
    let ended = store
        .finish_session(
            s.guild_id,
            s.session_id,
            SessionStatus::Expired,
            s.expires_at,
        )
        .await
        .expect("Abgelaufen");
    assert_eq!(ended.status, SessionStatus::Expired);
    assert!(store
        .due_sessions(s.guild_id, s.expires_at)
        .await
        .expect("Nicht mehr fällig")
        .is_empty());
}

#[tokio::test]
async fn profil_loeschen_entfernt_gemeinsame_session_und_alle_poolspuren() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let snap = snapshot("76561198000000200");
    store
        .link_verified_steam(scope(200), &snap.steam_id)
        .await
        .expect("Link");
    store
        .store_api_snapshot(scope(200), &snap, &[])
        .await
        .expect("Ingest");
    let mut p = preferences();
    p.preferred_discord_ids = vec![200];
    store
        .save_preferences(scope(100), &p, true)
        .await
        .expect("Präferenz");
    let s = store
        .create_session(scope(100), &[200], Mode::Casual)
        .await
        .expect("Session");
    store
        .attach_channel(s.guild_id, s.session_id, 999002)
        .await
        .expect("Kanal");
    store
        .record_join(
            scope(100),
            s.session_id,
            s.created_at + Duration::seconds(1),
        )
        .await
        .expect("Beitritt");
    store
        .finish_session(
            s.guild_id,
            s.session_id,
            SessionStatus::Ended,
            s.created_at + Duration::minutes(5),
        )
        .await
        .expect("Beendet");
    store
        .save_feedback(scope(100), s.session_id, Feedback::default())
        .await
        .expect("Fremdes Feedback");
    assert_eq!(store.delete_profile(scope(200)).await.expect("Gelöscht"), 1);
    assert_eq!(
        store.delete_profile(scope(200)).await.expect("Idempotent"),
        0
    );
    for table in [
        "api_snapshots",
        "sessions",
        "session_participants",
        "feedback",
        "preferred_players",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM pool.{table}"))
                .fetch_one(&*db)
                .await
                .expect("Leere Tabelle"),
            0,
            "{table}"
        );
    }
    assert!(store
        .own_profile(scope(100))
        .await
        .expect("Fremdes Profil")
        .is_some());
    assert_eq!(
        store
            .steam_links_for_ingest(s.guild_id)
            .await
            .expect("Pool-Linkfilter")
            .len(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM core.steam_links WHERE discord_id=200")
            .fetch_one(&*db)
            .await
            .expect("Globaler Steam-Link bleibt"),
        1
    );
}

#[tokio::test]
async fn optout_sperrt_alle_pool_writes_und_oeffentliche_leser() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    sqlx::query("INSERT INTO core.user_privacy(user_id,opted_out,deleted_at,updated_at) VALUES(200,true,now(),now())")
        .execute(&*db).await.expect("Grabstein");
    assert!(matches!(
        store
            .save_preferences(scope(200), &preferences(), true)
            .await,
        Err(PoolError::OptedOut)
    ));
    assert!(matches!(
        store.set_dm_opt_in(scope(200), true).await,
        Err(PoolError::OptedOut)
    ));
    assert!(matches!(
        store
            .link_verified_steam(scope(200), "76561198000000200")
            .await,
        Err(PoolError::OptedOut)
    ));
    assert!(matches!(
        store
            .store_api_snapshot(scope(200), &snapshot("76561198000000200"), &[])
            .await,
        Err(PoolError::OptedOut)
    ));
    assert!(matches!(
        store.create_session(scope(100), &[200], Mode::Casual).await,
        Err(PoolError::OptedOut)
    ));
    assert!(store
        .public_profile(scope(200))
        .await
        .expect("Öffentlich")
        .is_none());
    assert_eq!(
        store
            .list_profiles(scope(100).guild_id, &PoolFilter::default())
            .await
            .expect("Pool")
            .len(),
        2
    );
    let mut tx = db.begin().await.expect("Löschtransaktion");
    assert_eq!(
        delete_all_for_user_tx(&mut tx, 200)
            .await
            .expect("Alle Guilds gelöscht"),
        1
    );
    tx.rollback().await.expect("Rollback");
    assert!(store
        .own_profile(scope(200))
        .await
        .expect("Rollback erhält Profil")
        .is_some());
}

#[tokio::test]
async fn privacy_lock_verhindert_wiederanlage_nach_loeschung() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let mut tx = db.begin().await.expect("Transaktion");
    dl_central_db::lock_user_privacy(&mut tx, 200)
        .await
        .expect("Lock");
    let writer = tokio::spawn(async move {
        store
            .save_preferences(scope(200), &preferences(), true)
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    assert!(!writer.is_finished());
    delete_all_for_user_tx(&mut tx, 200)
        .await
        .expect("Gelöscht");
    sqlx::query("INSERT INTO core.user_privacy(user_id,opted_out,deleted_at,updated_at) VALUES(200,true,now(),now())")
        .execute(&mut *tx).await.expect("Grabstein");
    tx.commit().await.expect("Commit");
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), writer)
        .await
        .expect("Kein Deadlock")
        .expect("Writer");
    assert!(matches!(result, Err(PoolError::OptedOut)));
}

#[tokio::test]
async fn steam_unlink_entfernt_statistik_und_paare() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    store
        .link_verified_steam(scope(200), "76561198000000200")
        .await
        .expect("Zweiter Link");
    let snap = snapshot("76561198000000100");
    store
        .link_verified_steam(scope(100), &snap.steam_id)
        .await
        .expect("Link");
    store
        .store_api_snapshot(scope(100), &snap, &[])
        .await
        .expect("Ingest");
    let pair = CoPlayer {
        discord_id: 100,
        target_discord_id: 200,
        games_together: 1,
        last_played_at: snap.window_end,
        window_start: snap.window_start,
        window_end: snap.window_end,
        fetched_at: snap.fetched_at,
    };
    store
        .store_co_player(scope(100).guild_id, &pair)
        .await
        .expect("Paar");
    sqlx::query("DELETE FROM core.steam_links WHERE discord_id=100")
        .execute(&*db)
        .await
        .expect("Unlink");
    assert!(store
        .public_profile(scope(100))
        .await
        .expect("Profil")
        .expect("Vorhanden")
        .rank_tier
        .is_none());
    assert!(store
        .recently_played_with(scope(200), 10)
        .await
        .expect("Paare")
        .is_empty());
    assert!(matches!(
        store.store_co_player(scope(100).guild_id, &pair).await,
        Err(PoolError::UnverifiedSteamLink)
    ));
}

#[tokio::test]
async fn db_constraints_verhindern_fremde_guild_und_falsche_heatmap() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let s = store
        .create_session(scope(100), &[200], Mode::Casual)
        .await
        .expect("Session");
    assert!(sqlx::query(
        "INSERT INTO pool.session_participants(guild_id,session_id,discord_id) VALUES(77,$1,300)"
    )
    .bind(s.session_id)
    .execute(&*db)
    .await
    .is_err());
    assert!(
        sqlx::query("INSERT INTO pool.availability VALUES($1,100,7,0,60)")
            .bind(scope(100).guild_id)
            .execute(&*db)
            .await
            .is_err()
    );
    store
        .link_verified_steam(scope(100), "76561198000000100")
        .await
        .expect("Link");
    store
        .store_api_snapshot(scope(100), &snapshot("76561198000000100"), &[])
        .await
        .expect("Ingest");
    assert!(
        sqlx::query("UPDATE pool.api_snapshots SET heatmap=ARRAY[1,2]")
            .execute(&*db)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn eigene_sessionliste_ueberlebt_neuen_leser_und_zeigt_nur_eigenes_feedback() {
    let db = peer_database::database().await;
    let store = profiles(&db).await;
    let mut ended = Vec::new();
    for minutes in [5, 10] {
        let s = store
            .create_session(scope(100), &[200], Mode::Casual)
            .await
            .expect("Session");
        store
            .attach_channel(s.guild_id, s.session_id, 900010 + minutes)
            .await
            .expect("Kanal");
        store
            .record_join(
                scope(100),
                s.session_id,
                s.created_at + Duration::seconds(1),
            )
            .await
            .expect("Beitritt");
        store
            .record_join(
                scope(200),
                s.session_id,
                s.created_at + Duration::seconds(2),
            )
            .await
            .expect("Zweiter Beitritt");
        store
            .finish_session(
                s.guild_id,
                s.session_id,
                SessionStatus::Ended,
                s.created_at + Duration::minutes(minutes),
            )
            .await
            .expect("Beendet");
        ended.push(s);
    }
    store
        .save_feedback(
            scope(200),
            ended[1].session_id,
            Feedback {
                friendly: true,
                ..Feedback::default()
            },
        )
        .await
        .expect("Feedback des anderen Nutzers");
    store
        .save_feedback(
            scope(100),
            ended[0].session_id,
            Feedback {
                play_again: true,
                ..Feedback::default()
            },
        )
        .await
        .expect("Eigenes Feedback");
    let pending = store
        .create_session(scope(100), &[200], Mode::Ranked)
        .await
        .expect("Offene Session");
    let reader = PoolStore::new(db.pool().clone());
    let sessions = reader
        .own_sessions(scope(100), 0, 0)
        .await
        .expect("Persistente eigene Liste");
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[0].session.session_id, ended[1].session_id);
    assert!(sessions[0].feedback.is_none());
    assert!(sessions[1].feedback.expect("Eigene Antwort").play_again);
    assert!(!sessions
        .iter()
        .any(|entry| entry.session.session_id == pending.session_id));
    assert_eq!(
        reader
            .own_sessions(scope(100), 1, 1)
            .await
            .expect("Zweite Seite")[0]
            .session
            .session_id,
        ended[0].session_id
    );
    assert!(reader
        .own_sessions(scope(300), 10, 0)
        .await
        .expect("Fremde Sessionliste")
        .is_empty());
    assert!(reader
        .own_sessions(
            Scope {
                guild_id: 77,
                ..scope(100)
            },
            10,
            0
        )
        .await
        .expect("Andere Guild")
        .is_empty());
    assert!(matches!(
        reader.own_sessions(scope(100), 1, -1).await,
        Err(PoolError::Invalid(_))
    ));
    sqlx::query(
        "INSERT INTO core.user_privacy(user_id,opted_out,updated_at) VALUES(100,true,now())",
    )
    .execute(&*db)
    .await
    .expect("Opt-out");
    assert!(reader
        .own_sessions(scope(100), 10, 0)
        .await
        .expect("Privacy-Grenze")
        .is_empty());
}
