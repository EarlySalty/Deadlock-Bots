# Invite lounge: the Brain reads every message (2026-11-16)

Worktrees: Bots `~/.worktrees/bots-invite-lounge-ki-intent-20261116` (branch `feat/invite-lounge-ki-intent-20261116`), Brain `~/.worktrees/brain-invite-turn-20261116` (branch `feat/invite-turn-20261116`). Both from origin/main.

## Goal
In #deadlock-invite no word list decides whether a message is an invite request. Every human message goes to the Brain with the member's lounge state; the Brain picks intent and next step, the bot validates and executes, the Brain writes the text.

Acceptance: Lachs replay gets a sensible reply and, after his answer, an invite; greenlight flow works; plain chat gets no reply; live proof after deploy.

## Ursache (four layers, proof 2026-11-16)
- Mechanism: `record_message` creates a request only when `request_kind()` (word lists) hits; a code without an invite word stores `friend_code` and no request. Live row `user:798192827573076019`: `friend_code` 381230343, `request: null`. Lachs's 19:11 reply in greenlight's check-in thread matched neither `request_kind` nor `check_in_open` (state per author) and was dropped. greenlight's unthreaded "nein, noch nicht" (19:02) was dropped too (`last_seen` still his request message); his FA at 19:20 came from the 60-minute auto-dispatch (`is_due`, ROOM_WAIT_SECONDS), not from his answer.
- Why possible: intent was inferred from vocabulary; every miss is silent by construction (`Ok(None)`).
- Why nobody noticed: an ignored message leaves no log line and no row change.
- Same pattern elsewhere: `is_offer`, `close_word`, `should_reply*`, `has_friend_code`, the backfill `record_history`, `check_in_open` (reply/mention gate for answers).

## Scope decisions
- Only #deadlock-invite (1464736918951432222) goes to the Brain. #frag-die-community (1426220702054355077) and the rest of the community category no longer start lounge requests; open product point.
- Supersedes `.tasks/2026-11-15-invite-lounge-silence/` (unmerged, local merge f34dc77d never pushed; it must not land on main). Its user rules are carried here: no FA on silence (60-min auto-dispatch removed), ask first (30-min check-in stays), "nein" -> FA, "ja" -> short OK and close, deleted request -> silent (`fail_if_not_exists: true`), late answers count (every later message is judged).

## Rollout
Brain and bot ship together. The Brain branch removes `discord_invite_lounge`, `discord_invite_check_in`, reply `judged` and notice update `check_in_answer`, which the current bot still sends, and the new bot needs `discord_invite_turn`, reply `decided` and notice updates `confirmed`/`reply`. Order: merge both, deploy the Brain, deploy and restart dl-bot right after.

Outbox cutover: the old bot can leave a transient `check_in_answer` outbox entry (0 rows live on 2026-11-16, 246 lounge rows). The new bot reads it as a retired entry that round-trips unchanged (the CAS parses the stored row and compares the state semantically, not the JSON text) and drops it silently on delivery. Rollback caveat: the old binary cannot read `confirmed`/`reply` outbox entries; drain the outbox before rolling back.

Deploy blocker to recheck: live dl-bot ran 59a0ac5f from the unmerged supporter branch.
