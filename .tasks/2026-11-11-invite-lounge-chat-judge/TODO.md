# 2026-11-11 Invite lounge: chat judge (state 2026-11-11 evening)

User feedback 2026-11-11: any member can invite; whether a request is handled comes from the Brain reading the chat. Invited: stay quiet. Unclear: ask once. The Steam invite backoff change was dropped (cosmetic: the first retry already comes on the next 60 s poll; GC timeout heals via steam-core reconnect).

## Brain (Deadlock-Brain)
- Worktree /home/nathanael/.worktrees/brain-invite-handled-20261111, remote branch feat/invite-handled-judge-r3-20261111, head 850e0d27 on origin/main e2ba4828.
- Commits: 30f449c6 (feat, discord_invite_handled task), 936f35c0 (all line terminators indented, all persona user-text sites, fits_request_limit), 850e0d27 (labels reject line separators, client uses fits_request_limit).
- Gate rounds 1 to 4 BLOCK. Round 5 needs the user's go (four-round cap). Round 4 findings are fixed in 850e0d27.
- After ALLOW: `git push origin HEAD:main`, deploy via the gateway release script.

## Bots (this worktree, branch feat/invite-lounge-chat-judge-20261111)
- dl-brain rev = 850e0d27. InviteHandled task, Handled reply, chat trimmed to MAX_CHAT_LINES and then to the request byte cap via fits_request_limit.
- invite_lounge: handling() maps Invited to Done (quiet), NotInvited to Open, Unclear to Ask once then Wait. Replies to the request message count as addressed to the requester.
- dl-community invite_lounge tests: 85 pass. 4 other dl-community failures need CENTRAL_TEST_DSN (test DB), untouched files.
- Next: gate after Brain is on main, push to main, deploy dl-bot (memory dl-bot-build-und-deploy), check a live lounge request.

## Steam
- Drop branch fix/invite-no-backoff-20261111 and worktree /home/nathanael/.worktrees/steam-no-backoff-20261111.
