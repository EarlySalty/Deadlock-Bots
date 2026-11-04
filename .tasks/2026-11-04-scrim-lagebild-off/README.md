# Scrim-Lagebild off, tournament proposals off

User request 2026-11-04: switch off the routine tournament proposals and the AI scrim Lagebild. The AI behind a Lagebild should simply be the Deadlock Brain, on request, able to act within the user's rights.

## Done
- Turniere: `routine_tournaments_enabled = false` in the live `~/.config/deadlock-turniere/bot.toml` (backup `.bak-20261104`). Scheduler logs `decision="skipped" reason="feature_disabled"`.
- Deadlock-Bots: `runtime.community.scrim_lagebild` (`DL_SCRIM_LAGEBILD_ENABLED`), default off. Off means no weekly run, no post-match Lagebild, no internal refresh/correction routes, no Lagebild AI provider, dashboard section hidden and correction API 404.

## Acceptance
- dl-bot log shows "Scrim-Lagebild deaktiviert" after deploy, no new rows in `scrim.lagebild_snapshots`.
- Admin dashboard Scrims tab no longer shows "Team-Lagebilder".

## Follow-up (separate task)
- Lagebild on request through the Deadlock Brain with permission-scoped data and actions. Related to the Brain permission-scoped knowledge wish of 2026-11-02.

## Known
- DB-backed Lagebild tests fail on the ionos test DB with `role "deadlock" does not exist` during migration 2026092801; harness issue, not caused by this change.
