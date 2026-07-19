use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use songbird::driver::{Channels, DecodeConfig, DecodeMode, SampleRate};
use songbird::events::context_data::VoiceTick;
use songbird::{Config, CoreEvent, Event, EventContext, EventHandler, Songbird};
use tokio::sync::{mpsc, oneshot};

use super::service::RecordingBackend;
use super::RecorderIdentity;

const SAMPLE_RATE_HZ: usize = 48_000;
const CHANNELS: usize = 2;
const TICK_MILLIS: usize = 20;
const SAMPLES_PER_TICK: usize = SAMPLE_RATE_HZ * CHANNELS * TICK_MILLIS / 1_000;
const MAX_CARRY_TICKS_PER_SSRC: usize = 6;
const MAX_CARRY_SAMPLES_PER_SSRC: usize = SAMPLES_PER_TICK * MAX_CARRY_TICKS_PER_SSRC;
const FRAME_QUEUE_CAPACITY: usize = 50;

const WAV_SPEC: hound::WavSpec = hound::WavSpec {
    channels: CHANNELS as u16,
    sample_rate: SAMPLE_RATE_HZ as u32,
    bits_per_sample: 16,
    sample_format: hound::SampleFormat::Int,
};

fn recording_songbird_config() -> Config {
    Config::default().decode_mode(DecodeMode::Decode(DecodeConfig::new(
        Channels::Stereo,
        SampleRate::Hz48000,
    )))
}

/// Creates a Songbird 0.6 manager configured for decoded stereo recording.
///
/// Songbird's 0.6 driver negotiates DAVE automatically and advertises its
/// supported DAVE protocol version during the voice handshake.
pub fn recording_songbird_manager() -> Arc<Songbird> {
    Songbird::serenity_from_config(recording_songbird_config())
}

#[derive(Debug)]
struct MixedFrame {
    samples: Vec<i16>,
    overflowed: bool,
}

#[derive(Debug, Default)]
struct StereoPcmMixer {
    carry_by_ssrc: HashMap<u32, VecDeque<i16>>,
}

impl StereoPcmMixer {
    fn mix_voice_tick<'a, I>(&mut self, decoded_speakers: I) -> MixedFrame
    where
        I: IntoIterator<Item = (u32, &'a [i16])>,
    {
        let mut overflowed = false;
        for (ssrc, samples) in decoded_speakers {
            let queue = self.carry_by_ssrc.entry(ssrc).or_default();
            // Keep the current output tick plus at most six carry ticks (120ms).
            let capacity = SAMPLES_PER_TICK + MAX_CARRY_SAMPLES_PER_SSRC;
            let remaining_capacity = capacity.saturating_sub(queue.len());
            overflowed |= samples.len() > remaining_capacity;
            queue.extend(samples.iter().copied().take(remaining_capacity));
        }

        let mut mixed = vec![0_i32; SAMPLES_PER_TICK];
        for queue in self.carry_by_ssrc.values_mut() {
            for (mixed_sample, sample) in mixed
                .iter_mut()
                .zip(queue.drain(..queue.len().min(SAMPLES_PER_TICK)))
            {
                *mixed_sample = mixed_sample.saturating_add(i32::from(sample));
            }
        }
        self.carry_by_ssrc.retain(|_, queue| !queue.is_empty());

        // Clipping overlapping speakers to i16 is normal mixer behaviour, not a
        // recording failure. `overflowed` only reports discarded carry data.
        let samples = mixed
            .into_iter()
            .map(|sample| sample.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16)
            .collect();
        MixedFrame {
            samples,
            overflowed,
        }
    }

    #[cfg(test)]
    fn buffered_samples(&self) -> usize {
        self.carry_by_ssrc.values().map(VecDeque::len).sum()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RecordingMetadata {
    guild_id: u64,
    voice_channel_id: u64,
    recorder: RecorderIdentity,
}

#[derive(Debug)]
struct RecordingFailure {
    metadata: RecordingMetadata,
    failed: AtomicBool,
    reason: Mutex<Option<String>>,
}

impl RecordingFailure {
    fn new(metadata: RecordingMetadata) -> Self {
        Self {
            metadata,
            failed: AtomicBool::new(false),
            reason: Mutex::new(None),
        }
    }

    fn fail(&self, reason: impl Into<String>) {
        let reason = reason.into();
        if self
            .failed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        match self.reason.lock() {
            Ok(mut stored) => *stored = Some(reason.clone()),
            Err(poisoned) => {
                let mut stored = poisoned.into_inner();
                *stored = Some(reason.clone());
            }
        }
        tracing::error!(
            guild_id = self.metadata.guild_id,
            channel_id = self.metadata.voice_channel_id,
            recorder = ?self.metadata.recorder,
            reason = %reason,
            "Scrim-Record: Audio-Backend ist fehlerhaft"
        );
    }

    fn health(&self) -> Result<(), String> {
        if !self.failed.load(Ordering::Acquire) {
            return Ok(());
        }
        match self.reason.lock() {
            Ok(reason) => Err(reason
                .clone()
                .unwrap_or_else(|| "recording_failed_without_reason".to_string())),
            Err(_) => {
                tracing::error!(
                    guild_id = self.metadata.guild_id,
                    channel_id = self.metadata.voice_channel_id,
                    recorder = ?self.metadata.recorder,
                    reason = "failure_reason_lock_poisoned",
                    "Scrim-Record: Failure-Reason-Lock ist vergiftet"
                );
                Err("failure_reason_lock_poisoned".to_string())
            }
        }
    }
}

#[derive(Debug)]
struct FrameQueue {
    sender: Mutex<Option<mpsc::Sender<Vec<i16>>>>,
    failure: Arc<RecordingFailure>,
}

impl FrameQueue {
    fn bounded(
        capacity: usize,
        failure: Arc<RecordingFailure>,
    ) -> (Arc<Self>, mpsc::Receiver<Vec<i16>>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                sender: Mutex::new(Some(sender)),
                failure,
            }),
            receiver,
        )
    }

    fn try_send(&self, samples: Vec<i16>) -> Result<(), String> {
        let sender = match self.sender.lock() {
            Ok(sender) => sender,
            Err(poisoned) => {
                self.failure.fail("queue_lock_poisoned");
                poisoned.into_inner()
            }
        };
        let Some(sender) = sender.as_ref() else {
            self.failure.fail("queue_closed");
            return Err("queue_closed".to_string());
        };
        match sender.try_send(samples) {
            Ok(()) => Ok(()),
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.failure.fail("queue_full");
                Err("queue_full".to_string())
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.failure.fail("queue_closed");
                Err("queue_closed".to_string())
            }
        }
    }

    fn close(&self) -> Result<(), String> {
        match self.sender.lock() {
            Ok(mut sender) => {
                sender.take();
                Ok(())
            }
            Err(poisoned) => {
                self.failure.fail("queue_lock_poisoned");
                poisoned.into_inner().take();
                Err("queue_lock_poisoned".to_string())
            }
        }
    }
}

struct WavWriterTask {
    join: tokio::task::JoinHandle<Result<(), String>>,
}

impl WavWriterTask {
    async fn start(
        wav_path: PathBuf,
        mut receiver: mpsc::Receiver<Vec<i16>>,
        failure: Arc<RecordingFailure>,
    ) -> Result<Self, String> {
        let (ready_sender, ready_receiver) = oneshot::channel();
        let join = tokio::task::spawn_blocking(move || {
            let mut writer = match hound::WavWriter::create(&wav_path, WAV_SPEC) {
                Ok(writer) => writer,
                Err(err) => {
                    let error = format!("writer_open_failed: {err}");
                    failure.fail(error.clone());
                    let _ = ready_sender.send(Err(error.clone()));
                    return Err(error);
                }
            };
            let _ = ready_sender.send(Ok(()));

            let mut errors = Vec::new();
            'frames: while let Some(frame) = receiver.blocking_recv() {
                for sample in frame {
                    if let Err(err) = writer.write_sample(sample) {
                        let error = format!("writer_write_failed: {err}");
                        failure.fail(error.clone());
                        errors.push(error);
                        break 'frames;
                    }
                }
            }
            if let Err(err) = writer.finalize() {
                let error = format!("writer_finalize_failed: {err}");
                failure.fail(error.clone());
                errors.push(error);
            }
            errors_to_result(errors)
        });

        match ready_receiver.await {
            Ok(Ok(())) => Ok(Self { join }),
            Ok(Err(start_error)) => {
                let _ = join.await;
                Err(start_error)
            }
            Err(_) => match join.await {
                Ok(Ok(())) => Err("writer_stopped_before_ready".to_string()),
                Ok(Err(err)) => Err(err),
                Err(err) => Err(format!("writer_task_failed_before_ready: {err}")),
            },
        }
    }

    async fn finish(self) -> Result<(), String> {
        match self.join.await {
            Ok(result) => result,
            Err(err) => Err(format!("writer_task_failed: {err}")),
        }
    }
}

fn errors_to_result(errors: Vec<String>) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

struct RecordingSink {
    active: AtomicBool,
    mixer: Mutex<StereoPcmMixer>,
    queue: Arc<FrameQueue>,
    failure: Arc<RecordingFailure>,
}

impl RecordingSink {
    fn new(queue: Arc<FrameQueue>, failure: Arc<RecordingFailure>) -> Self {
        Self {
            active: AtomicBool::new(true),
            mixer: Mutex::new(StereoPcmMixer::default()),
            queue,
            failure,
        }
    }

    fn handle_voice_tick(&self, tick: &VoiceTick) {
        if !self.active.load(Ordering::Acquire) {
            return;
        }
        let mixed = {
            let mut mixer = match self.mixer.lock() {
                Ok(mixer) => mixer,
                Err(_) => {
                    self.failure.fail("mixer_lock_poisoned");
                    return;
                }
            };
            mixer.mix_voice_tick(tick.speaking.iter().filter_map(|(ssrc, voice)| {
                voice
                    .decoded_voice
                    .as_deref()
                    .map(|decoded| (*ssrc, decoded))
            }))
        };
        if mixed.overflowed {
            self.failure.fail("mixer_overflow");
        }
        if self.active.load(Ordering::Acquire) {
            let _ = self.queue.try_send(mixed.samples);
        }
    }

    fn deactivate(&self) {
        self.active.store(false, Ordering::Release);
    }
}

struct VoiceTickHandler {
    sink: Arc<RecordingSink>,
}

#[async_trait::async_trait]
impl EventHandler for VoiceTickHandler {
    async fn act(&self, context: &EventContext<'_>) -> Option<Event> {
        if !self.sink.active.load(Ordering::Acquire) {
            return Some(Event::Cancel);
        }
        if let EventContext::VoiceTick(tick) = context {
            self.sink.handle_voice_tick(tick);
        }
        None
    }
}

struct ActiveRecording {
    metadata: RecordingMetadata,
    sink: Arc<RecordingSink>,
    writer: WavWriterTask,
}

enum SlotState {
    Idle,
    Starting(RecordingMetadata),
    Recording(ActiveRecording),
    Stopping(RecordingMetadata),
}

struct BackendSlot {
    manager: Arc<Songbird>,
    readiness: Arc<AtomicBool>,
    state: Mutex<SlotState>,
}

/// Real Songbird recording backend with one fixed manager per bot identity.
pub struct SongbirdRecordingBackend {
    slots: [BackendSlot; 2],
}

impl SongbirdRecordingBackend {
    pub fn new(
        main_manager: Arc<Songbird>,
        main_readiness: Arc<AtomicBool>,
        worker_manager: Arc<Songbird>,
        worker_readiness: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        if Arc::ptr_eq(&main_manager, &worker_manager) {
            return Err("recorder_managers_must_be_distinct".to_string());
        }
        Ok(Self {
            slots: [
                BackendSlot {
                    manager: main_manager,
                    readiness: main_readiness,
                    state: Mutex::new(SlotState::Idle),
                },
                BackendSlot {
                    manager: worker_manager,
                    readiness: worker_readiness,
                    state: Mutex::new(SlotState::Idle),
                },
            ],
        })
    }

    fn slot(&self, identity: RecorderIdentity) -> &BackendSlot {
        match identity {
            RecorderIdentity::MainBot => &self.slots[0],
            RecorderIdentity::DiscordTokenWorker => &self.slots[1],
        }
    }

    fn reserve_slot(&self, slot: &BackendSlot, metadata: RecordingMetadata) -> Result<(), String> {
        let mut state = slot.state.lock().map_err(|_| {
            log_slot_fault(metadata, "slot_state_lock_poisoned");
            "slot_state_lock_poisoned".to_string()
        })?;
        if matches!(*state, SlotState::Idle) {
            *state = SlotState::Starting(metadata);
            Ok(())
        } else {
            Err("recorder_slot_occupied".to_string())
        }
    }

    fn force_idle(&self, slot: &BackendSlot, metadata: RecordingMetadata) -> Result<(), String> {
        match slot.state.lock() {
            Ok(mut state) => {
                *state = SlotState::Idle;
                Ok(())
            }
            Err(poisoned) => {
                log_slot_fault(metadata, "slot_state_lock_poisoned");
                *poisoned.into_inner() = SlotState::Idle;
                Err("slot_state_lock_poisoned".to_string())
            }
        }
    }

    async fn cleanup_failed_start(
        &self,
        slot: &BackendSlot,
        metadata: RecordingMetadata,
        call: Option<Arc<tokio::sync::Mutex<songbird::Call>>>,
        sink: Arc<RecordingSink>,
        writer: WavWriterTask,
        wav_path: &Path,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        sink.deactivate();
        if let Some(call) = call {
            call.lock().await.remove_all_global_events();
            match songbird_guild_id(metadata.guild_id) {
                Ok(guild_id) => {
                    if let Err(err) = slot.manager.remove(guild_id).await {
                        errors.push(format!("songbird_remove_failed: {err}"));
                    }
                }
                Err(err) => errors.push(err),
            }
        }
        if let Err(err) = sink.queue.close() {
            errors.push(err);
        }
        if let Err(err) = writer.finish().await {
            errors.push(err);
        }
        if let Err(err) = remove_failed_start_file(wav_path).await {
            errors.push(err);
        }
        if let Err(err) = self.force_idle(slot, metadata) {
            errors.push(err);
        }
        errors
    }
}

fn songbird_guild_id(guild_id: u64) -> Result<songbird::id::GuildId, String> {
    NonZeroU64::new(guild_id)
        .map(songbird::id::GuildId)
        .ok_or_else(|| "invalid_zero_guild_id".to_string())
}

fn songbird_channel_id(channel_id: u64) -> Result<songbird::id::ChannelId, String> {
    NonZeroU64::new(channel_id)
        .map(songbird::id::ChannelId)
        .ok_or_else(|| "invalid_zero_channel_id".to_string())
}

fn log_slot_fault(metadata: RecordingMetadata, reason: &'static str) {
    tracing::error!(
        guild_id = metadata.guild_id,
        channel_id = metadata.voice_channel_id,
        recorder = ?metadata.recorder,
        reason,
        "Scrim-Record: Recorder-Slot ist fehlerhaft"
    );
}

async fn remove_failed_start_file(path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("failed_start_file_cleanup_failed: {err}")),
    }
}

#[async_trait::async_trait]
impl RecordingBackend for SongbirdRecordingBackend {
    async fn readiness(&self, identity: RecorderIdentity) -> Result<bool, String> {
        Ok(self.slot(identity).readiness.load(Ordering::Acquire))
    }

    async fn health(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<(), String> {
        let slot = self.slot(identity);
        if !slot.readiness.load(Ordering::Acquire) {
            return Err("recorder_not_ready".to_string());
        }
        let metadata = RecordingMetadata {
            guild_id,
            voice_channel_id,
            recorder: identity,
        };
        let state = slot.state.lock().map_err(|_| {
            log_slot_fault(metadata, "slot_state_lock_poisoned");
            "slot_state_lock_poisoned".to_string()
        })?;
        match &*state {
            SlotState::Recording(active) if active.metadata == metadata => active.failure_health(),
            SlotState::Starting(active) if *active == metadata => {
                Err("recording_still_starting".to_string())
            }
            SlotState::Stopping(active) if *active == metadata => {
                Err("recording_already_stopping".to_string())
            }
            _ => Err("recording_not_active".to_string()),
        }
    }

    async fn start(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
        wav_path: &Path,
    ) -> Result<(), String> {
        let slot = self.slot(identity);
        let metadata = RecordingMetadata {
            guild_id,
            voice_channel_id,
            recorder: identity,
        };
        if !slot.readiness.load(Ordering::Acquire) {
            return Err("recorder_not_ready".to_string());
        }
        let songbird_guild_id = songbird_guild_id(guild_id)?;
        let songbird_channel_id = songbird_channel_id(voice_channel_id)?;
        self.reserve_slot(slot, metadata)?;

        let failure = Arc::new(RecordingFailure::new(metadata));
        let (queue, receiver) = FrameQueue::bounded(FRAME_QUEUE_CAPACITY, failure.clone());
        let writer =
            match WavWriterTask::start(wav_path.to_path_buf(), receiver, failure.clone()).await {
                Ok(writer) => writer,
                Err(err) => {
                    let mut errors = vec![err];
                    if let Err(close_err) = queue.close() {
                        errors.push(close_err);
                    }
                    if let Err(cleanup_err) = remove_failed_start_file(wav_path).await {
                        errors.push(cleanup_err);
                    }
                    if let Err(state_err) = self.force_idle(slot, metadata) {
                        errors.push(state_err);
                    }
                    return errors_to_result(errors);
                }
            };
        let sink = Arc::new(RecordingSink::new(queue, failure));
        let call = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            slot.manager.get_or_insert(songbird_guild_id)
        })) {
            Ok(call) => call,
            Err(_) => {
                let mut errors = vec!["songbird_manager_not_initialized".to_string()];
                errors.extend(
                    self.cleanup_failed_start(slot, metadata, None, sink, writer, wav_path)
                        .await,
                );
                return errors_to_result(errors);
            }
        };
        {
            let mut call_guard = call.lock().await;
            call_guard.add_global_event(
                CoreEvent::VoiceTick.into(),
                VoiceTickHandler { sink: sink.clone() },
            );
        }

        if let Err(err) = slot
            .manager
            .join(songbird_guild_id, songbird_channel_id)
            .await
        {
            let mut errors = vec![format!("songbird_join_failed: {err}")];
            errors.extend(
                self.cleanup_failed_start(slot, metadata, Some(call), sink, writer, wav_path)
                    .await,
            );
            return errors_to_result(errors);
        }

        let activation_error = match slot.state.lock() {
            Ok(mut state) => match &*state {
                SlotState::Starting(starting) if *starting == metadata => {
                    *state = SlotState::Recording(ActiveRecording {
                        metadata,
                        sink,
                        writer,
                    });
                    return Ok(());
                }
                _ => "slot_state_changed_during_start".to_string(),
            },
            Err(_) => {
                log_slot_fault(metadata, "slot_state_lock_poisoned");
                "slot_state_lock_poisoned".to_string()
            }
        };
        let mut errors = vec![activation_error];
        errors.extend(
            self.cleanup_failed_start(slot, metadata, Some(call), sink, writer, wav_path)
                .await,
        );
        errors_to_result(errors)
    }

    async fn stop(
        &self,
        identity: RecorderIdentity,
        guild_id: u64,
        voice_channel_id: u64,
    ) -> Result<(), String> {
        let slot = self.slot(identity);
        let metadata = RecordingMetadata {
            guild_id,
            voice_channel_id,
            recorder: identity,
        };
        let active = {
            let mut state = slot.state.lock().map_err(|_| {
                log_slot_fault(metadata, "slot_state_lock_poisoned");
                "slot_state_lock_poisoned".to_string()
            })?;
            let previous = std::mem::replace(&mut *state, SlotState::Stopping(metadata));
            match previous {
                SlotState::Recording(active) if active.metadata == metadata => active,
                SlotState::Idle => {
                    *state = SlotState::Idle;
                    return Ok(());
                }
                SlotState::Stopping(stopping) if stopping == metadata => {
                    *state = SlotState::Stopping(stopping);
                    return Ok(());
                }
                other => {
                    *state = other;
                    return Err("recording_identity_or_channel_mismatch".to_string());
                }
            }
        };

        active.sink.deactivate();
        let mut errors = Vec::new();
        let songbird_guild_id = songbird_guild_id(guild_id)?;
        if let Err(err) = slot.manager.remove(songbird_guild_id).await {
            errors.push(format!("songbird_remove_failed: {err}"));
            if let Some(call) = slot.manager.get(songbird_guild_id) {
                call.lock().await.remove_all_global_events();
            }
        }
        if let Err(err) = active.sink.queue.close() {
            errors.push(err);
        }
        if let Err(err) = active.writer.finish().await {
            errors.push(err);
        }
        if let Err(err) = self.force_idle(slot, metadata) {
            errors.push(err);
        }
        errors_to_result(errors)
    }
}

impl ActiveRecording {
    fn failure_health(&self) -> Result<(), String> {
        self.sink.failure.health()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use songbird::driver::{Channels, DecodeConfig, DecodeMode, SampleRate};

    use super::*;
    use crate::scrim_record::RecorderIdentity;

    fn metadata() -> RecordingMetadata {
        RecordingMetadata {
            guild_id: 1,
            voice_channel_id: 2,
            recorder: RecorderIdentity::MainBot,
        }
    }

    #[test]
    fn silent_tick_is_one_exact_zero_frame() {
        let mut mixer = StereoPcmMixer::default();

        let mixed = mixer.mix_voice_tick(std::iter::empty::<(u32, &[i16])>());

        assert_eq!(SAMPLES_PER_TICK, 1_920);
        assert_eq!(mixed.samples.len(), SAMPLES_PER_TICK);
        assert_eq!(mixed.samples, vec![0; SAMPLES_PER_TICK]);
        assert!(!mixed.overflowed);
    }

    #[test]
    fn one_speaker_is_unchanged() {
        let mut mixer = StereoPcmMixer::default();
        let speaker = (0..SAMPLES_PER_TICK)
            .map(|sample| sample as i16 - 960)
            .collect::<Vec<_>>();

        let mixed = mixer.mix_voice_tick([(1, speaker.as_slice())]);

        assert_eq!(mixed.samples.len(), SAMPLES_PER_TICK);
        assert_eq!(mixed.samples, speaker);
        assert!(!mixed.overflowed);
    }

    #[test]
    fn two_speakers_are_summed_samplewise_with_i16_clamp() {
        let mut mixer = StereoPcmMixer::default();
        let first = [30_000, -30_000, 100, -100]
            .into_iter()
            .cycle()
            .take(SAMPLES_PER_TICK)
            .collect::<Vec<_>>();
        let second = [10_000, -10_000, 23, -23]
            .into_iter()
            .cycle()
            .take(SAMPLES_PER_TICK)
            .collect::<Vec<_>>();
        let expected = [i16::MAX, i16::MIN, 123, -123]
            .into_iter()
            .cycle()
            .take(SAMPLES_PER_TICK)
            .collect::<Vec<_>>();

        let mixed = mixer.mix_voice_tick([(1, first.as_slice()), (2, second.as_slice())]);

        assert_eq!(mixed.samples.len(), SAMPLES_PER_TICK);
        assert_eq!(mixed.samples, expected);
        assert!(!mixed.overflowed);
    }

    #[test]
    fn short_speaker_frame_is_zero_padded_to_one_tick() {
        let mut mixer = StereoPcmMixer::default();
        let short = [7, -7, 11, -11];
        let mut expected = vec![0; SAMPLES_PER_TICK];
        expected[..short.len()].copy_from_slice(&short);

        let mixed = mixer.mix_voice_tick([(1, short.as_slice())]);

        assert_eq!(mixed.samples.len(), SAMPLES_PER_TICK);
        assert_eq!(mixed.samples, expected);
        assert!(!mixed.overflowed);
    }

    #[test]
    fn oversized_frame_carries_only_surplus_into_following_tick() {
        let mut mixer = StereoPcmMixer::default();
        let oversized = (0..SAMPLES_PER_TICK + 4)
            .map(|sample| sample as i16)
            .collect::<Vec<_>>();

        let first = mixer.mix_voice_tick([(1, oversized.as_slice())]);
        assert_eq!(first.samples.len(), SAMPLES_PER_TICK);
        assert_eq!(first.samples, oversized[..SAMPLES_PER_TICK]);
        assert!(!first.overflowed);
        assert_eq!(mixer.buffered_samples(), 4);

        let second = mixer.mix_voice_tick(std::iter::empty::<(u32, &[i16])>());
        let mut expected = vec![0; SAMPLES_PER_TICK];
        expected[..4].copy_from_slice(&oversized[SAMPLES_PER_TICK..]);
        assert_eq!(second.samples, expected);
        assert!(!second.overflowed);
        assert_eq!(mixer.buffered_samples(), 0);
    }

    #[test]
    fn oversized_frame_caps_carry_at_six_ticks_and_reports_loss() {
        let mut mixer = StereoPcmMixer::default();
        let oversized = vec![7; SAMPLES_PER_TICK * 8];

        let mixed = mixer.mix_voice_tick([(1, oversized.as_slice())]);

        assert!(mixed.overflowed);
        assert_eq!(mixer.buffered_samples(), SAMPLES_PER_TICK * 6);
        for _ in 0..6 {
            let carried = mixer.mix_voice_tick(std::iter::empty::<(u32, &[i16])>());
            assert_eq!(carried.samples, vec![7; SAMPLES_PER_TICK]);
        }
        assert_eq!(mixer.buffered_samples(), 0);
    }

    #[test]
    fn recording_manager_config_is_stereo_48khz_decode() {
        assert_eq!(
            recording_songbird_config().decode_mode,
            DecodeMode::Decode(DecodeConfig::new(Channels::Stereo, SampleRate::Hz48000))
        );
    }

    #[test]
    fn backend_constructor_keeps_managers_and_readiness_in_fixed_slots() {
        let main_manager = recording_songbird_manager();
        let worker_manager = recording_songbird_manager();
        let main_ready = Arc::new(AtomicBool::new(true));
        let worker_ready = Arc::new(AtomicBool::new(false));
        let backend = SongbirdRecordingBackend::new(
            main_manager.clone(),
            main_ready.clone(),
            worker_manager.clone(),
            worker_ready.clone(),
        )
        .expect("distinct managers");

        let main = backend.slot(RecorderIdentity::MainBot);
        let worker = backend.slot(RecorderIdentity::DiscordTokenWorker);
        assert!(Arc::ptr_eq(&main.manager, &main_manager));
        assert!(Arc::ptr_eq(&worker.manager, &worker_manager));
        assert!(!Arc::ptr_eq(&main.manager, &worker.manager));
        assert!(Arc::ptr_eq(&main.readiness, &main_ready));
        assert!(Arc::ptr_eq(&worker.readiness, &worker_ready));
        assert!(main.readiness.load(Ordering::Acquire));
        assert!(!worker.readiness.load(Ordering::Acquire));
    }

    #[test]
    fn backend_constructor_rejects_a_shared_manager() {
        let manager = recording_songbird_manager();

        let result = SongbirdRecordingBackend::new(
            manager.clone(),
            Arc::new(AtomicBool::new(true)),
            manager,
            Arc::new(AtomicBool::new(true)),
        );

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn bounded_queue_reports_full_and_sets_visible_failure() {
        let failure = Arc::new(RecordingFailure::new(metadata()));
        let (queue, _receiver) = FrameQueue::bounded(1, failure.clone());

        assert!(queue.try_send(vec![1, -1]).is_ok());
        let error = queue
            .try_send(vec![2, -2])
            .expect_err("second frame must exceed the bounded queue");

        assert!(error.contains("full"));
        assert!(failure.health().is_err());
    }

    #[tokio::test]
    async fn writer_creates_readable_stereo_48khz_wav_with_exact_samples() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("recording.wav");
        let samples = vec![1, -1, i16::MAX, i16::MIN, 23, -42];
        let failure = Arc::new(RecordingFailure::new(metadata()));
        let (queue, receiver) = FrameQueue::bounded(2, failure.clone());
        let writer = WavWriterTask::start(wav_path.clone(), receiver, failure)
            .await
            .expect("writer starts after opening the file");

        queue.try_send(samples.clone()).expect("queue frame");
        queue.close().expect("first close");
        queue.close().expect("idempotent close");
        writer.finish().await.expect("writer finalizes");

        let mut reader = hound::WavReader::open(&wav_path).expect("read wav");
        assert_eq!(
            reader.spec(),
            hound::WavSpec {
                channels: 2,
                sample_rate: 48_000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            }
        );
        let actual = reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .expect("wav samples");
        assert_eq!(actual, samples);
    }

    #[tokio::test]
    async fn writer_open_failure_is_returned_without_hanging() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let failure = Arc::new(RecordingFailure::new(metadata()));
        let (_queue, receiver) = FrameQueue::bounded(1, failure.clone());

        let result =
            WavWriterTask::start(temp_dir.path().to_path_buf(), receiver, failure.clone()).await;

        assert!(result.is_err());
        assert!(failure.health().is_err());
    }
}
