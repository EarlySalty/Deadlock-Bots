use std::sync::Arc;
use std::time::Duration;

use dl_discord::{CommandSpec, InteractionHandler, InteractionRouter, VoiceEvent};
use serde_json::json;

pub mod audio;
pub mod service;

pub use audio::{recording_songbird_manager, SongbirdRecordingBackend};
pub use service::{
    prepare_recording_temp_dir, register, spawn, AudioTranscoder, FfmpegTranscoder,
    RecordCommandHandler, RecordingBackend, ScrimRecordPort, ScrimRecorder, StartError, StopError,
};

const SCRIM_GUILD_ID: u64 = 1_289_721_245_281_292_288;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamChannelConfig {
    pub voice_channel_id: u64,
    pub team_role_id: u64,
    pub text_channel_id: u64,
}

const COACH_ROLE_ID: u64 = 1_494_372_744_286_965_941;
const MAX_RECORDING_DURATION: Duration = Duration::from_secs(60 * 60);
const TEAM_VOICE_CHANNELS: &[TeamChannelConfig] = &[
    TeamChannelConfig {
        voice_channel_id: 1_521_167_264_533_970_954,
        team_role_id: 1_521_163_175_318_388_847,
        text_channel_id: 1_521_164_348_976_922_795,
    },
    TeamChannelConfig {
        voice_channel_id: 1_521_167_309_828_526_183,
        team_role_id: 1_521_163_233_245_794_465,
        text_channel_id: 1_521_164_426_424_750_171,
    },
    TeamChannelConfig {
        voice_channel_id: 1_521_167_476_711_358_505,
        team_role_id: 1_521_163_300_480_483_498,
        text_channel_id: 1_521_164_444_787_540_110,
    },
    TeamChannelConfig {
        voice_channel_id: 1_521_939_025_982_652_416,
        team_role_id: 1_521_163_334_211_338_391,
        text_channel_id: 1_521_164_466_169_970_728,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperateDenied {
    UnknownChannel,
    NotPermitted,
}

fn team_channel_config(voice_channel_id: u64) -> Option<TeamChannelConfig> {
    TEAM_VOICE_CHANNELS
        .iter()
        .copied()
        .find(|config| config.voice_channel_id == voice_channel_id)
}

pub fn can_operate(
    voice_channel_id: u64,
    member_role_ids: &[u64],
) -> Result<TeamChannelConfig, OperateDenied> {
    let config = team_channel_config(voice_channel_id).ok_or(OperateDenied::UnknownChannel)?;
    if member_role_ids.contains(&config.team_role_id) || member_role_ids.contains(&COACH_ROLE_ID) {
        Ok(config)
    } else {
        Err(OperateDenied::NotPermitted)
    }
}

fn duration_cap_reached(elapsed: Duration) -> bool {
    elapsed >= MAX_RECORDING_DURATION
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionState {
    Starting,
    Recording,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecorderIdentity {
    MainBot,
    SecondaryBot,
}

fn affected_team_voice_channels(event: &VoiceEvent) -> Vec<u64> {
    let guild_id = match event {
        VoiceEvent::Join { guild_id, .. }
        | VoiceEvent::Leave { guild_id, .. }
        | VoiceEvent::Move { guild_id, .. }
        | VoiceEvent::Update { guild_id, .. } => *guild_id,
    };
    if guild_id != SCRIM_GUILD_ID {
        return Vec::new();
    }

    let candidates = match event {
        VoiceEvent::Join { channel_id, .. }
        | VoiceEvent::Leave { channel_id, .. }
        | VoiceEvent::Update { channel_id, .. } => [Some(*channel_id), None],
        VoiceEvent::Move {
            from_channel_id,
            to_channel_id,
            ..
        } => [Some(*from_channel_id), Some(*to_channel_id)],
    };
    let mut affected = Vec::with_capacity(2);
    for channel_id in candidates.into_iter().flatten() {
        if team_channel_config(channel_id).is_some() && !affected.contains(&channel_id) {
            affected.push(channel_id);
        }
    }
    affected
}

fn record_command_spec() -> CommandSpec {
    const RECORD_DESCRIPTION: &str =
        "Aufnahme im Team-Sprachkanal starten, stoppen und als MP3 bereitstellen";
    const START_DESCRIPTION: &str = "Aufnahme im aktuellen Team-Sprachkanal starten";
    const STOP_DESCRIPTION: &str = "Aufnahme im aktuellen Team-Sprachkanal beenden";
    CommandSpec {
        definition: json!({
            "name": "record",
            "description": RECORD_DESCRIPTION,
            "options": [
                {
                    "type": 1,
                    "name": "start",
                    "description": START_DESCRIPTION,
                },
                {
                    "type": 1,
                    "name": "stop",
                    "description": STOP_DESCRIPTION,
                },
            ],
        }),
    }
}

fn register_record_commands(router: &mut InteractionRouter, handler: Arc<dyn InteractionHandler>) {
    let spec = record_command_spec();
    router.on_command("record start", spec.clone(), handler.clone());
    router.on_command("record stop", spec, handler);
}

#[cfg(test)]
mod tests {
    use super::service::{RegistryAllocator, StartError};
    use super::*;
    use dl_discord::{BridgeInteraction, BridgeReply};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    const AFTER_SCRIMCHANNEL_ID: u64 = 1_528_447_165_926_604_991;
    const EXPECTED_COACH_ROLE_ID: u64 = 1_494_372_744_286_965_941;
    const UNKNOWN_CHANNEL_ID: u64 = 42;
    const UNRELATED_ROLE_ID: u64 = 7;
    const SIXTY_MINUTES: Duration = Duration::from_secs(60 * 60);

    struct RouterContractHandler;

    #[async_trait::async_trait]
    impl InteractionHandler for RouterContractHandler {
        async fn handle(&self, _interaction: BridgeInteraction) -> BridgeReply {
            unreachable!()
        }
    }

    const EXPECTED_TEAM_CHANNELS: [TeamChannelConfig; 4] = [
        TeamChannelConfig {
            voice_channel_id: 1_521_167_264_533_970_954,
            team_role_id: 1_521_163_175_318_388_847,
            text_channel_id: 1_521_164_348_976_922_795,
        },
        TeamChannelConfig {
            voice_channel_id: 1_521_167_309_828_526_183,
            team_role_id: 1_521_163_233_245_794_465,
            text_channel_id: 1_521_164_426_424_750_171,
        },
        TeamChannelConfig {
            voice_channel_id: 1_521_167_476_711_358_505,
            team_role_id: 1_521_163_300_480_483_498,
            text_channel_id: 1_521_164_444_787_540_110,
        },
        TeamChannelConfig {
            voice_channel_id: 1_521_939_025_982_652_416,
            team_role_id: 1_521_163_334_211_338_391,
            text_channel_id: 1_521_164_466_169_970_728,
        },
    ];

    fn claim_start(
        state: &mut RegistryAllocator,
        voice_channel_id: u64,
        started_at: Instant,
        ready: [bool; 2],
    ) -> Result<RecorderIdentity, StartError> {
        state.claim_start(
            TeamChannelConfig {
                voice_channel_id,
                team_role_id: 1,
                text_channel_id: 2,
            },
            started_at,
            PathBuf::from(format!("{voice_channel_id}.wav")),
            ready,
        )
    }

    #[test]
    fn team_channel_config_maps_all_full_configs() {
        let actual =
            EXPECTED_TEAM_CHANNELS.map(|expected| team_channel_config(expected.voice_channel_id));
        let expected = EXPECTED_TEAM_CHANNELS.map(Some);

        assert_eq!(actual, expected);
    }

    #[test]
    fn team_channel_config_rejects_arbitrary_unknown_channel() {
        assert_eq!(team_channel_config(UNKNOWN_CHANNEL_ID), None);
    }

    #[test]
    fn team_channel_config_rejects_after_scrimchannel_explicitly() {
        assert_eq!(team_channel_config(AFTER_SCRIMCHANNEL_ID), None);
    }

    #[test]
    fn can_operate_accepts_matching_team_role() {
        for config in EXPECTED_TEAM_CHANNELS {
            assert_eq!(
                can_operate(config.voice_channel_id, &[config.team_role_id]),
                Ok(config)
            );
        }
    }

    #[test]
    fn can_operate_rejects_team_role_in_another_team_channel() {
        for index in 0..EXPECTED_TEAM_CHANNELS.len() {
            let own_team = EXPECTED_TEAM_CHANNELS[index];
            let other_team = EXPECTED_TEAM_CHANNELS[(index + 1) % EXPECTED_TEAM_CHANNELS.len()];

            assert_eq!(
                can_operate(other_team.voice_channel_id, &[own_team.team_role_id]),
                Err(OperateDenied::NotPermitted)
            );
        }
    }

    #[test]
    fn can_operate_accepts_coach_role_in_every_team_channel() {
        for config in EXPECTED_TEAM_CHANNELS {
            assert_eq!(
                can_operate(config.voice_channel_id, &[EXPECTED_COACH_ROLE_ID]),
                Ok(config)
            );
        }
    }

    #[test]
    fn can_operate_rejects_unrelated_role_in_every_team_channel() {
        for config in EXPECTED_TEAM_CHANNELS {
            assert_eq!(
                can_operate(config.voice_channel_id, &[UNRELATED_ROLE_ID]),
                Err(OperateDenied::NotPermitted)
            );
        }
    }

    #[test]
    fn can_operate_distinguishes_unknown_channels_even_for_coach() {
        for voice_channel_id in [UNKNOWN_CHANNEL_ID, AFTER_SCRIMCHANNEL_ID] {
            assert_eq!(
                can_operate(voice_channel_id, &[EXPECTED_COACH_ROLE_ID]),
                Err(OperateDenied::UnknownChannel)
            );
        }
    }

    #[test]
    fn duration_cap_does_not_trigger_just_below_sixty_minutes() {
        assert!(!duration_cap_reached(
            SIXTY_MINUTES - Duration::from_millis(1)
        ));
    }

    #[test]
    fn duration_cap_triggers_at_exactly_sixty_minutes() {
        assert!(duration_cap_reached(SIXTY_MINUTES));
    }

    #[test]
    fn duration_cap_triggers_just_above_sixty_minutes() {
        assert!(duration_cap_reached(
            SIXTY_MINUTES + Duration::from_millis(1)
        ));
    }

    #[test]
    fn registry_rejects_duplicate_start_claim() {
        let mut registry = RegistryAllocator::default();
        let started_at = Instant::now();

        assert_eq!(
            claim_start(&mut registry, 10, started_at, [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
        assert_eq!(
            claim_start(&mut registry, 10, started_at, [true, true]),
            Err(StartError::AlreadyRecording)
        );
    }

    #[test]
    fn registry_stop_without_active_session_is_noop() {
        let mut registry = RegistryAllocator::default();

        assert!(registry.claim_stop(10).is_none());
        assert_eq!(registry.state(10), None);
    }

    #[test]
    fn registry_two_stop_claims_have_exactly_one_winner() {
        let mut registry = RegistryAllocator::default();
        assert_eq!(
            claim_start(&mut registry, 10, Instant::now(), [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
        assert!(registry.mark_recording(10, RecorderIdentity::MainBot));

        let claims = [
            registry.claim_stop(10).is_some(),
            registry.claim_stop(10).is_some(),
        ];

        assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);
        assert_eq!(registry.state(10), Some(SessionState::Stopping));
    }

    #[test]
    fn registry_starting_blocks_restart_until_session_is_removed() {
        let mut registry = RegistryAllocator::default();
        let started_at = Instant::now();

        let first = claim_start(&mut registry, 10, started_at, [true, true]);
        let while_starting = claim_start(&mut registry, 10, started_at, [true, true]);
        assert!(registry.rollback_start(10, RecorderIdentity::MainBot));
        let after_removal = claim_start(&mut registry, 10, started_at, [true, true]);

        assert_eq!(
            (first, while_starting, after_removal),
            (
                Ok(RecorderIdentity::MainBot),
                Err(StartError::AlreadyRecording),
                Ok(RecorderIdentity::MainBot)
            )
        );
    }

    #[test]
    fn registry_stopping_blocks_restart_until_session_is_removed() {
        let mut registry = RegistryAllocator::default();
        let started_at = Instant::now();
        assert_eq!(
            claim_start(&mut registry, 10, started_at, [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
        assert!(registry.mark_recording(10, RecorderIdentity::MainBot));
        assert!(registry.claim_stop(10).is_some());

        let while_stopping = claim_start(&mut registry, 10, started_at, [true, true]);
        assert!(registry.complete_stop(10, RecorderIdentity::MainBot));
        let after_removal = claim_start(&mut registry, 10, started_at, [true, true]);

        assert_eq!(
            (while_stopping, after_removal),
            (
                Err(StartError::AlreadyRecording),
                Ok(RecorderIdentity::MainBot)
            )
        );
    }

    #[test]
    fn registry_rejects_every_state_skip() {
        let mut registry = RegistryAllocator::default();

        assert!(!registry.mark_recording(10, RecorderIdentity::MainBot));
        assert!(registry.claim_stop(10).is_none());
        assert!(!registry.rollback_start(10, RecorderIdentity::MainBot));
        assert!(!registry.complete_stop(10, RecorderIdentity::MainBot));

        assert_eq!(
            claim_start(&mut registry, 10, Instant::now(), [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
        assert!(registry.claim_stop(10).is_none());
        assert!(!registry.complete_stop(10, RecorderIdentity::MainBot));
        assert_eq!(registry.state(10), Some(SessionState::Starting));

        assert!(registry.mark_recording(10, RecorderIdentity::MainBot));
        assert!(!registry.mark_recording(10, RecorderIdentity::MainBot));
        assert!(!registry.rollback_start(10, RecorderIdentity::MainBot));
        assert!(!registry.complete_stop(10, RecorderIdentity::MainBot));
        assert_eq!(registry.state(10), Some(SessionState::Recording));

        assert!(registry.claim_stop(10).is_some());
        assert!(!registry.mark_recording(10, RecorderIdentity::MainBot));
        assert!(!registry.rollback_start(10, RecorderIdentity::MainBot));
        assert_eq!(registry.state(10), Some(SessionState::Stopping));
    }

    #[test]
    fn allocator_uses_both_recorders_and_rejects_third_channel() {
        let mut state = RegistryAllocator::default();
        let started_at = Instant::now();

        assert_eq!(
            claim_start(&mut state, 10, started_at, [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
        assert_eq!(
            claim_start(&mut state, 20, started_at, [true, true]),
            Ok(RecorderIdentity::SecondaryBot)
        );
        assert_eq!(
            claim_start(&mut state, 30, started_at, [true, true]),
            Err(StartError::NoCapacity)
        );
        assert_eq!(
            claim_start(&mut state, 10, started_at, [true, true]),
            Err(StartError::AlreadyRecording)
        );
    }

    #[test]
    fn allocator_skips_an_offline_recorder() {
        let mut state = RegistryAllocator::default();
        let started_at = Instant::now();

        assert_eq!(
            claim_start(&mut state, 10, started_at, [false, true]),
            Ok(RecorderIdentity::SecondaryBot)
        );
        assert_eq!(
            claim_start(&mut state, 20, started_at, [false, true]),
            Err(StartError::NoCapacity)
        );

        let mut all_offline = RegistryAllocator::default();
        assert_eq!(
            claim_start(&mut all_offline, 30, started_at, [false, false]),
            Err(StartError::NoCapacity)
        );
    }

    #[test]
    fn allocator_releases_only_matching_slot_after_stopping_cleanup() {
        let mut state = RegistryAllocator::default();
        let started_at = Instant::now();
        assert_eq!(
            claim_start(&mut state, 10, started_at, [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
        assert_eq!(
            claim_start(&mut state, 20, started_at, [true, true]),
            Ok(RecorderIdentity::SecondaryBot)
        );
        assert!(state.mark_recording(10, RecorderIdentity::MainBot));
        assert!(state.mark_recording(20, RecorderIdentity::SecondaryBot));

        assert!(!state.complete_stop(10, RecorderIdentity::MainBot));
        assert!(state.claim_stop(10).is_some());
        assert!(!state.complete_stop(20, RecorderIdentity::MainBot));
        assert!(!state.complete_stop(10, RecorderIdentity::SecondaryBot));
        assert_eq!(
            claim_start(&mut state, 30, started_at, [true, true]),
            Err(StartError::NoCapacity)
        );

        assert!(state.complete_stop(10, RecorderIdentity::MainBot));
        assert_eq!(
            claim_start(&mut state, 30, started_at, [true, true]),
            Ok(RecorderIdentity::MainBot)
        );
    }

    #[test]
    fn voice_events_identify_affected_team_channels() {
        let first = EXPECTED_TEAM_CHANNELS[0].voice_channel_id;
        let second = EXPECTED_TEAM_CHANNELS[1].voice_channel_id;

        let cases = [
            (
                VoiceEvent::Join {
                    guild_id: SCRIM_GUILD_ID,
                    user_id: 1,
                    channel_id: first,
                },
                vec![first],
            ),
            (
                VoiceEvent::Leave {
                    guild_id: SCRIM_GUILD_ID,
                    user_id: 1,
                    channel_id: first,
                },
                vec![first],
            ),
            (
                VoiceEvent::Move {
                    guild_id: SCRIM_GUILD_ID,
                    user_id: 1,
                    from_channel_id: first,
                    to_channel_id: second,
                },
                vec![first, second],
            ),
            (
                VoiceEvent::Update {
                    guild_id: SCRIM_GUILD_ID,
                    user_id: 1,
                    channel_id: second,
                    was_muted: false,
                    is_muted: true,
                },
                vec![second],
            ),
        ];

        for (event, expected) in cases {
            assert_eq!(affected_team_voice_channels(&event), expected);
        }
    }

    #[test]
    fn voice_event_move_deduplicates_the_same_channel() {
        let channel_id = EXPECTED_TEAM_CHANNELS[0].voice_channel_id;
        let event = VoiceEvent::Move {
            guild_id: SCRIM_GUILD_ID,
            user_id: 1,
            from_channel_id: channel_id,
            to_channel_id: channel_id,
        };

        assert_eq!(affected_team_voice_channels(&event), vec![channel_id]);
    }

    #[test]
    fn voice_events_exclude_after_scrim_and_other_guilds() {
        let team_channel_id = EXPECTED_TEAM_CHANNELS[0].voice_channel_id;
        let after_scrim = VoiceEvent::Join {
            guild_id: SCRIM_GUILD_ID,
            user_id: 1,
            channel_id: AFTER_SCRIMCHANNEL_ID,
        };
        let other_guild = VoiceEvent::Leave {
            guild_id: SCRIM_GUILD_ID + 1,
            user_id: 1,
            channel_id: team_channel_id,
        };
        let move_to_after_scrim = VoiceEvent::Move {
            guild_id: SCRIM_GUILD_ID,
            user_id: 1,
            from_channel_id: team_channel_id,
            to_channel_id: AFTER_SCRIMCHANNEL_ID,
        };

        assert!(affected_team_voice_channels(&after_scrim).is_empty());
        assert!(affected_team_voice_channels(&other_guild).is_empty());
        assert_eq!(
            affected_team_voice_channels(&move_to_after_scrim),
            vec![team_channel_id]
        );
    }

    #[test]
    fn record_command_definition_has_exactly_start_and_stop_without_options() {
        let spec = record_command_spec();
        assert_eq!(
            spec.definition.get("name").and_then(|v| v.as_str()),
            Some("record")
        );
        let options = spec
            .definition
            .get("options")
            .and_then(|value| value.as_array())
            .expect("record options");

        assert_eq!(options.len(), 2);
        for (option, expected_name) in options.iter().zip(["start", "stop"]) {
            assert_eq!(option.get("type").and_then(|value| value.as_u64()), Some(1));
            assert_eq!(
                option.get("name").and_then(|value| value.as_str()),
                Some(expected_name)
            );
            assert!(option.get("options").is_none());
            let description = option
                .get("description")
                .and_then(|value| value.as_str())
                .expect("subcommand description");
            assert!(!description.contains("Platzhalter"));
            assert!(description.contains("Team-Sprachkanal"));
        }
        let description = spec
            .definition
            .get("description")
            .and_then(|value| value.as_str())
            .expect("record description");
        assert!(!description.contains("Platzhalter"));
        assert!(description.contains("MP3"));
    }

    #[test]
    fn record_routes_share_one_deduplicated_top_level_definition() {
        let mut router = InteractionRouter::new();
        register_record_commands(&mut router, Arc::new(RouterContractHandler));

        assert!(router.resolve_command("record start").is_some());
        assert!(router.resolve_command("record stop").is_some());
        let definitions = router.command_definitions();
        assert_eq!(definitions, vec![record_command_spec().definition]);
    }
}
