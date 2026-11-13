# Support-Panel, 2026-11-08

Historical task of branch `feat/support-panel-20261108`. The panel has since been rebuilt by Supporter P1 (`feat/supporter-p1-20261111`); the current state is in `.tasks/2026-11-11-supporter-panel/` and `Documents/.tasks/2026-11-11-server-support-kanal/TODO.md`. This file only records what still holds from the first branch.

The panel is three Components V2 messages in `#server-support` (`1483136301271355532`), built by `/serversync/support-apply`. Dry-run is the default. Confirm follows the history plan. When three ids are stored, a non-empty scan that shares none of them blocks. When three ids are stored and the scan still contains every one of them, an unchanged payload is a no-op and own messages outside that set are deleted on confirm. When three ids are stored and the scan is empty or missing some of them, confirm deletes own messages outside the stored ids, edits every stored card and posts a new card only when that edit returns 404. The scan covers at most 500 messages. Support applies and the funding refresh take their own non-blocking claim: a second writer gets "Support-Panel wird gerade geschrieben" at once and never waits.

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

Grant and revoke are implemented in `rust/bin/dl-bot/src/supporter.rs` and gated by `supporter_role_grant_apply` and `supporter_role_sync_apply`. The panel, the perks block and the tier and tip replies mention the role only when `supporter_role_grant_apply` is on. The reply names the sync cadence from `supporter_sync_interval_seconds`, not a delivery time. The one-off payment text names no minimum and no validity; the Steam Bot payment page shows the minimum.

## Known limits

- Every serversync Discord call is bounded by `runtime.bridges.http_connect_timeout_ms` and `http_request_timeout_ms`; a 429 waits at most the request limit, a longer `retry_after` returns the 429 at once.
- No panel apply waits on a lock. The support panel takes its own claim; the other panels (Regelwerk, FAQ, Welcome, Rang-Guide, Router) share one claim, so they still never write at the same time. A second apply while a claim is taken gets "bitte gleich noch einmal versuchen" at once instead of queueing, and the support panel and its funding refresh never wait for the other panels.
