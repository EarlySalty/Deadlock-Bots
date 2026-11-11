# TODO: ticket scam shadow (current state)

Worktree: /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107
Branch: feat/ticket-scam-shadow-20261107
Remote: origin/feat/ticket-scam-shadow-20261107 = a3a6abb6. Everything after that is local and still has to be pushed.
Last origin/main merge: 11f721bd (2026-11-11), merge commit fbd40262.

## What the branch does
- Collector keeps the opener's images (max 4, max 8 MiB each, same author only). `MessageAttachment.size` added in dl-discord.
- One shadow entry per ticket opening in log channel 1374364800817303632 (capped at 3900 UTF-16 units, Components V2 allows 4000): Brain state (`yes`, `no`, `uncertain`, `not_asked`) with candidate, `needs_mod` with reasons, and for scam reports the screenshot review state, the reported accounts with guild membership, the moderation policy action as its own line and `Bann-Entscheidung (Shadow, nur Log): would ban <id>` or `kein Bann (...)`.
- No ban, no ping, no member-visible message.
- Screenshot review runs only with `moderation.ticket_screenshot_review = true` in config/bot.toml (default `false`). While off the entry says "Bildprüfung: nicht gelaufen (deaktiviert)" and the decision is always "kein Bann". A failed review is logged as "Bildprüfung fehlgeschlagen (...)", never as a verdict.
- Reported accounts come from the ticket text only (user mention, Discord user ID, `@name`, labels such as `Username:` or `Discord-Name:`). Reading account names out of the screenshots is NOT built; open follow-up.
- "would ban" needs exactly one distinct member who is not the opener, not staff and not a bot, no unclear or non-member ref beside it, and a confirmed scam verdict above the policy auto threshold. More than five refs, unclear membership or several accounts give "kein Bann" with a German reason.
- New module rust/crates/dl-community/src/ticket_triage.rs; the reviewer is installed in dl-bot main.rs only when the switch is on and a moderator exists.

## Checks at the last fixer run
- dl-community lib 324 passed, 4 failed (the known ionos DB tests); dl-moderation 66 passed; dl-answer 35 passed; `cargo-slot check -p dl-bot --tests` exit 0 (also after the 2026-11-11 merge).
- clippy: only workspace baseline lints (`double_must_use` from async_trait, `result_large_err`, one `needless_borrow` at gateway.rs:722 that predates this branch).

## Next steps
1. `gate-review --repo /home/nathanael/.worktrees/bots-ticket-scam-shadow-20261107 --base origin/main`. Quota failures (reset times differ) give "kein Urteil", not a finding; rerun after the reset.
2. On ALLOW: push the branch, `git push origin HEAD:main`, deploy dl-bot via a new copy of /opt/deadlock/bots/.install-*.sh, prove the shadow entry live in channel 1374364800817303632, then remove branch and worktree.
3. Live replay with a real scam screenshot is still open: no closed ticket with an image has been found, channel 1567507300216737894 contains text only. The replay needs the switch on, which needs the user's approval first.

## Gate round 3 (HEAD 10da8823, 2026-11-11 afternoon)
- No verdict (Fable answers rejected as malformed JSON, Opus 5.5 hit the 5-hour limit). The user gave the go for a fifth run on 2026-11-11.
- Fixed in e086af40: skipped images are counted and shown in the shadow entry ("Nicht geprüft: n Bild(er)"), the fail-closed logs say `not_asked`, scam and ban stems cover gescammt, betrogen, bannen.
- Left as is: `handle_ticket_message` stays as the text-only entry point used by tests; no test for the `install_ticket_screenshot_review` wiring in dl-bot main.rs (wiring is a plain `if switch && moderator` install).
- The Ticket Tool bot guard claim was removed from these docs because no such guard exists in the code.

## Decisions for the user
- Ticket screenshots go to the moderation vision provider (same one message moderation uses). There is no explicit approval for tickets yet, so the switch stays off after deploy.
- Policy maps a confirmed scam to a timeout; the shadow entry logs it as "would ban" only.

## History
- 2ecf3c26 keeps its subject without its own Ursache block; the gate asked for a reword, but no amend or force is allowed. Its mechanism (adjacent-only dedup) is described in 38353890.
- Gate round 1 BLOCK findings (dedup twins, config switch, ID safety, account extraction, ban rule, member lookups, failed review state, log truthfulness, docs) are fixed in 38353890, 65885803, 4ef7c633, d59239d1 and 0c680fc2.
