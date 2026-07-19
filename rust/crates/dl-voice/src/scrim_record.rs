use std::collections::{hash_map::Entry, HashMap};
use std::time::{Duration, Instant};

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

#[derive(Debug)]
struct RecordingSession {
    state: SessionState,
    _started_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartClaimError {
    AlreadyRecording,
}

#[derive(Debug, Default)]
struct RecordingRegistry {
    sessions: HashMap<u64, RecordingSession>,
}

impl RecordingRegistry {
    fn claim_start(
        &mut self,
        voice_channel_id: u64,
        started_at: Instant,
    ) -> Result<(), StartClaimError> {
        match self.sessions.entry(voice_channel_id) {
            Entry::Occupied(_) => Err(StartClaimError::AlreadyRecording),
            Entry::Vacant(entry) => {
                entry.insert(RecordingSession {
                    state: SessionState::Starting,
                    _started_at: started_at,
                });
                Ok(())
            }
        }
    }

    fn mark_recording(&mut self, voice_channel_id: u64) -> bool {
        match self.sessions.get_mut(&voice_channel_id) {
            Some(session) if session.state == SessionState::Starting => {
                session.state = SessionState::Recording;
                true
            }
            _ => false,
        }
    }

    fn claim_stop(&mut self, voice_channel_id: u64) -> bool {
        match self.sessions.get_mut(&voice_channel_id) {
            Some(session) if session.state == SessionState::Recording => {
                session.state = SessionState::Stopping;
                true
            }
            _ => false,
        }
    }

    fn rollback_start(&mut self, voice_channel_id: u64) -> bool {
        if self.state(voice_channel_id) != Some(SessionState::Starting) {
            return false;
        }
        self.sessions.remove(&voice_channel_id).is_some()
    }

    fn complete_stop(&mut self, voice_channel_id: u64) -> bool {
        if self.state(voice_channel_id) != Some(SessionState::Stopping) {
            return false;
        }
        self.sessions.remove(&voice_channel_id).is_some()
    }

    fn state(&self, voice_channel_id: u64) -> Option<SessionState> {
        self.sessions
            .get(&voice_channel_id)
            .map(|session| session.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    const AFTER_SCRIMCHANNEL_ID: u64 = 1_528_447_165_926_604_991;
    const EXPECTED_COACH_ROLE_ID: u64 = 1_494_372_744_286_965_941;
    const UNKNOWN_CHANNEL_ID: u64 = 42;
    const UNRELATED_ROLE_ID: u64 = 7;
    const SIXTY_MINUTES: Duration = Duration::from_secs(60 * 60);

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
        let mut registry = RecordingRegistry::default();
        let started_at = Instant::now();

        assert_eq!(registry.claim_start(10, started_at), Ok(()));
        assert_eq!(
            registry.claim_start(10, started_at),
            Err(StartClaimError::AlreadyRecording)
        );
    }

    #[test]
    fn registry_stop_without_active_session_is_noop() {
        let mut registry = RecordingRegistry::default();

        assert!(!registry.claim_stop(10));
        assert_eq!(registry.state(10), None);
    }

    #[test]
    fn registry_two_stop_claims_have_exactly_one_winner() {
        let mut registry = RecordingRegistry::default();
        assert_eq!(registry.claim_start(10, Instant::now()), Ok(()));
        assert!(registry.mark_recording(10));

        let claims = [registry.claim_stop(10), registry.claim_stop(10)];

        assert_eq!(claims.into_iter().filter(|claimed| *claimed).count(), 1);
        assert_eq!(registry.state(10), Some(SessionState::Stopping));
    }

    #[test]
    fn registry_starting_blocks_restart_until_session_is_removed() {
        let mut registry = RecordingRegistry::default();
        let started_at = Instant::now();

        let first = registry.claim_start(10, started_at);
        let while_starting = registry.claim_start(10, started_at);
        assert!(registry.rollback_start(10));
        let after_removal = registry.claim_start(10, started_at);

        assert_eq!(
            (first, while_starting, after_removal),
            (Ok(()), Err(StartClaimError::AlreadyRecording), Ok(()))
        );
    }

    #[test]
    fn registry_stopping_blocks_restart_until_session_is_removed() {
        let mut registry = RecordingRegistry::default();
        let started_at = Instant::now();
        assert_eq!(registry.claim_start(10, started_at), Ok(()));
        assert!(registry.mark_recording(10));
        assert!(registry.claim_stop(10));

        let while_stopping = registry.claim_start(10, started_at);
        assert!(registry.complete_stop(10));
        let after_removal = registry.claim_start(10, started_at);

        assert_eq!(
            (while_stopping, after_removal),
            (Err(StartClaimError::AlreadyRecording), Ok(()))
        );
    }

    #[test]
    fn registry_rejects_every_state_skip() {
        let mut registry = RecordingRegistry::default();

        assert!(!registry.mark_recording(10));
        assert!(!registry.claim_stop(10));
        assert!(!registry.rollback_start(10));
        assert!(!registry.complete_stop(10));

        assert_eq!(registry.claim_start(10, Instant::now()), Ok(()));
        assert!(!registry.claim_stop(10));
        assert!(!registry.complete_stop(10));
        assert_eq!(registry.state(10), Some(SessionState::Starting));

        assert!(registry.mark_recording(10));
        assert!(!registry.mark_recording(10));
        assert!(!registry.rollback_start(10));
        assert!(!registry.complete_stop(10));
        assert_eq!(registry.state(10), Some(SessionState::Recording));

        assert!(registry.claim_stop(10));
        assert!(!registry.mark_recording(10));
        assert!(!registry.rollback_start(10));
        assert_eq!(registry.state(10), Some(SessionState::Stopping));
    }
}
