# Support-Panel, 2026-11-08

State document of the support panel as built by branch `feat/supporter-p1-slim-20261116`. The Ko-fi and Stripe link buttons of the earlier branch `feat/support-panel-20261108` are gone. `.tasks/2026-11-11-supporter-panel/dry-run.md` is one rendered dry-run of the cards; the test `dry_run_doku_entspricht_dem_render` fails when it differs from the render.

## Messages

Three Components V2 messages in `#server-support`, built by `/serversync/support-apply`. Dry-run is the default. History handling (adopt, edit, repost, delete) follows `plan_support_history` in `rust/bin/dl-bot/src/serversync/support_publish.rs`; a stored payload format other than `SUPPORT_PAYLOAD_FORMAT` reposts all three cards.

1. `support`: banner, intro, perks (only while `supporter_role_grant_apply` is on), Steam Bot plans.
2. `support:vorteile`: planned features and what stays free.
3. `support:zahlung`: monthly goal bar, payment paths and the buttons.

## Texts

`assets/support_texts.toml` holds empty `[texts]` and `[buttons]` tables, so every card text and button label comes from the compile defaults in `support_publish.rs`. A field set in the file replaces its default. Fields of the old panel (`title`, `boost`, `donation`, `benefits`, `why`, `ai`, `brain`, `discord`, the `donate` and `stripe` buttons, `[urls]`, `[stripe]`) still parse, are not shown and produce a warning. A missing file falls back to the defaults with a warning; any other read or parse error stops the apply.

Placeholders are filled per block:

- `funding`: `{balken}`, `{stand}`, `{ziel}`
- `pay`: `{stufen}`
- `twitch`: `{kanaele}`

For every shown block the check warns about a placeholder of that block that the text leaves out and about every other `{name}` made of letters, digits and underscores, once per name, which would appear literally in the panel.

## Banner

The banner is read once when the cards are built; the payload hash and the upload use those bytes. A missing banner is a warning and the card goes out without it. An empty file, a directory, a symlink without target or a read error stops the apply. The Regelwerk, FAQ, Rang-Guide and Voice-UX panels use the same reader (`serversync::read_optional_banner`), and the Welcome banners and dividers use the same empty-file rule (`serversync::read_banner_file`).

## Buttons

All buttons carry a custom_id, none is a link button:

- `support:become` opens an ephemeral tier picker (`support:tier:<eur>`); each tier asks the Steam Bot for a personal Stripe checkout link.
- `support:tip` asks the Steam Bot for a one-off payment link.
- `support:manage` asks the Steam Bot for the Stripe customer portal.
- `support:twitch` is shown only when `supporter_twitch_api_base` and the Twitch token are set.

## Funding

The monthly amount comes from the Steam Bot route `supporter_wire::FUNDING_PATH`. The support apply, dry-run or confirm, takes its own non-waiting claim first; a second support writer gets "Support-Panel wird gerade geschrieben". The apply fetches the funding before it takes the shared panel apply lock. When the fetch fails, a stored published amount is used; without one a confirm is rejected with "Monatsstand unbekannt". A one-off tip is a `core.supporter_sources` row with `source = stripe` and a `supporter_wire::TIP_REF_PREFIX` reference; migration `20261116130000_supporter_sources_tip_paid_at.sql` adds its payment time `paid_at`. The Steam Bot side of that contract is an open row in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.

dl-bot calls the Steam Bot supporter routes with the first non-empty token of `supporter_wire::SUPPORTER_TOKEN_NAMES`.

## Roles

Grant and revoke live in `rust/bin/dl-bot/src/supporter/roles.rs`, driven from `rust/bin/dl-bot/src/supporter.rs` and gated by `supporter_role_grant_apply` and `supporter_role_sync_apply`. The sync interval `supporter_sync_interval_seconds` is validated against `SUPPORTER_SYNC_INTERVAL_SECONDS_RANGE` when the configuration loads; a value outside the range stops the start. The sync pauses without writing while `core.supporter_sources` or its trigger `supporter_sources_privacy_gate` is missing or the trigger does not fire for normal sessions, until the table and trigger exist and fire again. A tick that fails after some role changes keeps those changes and logs the error.

## Known limits

- A single serversync Discord request is bounded by `DISCORD_REQUEST_TIMEOUT_MS` in `discord_http_client()`. A call through `discord_regelwerk_send_with_retry`, retries and rate-limit waits included, is bounded by `discord_pages::discord_call_timeout()` (`DISCORD_CALL_TIMEOUT_MS`); the waits alone stop at `DISCORD_RATE_LIMIT_WAIT_BUDGET_MS`.
- The live guild model that the Regelwerk publish and the Welcome apply load while they hold `panel_apply_lock` is bounded by `discord_call_timeout()` times `LIVE_GUILD_MODEL_DISCORD_CALLS`.
- The Regelwerk, Welcome and panel history scans share `scan_message_history`: at most `HISTORY_SCAN_MAX_PAGES` pages of `DISCORD_HISTORY_PAGE_LIMIT` messages, and after a full last page one probe request for `HISTORY_SCAN_PROBE_LIMIT` older message. A scan counts as cut short only when that probe finds an older message, so a history of exactly `HISTORY_SCAN_MAX_PAGES * DISCORD_HISTORY_PAGE_LIMIT` messages is complete.
- The role sync reads members through `discord_pages::fetch_all_guild_members` in `rust/bin/dl-bot/src/discord_pages.rs` with `discord_call_timeout()` per page.
- The role sync waits at most `TABLE_LOCK_TIMEOUT_MS` in `rust/bin/dl-bot/src/supporter/store.rs` for the table lock on `core.supporter_sources`.
