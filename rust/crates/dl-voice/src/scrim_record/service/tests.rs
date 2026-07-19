use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use dl_discord::{BridgeInteraction, InteractionHandler, InteractionRouter, VoiceEvent};
use tempfile::TempDir;
use tokio::sync::Semaphore;

use super::*;
use crate::scrim_record::{RecorderIdentity, SCRIM_GUILD_ID, TEAM_VOICE_CHANNELS};

#[derive(Default)]
struct MockPortState {
    voice_channels: HashMap<u64, Result<Option<u64>, String>>,
    member_counts: HashMap<u64, Result<Option<usize>, String>>,
    text_failures_remaining: usize,
    successful_posts: Vec<(u64, String)>,
    upload_attempts: Vec<(u64, PathBuf)>,
    fail_upload: bool,
}

struct MockPort {
    state: StdMutex<MockPortState>,
    block_next_text: AtomicBool,
    text_entered: Semaphore,
    release_text: Semaphore,
}

impl Default for MockPort {
    fn default() -> Self {
        Self {
            state: StdMutex::new(MockPortState::default()),
            block_next_text: AtomicBool::new(false),
            text_entered: Semaphore::new(0),
            release_text: Semaphore::new(0),
        }
    }
}

impl MockPort {
    fn set_voice_channel(&self, user_id: u64, channel_id: Option<u64>) {
        self.state
            .lock()
            .expect("mock port lock")
            .voice_channels
            .insert(user_id, Ok(channel_id));
    }

    fn set_member_count(&self, channel_id: u64, count: Option<usize>) {
        self.state
            .lock()
            .expect("mock port lock")
            .member_counts
            .insert(channel_id, Ok(count));
    }

    fn fail_next_text_post(&self) {
        self.state
            .lock()
            .expect("mock port lock")
            .text_failures_remaining += 1;
    }

    fn block_next_text_post(&self) {
        self.block_next_text.store(true, Ordering::SeqCst);
    }

    async fn wait_for_text_post(&self) {
        self.text_entered
            .acquire()
            .await
            .expect("text-entered semaphore")
            .forget();
    }

    fn unblock_text_post(&self) {
        self.release_text.add_permits(1);
    }

    fn set_upload_failure(&self, fail: bool) {
        self.state.lock().expect("mock port lock").fail_upload = fail;
    }

    fn successful_post_count(&self) -> usize {
        self.state
            .lock()
            .expect("mock port lock")
            .successful_posts
            .len()
    }

    fn upload_attempts(&self) -> Vec<(u64, PathBuf)> {
        self.state
            .lock()
            .expect("mock port lock")
            .upload_attempts
            .clone()
    }
}

#[async_trait::async_trait]
impl ScrimRecordPort for MockPort {
    async fn current_voice_channel(
        &self,
        _guild_id: u64,
        user_id: u64,
    ) -> Result<Option<u64>, String> {
        self.state
            .lock()
            .expect("mock port lock")
            .voice_channels
            .get(&user_id)
            .cloned()
            .unwrap_or(Ok(None))
    }

    async fn non_bot_member_count(
        &self,
        _guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<Option<usize>, String> {
        self.state
            .lock()
            .expect("mock port lock")
            .member_counts
            .get(&voice_channel_id)
            .cloned()
            .unwrap_or(Ok(None))
    }

    async fn post_text(&self, channel_id: u64, content: &str) -> Result<(), String> {
        if self.block_next_text.swap(false, Ordering::SeqCst) {
            self.text_entered.add_permits(1);
            self.release_text
                .acquire()
                .await
                .expect("text-release semaphore")
                .forget();
        }
        let mut state = self.state.lock().expect("mock port lock");
        if state.text_failures_remaining > 0 {
            state.text_failures_remaining -= 1;
            return Err("text failure".to_string());
        }
        state
            .successful_posts
            .push((channel_id, content.to_string()));
        Ok(())
    }

    async fn upload_attachment(&self, channel_id: u64, path: &Path) -> Result<(), String> {
        let mut state = self.state.lock().expect("mock port lock");
        state.upload_attempts.push((channel_id, path.to_path_buf()));
        if state.fail_upload {
            Err("upload failure".to_string())
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
struct BackendCall {
    identity: RecorderIdentity,
    guild_id: u64,
    voice_channel_id: u64,
    wav_path: Option<PathBuf>,
}

struct MockBackendState {
    ready: [Result<bool, String>; 2],
    health: [Result<(), String>; 2],
    fail_starts_remaining: usize,
    starts: Vec<BackendCall>,
    stops: Vec<BackendCall>,
}

impl Default for MockBackendState {
    fn default() -> Self {
        Self {
            ready: [Ok(true), Ok(true)],
            health: [Ok(()), Ok(())],
            fail_starts_remaining: 0,
            starts: Vec::new(),
            stops: Vec::new(),
        }
    }
}

struct MockBackend {
    state: StdMutex<MockBackendState>,
    block_stop: AtomicBool,
    stop_entered: Semaphore,
    release_stop: Semaphore,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self {
            state: StdMutex::new(MockBackendState::default()),
            block_stop: AtomicBool::new(false),
            stop_entered: Semaphore::new(0),
            release_stop: Semaphore::new(0),
        }
    }
}

impl MockBackend {
    fn identity_index(identity: RecorderIdentity) -> usize {
        match identity {
            RecorderIdentity::MainBot => 0,
            RecorderIdentity::DiscordTokenWorker => 1,
        }
    }

    fn set_ready(&self, identity: RecorderIdentity, ready: bool) {
        self.state.lock().expect("mock backend lock").ready[Self::identity_index(identity)] =
            Ok(ready);
    }

    fn fail_next_start(&self) {
        self.state
            .lock()
            .expect("mock backend lock")
            .fail_starts_remaining += 1;
    }

    fn fail_health(&self, identity: RecorderIdentity) {
        self.state.lock().expect("mock backend lock").health[Self::identity_index(identity)] =
            Err("recording unhealthy".to_string());
    }

    fn starts(&self) -> Vec<BackendCall> {
        self.state.lock().expect("mock backend lock").starts.clone()
    }

    fn stops(&self) -> Vec<BackendCall> {
        self.state.lock().expect("mock backend lock").stops.clone()
    }

    fn block_stop(&self) {
        self.block_stop.store(true, Ordering::SeqCst);
    }

    async fn wait_for_stop(&self) {
        self.stop_entered
            .acquire()
            .await
            .expect("stop-entered semaphore")
            .forget();
    }

    fn unblock_stop(&self) {
        self.release_stop.add_permits(1);
    }
}

#[async_trait::async_trait]
impl RecordingBackend for MockBackend {
    async fn readiness(&self, identity: RecorderIdentity) -> Result<bool, String> {
        self.state.lock().expect("mock backend lock").ready[Self::identity_index(identity)].clone()
    }

    async fn health(
        &self,
        identity: RecorderIdentity,
        _guild_id: u64,
        _voice_channel_id: u64,
    ) -> Result<(), String> {
        self.state.lock().expect("mock backend lock").health[Self::identity_index(identity)].clone()
    }

    async fn start(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
        wav_path: &Path,
    ) -> Result<(), String> {
        tokio::fs::write(wav_path, b"wav")
            .await
            .map_err(|err| err.to_string())?;
        let mut state = self.state.lock().expect("mock backend lock");
        state.starts.push(BackendCall {
            identity,
            guild_id,
            voice_channel_id,
            wav_path: Some(wav_path.to_path_buf()),
        });
        if state.fail_starts_remaining > 0 {
            state.fail_starts_remaining -= 1;
            Err("start failure".to_string())
        } else {
            Ok(())
        }
    }

    async fn stop(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<(), String> {
        self.state
            .lock()
            .expect("mock backend lock")
            .stops
            .push(BackendCall {
                identity,
                guild_id,
                voice_channel_id,
                wav_path: None,
            });
        if self.block_stop.load(Ordering::SeqCst) {
            self.stop_entered.add_permits(1);
            self.release_stop
                .acquire()
                .await
                .expect("stop-release semaphore")
                .forget();
        }
        Ok(())
    }
}

#[derive(Default)]
struct MockTranscoder {
    calls: StdMutex<Vec<(PathBuf, PathBuf)>>,
}

impl MockTranscoder {
    fn call_count(&self) -> usize {
        self.calls.lock().expect("mock transcoder lock").len()
    }
}

#[async_trait::async_trait]
impl AudioTranscoder for MockTranscoder {
    async fn transcode(&self, wav_path: &Path, mp3_path: &Path) -> Result<(), String> {
        self.calls
            .lock()
            .expect("mock transcoder lock")
            .push((wav_path.to_path_buf(), mp3_path.to_path_buf()));
        tokio::fs::write(mp3_path, b"mp3")
            .await
            .map_err(|err| err.to_string())
    }
}

struct Harness {
    _temp_dir: TempDir,
    port: Arc<MockPort>,
    backend: Arc<MockBackend>,
    transcoder: Arc<MockTranscoder>,
    recorder: Arc<ScrimRecorder>,
}

impl Harness {
    fn new() -> Self {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let port = Arc::new(MockPort::default());
        let backend = Arc::new(MockBackend::default());
        let transcoder = Arc::new(MockTranscoder::default());
        let recorder = ScrimRecorder::new(
            port.clone(),
            backend.clone(),
            transcoder.clone(),
            temp_dir.path().to_path_buf(),
        );
        Self {
            _temp_dir: temp_dir,
            port,
            backend,
            transcoder,
            recorder,
        }
    }

    fn set_user_channel(&self, user_id: u64, team_index: usize) -> u64 {
        let config = TEAM_VOICE_CHANNELS[team_index];
        self.port
            .set_voice_channel(user_id, Some(config.voice_channel_id));
        config.team_role_id
    }

    fn assert_temp_files_removed(&self) {
        let files = std::fs::read_dir(self._temp_dir.path())
            .expect("read tempdir")
            .collect::<Result<Vec<_>, _>>()
            .expect("read temp entries");
        assert!(files.is_empty(), "temporary audio files remain");
    }
}

#[tokio::test]
async fn wrong_channel_and_missing_role_are_distinct() {
    let harness = Harness::new();
    harness.port.set_voice_channel(1, Some(42));
    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[]).await,
        Err(StartError::UnknownChannel)
    );

    harness
        .port
        .set_voice_channel(1, Some(TEAM_VOICE_CHANNELS[0].voice_channel_id));
    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[]).await,
        Err(StartError::NotPermitted)
    );
}

#[tokio::test]
async fn two_starts_claim_both_slots_and_third_is_no_capacity() {
    let harness = Harness::new();
    let role_1 = harness.set_user_channel(1, 0);
    let role_2 = harness.set_user_channel(2, 1);
    let role_3 = harness.set_user_channel(3, 2);

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role_1]).await,
        Ok(RecorderIdentity::MainBot)
    );
    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 2, &[role_2]).await,
        Ok(RecorderIdentity::DiscordTokenWorker)
    );
    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 3, &[role_3]).await,
        Err(StartError::NoCapacity)
    );
    assert_eq!(harness.recorder.session_count().await, 2);
}

#[tokio::test]
async fn offline_recorders_are_no_capacity() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    harness.backend.set_ready(RecorderIdentity::MainBot, false);
    harness
        .backend
        .set_ready(RecorderIdentity::DiscordTokenWorker, false);

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Err(StartError::NoCapacity)
    );
}

#[tokio::test]
async fn duplicate_channel_start_is_rejected() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    harness.set_user_channel(2, 0);

    assert!(harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .is_ok());
    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 2, &[role]).await,
        Err(StartError::AlreadyRecording)
    );
}

#[tokio::test]
async fn backend_start_failure_rolls_back_and_frees_slot() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    harness.backend.fail_next_start();

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Err(StartError::SetupFailed)
    );
    assert_eq!(harness.recorder.session_count().await, 0);
    assert_eq!(harness.backend.stops().len(), 1);
    harness.assert_temp_files_removed();

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Ok(RecorderIdentity::MainBot)
    );
}

#[tokio::test]
async fn consent_failure_stops_cleans_and_frees_slot() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    harness.port.fail_next_text_post();

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Err(StartError::SetupFailed)
    );
    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.recorder.session_count().await, 0);
    harness.assert_temp_files_removed();

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Ok(RecorderIdentity::MainBot)
    );
}

#[tokio::test]
async fn consent_failure_cannot_be_stolen_by_parallel_stop_triggers() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness.port.set_member_count(channel_id, Some(0));
    harness.port.fail_next_text_post();
    harness.port.block_next_text_post();

    let recorder = harness.recorder.clone();
    let start = tokio::spawn(async move { recorder.start(SCRIM_GUILD_ID, 1, &[role]).await });
    harness.port.wait_for_text_post().await;

    let ((), manual_won) = tokio::join!(
        harness.recorder.handle_voice_event(VoiceEvent::Leave {
            guild_id: SCRIM_GUILD_ID,
            user_id: 2,
            channel_id,
        }),
        harness
            .recorder
            .stop_for_channel(channel_id, StopTrigger::Manual),
    );
    let stops_before_consent_result = harness.backend.stops().len();
    harness.port.unblock_text_post();

    assert_eq!(
        start.await.expect("start task"),
        Err(StartError::SetupFailed)
    );
    assert!(!manual_won);
    assert_eq!(stops_before_consent_result, 0);
    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.transcoder.call_count(), 0);
    assert!(harness.port.upload_attempts().is_empty());
    assert_eq!(harness.recorder.session_count().await, 0);
    harness.assert_temp_files_removed();

    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Ok(RecorderIdentity::MainBot)
    );
}

#[tokio::test]
async fn leave_during_consent_rechecks_empty_channel_after_success() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness.port.set_member_count(channel_id, Some(0));
    harness.port.block_next_text_post();

    let recorder = harness.recorder.clone();
    let start = tokio::spawn(async move { recorder.start(SCRIM_GUILD_ID, 1, &[role]).await });
    harness.port.wait_for_text_post().await;
    harness
        .recorder
        .handle_voice_event(VoiceEvent::Leave {
            guild_id: SCRIM_GUILD_ID,
            user_id: 2,
            channel_id,
        })
        .await;
    let stops_before_consent = harness.backend.stops().len();
    harness.port.unblock_text_post();

    assert_eq!(
        start.await.expect("start task"),
        Ok(RecorderIdentity::MainBot)
    );
    assert_eq!(stops_before_consent, 0);
    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.transcoder.call_count(), 1);
    assert_eq!(harness.port.upload_attempts().len(), 1);
    assert_eq!(harness.recorder.session_count().await, 0);
    harness.assert_temp_files_removed();
}

#[tokio::test]
async fn leave_during_consent_cache_miss_does_not_stop_after_success() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness.port.set_member_count(channel_id, None);
    harness.port.block_next_text_post();

    let recorder = harness.recorder.clone();
    let start = tokio::spawn(async move { recorder.start(SCRIM_GUILD_ID, 1, &[role]).await });
    harness.port.wait_for_text_post().await;
    harness
        .recorder
        .handle_voice_event(VoiceEvent::Leave {
            guild_id: SCRIM_GUILD_ID,
            user_id: 2,
            channel_id,
        })
        .await;
    harness.port.unblock_text_post();

    assert_eq!(
        start.await.expect("start task"),
        Ok(RecorderIdentity::MainBot)
    );
    assert!(harness.backend.stops().is_empty());
    assert_eq!(harness.transcoder.call_count(), 0);
    assert!(harness.port.upload_attempts().is_empty());
    assert_eq!(harness.recorder.session_count().await, 1);
}

#[tokio::test]
async fn manual_stop_without_active_session_is_distinct() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);

    assert_eq!(
        harness.recorder.stop(SCRIM_GUILD_ID, 1, &[role]).await,
        Err(StopError::NoActiveRecording)
    );
    assert!(harness.backend.stops().is_empty());
    assert!(harness.port.upload_attempts().is_empty());
}

#[tokio::test]
async fn concurrent_stop_triggers_run_backend_and_upload_once() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");

    let (first, second) = tokio::join!(
        harness
            .recorder
            .stop_for_channel(channel_id, StopTrigger::Manual),
        harness
            .recorder
            .stop_for_channel(channel_id, StopTrigger::EmptyChannel),
    );

    assert_eq!([first, second].into_iter().filter(|won| *won).count(), 1);
    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.transcoder.call_count(), 1);
    assert_eq!(harness.port.upload_attempts().len(), 1);
    harness.assert_temp_files_removed();
}

#[tokio::test]
async fn concurrent_health_sweeps_stop_an_unhealthy_session_once() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");
    harness.backend.fail_health(RecorderIdentity::MainBot);

    let (first, second) = tokio::join!(
        harness.recorder.health_sweep(),
        harness.recorder.health_sweep()
    );

    assert_eq!(first + second, 1);
    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.transcoder.call_count(), 1);
    assert_eq!(harness.port.upload_attempts().len(), 1);
    assert_eq!(harness.recorder.session_count().await, 0);
}

#[tokio::test]
async fn upload_failure_posts_fallback_cleans_and_frees_slot() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    harness.port.set_upload_failure(true);
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");

    assert_eq!(
        harness.recorder.stop(SCRIM_GUILD_ID, 1, &[role]).await,
        Ok(RecorderIdentity::MainBot)
    );
    assert_eq!(harness.port.successful_post_count(), 2);
    assert_eq!(harness.recorder.session_count().await, 0);
    harness.assert_temp_files_removed();

    harness.port.set_upload_failure(false);
    assert_eq!(
        harness.recorder.start(SCRIM_GUILD_ID, 1, &[role]).await,
        Ok(RecorderIdentity::MainBot)
    );
}

#[tokio::test]
async fn cap_triggers_at_exact_boundary_and_posts_hint() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let started_at = Instant::now();
    harness
        .recorder
        .start_at(SCRIM_GUILD_ID, 1, &[role], started_at)
        .await
        .expect("start");

    assert_eq!(
        harness
            .recorder
            .cap_sweep_at(started_at + Duration::from_secs(60 * 60) - Duration::from_nanos(1))
            .await,
        0
    );
    assert_eq!(
        harness
            .recorder
            .cap_sweep_at(started_at + Duration::from_secs(60 * 60))
            .await,
        1
    );
    assert_eq!(harness.port.successful_post_count(), 2);
}

#[tokio::test]
async fn cap_loser_does_not_post_hint() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let started_at = Instant::now();
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness
        .recorder
        .start_at(SCRIM_GUILD_ID, 1, &[role], started_at)
        .await
        .expect("start");
    harness.backend.block_stop();

    let recorder = harness.recorder.clone();
    let manual = tokio::spawn(async move {
        recorder
            .stop_for_channel(channel_id, StopTrigger::Manual)
            .await
    });
    harness.backend.wait_for_stop().await;
    assert_eq!(
        harness
            .recorder
            .cap_sweep_at(started_at + Duration::from_secs(60 * 60))
            .await,
        0
    );
    assert_eq!(harness.port.successful_post_count(), 1);
    harness.backend.unblock_stop();
    assert!(manual.await.expect("manual stop task"));
}

#[tokio::test]
async fn empty_channel_cache_miss_does_not_stop() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness.port.set_member_count(channel_id, None);
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");

    harness
        .recorder
        .handle_voice_event(VoiceEvent::Leave {
            guild_id: SCRIM_GUILD_ID,
            user_id: 2,
            channel_id,
        })
        .await;

    assert!(harness.backend.stops().is_empty());
    assert_eq!(harness.recorder.session_count().await, 1);
}

#[tokio::test]
async fn voice_event_stops_a_confirmed_empty_channel() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness.port.set_member_count(channel_id, Some(0));
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");

    harness
        .recorder
        .handle_voice_event(VoiceEvent::Leave {
            guild_id: SCRIM_GUILD_ID,
            user_id: 2,
            channel_id,
        })
        .await;

    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.port.upload_attempts().len(), 1);
    assert_eq!(harness.recorder.session_count().await, 0);
}

#[tokio::test]
async fn reconcile_stops_a_confirmed_empty_channel() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness.port.set_member_count(channel_id, Some(0));
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");

    harness.recorder.reconcile().await;

    assert_eq!(harness.backend.stops().len(), 1);
    assert_eq!(harness.port.upload_attempts().len(), 1);
    assert_eq!(harness.recorder.session_count().await, 0);
}

#[tokio::test]
async fn register_uses_the_existing_exact_record_routes() {
    let harness = Harness::new();
    let mut router = InteractionRouter::new();
    register(
        &mut router,
        RecordCommandHandler::new(harness.recorder.clone()),
    );

    assert!(router.resolve_command("record start").is_some());
    assert!(router.resolve_command("record stop").is_some());
    assert!(router.resolve_command("record").is_none());
}

#[tokio::test]
async fn command_handler_dispatches_only_exact_start_and_stop_routes() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let handler = RecordCommandHandler::new(harness.recorder.clone());

    let start = handler
        .handle(BridgeInteraction {
            command: "record start".to_string(),
            guild_id: SCRIM_GUILD_ID,
            user_id: 1,
            role_ids: vec![role],
            ..BridgeInteraction::default()
        })
        .await;
    assert_eq!(start.content.as_deref(), Some(START_CONFIRMATION_TEXT));
    assert_eq!(harness.backend.starts().len(), 1);

    let stop = handler
        .handle(BridgeInteraction {
            command: "record stop".to_string(),
            guild_id: SCRIM_GUILD_ID,
            user_id: 1,
            role_ids: vec![role],
            ..BridgeInteraction::default()
        })
        .await;
    assert_eq!(stop.content.as_deref(), Some(STOP_CONFIRMATION_TEXT));
    assert_eq!(harness.backend.stops().len(), 1);

    let unknown = handler
        .handle(BridgeInteraction {
            command: "record".to_string(),
            guild_id: SCRIM_GUILD_ID,
            user_id: 1,
            role_ids: vec![role],
            ..BridgeInteraction::default()
        })
        .await;
    assert_eq!(unknown.content.as_deref(), Some(SETUP_ERROR_TEXT));
    assert_eq!(harness.backend.starts().len(), 1);
    assert_eq!(harness.backend.stops().len(), 1);
}

#[tokio::test]
async fn backend_calls_keep_fixed_guild_channel_and_identity() {
    let harness = Harness::new();
    let role = harness.set_user_channel(1, 0);
    let channel_id = TEAM_VOICE_CHANNELS[0].voice_channel_id;
    harness
        .recorder
        .start(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("start");
    harness
        .recorder
        .stop(SCRIM_GUILD_ID, 1, &[role])
        .await
        .expect("stop");

    let starts = harness.backend.starts();
    let stops = harness.backend.stops();
    assert_eq!(starts[0].identity, RecorderIdentity::MainBot);
    assert_eq!(starts[0].guild_id, SCRIM_GUILD_ID);
    assert_eq!(starts[0].voice_channel_id, channel_id);
    assert!(starts[0].wav_path.is_some());
    assert_eq!(stops[0].identity, RecorderIdentity::MainBot);
    assert_eq!(stops[0].guild_id, SCRIM_GUILD_ID);
    assert_eq!(stops[0].voice_channel_id, channel_id);
}
