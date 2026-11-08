# Support-Panel, 2026-11-08

Branch `feat/support-panel-20261108`. Not applied, not deployed, no Discord post.

The panel is three Components V2 messages in `#server-support` (`1483136301271355532`), built by `/serversync/support-apply`. Dry-run is the default. Confirm replaces the stored message when the stored count is not 3.

## Messages

1. `support`: banner inside the gold card, why the costs exist, KI subscriptions.
2. `support:vorteile`: Steam Bot (planned supporter model), Brain, Discord perks, planned features, what stays free.
3. `support:zahlung`: payment paths and buttons.

## Buttons

- Ko-fi: `https://ko-fi.com/deutschedeadlockcommunity`, always on.
- Stripe: only if `[urls].stripe` is `https://buy.stripe.com/<slug>` and `[stripe].enabled = true`. Default is off. No Discord server-subscription button, no Twitch checkout button. Those products do not have a link that belongs on this panel.

## Open before a live apply

- Stripe server-supporter product does not exist here. Steam Plus (`Deadlock-Steam-Bot`, checkout session with `discord_id`, Plus role) and Twitch billing (`Deadlock-Twitch-Bot` plan catalog) are different products. A bare payment link does not bind a Discord account. Grant and revoke of the supporter role on cancel are not implemented for this panel.
- Role choice is open: `Server Unterstützer` already sees the VIP category and Streamer VC. `Plus` is the Steam Plus role (lounge, votes, no extra permissions). The panel does not name either role.
- A paid Twitch-Bot subscription does not grant Discord supporter rights automatically.
- The Steam Bot is not paywalled yet. The panel says it will be. The Plus panel text still says linking, rank checks and match delivery stay free (`steam-flows/src/plus/texts.rs`). Update that before publishing this panel.
- Brain "ohne Nutzungslimit" is the offer. There is no supporter bypass of a per-user quota in this change.
- Build helper, patch watchlist and extended match queries are described as planned. Extended queries do not promise realtime.
- Ko-fi still grants the supporter role manually for 31 days. That is the existing panel policy, not a new webhook.
