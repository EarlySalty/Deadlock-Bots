# TODO: ticket scam shadow (worker state)

Worktree: /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107
Branch: feat/ticket-scam-shadow-20261107 (base origin/main 5700ed1a)
Remote: origin/feat/ticket-scam-shadow-20261107 = a3a6abb6 (checked with `git rev-parse`).
Local head: see "Fixer state 2026-11-11" below; those commits are local only.

## Done
- Collector keeps the opener's images (max 4, max 8 MiB each, same author only, Ticket Tool bot 557628352828014614 ignored). `MessageAttachment.size` added in dl-discord.
- One shadow entry per opening in log channel 1374364800817303632: Brain candidate, `needs_mod` with reasons, and for scam reports the dl-moderation analyzer+verifier verdict (`ModerationSystem::review_reported_screenshots`, trigger `ticket_scam_report`), the reported account (mention, ID, `@name`, `Username: name`) with guild membership via FaqGlue `member_by_id` / `member_by_name`, and `Aktion (Shadow): would ban <id>` / `kein Bann (...)`.
- New module rust/crates/dl-community/src/ticket_triage.rs; reviewer installed in dl-bot main.rs after the moderator is built.
- Checks: fmt clean on touched files; clippy --no-deps on dl-community, dl-moderation, dl-bot is clean except workspace-wide baseline lints from clippy 1.99 (double_must_use from async_trait, result_large_err), which also fail on untouched crates; dl-community lib 285 passed / 4 failed (the known ionos DB tests); dl-moderation 65/65; dl-discord 67/67.

## Gate round 1
BLOCK only because the reviewer quota ran out (no verified finding). Its diff-only NITs to address in a new commit:
1. Shadow entry is cut to 3900 UTF-16 units. Confirm the send path: FaqGlue::send_message uses brain_v2_body (Components V2, 4000 limit). If that holds, keep it; otherwise cut to 2000.
2. ticket_triage::shadow_action: `members.dedup()` only removes adjacent duplicates. Use sort_unstable + dedup and add the test case [Id(7), other, Name -> 7].
3. Data flow: ticket screenshots go to the moderation vision provider. Flag this for the integrator/user (message moderation already sends images there).
4. More log volume: every opening now gets an entry, including Brain NoAnswer cases. The spec requires this.
5. Verifier prompt with empty text + trigger `ticket_scam_report`: confirm the analyzer/verifier judge the screenshot content, not a report context.

## Next steps
1. Fix 1 and 2, then rerun `cargo-slot test -p dl-community --lib` and clippy (see above).
2. Commit, then rerun `gate-review --repo /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107 --base origin/main`.
3. `git push origin feat/ticket-scam-shadow-20261107`.
4. Write WORKER-ERGEBNIS.md next to AUFTRAG.md.
5. Open item for the integrator: live replay of a real scam screenshot. No closed ticket with a screenshot has been found yet; channel 1567507300216737894 contains text only.

## Integrator state 2026-11-07 (main session stopped on quota)
- Worker stopped on quota; worktree clean except this TODO, branch not pushed.
- On resume: fresh fixer (Opus, medium) for steps 1 to 4 above in this worktree. Then integrator review against AUFTRAG.md, gate until ALLOW, merge HEAD:main, deploy dl-bot via a new copy of /opt/deadlock/bots/.install-*.sh, live proof in log channel 1374364800817303632.
- 2026-11-07 resume: fresh fixer (Opus, medium) launched for steps 1 to 4. If it died on quota, check `git log` here; relaunch a fresh fixer with the same brief if not pushed.
- Ask the user at report time: ticket screenshots go to the moderation vision provider (not explicitly approved for tickets yet); policy maps a confirmed scam to timeout, logged as "would ban".

## Fixer state 2026-11-07 (stopped on quota before any change)
- Fixer (Opus, medium) read AUFTRAG.md and this TODO, then hit the account quota. No code changed, no commit, head still 0edeeb35, branch not pushed.
- On resume, in order: (1) check FaqGlue::send_message / brain_v2_body in dl-bot: Components V2 means keep the 3900 cap, else 2000; (2) ticket_triage::shadow_action: sort_unstable + dedup, add test [Id(7), other, Name -> 7] (one distinct non-opener member means would ban, two distinct means no ban); (3) item 5: confirm analyzer/verifier judge screenshot content with trigger ticket_scam_report, change only if wrong; (4) rustfmt touched files, cargo-slot test -p dl-community --lib, -p dl-moderation, -p dl-discord, clippy; (5) commit, gate-review, upload the branch to origin as its own plain Bash call, write WORKER-ERGEBNIS.md.

## Fixer state 2026-11-07 (done)
- Head 2ecf3c26 on feat/ticket-scam-shadow-20261107, origin/main 20c9bff4 merged in.
- NIT 1: send path is Components V2 (4000 limit), 3900 cap stays, no code change.
- NIT 2: shadow_action sort_unstable + dedup, test derselbe_gemeldete_account_ueber_id_und_name_zaehlt_einmal added.
- NIT 5: analyzer and verifier judge the screenshot content with trigger ticket_scam_report, no code change.
- Checks: fmt clean; dl-community 313 passed / 4 known DB failures; dl-moderation 65; dl-discord 67 + 3; check dl-bot exit 0; clippy exit 0 with only workspace baseline lints.
- WORKER-ERGEBNIS.md written. Next: gate-review, push, integrator review and deploy.

## Gate state 2026-11-07 17:00
- Branch feat/ticket-scam-shadow-20261107 pushed, remote head a3a6abb6 (docs commit on top of 1ba0d376), origin/main 20c9bff4 merged.
- gate-review run twice: "kein Modell der Kette hat geurteilt: claude-opus-5-5: ... weekly limit · resets 4am (Europe/Berlin)". No BLOCK finding; the configured chain has only Opus.
- Wakeup scheduled for 04:05 Europe/Berlin to rerun gate-review, merge HEAD:main on ALLOW, deploy dl-bot and prove the shadow log live. Shadow only, no bans, no pings.
- If the 04:05 gate still has no judgment: report and stop, no bypass. Only the user can push main in that case.

## Fixer state 2026-11-11 (gate round 1 findings)
- 527220e6 merges origin/main 987e587c (2026-11-11 00:32). On top, local only, not pushed: dedup twins, config switch for the screenshot review, account extraction and member lookup hardening, shadow log truthfulness, docs.
- Screenshot review is behind `moderation.ticket_screenshot_review` in config/bot.toml, default `false`. While off, the shadow entry says "Bildprüfung: nicht gelaufen (deaktiviert)" and the decision is always "kein Bann". A failed review is logged as "Bildprüfung fehlgeschlagen (...)", never as a verdict.
- Scope item 2 resolves reported accounts from the ticket text only (user mention, Discord user ID, `@name`, explicit labels such as `Username:` or `Discord-Name:`). Extracting account names from the screenshots is NOT built; open follow-up.
- "would ban" only for exactly one distinct non-staff, non-bot member who is not the opener, with no unclear or non-member refs alongside and a confirmed scam verdict above the auto threshold. More than five refs, unclear membership or several accounts give "kein Bann" with a German reason.
- Brain state per opening is logged as it happened: `yes`, `no`, `uncertain` (Brain unreachable) or `not_asked` (image-only opening), each with its own candidate placeholder.
- Commit 2ecf3c26 keeps its subject; the gate asked for a reword, but no amend or force is allowed, so the fixes are new commits.
- Next: gate-review on this head, then integrator review, merge, deploy. The user still has to approve ticket screenshots going to the moderation vision provider before the switch is turned on.
