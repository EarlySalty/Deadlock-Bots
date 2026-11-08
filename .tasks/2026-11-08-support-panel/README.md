# Support-Panel, 2026-11-08

Branch `feat/support-panel-20261108`. Not applied, not deployed, no Discord post.

The panel is three Components V2 messages in `#server-support` (`1483136301271355532`), built by `/serversync/support-apply`. Dry-run is the default. Confirm follows the history plan. When three ids are stored, a non-empty scan that shares none of them blocks. Any other scan with three stored ids keeps those ids and deletes only the extra own messages, including a later confirm after a failed delete. An empty scan with three stored ids keeps those ids. A scan of exactly three messages adopts those ids whenever the stored count is not three. An empty scan with a stored count other than three replaces that stored set with the three cards, and so does a scan whose ids equal a stored count of one or two. An empty KV and one or two scanned messages deletes those messages and posts the three cards. An empty KV and an empty scan posts the three cards. Every other combination blocks.

## Messages

1. `support`: banner inside the gold card, why the costs exist, KI subscriptions.
2. `support:vorteile`: Steam Bot (planned supporter model), Brain, Discord perks, planned features, what stays free.
3. `support:zahlung`: payment paths and buttons.

## Buttons

- Ko-fi: `https://ko-fi.com/deutschedeadlockcommunity`, always on.
- Stripe: only if `[urls].stripe` is `https://buy.stripe.com/<slug>` and `[stripe].enabled = true`. Default is off. No Discord server-subscription button, no Twitch checkout button. Those products do not have a link that belongs on this panel.

## Open before a live apply

- Stripe server-supporter product does not exist here. Steam Plus (`Deadlock-Steam-Bot`, checkout session with `discord_id`, Plus role) and Twitch billing (`Deadlock-Twitch-Bot` plan catalog) are different products. A bare payment link does not bind a Discord account. Grant and revoke of the supporter role on cancel are not implemented for this panel. Turning `[stripe].enabled` on shows the button and a warning that this panel does not check that binding.
- Ko-fi names the Unterstützer-Rolle for 31 days. Which role a future Stripe subscription grants is still open: `Server Unterstützer` already sees the VIP category and Streamer VC. `Plus` is the Steam Plus role (lounge, votes, no extra permissions).
- Discord server subscriptions are described as a path through the server name. This repo has no SKU button and does not grant a role from them. Check the guild settings before a live apply.
- A paid Twitch-Bot subscription does not grant Discord supporter rights automatically.
- The Steam Bot is not paywalled yet. The panel says it will be. The Plus panel in Deadlock-Steam-Bot `steam-flows/src/plus/texts.rs` still says linking, rank checks and match delivery stay free. Update that before publishing this panel.
- Brain use without a usage limit is written as not switched on. The daily limit still applies to every user. There is no supporter bypass in this change.
- Build helper, patch watchlist and extended match queries are described as planned. Extended queries do not promise realtime.
- Ko-fi still grants the supporter role manually for 31 days. That is the existing panel policy, not a new webhook.
