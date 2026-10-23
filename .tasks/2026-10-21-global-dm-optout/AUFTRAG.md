# Global bot DM opt-out

Trigger: user 292362192219668480 received four unrelated bot DMs (router lane, solo LFG nudge, then after "Nie fragen" still a voice feedback DM on 2026-10-21).

## Goal

- One persisted global opt-out: `core.dm_optout` (migration `rust/crates/dl-central-db/migrations/20261021120000_global_dm_optout.sql`).
- One gate: `rust/crates/dl-discord/src/dm_gate.rs`. `DiscordAdapter::open_dm_channel` is the only place in the workspace that opens a DM channel; `deliver_dm` sends through it and appends the opt-out button to every proactive DM.
- One button for every proactive DM: custom_id `dm_optout:global`, label "Keine Bot-Nachrichten mehr", confirmation "Alles klar, der Bot schreibt dir ab jetzt keine Nachrichten mehr von sich aus. Antworten auf deine eigenen Befehle und Klicks bekommst du weiterhin."
- Undo: the ephemeral button confirmation carries `dm_optout:undo` ("Doch wieder Nachrichten erhalten"), which deletes the user's `core.dm_optout` row and replies "Alles klar, der Bot darf dir wieder schreiben."
- Every feature opt-out sets the global flag through DB triggers (works for sibling repos without code changes). The triggers fire on insert, update and delete and call `core.refresh_dm_optout(user_id)`, which recomputes the flag from all feature sources: a reversal removes a feature-sourced row when no feature source is left and switches to another still-active source otherwise. Rows with source `dm_optout_button` are never touched by feature reversals, and rows of erased users (`core.user_privacy.deleted_at`) survive the source deletes of the erasure. The "Nie fragen" confirmations of pairing, mate survey and solo LFG now show the global confirmation text.
- Backfill: every user with any feature opt-out gets the global flag. `core.user_privacy.opted_out` (data privacy) is not a DM opt-out and is not used.
- Solo LFG nudge switched off: `bin/dl-bot/src/main.rs` builds `SoloWatch::without_prompts`, so the loop still restores, expires and deletes LFG posts but never starts a nudge DM. Stored kv data stays; old "Nie fragen" buttons keep working.
- `DmError::OptedOut` is a definitive outcome, never "uncertain": concierge gets `ConciergeDmDelivery::OptedOut` (no fallback, Pate sees `PATE_CLAIM_OPTED_OUT_TEXT`), team application status reports the opt-out to the moderator, scrim DM effects go `dead` with `err_discord_dm_opted_out`.
- Contract test: `rust/crates/dl-discord/tests/dm_gate_contract.rs` (opted-out user gets no proactive DM, exempt paths pass, only the gate opens DM channels).

## Exemptions (not gated)

| Intent | Where | Reason |
|---|---|---|
| Moderation | `bin/dl-bot/src/modglue.rs:1538` Ragebaiter notice | moderation notice |
| Reply | `bin/dl-bot/src/modglue.rs:3603` reaction role DM | direct answer to the user's own reaction |
| Cleanup | `crates/dl-voice/src/glue.rs:2429` delete stale feedback prompt | deletes a bot message, sends nothing |
| Internal | `bin/dl-bot/src/modglue.rs:3489` feedback hub forward, `bin/dl-bot/src/modglue.rs:2730` clip contest curator, `crates/dl-voice/src/glue.rs:2419` voice feedback forward to owner | staff notifications, no opt-out button |
| Moderation / Security | broker `send-message` / `send_dm` with `dm_purpose` = `moderation` / `security` (`crates/dl-discord/src/adapter.rs:543`) | callers must state it explicitly; default is proactive |

## DM senders in dl-bot (before at 4badadd1, after on this branch)

| Before (file:line) | After (file:line) | Feature | Before | After |
|---|---|---|---|---|
| crates/dl-voice/src/glue.rs:385 | crates/dl-voice/src/glue.rs:383 (callers 2553, 3121, 3226) | router intro, mate survey, voice pairing | ungated | gated + button |
| crates/dl-voice/src/glue.rs:1676 | crates/dl-voice/src/glue.rs:1678 | Steam link nudge | ungated | gated + button |
| crates/dl-voice/src/glue.rs:1805, bin/dl-bot/src/main.rs:1664 | crates/dl-voice/src/glue.rs:1798, loop runs `without_prompts` | solo LFG nudge | ungated, running | off (cleanup only), gated |
| crates/dl-voice/src/glue.rs:2409 | crates/dl-voice/src/glue.rs:2404 | voice feedback | ungated | gated + button |
| crates/dl-voice/src/glue.rs:2438 | crates/dl-voice/src/glue.rs:2419 | voice feedback forward to owner | ungated | exempt (internal) |
| crates/dl-voice/src/glue.rs:2451 | crates/dl-voice/src/glue.rs:2429 | delete feedback prompt | ungated | exempt (cleanup) |
| crates/dl-voice/src/glue.rs:2559 | crates/dl-voice/src/glue.rs:2538 | router text DM | ungated | gated + button |
| crates/dl-voice/src/glue.rs:2818 | crates/dl-voice/src/glue.rs:2797 | LFG watch | ungated | gated + button |
| crates/dl-activity/src/survey_pulse.rs:497 | crates/dl-activity/src/survey_pulse.rs:497 | survey pulse (eligibility SQL also checks `core.dm_optout`) | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:1536 | bin/dl-bot/src/modglue.rs:1538 | Ragebaiter moderation notice | ungated | exempt (moderation) |
| bin/dl-bot/src/modglue.rs:2460 | bin/dl-bot/src/modglue.rs:2460 | coaching notification | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:2544 | bin/dl-bot/src/modglue.rs:2540 | leave survey | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:2638 | bin/dl-bot/src/modglue.rs:2638 | clip dump | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:2728 | bin/dl-bot/src/modglue.rs:2730 | clip contest curator | ungated | exempt (internal) |
| bin/dl-bot/src/modglue.rs:3117 | bin/dl-bot/src/modglue.rs:3122 | concierge DM (no guild-channel fallback when opted out) | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:3485 | bin/dl-bot/src/modglue.rs:3489 | feedback hub forward | ungated | exempt (internal) |
| bin/dl-bot/src/modglue.rs:3598 | bin/dl-bot/src/modglue.rs:3603 | reaction role DM | ungated | exempt (reply) |
| bin/dl-bot/src/modglue.rs:4131 | bin/dl-bot/src/modglue.rs:4134 | team application status | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:4372 | bin/dl-bot/src/modglue.rs:4374 | coaching request (incl. ban notice, mirrored in channel) | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:4427 | bin/dl-bot/src/modglue.rs:4421 | coaching survey | ungated | gated + button |
| bin/dl-bot/src/modglue.rs:4550 | bin/dl-bot/src/modglue.rs:4538 | retention "miss you" | ungated | gated + button |
| bin/dl-bot/src/scrimglue.rs:3039 | bin/dl-bot/src/scrimglue.rs:3042 | scrim DMs | ungated | gated + button |
| crates/dl-discord/src/adapter.rs:314 (broker `send-message`/`send_dm`) | crates/dl-discord/src/adapter.rs:543 | broker relay for siblings | ungated | gated (proactive default) + button; 403 `dm_opted_out` |

## Sibling repos (read only)

| Repo | file:line | Path | State after this change |
|---|---|---|---|
| Deadlock-Turniere | reminders.rs:165/243/300, loop_runner.rs:314/325, admin/tournaments.rs:264, lobby.rs:454/578, operations.rs:263, invitations.rs:201, teams.rs:602 | dl-bot broker `send-message` | gated; turnier opt-outs (`turnier.tournament_dm_optout`, `turnier.user_profiles.notify_discord_dm=false`) set the global flag by trigger |
| Deadlock-Twitch-Bot | token_lifecycle.rs:425, 537, 768; deadlock_pause.rs:450; social_reauth.rs:54; social_autopilot_notify.rs:26; raid_oauth_impl.rs:1102; eventsub_hooks.rs:115; deadlock_twitch_bot_watchdog.py:167 | broker relay for the community guild; own token for other guilds | community-guild DMs gated; open: security DMs (token_lifecycle.rs:425/768) should send `dm_purpose: "security"`, own-token DMs for other guilds are not gated |
| Deadlock-Steam-Bot | send_dm exists, no production callers | none | nothing to do |
| Deadlock-Brain | no DMs | none | nothing to do |
