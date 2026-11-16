# Support-Panel, 2026-11-08

This file is the current state document of the support panel as Supporter P1 (`feat/supporter-p1-slim-20261116`) builds it. It started as the task of branch `feat/support-panel-20261108`; the Ko-fi and Stripe link buttons of that branch are gone. `.tasks/2026-11-11-supporter-panel/dry-run.md` holds one rendered dry-run of the cards.

The panel is three Components V2 messages in `#server-support` (`1483136301271355532`), built by `/serversync/support-apply`. Dry-run is the default. Confirm follows the history plan. When three ids are stored, a non-empty scan that shares none of them blocks. When three ids are stored and the scan still contains every one of them, an unchanged payload is a no-op and own messages outside that set are deleted on confirm. When three ids are stored and the scan is empty or missing some of them, confirm deletes own messages outside the stored ids, edits every stored card and posts a new card only when that edit returns 404. The scan covers at most 500 messages. Support applies, support previews and the funding refresh take their own non-blocking claim before anything else, including the Steam funding fetch: a second support writer gets "Support-Panel wird gerade geschrieben" at once and never waits for the Steam Bot. The apply fetches the funding before it takes the shared panel apply lock, so the other panel applies never wait for the Steam Bot either. When the funding fetch fails and a published amount is stored, apply and refresh both use that amount with the currently configured goal; the refresh publishes that fallback when the goal or the panel options changed and fails only when nothing changed. When no published amount is stored or it cannot be read, a confirm is rejected with "Monatsstand unbekannt", the preview shows 0 € with that warning, and the refresh skips with "kein veröffentlichter Monatsstand" when none is stored and fails with "veröffentlichter Monatsstand unlesbar" when it cannot be read. The refresh never publishes a first amount, so the first confirm needs a working Steam Bot funding endpoint.

A stored payload format other than `3` (`SUPPORT_PAYLOAD_FORMAT`) reposts three cards and deletes the stored ids. A changed payload edits the stored cards. A scan of exactly three messages adopts those ids whenever the stored count is not three. Every other combination follows `plan_support_history` in `serversync/support_publish.rs`.

## Messages

1. `support`: banner, intro, what supporters get now (only while `supporter_role_grant_apply` is on), Steam Bot plans.
2. `support:vorteile`: planned features and what stays free.
3. `support:zahlung`: monthly goal bar, payment paths and the buttons.

## Buttons

All buttons are interactive (custom_id), none is a link button:

- `support:become` opens an ephemeral tier picker (`support:tier:<eur>`), each tier asks the Steam Bot for a personal Stripe checkout link.
- `support:tip` asks the Steam Bot for a one-off payment link.
- `support:manage` asks the Steam Bot for the Stripe customer portal.
- `support:twitch` is shown only when `supporter_twitch_api_base` and the Twitch token are set.

## Roles

Grant and revoke are implemented in `rust/bin/dl-bot/src/supporter/roles.rs`, driven from `rust/bin/dl-bot/src/supporter.rs`, and gated by `supporter_role_grant_apply` and `supporter_role_sync_apply`. The panel, the perks block and the tier and tip replies mention the role only when `supporter_role_grant_apply` is on. The reply names the sync cadence from `supporter_sync_interval_seconds`, not a delivery time. The one-off payment text names no minimum and no validity; the Steam Bot payment page shows the minimum.

## Known limits

- The shared serversync write path stays as on main. The Regelwerk, FAQ, Welcome, Rang-Guide and Router applies wait on the shared panel apply lock without a wait limit, and the serversync Discord client has no request timeout, so a stalled Discord call holds those applies until it returns or the bot restarts.
- The support apply takes its own non-waiting claim first, then waits on the shared panel apply lock like the other panels. The funding refresh takes only the support claim, so it never waits for the other panels.
- Supporter calls to the Steam Bot and the Twitch dashboard use their own client, bound by `supporter_request_timeout_ms`, without redirects and without proxy. In the role sync every member page and every role change has the same limit; a member list whose cursor does not advance ends the tick without changes.
