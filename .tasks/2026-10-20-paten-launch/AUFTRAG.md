# Paten launch (2026-10-20)

Role: leaf worker. Do exactly this package, start no further orchestrators or T3 threads.
Client and main orchestrator: Claude main session in Documents (session 5e337bdd-f708-49ba-8665-ae991eff8977).
Worktree: /home/nathanael/.worktrees/deadlock-bots-paten-launch
Branch: feat/paten-launch-20261020 (from origin/main 8c071bec)
Handover point: branch pushed, critic + `gate_hook.py --review` (skill `rolle-merge-schleuse`) ALLOW, then merge to main, deploy dl-bot via the documented wrapper (memory `dl-bot-build-und-deploy`), restart, live check. Report merge SHA and live evidence.

Rules: CLAUDE.md applies. Rust only, no code comments, no `.unwrap()` in production paths, new migrations only (never edit applied ones), refresh `rust/.sqlx` offline data, checks via `cargo-slot`. Read each file once, bundle edits, run checks once at the end. User-facing texts in natural German with real umlauts, short, factual, no pressure and no "cringe" phrasing. No community points.

## Goal

The existing concierge Paten system (`rust/crates/dl-community/src/concierge.rs`, merged via PR #471) is relaxed and given a real end of life. Newcomers ask for a Pate via a button, a Pate takes the request whenever someone has time, the Patenschaft runs in a private channel and ends on its own.

## Live Discord IDs

- Guild 1289721245281292288
- Category „🌱 ─ NEUE SPIELER ─“ 1465839366634209361 (private pate channels go here)
- 📌so-läufts-hier 1562083323507900517 (bot posts and maintains the panel with the request button here; channel is read-only for members)
- 💬frag-die-community 1426220702054355077
- 🎥streamer-partner-werden 1562083341036032020
- 🤝paten-zentrale 1524083665838276860 (request cards for Paten)
- 🤝teil-vom-team-werden 1544026617733710006 (team application panel)
- Pate role 1524047896297738311, Frischling role 1522384961481609236, new role „Streamt Deadlock“ 1562083299428401272
- Channel and role IDs belong into the operating TOML config like the existing concierge IDs, not hard-coded.

## Scope

1. Requests
   - Remove the 2h owner ping/escalation (`PATE_ESCALATION_2H_*`) and the 24h close + 24h DM stage entirely. Requests stay `open` in paten-zentrale until a Pate takes them or the newcomer withdraws.
   - On request the newcomer immediately gets (ephemeral): „Ein Pate meldet sich, sobald jemand Zeit hat. Bis dahin kannst du gern in 💬frag-die-community fragen.“ (link the channel mention).
   - The newcomer can withdraw an open request (button on that reply or a repeated button press offering withdraw). The card in paten-zentrale is then updated to withdrawn without a take-over button.
   - New entry point: a persistent panel message in 📌so-läufts-hier with the button „Ich brauche einen Deadlock-Paten“, using the existing request path. Panel text comes from a TOML asset; use the placeholder text in `assets/` marked as draft, the orchestrator delivers the approved text before visibility. The existing proactive DM offer stays as is but must not be enabled by this package.
2. Patenschaft lifecycle (new)
   - On take-over the private channel is created in category 1465839366634209361 (newcomer + Pate + bot).
   - Fixed duration 30 days from creation, then ends automatically. No end button, no check-in question.
   - Inactivity: if no human message in the channel for 14 days, the bot posts once: „Hier war länger nichts los. Wenn ihr den Kanal noch braucht, schreibt einfach weiter.“ If 7 more days pass without a human message, the Patenschaft ends early. A new message resets the inactivity clock.
   - End: short factual bot message in the channel, channel becomes read-only (archived), deleted one week later. Set `released_at` so the Pate gets the slot back (claim limit 3 stays).
   - Scheduler must be idempotent and restart-safe (persist stage timestamps in Postgres, new migration).
3. Team application: add a „Pate“ area to the team application panel in 🤝teil-vom-team-werden (`team_applications.rs`, `assets/team_application_texts.toml`; a pate kind migration `2026090919_team_applications_pate_kind.sql` already exists, check what is missing). On acceptance the Pate role 1524047896297738311 is assigned automatically.
4. Config: find where the concierge runtime config lives (`config/bot.toml` template, `[runtime.community]`, operating TOML `~/.config/deadlock-bots/bot.toml`). Add the new IDs there. Enabling: the Paten button flow and lifecycle must work with concierge enabled but `concierge_proactive` unchanged. Do not change the operating TOML for proactive DMs.

## Out of scope

Onboarding prompt changes, channel visibility, streamer partner text and the community announcement (orchestrator does these directly). No posts to visible channels; the panel may only be posted into the hidden so-läufts-hier channel.

## Acceptance

- Builds, fmt, clippy and existing tests green; targeted tests for the lifecycle stages (30 days, 14-day notice, 7-day early end, deletion after 7 days, slot release) and for request withdraw.
- Live: after deploy the panel exists in so-läufts-hier (hidden channel), a test request by a throwaway account appears in paten-zentrale without any owner ping, withdraw works. Never use real member accounts for live tests.
