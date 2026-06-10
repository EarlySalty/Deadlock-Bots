# Phase 4: dl-voice — der Voice-Cluster

Status: Kern KOMPLETT (4a #76, 4b #77, 4c/1–4 #78–#81). Stand: 2026-06-10

Querschnitts-Redesign umgesetzt: Statt fünf unkoordinierter
`voice_state_update`-Listener sind alle Teilsysteme Subscriber des EINEN
Voice-Dispatchers (dl-discord), plus `VoiceEvent::Update` für
Mute-Wechsel im selben Kanal.

## Module

| Modul | Original | Kern |
|---|---|---|
| `tracker` | voice_activity_tracker.py | Session-Lifecycle → voice_stats/voice_session_log (formatgleich), Grace-Rolle, Privacy, 3 Loops |
| `tempvoice::{logic,store,engine}` | tempvoice/core.py | Join-to-create (3 Stagings), Owner-Transfer/Backfill, Bann-Overwrites, Rename-Fenster, Rang-/Namens-Logik CPython-referenziert |
| `tempvoice::interface` | tempvoice/interface.py | Panel-Kern mit identischen custom_ids (Region/Claim/Limit/Kick/Ban/Unban/Templates/Presets/RankPref/Rename) |
| `status` | deadlock_voice_status.py + service/deadlock_voice_cohort.py | Presence→Kohorte→Suffix, Party-Aufstockung, Rename-Disziplin, deadlock_voice_watch |
| `rank` | rank_voice_manager.py | Anker (Erstbesitzer-Vorrang), ±9-Score-Fenster, Batch-Overwrites, voice_channel_anchors |
| `nudge` | steam_link_voice_nudge.py | 2.-Tag+30min-DM, kv-/steam_nudge_state-Verträge, nudge_close-Button |

Nicht portiert: `voice_reaction_dm.py` — falsch einsortierter
Twitch-Sales-Lead-Poller (Postgres, default aus) → gehört in den
tb-*-Stack des Twitch-Bots.

## Offene Rest-Lücken (vor dem Voice-Cutover nachziehen)

1. **Tag-Filter** (LaneTagFilter, ragebaiter/min_age/tone) + Panel `tv_tag_filter`.
2. **Lurker-Modus** (Rolle+Nick+Limit-Anpassung) + Panel `tv_lurker`.
3. **Router-/Duo-/New-Player-Lanes** (router.py, duo_lanes.py,
   new_player_lanes.py, lane_sorting.py) + Panel `tv_mode_switch_*`.
4. **Min-Rang-Caps via Panel** (`tv_minrank`/`tv_subrank_perm` — Backend
   in dl-voice::rank vorhanden, Verdrahtung fehlt).
5. **Feedback-DM-System** des Trackers (erste Session ≥ 5 min →
   Feedback-Modal, Zweit-Feedback ≥ 4 Tage).
6. Startup-Purge leerer Lanes (engine.rehydrate räumt noch nicht).

Die nicht freigeschalteten Panel-Buttons antworten ehrlich
("noch nicht freigeschaltet") statt still zu scheitern.

## Cutover (user-gated, gemeinsam mit dem Bot-Cutover aus docs/04)

Blocklist-Einträge: voice_activity_tracker, tempvoice (alle Module),
deadlock_voice_status, rank_voice_manager, steam_link_voice_nudge.
Reihenfolge: Rest-Lücken schließen → DL_BOT_GATEWAY=1-Flip zusammen mit
Broker/Bridges → Python-Cogs blocklisten → Bot-Neustart.
