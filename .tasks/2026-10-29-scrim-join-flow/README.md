# Scrim join flow: bot grants the participant role

Date: 2026-10-29

## Trigger

The operator asked the bot by DM "Kann ich bei den Scrims mitmachen". The Brain answered from
Deadlock-Docs `public/discord-server/scrims.html` with outdated steps (read rules first, get the role
via checkmark or by asking the bot by DM, then `/scrim-signup`).

## Live state found (2026-10-29)

- Roles: `Scrims Teilnehmer` (1520849762851618817, pos 170), team roles `Shiv-support - Team`,
  `Urn Runners Team`, `Return Fire Department`, `TetraHex - Team`, plus `Team Coach` and
  `Scrim Match Leiter` (dashboard access). There is no role called Scrim-Manager.
- Bot role `Deadlock Master Bot` sits at pos 211, so it can assign `Scrims Teilnehmer`.
- Channels in category Scrims: `scrim-channel` and `scrimregeln-channel` are visible to
  `Scrims Teilnehmer`; team text/voice channels and `announcement-scrims` only to team roles and
  coaches. Without the participant role the rules channel is invisible, so "read the rules first"
  could not work.
- Runtime gate: `scrim.runtime_control` is `legacy/dl-bots`, so `/scrim-signup` writes are allowed
  live; `SCRIM_SIGNUP_REPLY_RUNTIME_DENIED` does not fire.
- Checkmark flow: `bot.reaction_role_mappings` ids 1 and 2 (#ankündigungen messages from 28.06. and
  16.07.) are still active and grant the role; last reaction DM 2026-07-31. It only works on those two
  old posts.
- "Ask the bot by DM for the role": no such action exists in code.
- `/scrim-signup` stored the signup but granted no role. Web form `/coaching/scrims/signup` exists
  (HTTP 200) and also grants no role.

## Change

- `dl-community::scrim_signup`: after a successful save, the bot grants `Scrims Teilnehmer` through the
  existing `ReactionRolePort` (new `grant_role` with Discord audit reason
  `scrim-signup:teilnehmer-rolle`, mirrored into `core.discord_audit_log`). It skips members who already
  have the role. If the grant fails, the signup stays saved and the reply says the role is missing.
  Team roles are never touched.
- Deadlock-Docs `public/discord-server/scrims.html` rewritten to the real flow.

## Checks

- clippy `-p dl-community -p dl-bot --all-targets`: no new findings (only pre-existing async_trait
  `must_use` noise repo-wide).
- `cargo-slot test -p dl-community --features testing -- --include-ignored scrim_signup reaction_roles`
  on an ionos throwaway DB: 21 passed, 0 failed, 0 ignored. The throwaway DB needs `CREATE ROLE deadlock`
  first, because migration 2026092801 grants to that role.

## Open questions for the operator

- Should the two old checkmark mappings in #ankündigungen stay active?
- Should the web form also grant the role?
