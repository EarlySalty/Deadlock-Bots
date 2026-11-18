# Invite lounge: the Brain reads every message (2026-11-16)

Worktrees: Bots `~/.worktrees/bots-invite-lounge-ki-intent-20261116` (branch `feat/invite-lounge-ki-intent-20261116`), Brain `~/.worktrees/brain-invite-turn-20261116` (branch `feat/invite-turn-20261116`). Both from origin/main.

## Goal
In #deadlock-invite no word list decides whether a message is an invite request. Every human message goes to the Brain with the member's lounge state; the Brain picks intent and next step, the bot validates and executes, the Brain writes the text.

Acceptance: replay of member A (answer in another member's check-in) gets a sensible reply and, after the answer, an invite; the check-in flow of member B works; plain chat gets no reply; live proof after deploy.

## Ursache (four layers, proof 2026-11-16)
- Mechanism: `record_message` creates a request only when `request_kind()` (word lists) hits; a code without an invite word stores `friend_code` and no request. A live row of member A had a stored `friend_code` and `request: null`. Member A's reply in member B's check-in thread matched neither `request_kind` nor `check_in_open` (state per author) and was dropped. Member B's unthreaded "nein, noch nicht" was dropped too (`last_seen` still the request message); B's friend request came 18 minutes later from the 60-minute auto-dispatch (`is_due`, ROOM_WAIT_SECONDS), not from the answer.
- Why possible: intent was inferred from vocabulary; every miss is silent by construction (`Ok(None)`).
- Why nobody noticed: an ignored message leaves no log line and no row change.
- Same pattern elsewhere: `is_offer`, `close_word`, `should_reply*`, `has_friend_code`, the backfill `record_history`, `check_in_open` (reply/mention gate for answers).

## Scope decisions
- Only #deadlock-invite (1464736918951432222) goes to the Brain. #frag-die-community (1426220702054355077) and the rest of the community category no longer start lounge requests; open product point.
- Supersedes `.tasks/2026-11-15-invite-lounge-silence/` (unmerged, local merge f34dc77d never pushed; it must not land on main). Its user rules are carried here: no FA on silence (60-min auto-dispatch removed), ask first (30-min check-in stays), "nein" -> FA, "ja" -> short OK and close, deleted request -> silent (`fail_if_not_exists: true`), late answers count (every later message is judged).

## Rollout
Brain and bot ship together. The Brain branch removes `discord_invite_lounge`, `discord_invite_check_in`, reply `judged` and notice update `check_in_answer`, which the current bot still sends, and the new bot needs `discord_invite_turn`, reply `decided` and notice updates `confirmed`/`reply`. Order: merge both, deploy the Brain, deploy and restart dl-bot right after.

Outbox cutover: the old bot can leave a transient `check_in_answer` outbox entry (0 rows live on 2026-11-16, 246 lounge rows). The new bot reads it as a retired entry that round-trips unchanged (the CAS parses the stored row and compares the state semantically, not the JSON text) and drops it silently on delivery. Rollback: Brain and bot go back together, because the old bot sends task kinds the new Brain no longer accepts. The old binary's compare-and-swap compares the stored JSON text (`v = $4`), so it can write a row back only if its own parse and re-serialization reproduce the stored bytes; otherwise its message loop retries that member forever. The new bot therefore writes every field the old bot reads and writes, in the old order and encoding: `hint_sent`, `last_hint_at` and `last_dispatch_finished_at_millis` are always serialized and carried through unchanged, `status.check_in_message_id` is kept. Tests parse rows written by the new code with a copy of the merge-base structs and compare the bytes, and the reverse. One exception stays: the old binary cannot parse an outbox entry of kind `confirmed` or `reply`; for a row holding one, every old load fails, so that member's messages, dispatch and outbox stop with a warning on each pass while other members continue. Before switching the bot back, list such rows and wait until the new bot has delivered them, or remove the entries:

```sql
SELECT k FROM bot.kv_store
WHERE ns = 'invite_lounge:requests'
  AND jsonb_path_exists(v::jsonb, '$.outbox[*].update ? (@.kind == "confirmed" || @.kind == "reply")');
```

Deploy blocker to recheck: live dl-bot ran 59a0ac5f from the unmerged supporter branch.
