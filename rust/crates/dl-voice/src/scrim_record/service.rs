use std::collections::HashMap;
use std::io::ErrorKind;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dl_discord::{
    BridgeInteraction, BridgeReply, Dispatcher, InteractionHandler, InteractionRouter, VoiceEvent,
};
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};

use super::{
    affected_team_voice_channels, can_operate, duration_cap_reached, register_record_commands,
    OperateDenied, RecorderIdentity, SessionState, TeamChannelConfig, SCRIM_GUILD_ID,
};

const RECORDER_IDENTITIES: [RecorderIdentity; 2] =
    [RecorderIdentity::MainBot, RecorderIdentity::SecondaryBot];
const CAP_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
const HEALTH_SWEEP_INTERVAL: Duration = Duration::from_secs(1);
const RECORDING_FILE_PREFIX: &str = "scrim-record-";

const CONSENT_TEXT: &str = "Die Sprachaufnahme läuft bereits.";
const UNKNOWN_CHANNEL_TEXT: &str =
    "Du musst einem der vier Scrim-Team-Sprachkanäle beitreten, bevor du die Aufnahme steuerst.";
const NOT_PERMITTED_TEXT: &str = "Du brauchst die Teamrolle dieses Kanals oder die Coach-Rolle.";
const ALREADY_RECORDING_TEXT: &str = "In diesem Team-Sprachkanal läuft bereits eine Aufnahme.";
const NO_CAPACITY_TEXT: &str =
    "Beide Aufnahmeplätze sind gerade belegt oder technisch nicht verfügbar. Versuch es später erneut.";
const SETUP_ERROR_TEXT: &str =
    "Der Aufnahmebefehl konnte technisch nicht ausgeführt werden. Versuch es erneut oder melde dich beim Community-Team.";
const START_CONFIRMATION_TEXT: &str =
    "Aufnahme gestartet. Im Team-Textkanal steht jetzt der Einwilligungshinweis.";
const NO_ACTIVE_RECORDING_TEXT: &str = "In diesem Team-Sprachkanal läuft keine Aufnahme.";
const STOP_CONFIRMATION_TEXT: &str =
    "Aufnahme beendet. Im Team-Textkanal findest du die MP3 oder bei einem Fehler einen Hinweis.";
const UPLOAD_FALLBACK_TEXT: &str =
    "Die Aufnahme wurde beendet, aber die MP3 konnte nicht bereitgestellt werden. Bitte meldet den technischen Fehler beim Community-Team; die Aufnahme kann nicht nachträglich aus dem Bot abgerufen werden.";
const CAP_REACHED_TEXT: &str =
    "Die Aufnahme wurde wegen des 60-Minuten-Limits automatisch beendet. Startet bei Bedarf mit /record start eine neue Aufnahme.";

#[async_trait::async_trait]
pub trait ScrimRecordPort: Send + Sync {
    async fn current_voice_channel(
        &self,
        guild_id: u64,
        user_id: u64,
    ) -> Result<Option<u64>, String>;

    /// `Some(0)` bedeutet sicher leer; ein Cache-Miss ist `None` oder `Err`.
    async fn non_bot_member_count(
        &self,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<Option<usize>, String>;

    async fn post_text(&self, channel_id: u64, content: &str) -> Result<(), String>;

    async fn upload_attachment(
        &self,
        channel_id: u64,
        content: Option<&str>,
        path: &Path,
    ) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RecordingLoss {
    pub dropped_frames: u64,
    pub dropped_audio: Duration,
}

#[derive(Debug, thiserror::Error)]
#[error("{reason}")]
pub struct RecordingStopError {
    reason: String,
    pub loss: RecordingLoss,
}

impl RecordingStopError {
    pub(crate) fn new(reason: String, loss: RecordingLoss) -> Self {
        Self { reason, loss }
    }
}

#[async_trait::async_trait]
pub trait RecordingBackend: Send + Sync {
    async fn readiness(&self, identity: RecorderIdentity) -> Result<bool, String>;

    async fn health(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<(), String>;

    async fn start(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
        wav_path: &Path,
    ) -> Result<(), String>;

    async fn stop(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<RecordingLoss, RecordingStopError>;
}

fn recording_loss_notice(loss: RecordingLoss) -> Option<String> {
    (loss.dropped_frames > 0).then(|| {
        let frame_label = if loss.dropped_frames == 1 {
            "Audioframe"
        } else {
            "Audioframes"
        };
        format!(
            "Die Aufnahme enthält Lücken: {} {frame_label} ({} ms) konnten nicht aufgezeichnet werden.",
            loss.dropped_frames,
            loss.dropped_audio.as_millis()
        )
    })
}

#[async_trait::async_trait]
pub trait AudioTranscoder: Send + Sync {
    async fn transcode(&self, wav_path: &Path, mp3_path: &Path) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub struct FfmpegTranscoder;

#[async_trait::async_trait]
impl AudioTranscoder for FfmpegTranscoder {
    async fn transcode(&self, wav_path: &Path, mp3_path: &Path) -> Result<(), String> {
        let status = Command::new("ffmpeg")
            .arg("-nostdin")
            .arg("-y")
            .arg("-i")
            .arg(wav_path)
            .arg("-b:a")
            .arg("96k")
            .arg(mp3_path)
            .status()
            .await
            .map_err(|err| format!("ffmpeg konnte nicht gestartet werden: {err}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("ffmpeg endete mit Status {status}"))
        }
    }
}

pub async fn prepare_recording_temp_dir(path: &Path) -> Result<(), String> {
    let process_uid = std::fs::metadata("/proc/self")
        .map_err(|err| format!("process_uid_lookup_failed: {err}"))?
        .uid();
    let parent = path
        .parent()
        .ok_or_else(|| "recording_temp_dir_has_no_parent".to_string())?;
    let parent_metadata = tokio::fs::symlink_metadata(parent)
        .await
        .map_err(|err| format!("recording_temp_parent_metadata_failed: {err}"))?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err("recording_temp_parent_not_real_directory".to_string());
    }
    if parent_metadata.uid() != process_uid {
        return Err("recording_temp_parent_wrong_owner".to_string());
    }
    if parent_metadata.permissions().mode() & 0o077 != 0 {
        return Err("recording_temp_parent_not_private".to_string());
    }

    match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err("recording_temp_path_not_real_directory".to_string());
            }
            if metadata.uid() != process_uid {
                return Err("recording_temp_dir_wrong_owner".to_string());
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            tokio::fs::create_dir(path)
                .await
                .map_err(|err| format!("recording_temp_dir_create_failed: {err}"))?;
        }
        Err(err) => return Err(format!("recording_temp_dir_metadata_failed: {err}")),
    }
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(|err| format!("recording_temp_dir_permissions_failed: {err}"))?;
    let directory_metadata = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|err| format!("recording_temp_dir_verify_failed: {err}"))?;
    if directory_metadata.file_type().is_symlink()
        || !directory_metadata.is_dir()
        || directory_metadata.uid() != process_uid
    {
        return Err("recording_temp_dir_verification_failed".to_string());
    }

    let mut entries = tokio::fs::read_dir(path)
        .await
        .map_err(|err| format!("recording_temp_dir_read_failed: {err}"))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|err| format!("recording_temp_dir_entry_failed: {err}"))?
    {
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        let is_recording_file = file_name.starts_with(RECORDING_FILE_PREFIX)
            && (file_name.ends_with(".wav") || file_name.ends_with(".mp3"));
        if !is_recording_file {
            continue;
        }
        let file_type = entry
            .file_type()
            .await
            .map_err(|err| format!("recording_temp_file_type_failed: {err}"))?;
        if !file_type.is_file() && !file_type.is_symlink() {
            continue;
        }
        tokio::fs::remove_file(entry.path())
            .await
            .map_err(|err| format!("stale_recording_cleanup_failed: {err}"))?;
        tracing::info!(
            path = %entry.path().display(),
            reason = "startup_stale_audio_removed",
            "Scrim-Record: Verwaiste Temp-Datei beim Start entfernt"
        );
    }
    Ok(())
}

async fn create_private_output_file(path: &Path) -> Result<(), String> {
    tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .await
        .map(|_| ())
        .map_err(|err| format!("private_output_create_failed: {err}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartError {
    UnknownChannel,
    NotPermitted,
    AlreadyRecording,
    NoCapacity,
    SetupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopError {
    UnknownChannel,
    NotPermitted,
    NoActiveRecording,
    SetupFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StopTrigger {
    Manual,
    EmptyChannel,
    Cap,
    Health,
    Reconcile,
}

impl StopTrigger {
    fn reason(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::EmptyChannel => "empty_channel",
            Self::Cap => "duration_cap",
            Self::Health => "backend_health",
            Self::Reconcile => "reconcile_empty_channel",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct RecordingSession {
    state: SessionState,
    started_at: Instant,
    wav_path: PathBuf,
    text_channel_id: u64,
    recorder: RecorderIdentity,
}

#[derive(Debug, Default)]
pub(super) struct RegistryAllocator {
    sessions: HashMap<u64, RecordingSession>,
    slots: [Option<u64>; 2],
}

impl RegistryAllocator {
    pub(super) fn claim_start(
        &mut self,
        config: TeamChannelConfig,
        started_at: Instant,
        wav_path: PathBuf,
        ready: [bool; 2],
    ) -> Result<RecorderIdentity, StartError> {
        if self.sessions.contains_key(&config.voice_channel_id) {
            return Err(StartError::AlreadyRecording);
        }
        let slot_index = ready
            .iter()
            .zip(self.slots.iter())
            .position(|(is_ready, channel_id)| *is_ready && channel_id.is_none())
            .ok_or(StartError::NoCapacity)?;
        let recorder = RECORDER_IDENTITIES[slot_index];
        self.slots[slot_index] = Some(config.voice_channel_id);
        self.sessions.insert(
            config.voice_channel_id,
            RecordingSession {
                state: SessionState::Starting,
                started_at,
                wav_path,
                text_channel_id: config.text_channel_id,
                recorder,
            },
        );
        Ok(recorder)
    }

    pub(super) fn mark_recording(
        &mut self,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(&voice_channel_id) else {
            return false;
        };
        if session.state != SessionState::Starting || session.recorder != recorder {
            return false;
        }
        session.state = SessionState::Recording;
        true
    }

    pub(super) fn rollback_start(
        &mut self,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
    ) -> bool {
        let matches = self.sessions.get(&voice_channel_id).is_some_and(|session| {
            session.state == SessionState::Starting && session.recorder == recorder
        });
        if !matches {
            return false;
        }
        self.remove_session_and_slot(voice_channel_id, recorder)
    }

    pub(super) fn claim_stop(&mut self, voice_channel_id: u64) -> Option<RecordingSession> {
        let session = self.sessions.get_mut(&voice_channel_id)?;
        if session.state != SessionState::Recording {
            return None;
        }
        session.state = SessionState::Stopping;
        Some(session.clone())
    }

    fn claim_shutdown(&mut self) -> Vec<(u64, RecordingSession)> {
        self.sessions
            .iter_mut()
            .map(|(voice_channel_id, session)| {
                session.state = SessionState::Stopping;
                (*voice_channel_id, session.clone())
            })
            .collect()
    }

    pub(super) fn complete_stop(
        &mut self,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
    ) -> bool {
        let matches = self.sessions.get(&voice_channel_id).is_some_and(|session| {
            session.state == SessionState::Stopping && session.recorder == recorder
        });
        if !matches {
            return false;
        }
        self.remove_session_and_slot(voice_channel_id, recorder)
    }

    fn remove_session_and_slot(
        &mut self,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
    ) -> bool {
        let slot_index = recorder_slot_index(recorder);
        if self.slots[slot_index] != Some(voice_channel_id) {
            return false;
        }
        self.sessions.remove(&voice_channel_id);
        self.slots[slot_index] = None;
        true
    }

    #[cfg(test)]
    pub(super) fn state(&self, voice_channel_id: u64) -> Option<SessionState> {
        self.sessions
            .get(&voice_channel_id)
            .map(|session| session.state)
    }
}

fn recorder_slot_index(recorder: RecorderIdentity) -> usize {
    match recorder {
        RecorderIdentity::MainBot => 0,
        RecorderIdentity::SecondaryBot => 1,
    }
}

struct AudioFileGuard {
    wav_path: PathBuf,
    mp3_path: PathBuf,
    guild_id: u64,
    voice_channel_id: u64,
    recorder: RecorderIdentity,
    armed: bool,
}

impl AudioFileGuard {
    fn new(
        wav_path: PathBuf,
        guild_id: u64,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
    ) -> Self {
        let mp3_path = wav_path.with_extension("mp3");
        Self {
            wav_path,
            mp3_path,
            guild_id,
            voice_channel_id,
            recorder,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }

    async fn cleanup(&mut self) {
        for path in [&self.wav_path, &self.mp3_path] {
            match tokio::fs::remove_file(path).await {
                Ok(()) => {
                    tracing::debug!(
                        guild_id = self.guild_id,
                        channel_id = self.voice_channel_id,
                        recorder = ?self.recorder,
                        reason = "audio_file_removed",
                        path = %path.display(),
                        "Scrim-Record: Temp-Datei entfernt"
                    );
                }
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(
                        %err,
                        guild_id = self.guild_id,
                        channel_id = self.voice_channel_id,
                        recorder = ?self.recorder,
                        reason = "audio_file_cleanup_failed",
                        path = %path.display(),
                        "Scrim-Record: Temp-Datei konnte nicht entfernt werden"
                    );
                }
            }
        }
        self.armed = false;
    }
}

impl Drop for AudioFileGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        for path in [&self.wav_path, &self.mp3_path] {
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => {
                    tracing::warn!(
                        %err,
                        guild_id = self.guild_id,
                        channel_id = self.voice_channel_id,
                        recorder = ?self.recorder,
                        reason = "audio_file_drop_cleanup_failed",
                        path = %path.display(),
                        "Scrim-Record: Drop-Cleanup der Temp-Datei fehlgeschlagen"
                    );
                }
            }
        }
    }
}

pub struct ScrimRecorder {
    port: Arc<dyn ScrimRecordPort>,
    backend: Arc<dyn RecordingBackend>,
    transcoder: Arc<dyn AudioTranscoder>,
    temp_dir: PathBuf,
    state: Mutex<RegistryAllocator>,
    lifecycle: RwLock<()>,
    shutting_down: AtomicBool,
    next_file_id: AtomicU64,
}

impl ScrimRecorder {
    pub fn new(
        port: Arc<dyn ScrimRecordPort>,
        backend: Arc<dyn RecordingBackend>,
        transcoder: Arc<dyn AudioTranscoder>,
        temp_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            port,
            backend,
            transcoder,
            temp_dir,
            state: Mutex::new(RegistryAllocator::default()),
            lifecycle: RwLock::new(()),
            shutting_down: AtomicBool::new(false),
            next_file_id: AtomicU64::new(1),
        })
    }

    pub async fn start(
        &self,
        guild_id: u64,
        user_id: u64,
        member_role_ids: &[u64],
    ) -> Result<RecorderIdentity, StartError> {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            tracing::warn!(
                guild_id,
                channel_id = 0_u64,
                recorder = "unassigned",
                reason = "service_shutting_down",
                "Scrim-Record: Start waehrend Shutdown abgewiesen"
            );
            return Err(StartError::SetupFailed);
        }
        self.start_inner(guild_id, user_id, member_role_ids, None)
            .await
    }

    #[cfg(test)]
    async fn start_at(
        &self,
        guild_id: u64,
        user_id: u64,
        member_role_ids: &[u64],
        started_at: Instant,
    ) -> Result<RecorderIdentity, StartError> {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(StartError::SetupFailed);
        }
        self.start_inner(guild_id, user_id, member_role_ids, Some(started_at))
            .await
    }

    async fn start_inner(
        &self,
        guild_id: u64,
        user_id: u64,
        member_role_ids: &[u64],
        started_at: Option<Instant>,
    ) -> Result<RecorderIdentity, StartError> {
        let (voice_channel_id, config) = self
            .resolve_start_target(guild_id, user_id, member_role_ids)
            .await?;
        self.ensure_start_available(guild_id, voice_channel_id)
            .await?;
        let ready = self.recorder_readiness(guild_id, voice_channel_id).await?;
        let started_at = started_at.unwrap_or_else(Instant::now);
        let file_id = self.next_file_id.fetch_add(1, Ordering::Relaxed);
        let wav_path = self.temp_dir.join(format!(
            "scrim-record-{guild_id}-{voice_channel_id}-{file_id}.wav"
        ));
        let recorder = self
            .claim_start(config, started_at, wav_path.clone(), ready)
            .await?;
        let mut files = AudioFileGuard::new(wav_path.clone(), guild_id, voice_channel_id, recorder);
        tracing::info!(
            guild_id,
            channel_id = voice_channel_id,
            recorder = ?recorder,
            reason = "start_claimed",
            "Scrim-Record: Start-Claim gewonnen"
        );

        if let Err(err) = self
            .backend
            .start(recorder, guild_id, voice_channel_id, &wav_path)
            .await
        {
            tracing::error!(
                %err,
                guild_id,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason = "backend_start_failed",
                "Scrim-Record: Backend-Start fehlgeschlagen"
            );
            self.abort_start(voice_channel_id, recorder, &mut files, "start_rollback")
                .await;
            return Err(StartError::SetupFailed);
        }

        let consent_text = format!(
            "{CONSENT_TEXT} Gestartet von <@{user_id}>. Mit deinem Verbleib im Sprachkanal stimmst du der Aufnahme zu; wenn du nicht zustimmst, verlasse ihn jetzt. Wer die Teamrolle dieses Kanals oder die Coach-Rolle hat und im Sprachkanal sitzt, kann sie mit /record stop beenden."
        );
        if let Err(err) = self
            .port
            .post_text(config.text_channel_id, &consent_text)
            .await
        {
            tracing::error!(
                %err,
                guild_id,
                channel_id = voice_channel_id,
                text_channel_id = config.text_channel_id,
                recorder = ?recorder,
                reason = "consent_post_failed",
                "Scrim-Record: Consent-Post fehlgeschlagen, Aufnahme wird abgebrochen"
            );
            self.abort_start(voice_channel_id, recorder, &mut files, "consent_rollback")
                .await;
            return Err(StartError::SetupFailed);
        }

        if !self.mark_recording(voice_channel_id, recorder).await {
            tracing::error!(
                guild_id,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason = "recording_transition_failed",
                "Scrim-Record: Starting-zu-Recording fehlgeschlagen"
            );
            self.abort_start(
                voice_channel_id,
                recorder,
                &mut files,
                "transition_rollback",
            )
            .await;
            return Err(StartError::SetupFailed);
        }

        files.disarm();
        tracing::info!(
            guild_id,
            channel_id = voice_channel_id,
            recorder = ?recorder,
            reason = "recording_started",
            "Scrim-Record: Aufnahme gestartet"
        );
        self.stop_if_empty(voice_channel_id, StopTrigger::EmptyChannel)
            .await;
        Ok(recorder)
    }

    async fn resolve_start_target(
        &self,
        guild_id: u64,
        user_id: u64,
        member_role_ids: &[u64],
    ) -> Result<(u64, TeamChannelConfig), StartError> {
        if guild_id != SCRIM_GUILD_ID {
            tracing::warn!(
                guild_id,
                channel_id = 0_u64,
                recorder = "unassigned",
                reason = "wrong_guild",
                "Scrim-Record: Start abgewiesen"
            );
            return Err(StartError::SetupFailed);
        }
        let voice_channel_id = match self.port.current_voice_channel(guild_id, user_id).await {
            Ok(Some(channel_id)) => channel_id,
            Ok(None) => {
                tracing::info!(
                    guild_id,
                    channel_id = 0_u64,
                    recorder = "unassigned",
                    reason = "caller_not_in_voice",
                    "Scrim-Record: Start abgewiesen"
                );
                return Err(StartError::UnknownChannel);
            }
            Err(err) => {
                tracing::error!(
                    %err,
                    guild_id,
                    channel_id = 0_u64,
                    recorder = "unassigned",
                    reason = "voice_lookup_failed",
                    "Scrim-Record: Start-Setup fehlgeschlagen"
                );
                return Err(StartError::SetupFailed);
            }
        };
        let config = match can_operate(voice_channel_id, member_role_ids) {
            Ok(config) => config,
            Err(OperateDenied::UnknownChannel) => {
                tracing::info!(
                    guild_id,
                    channel_id = voice_channel_id,
                    recorder = "unassigned",
                    reason = "unknown_channel",
                    "Scrim-Record: Start abgewiesen"
                );
                return Err(StartError::UnknownChannel);
            }
            Err(OperateDenied::NotPermitted) => {
                tracing::info!(
                    guild_id,
                    channel_id = voice_channel_id,
                    recorder = "unassigned",
                    reason = "not_permitted",
                    "Scrim-Record: Start abgewiesen"
                );
                return Err(StartError::NotPermitted);
            }
        };
        Ok((voice_channel_id, config))
    }

    async fn ensure_start_available(
        &self,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<(), StartError> {
        let occupied = {
            let state = self.state.lock().await;
            state.sessions.contains_key(&voice_channel_id)
        };
        if occupied {
            tracing::info!(
                guild_id,
                channel_id = voice_channel_id,
                recorder = "assigned",
                reason = "already_recording",
                "Scrim-Record: Start abgewiesen"
            );
            return Err(StartError::AlreadyRecording);
        }
        Ok(())
    }

    async fn recorder_readiness(
        &self,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<[bool; 2], StartError> {
        let mut ready = [false; 2];
        let mut readiness_failed = false;
        for (index, identity) in RECORDER_IDENTITIES.into_iter().enumerate() {
            match self.backend.readiness(identity).await {
                Ok(is_ready) => ready[index] = is_ready,
                Err(err) => {
                    readiness_failed = true;
                    tracing::error!(
                        %err,
                        guild_id,
                        channel_id = voice_channel_id,
                        recorder = ?identity,
                        reason = "readiness_failed",
                        "Scrim-Record: Recorder-Readiness fehlgeschlagen"
                    );
                }
            }
        }
        if !ready.into_iter().any(|is_ready| is_ready) {
            let error = if readiness_failed {
                StartError::SetupFailed
            } else {
                StartError::NoCapacity
            };
            tracing::warn!(
                guild_id,
                channel_id = voice_channel_id,
                recorder = "none_ready",
                reason = ?error,
                "Scrim-Record: Start ohne Recorder-Kapazitaet"
            );
            return Err(error);
        }
        Ok(ready)
    }

    async fn claim_start(
        &self,
        config: TeamChannelConfig,
        started_at: Instant,
        wav_path: PathBuf,
        ready: [bool; 2],
    ) -> Result<RecorderIdentity, StartError> {
        let voice_channel_id = config.voice_channel_id;
        let claim = {
            let mut state = self.state.lock().await;
            state.claim_start(config, started_at, wav_path, ready)
        };
        match claim {
            Ok(recorder) => Ok(recorder),
            Err(error) => {
                tracing::warn!(
                    guild_id = SCRIM_GUILD_ID,
                    channel_id = voice_channel_id,
                    recorder = "unassigned",
                    reason = ?error,
                    "Scrim-Record: Atomarer Start-Claim abgewiesen"
                );
                Err(error)
            }
        }
    }

    async fn mark_recording(&self, voice_channel_id: u64, recorder: RecorderIdentity) -> bool {
        {
            let mut state = self.state.lock().await;
            state.mark_recording(voice_channel_id, recorder)
        }
    }

    async fn abort_start(
        &self,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
        files: &mut AudioFileGuard,
        reason: &'static str,
    ) {
        self.stop_backend_best_effort(recorder, voice_channel_id, reason)
            .await;
        files.cleanup().await;
        let rolled_back = {
            let mut state = self.state.lock().await;
            state.rollback_start(voice_channel_id, recorder)
        };
        if !rolled_back {
            tracing::error!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason,
                "Scrim-Record: Start-Rollback konnte Zustand nicht freigeben"
            );
        }
    }

    pub async fn stop(
        &self,
        guild_id: u64,
        user_id: u64,
        member_role_ids: &[u64],
    ) -> Result<RecorderIdentity, StopError> {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(StopError::SetupFailed);
        }
        if guild_id != SCRIM_GUILD_ID {
            tracing::warn!(
                guild_id,
                channel_id = 0_u64,
                recorder = "unassigned",
                reason = "wrong_guild",
                "Scrim-Record: Manueller Stop abgewiesen"
            );
            return Err(StopError::SetupFailed);
        }
        let voice_channel_id = match self.port.current_voice_channel(guild_id, user_id).await {
            Ok(Some(channel_id)) => channel_id,
            Ok(None) => {
                tracing::info!(
                    guild_id,
                    channel_id = 0_u64,
                    recorder = "unassigned",
                    reason = "caller_not_in_voice",
                    "Scrim-Record: Manueller Stop abgewiesen"
                );
                return Err(StopError::UnknownChannel);
            }
            Err(err) => {
                tracing::error!(
                    %err,
                    guild_id,
                    channel_id = 0_u64,
                    recorder = "unassigned",
                    reason = "voice_lookup_failed",
                    "Scrim-Record: Stop-Setup fehlgeschlagen"
                );
                return Err(StopError::SetupFailed);
            }
        };
        match can_operate(voice_channel_id, member_role_ids) {
            Ok(_) => {}
            Err(OperateDenied::UnknownChannel) => {
                tracing::info!(
                    guild_id,
                    channel_id = voice_channel_id,
                    recorder = "unassigned",
                    reason = "unknown_channel",
                    "Scrim-Record: Manueller Stop abgewiesen"
                );
                return Err(StopError::UnknownChannel);
            }
            Err(OperateDenied::NotPermitted) => {
                tracing::info!(
                    guild_id,
                    channel_id = voice_channel_id,
                    recorder = "unassigned",
                    reason = "not_permitted",
                    "Scrim-Record: Manueller Stop abgewiesen"
                );
                return Err(StopError::NotPermitted);
            }
        }
        self.stop_for_channel_identity(voice_channel_id, StopTrigger::Manual)
            .await
            .ok_or(StopError::NoActiveRecording)
    }

    async fn stop_for_channel(&self, voice_channel_id: u64, trigger: StopTrigger) -> bool {
        self.stop_for_channel_identity(voice_channel_id, trigger)
            .await
            .is_some()
    }

    async fn stop_for_channel_identity(
        &self,
        voice_channel_id: u64,
        trigger: StopTrigger,
    ) -> Option<RecorderIdentity> {
        let (session, existing_recorder) = {
            let mut state = self.state.lock().await;
            let existing_recorder = state
                .sessions
                .get(&voice_channel_id)
                .map(|session| session.recorder);
            (state.claim_stop(voice_channel_id), existing_recorder)
        };
        let Some(session) = session else {
            tracing::info!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?existing_recorder,
                reason = trigger.reason(),
                "Scrim-Record: Stop-Claim nicht gewonnen"
            );
            return None;
        };
        let recorder = session.recorder;
        tracing::info!(
            guild_id = SCRIM_GUILD_ID,
            channel_id = voice_channel_id,
            recorder = ?recorder,
            reason = trigger.reason(),
            "Scrim-Record: Stop-Claim gewonnen"
        );
        let mut files = AudioFileGuard::new(
            session.wav_path.clone(),
            SCRIM_GUILD_ID,
            voice_channel_id,
            recorder,
        );

        let recording_loss = match self
            .backend
            .stop(recorder, SCRIM_GUILD_ID, voice_channel_id)
            .await
        {
            Ok(loss) => {
                tracing::info!(
                    guild_id = SCRIM_GUILD_ID,
                    channel_id = voice_channel_id,
                    recorder = ?recorder,
                    reason = trigger.reason(),
                    dropped_frames = loss.dropped_frames,
                    dropped_audio_ms = loss.dropped_audio.as_millis(),
                    "Scrim-Record: Backend gestoppt und finalisiert"
                );
                loss
            }
            Err(err) => {
                let loss = err.loss;
                tracing::error!(
                    %err,
                    guild_id = SCRIM_GUILD_ID,
                    channel_id = voice_channel_id,
                    recorder = ?recorder,
                    reason = "backend_stop_failed",
                    trigger = trigger.reason(),
                    "Scrim-Record: Backend-Stop oder Finalisierung fehlgeschlagen"
                );
                loss
            }
        };

        let transcoded = match create_private_output_file(&files.mp3_path).await {
            Ok(()) => match self
                .transcoder
                .transcode(&files.wav_path, &files.mp3_path)
                .await
            {
                Ok(()) => true,
                Err(err) => {
                    tracing::error!(
                        %err,
                        guild_id = SCRIM_GUILD_ID,
                        channel_id = voice_channel_id,
                        recorder = ?recorder,
                        reason = "transcode_failed",
                        trigger = trigger.reason(),
                        "Scrim-Record: Audio-Transkodierung fehlgeschlagen"
                    );
                    false
                }
            },
            Err(err) => {
                tracing::error!(
                    %err,
                    guild_id = SCRIM_GUILD_ID,
                    channel_id = voice_channel_id,
                    recorder = ?recorder,
                    reason = "private_output_create_failed",
                    trigger = trigger.reason(),
                    "Scrim-Record: Private MP3-Datei konnte nicht angelegt werden"
                );
                false
            }
        };

        let upload_succeeded = if transcoded {
            let upload_content = recording_loss_notice(recording_loss);
            match self
                .port
                .upload_attachment(
                    session.text_channel_id,
                    upload_content.as_deref(),
                    &files.mp3_path,
                )
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        guild_id = SCRIM_GUILD_ID,
                        channel_id = voice_channel_id,
                        text_channel_id = session.text_channel_id,
                        recorder = ?recorder,
                        reason = "attachment_uploaded",
                        trigger = trigger.reason(),
                        dropped_frames = recording_loss.dropped_frames,
                        dropped_audio_ms = recording_loss.dropped_audio.as_millis(),
                        "Scrim-Record: Aufnahme hochgeladen"
                    );
                    true
                }
                Err(err) => {
                    tracing::error!(
                        %err,
                        guild_id = SCRIM_GUILD_ID,
                        channel_id = voice_channel_id,
                        text_channel_id = session.text_channel_id,
                        recorder = ?recorder,
                        reason = "attachment_upload_failed",
                        trigger = trigger.reason(),
                        dropped_frames = recording_loss.dropped_frames,
                        dropped_audio_ms = recording_loss.dropped_audio.as_millis(),
                        "Scrim-Record: Attachment-Upload fehlgeschlagen"
                    );
                    false
                }
            }
        } else {
            false
        };
        if !upload_succeeded {
            self.post_fallback(session.text_channel_id, voice_channel_id, recorder, trigger)
                .await;
        }

        files.cleanup().await;
        self.complete_stop(voice_channel_id, recorder, trigger.reason())
            .await;

        if trigger == StopTrigger::Cap {
            if let Err(err) = self
                .port
                .post_text(session.text_channel_id, CAP_REACHED_TEXT)
                .await
            {
                tracing::error!(
                    %err,
                    guild_id = SCRIM_GUILD_ID,
                    channel_id = voice_channel_id,
                    text_channel_id = session.text_channel_id,
                    recorder = ?recorder,
                    reason = "cap_notice_failed",
                    "Scrim-Record: Cap-Hinweis konnte nicht gepostet werden"
                );
            }
        }
        tracing::info!(
            guild_id = SCRIM_GUILD_ID,
            channel_id = voice_channel_id,
            recorder = ?recorder,
            reason = trigger.reason(),
            "Scrim-Record: Stop und Cleanup abgeschlossen"
        );
        Some(recorder)
    }

    async fn post_fallback(
        &self,
        text_channel_id: u64,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
        trigger: StopTrigger,
    ) {
        match self
            .port
            .post_text(text_channel_id, UPLOAD_FALLBACK_TEXT)
            .await
        {
            Ok(()) => tracing::warn!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                text_channel_id,
                recorder = ?recorder,
                reason = "fallback_posted",
                trigger = trigger.reason(),
                "Scrim-Record: Upload-Fallback gepostet"
            ),
            Err(err) => tracing::error!(
                %err,
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                text_channel_id,
                recorder = ?recorder,
                reason = "fallback_post_failed",
                trigger = trigger.reason(),
                "Scrim-Record: Upload-Fallback konnte nicht gepostet werden"
            ),
        }
    }

    async fn stop_backend_best_effort(
        &self,
        recorder: RecorderIdentity,
        voice_channel_id: u64,
        reason: &'static str,
    ) {
        match self
            .backend
            .stop(recorder, SCRIM_GUILD_ID, voice_channel_id)
            .await
        {
            Ok(loss) => tracing::info!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason,
                dropped_frames = loss.dropped_frames,
                dropped_audio_ms = loss.dropped_audio.as_millis(),
                "Scrim-Record: Best-Effort-Backend-Stop abgeschlossen"
            ),
            Err(err) => tracing::error!(
                %err,
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason,
                "Scrim-Record: Best-Effort-Backend-Stop fehlgeschlagen"
            ),
        }
    }

    async fn complete_stop(
        &self,
        voice_channel_id: u64,
        recorder: RecorderIdentity,
        reason: &'static str,
    ) {
        let completed = {
            let mut state = self.state.lock().await;
            state.complete_stop(voice_channel_id, recorder)
        };
        if !completed {
            tracing::error!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason,
                "Scrim-Record: Registry und Recorder-Slot konnten nicht freigegeben werden"
            );
        }
    }

    pub async fn handle_voice_event(&self, event: VoiceEvent) {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return;
        }
        for voice_channel_id in affected_team_voice_channels(&event) {
            self.stop_if_empty(voice_channel_id, StopTrigger::EmptyChannel)
                .await;
        }
    }

    async fn stop_if_empty(&self, voice_channel_id: u64, trigger: StopTrigger) {
        let recorder = {
            let state = self.state.lock().await;
            state.sessions.get(&voice_channel_id).and_then(|session| {
                (session.state == SessionState::Recording).then_some(session.recorder)
            })
        };
        let Some(recorder) = recorder else {
            return;
        };
        match self
            .port
            .non_bot_member_count(SCRIM_GUILD_ID, voice_channel_id)
            .await
        {
            Ok(Some(0)) => {
                self.stop_for_channel(voice_channel_id, trigger).await;
            }
            Ok(Some(count)) => tracing::debug!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason = "channel_not_empty",
                non_bot_members = count,
                "Scrim-Record: Leerkanal-Stop nicht erforderlich"
            ),
            Ok(None) => tracing::warn!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason = "member_count_cache_miss",
                "Scrim-Record: Cache-Miss wird nicht als leerer Kanal behandelt"
            ),
            Err(err) => tracing::error!(
                %err,
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason = "member_count_failed",
                "Scrim-Record: Nicht-Bot-Anzahl konnte nicht gelesen werden"
            ),
        }
    }

    pub async fn cap_sweep(&self) -> usize {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return 0;
        }
        self.cap_sweep_at(Instant::now()).await
    }

    pub async fn health_sweep(&self) -> usize {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return 0;
        }
        let active_sessions = {
            let state = self.state.lock().await;
            state
                .sessions
                .iter()
                .filter_map(|(voice_channel_id, session)| {
                    (session.state == SessionState::Recording)
                        .then_some((*voice_channel_id, session.recorder))
                })
                .collect::<Vec<_>>()
        };
        let mut unhealthy_channels = Vec::new();
        for (voice_channel_id, recorder) in active_sessions {
            if let Err(err) = self
                .backend
                .health(recorder, SCRIM_GUILD_ID, voice_channel_id)
                .await
            {
                tracing::error!(
                    %err,
                    guild_id = SCRIM_GUILD_ID,
                    channel_id = voice_channel_id,
                    recorder = ?recorder,
                    reason = "backend_health_failed",
                    "Scrim-Record: Health-Sweep stoppt fehlerhafte Aufnahme"
                );
                unhealthy_channels.push(voice_channel_id);
            }
        }

        let mut won = 0;
        for voice_channel_id in unhealthy_channels {
            if self
                .stop_for_channel(voice_channel_id, StopTrigger::Health)
                .await
            {
                won += 1;
            }
        }
        won
    }

    async fn cap_sweep_at(&self, now: Instant) -> usize {
        let due_channels = {
            let state = self.state.lock().await;
            state
                .sessions
                .iter()
                .filter_map(|(voice_channel_id, session)| {
                    let elapsed = now
                        .checked_duration_since(session.started_at)
                        .unwrap_or_default();
                    (session.state == SessionState::Recording && duration_cap_reached(elapsed))
                        .then_some(*voice_channel_id)
                })
                .collect::<Vec<_>>()
        };
        let mut won = 0;
        for voice_channel_id in due_channels {
            if self
                .stop_for_channel(voice_channel_id, StopTrigger::Cap)
                .await
            {
                won += 1;
            }
        }
        won
    }

    pub async fn reconcile(&self) {
        let _lifecycle = self.lifecycle.read().await;
        if self.shutting_down.load(Ordering::Acquire) {
            return;
        }
        let active_channels = {
            let state = self.state.lock().await;
            state
                .sessions
                .iter()
                .filter_map(|(voice_channel_id, session)| {
                    (session.state == SessionState::Recording).then_some(*voice_channel_id)
                })
                .collect::<Vec<_>>()
        };
        for voice_channel_id in active_channels {
            self.stop_if_empty(voice_channel_id, StopTrigger::Reconcile)
                .await;
        }
    }

    pub async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        let _lifecycle = self.lifecycle.write().await;
        let sessions = {
            let mut state = self.state.lock().await;
            state.claim_shutdown()
        };
        for (voice_channel_id, session) in sessions {
            let recorder = session.recorder;
            tracing::info!(
                guild_id = SCRIM_GUILD_ID,
                channel_id = voice_channel_id,
                recorder = ?recorder,
                reason = "shutdown",
                "Scrim-Record: Aktive Aufnahme wird fuer Shutdown verworfen"
            );
            self.stop_backend_best_effort(recorder, voice_channel_id, "shutdown")
                .await;
            let mut files =
                AudioFileGuard::new(session.wav_path, SCRIM_GUILD_ID, voice_channel_id, recorder);
            files.cleanup().await;
            self.complete_stop(voice_channel_id, recorder, "shutdown")
                .await;
        }
    }

    #[cfg(test)]
    async fn session_count(&self) -> usize {
        self.state.lock().await.sessions.len()
    }
}

pub struct RecordCommandHandler {
    recorder: Arc<ScrimRecorder>,
}

impl RecordCommandHandler {
    pub fn new(recorder: Arc<ScrimRecorder>) -> Arc<Self> {
        Arc::new(Self { recorder })
    }
}

#[async_trait::async_trait]
impl InteractionHandler for RecordCommandHandler {
    async fn handle(&self, interaction: BridgeInteraction) -> BridgeReply {
        let text = match interaction.command.as_str() {
            "record start" => match self
                .recorder
                .start(
                    interaction.guild_id,
                    interaction.user_id,
                    &interaction.role_ids,
                )
                .await
            {
                Ok(_) => START_CONFIRMATION_TEXT,
                Err(StartError::UnknownChannel) => UNKNOWN_CHANNEL_TEXT,
                Err(StartError::NotPermitted) => NOT_PERMITTED_TEXT,
                Err(StartError::AlreadyRecording) => ALREADY_RECORDING_TEXT,
                Err(StartError::NoCapacity) => NO_CAPACITY_TEXT,
                Err(StartError::SetupFailed) => SETUP_ERROR_TEXT,
            },
            "record stop" => match self
                .recorder
                .stop(
                    interaction.guild_id,
                    interaction.user_id,
                    &interaction.role_ids,
                )
                .await
            {
                Ok(_) => STOP_CONFIRMATION_TEXT,
                Err(StopError::UnknownChannel) => UNKNOWN_CHANNEL_TEXT,
                Err(StopError::NotPermitted) => NOT_PERMITTED_TEXT,
                Err(StopError::NoActiveRecording) => NO_ACTIVE_RECORDING_TEXT,
                Err(StopError::SetupFailed) => SETUP_ERROR_TEXT,
            },
            _ => {
                tracing::warn!(
                    guild_id = interaction.guild_id,
                    channel_id = interaction.channel_id,
                    recorder = "unassigned",
                    reason = "unknown_record_command_route",
                    command = %interaction.command,
                    "Scrim-Record: Unbekannte Command-Route"
                );
                SETUP_ERROR_TEXT
            }
        };
        BridgeReply::ephemeral_text(text)
    }
}

pub fn register(router: &mut InteractionRouter, handler: Arc<RecordCommandHandler>) {
    let handler: Arc<dyn InteractionHandler> = handler;
    register_record_commands(router, handler);
}

pub fn spawn(
    recorder: Arc<ScrimRecorder>,
    dispatcher: &Dispatcher,
) -> Vec<tokio::task::JoinHandle<()>> {
    let mut handles = Vec::with_capacity(3);
    let mut voice_events = dispatcher.subscribe_voice();
    let event_recorder = recorder.clone();
    handles.push(tokio::spawn(async move {
        loop {
            match voice_events.recv().await {
                Ok(event) => event_recorder.handle_voice_event(event).await,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(
                        missed,
                        guild_id = SCRIM_GUILD_ID,
                        channel_id = 0_u64,
                        recorder = "multiple",
                        reason = "voice_events_lagged",
                        "Scrim-Record: Voice-Events verpasst, Reconcile wird ausgefuehrt"
                    );
                    event_recorder.reconcile().await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }));

    let cap_recorder = recorder.clone();
    handles.push(tokio::spawn(async move {
        loop {
            tokio::time::sleep(CAP_SWEEP_INTERVAL).await;
            cap_recorder.cap_sweep().await;
        }
    }));

    handles.push(tokio::spawn(async move {
        loop {
            tokio::time::sleep(HEALTH_SWEEP_INTERVAL).await;
            recorder.health_sweep().await;
        }
    }));
    handles
}

#[cfg(test)]
mod tests;
