# Bot HTTP deadline contract

The shared `dl-core::bot_http` client is used by dl-bot, dl-bridges, dl-voice,
dl-ai and dl-community. Its raw reqwest client is private. Construction reads
`runtime.bridges.steam_connect_timeout_ms` and `steam_request_timeout_ms` from
the immutable process configuration. The names remain compatible with the
previous round, but the catalog labels describe their scope across bot clients.

Both values must be present, positive, within `STEAM_HTTP_TIMEOUT_MS`, and
connect must not exceed total. The upper envelope is 120 seconds, matching the
longest existing Steam forwarding budget. Millisecond precision permits shorter
local-service budgets; the lower bound is a representable positive duration,
not a promise that every network can connect that quickly. The deployment policy
is a 5000 ms connection budget and a 120000 ms total budget. This preserves the
existing 120-second invite and rank-check allowance. Operators may intentionally
reduce the total. Existing shorter operation budgets remain effective.

Invalid TOML types, overflow and invalid ranges reject configuration loading.
Missing values disable client construction. Legacy constructors that keep running
retain a disabled client whose requests return an explicit configuration error
before network access. Startup paths that require a usable client reject
initialization. Builder failures cannot create an unbounded transport.
There is no raw-client fallback. Steam reports disabled requests in its logs;
the user message no longer promises that a configuration fault will recover in
a few seconds. The catalog rejects null resets and derives its numeric bounds
from the runtime constant. Its help describes both startup rejection and missing
configuration accurately.

Both request building for usage metadata and actual sending clamp the requested
deadline to the configured total. A longer override cannot extend it. The reqwest
deadline remains attached through response body consumption. Operating-config
requests reuse the existing client, including its no-proxy and no-redirect policy,
and no longer create a second client or install a literal request deadline.

Clients retain their process snapshot until restart. A newly built client cannot
alter the deadline of an existing client. The dashboard labels the settings as
requiring restart; saved configuration does not claim to update running clients.

## Fixed construction sites

| Consumer | Sites routed through the shared client |
| --- | --- |
| dl-bot | serversync production and test constructors; tournament client; both vanity workers and their helper signatures; evidence downloads; Discord coaching POST and PATCH; dashboard Twitch link; MCP production and test clients; self-invite tests; Twitch invite sync; all scrim relay clients and test builders |
| dl-bridges | Steam event constructor; Steam operating-config reuses that client; Twitch API constructor and its failed-builder path |
| dl-voice | Discord message GET; router panel multipart POST and PATCH |
| dl-ai | three provider constructors and their failed-builder paths; optional provider builder; retry client; Anthropic tool-chat HTTP client; usage test client |
| dl-community | coaching client and its failed-builder path |

`bot_crates_cannot_construct_raw_http_clients` scans all five consumers, including
tests, and rejects direct client types, builders and grouped client imports.

## Deployment handover

The repository config and `/home/nathanael/.config/deadlock-bots/bot.toml` have
connect 5000 ms and total 120000 ms. Only these non-secret keys were inspected
and changed. No service was restarted and no binary was deployed. This is a
configuration preparation statement, not proof that a running older binary
understands the new fields. Integration must deploy the matching binaries before
restarting readers that use `deny_unknown_fields`. The previous round had already
introduced these keys into the deployment file.

A read-only handover check found the user-level `dl-community-points-sync.service`
in `failed` state with exit status 1. Its last 50 journal entries included three
configuration-load error messages, without an explicit unknown-timeout-field
message. The cause was not established by that check. No restart or deployment
was performed; integration must check the service after deploying matching readers.


## Regression evidence

The core tests cover missing limits, zero, overflow, inverted budgets, shorter
builder budgets, longer and shorter request overrides, immutable client budgets,
catalog bounds and null-reset rejection. Steam tests call the production
`post_event` path against stalled headers and bodies, rather than calling raw
reqwest methods. A positive lookup test catches misspelled configuration keys.
The prior timing tests that asserted longer overrides survive were removed.

Unit test artifacts are built using cargo-slot, then executed from the repository
root so the normal default config path resolves to the committed fixture.
Verification results and the round-2 finding dispositions are recorded in
`http-client-timeout-round3-review.md`.
