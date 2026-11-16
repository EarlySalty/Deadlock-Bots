# Support-Panel, 2026-11-08

This file is the current state document of the support panel as Supporter P1 (`feat/supporter-p1-slim-20261116`) builds it. It started as the task of branch `feat/support-panel-20261108`; the Ko-fi and Stripe link buttons of that branch are gone. `.tasks/2026-11-11-supporter-panel/dry-run.md` holds one rendered dry-run of the cards.

The panel is three Components V2 messages in `#server-support` (`1483136301271355532`), built by `/serversync/support-apply`. Dry-run is the default. Confirm follows the history plan. When three ids are stored, a non-empty scan that shares none of them blocks. When three ids are stored and the scan still contains every one of them, an unchanged payload is a no-op and own messages outside that set are deleted on confirm. When three ids are stored and the scan is empty or missing some of them, confirm deletes own messages outside the stored ids, edits every stored card and posts a new card only when that edit returns 404. Message ids, payload format, payload hash, published amount and panel options are written in one transaction after all cards went out; when a card or that write fails, the cards posted in that run are deleted again and no id or state is stored, while cards already edited keep their new content until the next apply. The scan covers at most 500 messages. Support applies and support previews take their own non-blocking claim before anything else, including the Steam funding fetch; the funding refresh first checks that the supporter configuration is complete and then takes the same claim. A second support writer gets "Support-Panel wird gerade geschrieben" at once and never waits for the Steam Bot. The apply fetches the funding before it takes the shared panel apply lock, so the other panel applies never wait for the Steam Bot either. When the funding fetch fails and a published amount is stored, apply and refresh both use that amount with the currently configured goal; the refresh publishes that fallback when the goal or the panel options changed and fails only when nothing changed. When the funding fetch fails and no published amount is stored or it cannot be read, a confirm is rejected with "Monatsstand unbekannt" and the preview shows 0 € with that warning. Before any fetch, the refresh skips with "kein veröffentlichter Monatsstand" when no amount is stored and fails with "veröffentlichter Monatsstand unlesbar" when it cannot be read. The refresh therefore never publishes a first amount, and the first confirm needs a working Steam Bot funding endpoint.

The banner is read once when the cards are built; the payload hash and the upload use those same bytes. Only a banner path that does not exist is a warning, and the card goes out without it; a directory, a symlink without target, a failed check or a file that cannot be read stops the apply with an error. The Regelwerk, FAQ, Rang-Guide and Voice-UX panels work the same way, and the Welcome sections and dividers use the same presence rule (`serversync::optional_file_present`). The text file `assets/support_texts.toml` follows the same rule through `serversync::read_optional_text`: only a missing path falls back to the compile defaults with a warning, every other check or read error stops the apply; the Regelwerk, FAQ, Rang-Guide and Welcome text files work the same way.

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

## Funding

The monthly amount on the payment card comes from the Steam Bot route `/supporter/v1/funding`. A subscription counts while its row in `core.supporter_sources` is active. A one-off tip (row `source = stripe`, `source_ref = tip:<payment intent>`) counts in the Europe/Berlin calendar month of its payment time `paid_at` and grants the supporter role for 31 days from that time (`TIP_VALID_DAYS` in the Steam Bot), so its row carries `valid_until = paid_at + 31 days`. A tip paid on the last day of a month therefore counts for that month only, while the role stays for 31 days. `paid_at` comes from migration `20261116130000_supporter_sources_tip_paid_at.sql`, which backfills older tip rows from `updated_at`. The Steam Bot funding query reads `paid_at`, not `updated_at`, which stays the time the writer last confirmed the row.

dl-bot calls the Steam Bot supporter routes with the first non-empty token of `supporter_wire::SUPPORTER_TOKEN_NAMES` (`TWITCH_INTERNAL_API_TOKEN`, `MASTER_BROKER_TOKEN`, `MAIN_BOT_INTERNAL_TOKEN`); the Steam Bot reads the same constant.

## Roles

Grant and revoke are implemented in `rust/bin/dl-bot/src/supporter/roles.rs`, driven from `rust/bin/dl-bot/src/supporter.rs`, and gated by `supporter_role_grant_apply` and `supporter_role_sync_apply`. The perks block and the tier and tip replies mention the role only when `supporter_role_grant_apply` is on; the payment card never mentions it. The reply names the sync cadence from `supporter_sync_interval_seconds`, not a delivery time. That value is checked when the configuration loads (`SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE`, 60 to 86400 seconds); a value outside the range stops the start, it is not clamped.

The sync pauses without writing when `core.supporter_sources` or its privacy trigger `supporter_sources_privacy_gate` is missing ("Abgleich pausiert bis zur Migration") and when the trigger exists but does not fire for normal sessions, that is when it is disabled or set to replica only ("Abgleich pausiert bis ALTER TABLE core.supporter_sources ENABLE TRIGGER supporter_sources_privacy_gate"). A tick logs a new error once as a warning; the same error on later ticks is logged at debug level, and the first successful tick logs "Abgleich läuft wieder". The one-off payment text names no minimum and no validity; the Steam Bot payment page shows the minimum.

## Known limits

- The shared serversync write path stays as on main. The Regelwerk, FAQ, Welcome, Rang-Guide and Router applies wait on the shared panel apply lock without a wait limit, and the serversync Discord client has no request timeout, so a stalled Discord call holds those applies until it returns or the bot restarts.
- The support apply takes its own non-waiting claim first, then waits on the shared panel apply lock like the other panels. The funding refresh takes only the support claim, so it never waits for the other panels.
- Supporter calls to the Steam Bot and the Twitch dashboard use their own client, bound by `supporter_request_timeout_ms`, without redirects and without proxy. In the role sync every member page and every role change has the same limit; a member list whose cursor does not advance ends the tick without changes. Every REST guild member list (dl-bot role sync, serversync, modglue, vanity sweep and the dl-discord adapter itself) reads through `dl_discord::adapter::fetch_all_guild_members`, so all of them share the page size, a per-page timeout and the stalled-cursor stop. The broker member list and the streamer matcher read the gateway cache and send no member request.
