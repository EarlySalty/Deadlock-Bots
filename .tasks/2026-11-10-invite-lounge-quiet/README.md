# Invite lounge: silent when someone is on it (2026-11-10)

## Cause
- Mechanism: a room request waits 30 min for the check-in and 60 min for the automatic invite. When an admin invited by hand during that wait, the lounge still acted. It asked the check-in question, then at 60 min sent its own invite, Steam answered "already invited", and the lounge posted `Notice::AlreadyInvited`. Live proof on 2026-11-10: request with friend code 1193494661 (steam 76561199153760389), `steam.bot_event_log` `playtest_invite` rows `queued` 19:52 UTC and `sent` 20:02 UTC from the manual invite, lounge reply "Du hast Deadlock schon" at 20:17 UTC. The same shape happened in the morning case at 08:21/08:55 UTC.
- Why possible: the lounge looked only at the Steam audit row for the check-in and at channel replies to skip the check-in. Neither result stopped the 60 min dispatch, so a request somebody already handled still ran the full automatic path.
- Why unnoticed: the tests covered old audits and a human reply at check-in time, but not an invite during the wait or the 60 min dispatch after a human reply. The lounge has no metric for "replied after a human already handled it".
- Twins: every lounge decision that reads "somebody answered" goes through `answers_request`. `request_answered` is used by `someone_on_it` and `check_in`. `bot_replied_since` and the history backfill already skip bot authors. `answers_request` now skips every bot author, not only this bot, and both callers keep working when the history read fails.

## Rule (user decision 2026-11-10)
- Someone is on it (a human replied to the request or mentioned the user, or a `playtest_invite` event or audit row exists since the request): the lounge closes the request silently, drops undelivered notices for it, and sends no check-in, invite or notice.
- Nobody is on it: the lounge acts as before.
- Direct requests to the bot and a "not arrived" answer still act immediately. Replies from any bot never count as someone being on it.
- If the Discord history or the invite lookup cannot be read, the request continues as unattended instead of stalling until expiry.

Companion change: Deadlock-Brain `fix/invite-notice-no-ping` sends lounge notices without a ping.

## Check after deploy
- Next room request handled by hand: no lounge reply; `bot.kv_store` `invite_lounge:requests/user:<id>` shows `last_result = "Schon von Hand erledigt; keine Meldung"`.
