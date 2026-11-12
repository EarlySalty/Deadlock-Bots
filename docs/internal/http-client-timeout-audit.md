# Steam bridge HTTP timeout audit

Audited on 2026-11-12 against origin/main at 92d9654c, covering Rust sources in
`rust/crates/dl-bridges` and `rust/bin/dl-bot`.

## Mechanism and fix

`SteamBotClient::new` used `reqwest::Client::new()`. The resolved reqwest
0.12.28 source initializes both `connect_timeout` and `timeout` to `None` in
`async_impl/client.rs`. Its builder documentation states that the total deadline
covers connection establishment through response body completion and defaults
to no timeout. A request without an override therefore had no client deadline.

The Steam builders read `runtime.bridges.steam_connect_timeout_ms` and
`runtime.bridges.steam_request_timeout_ms` through the existing bot config loader.
Both accept 1 through 60000 milliseconds. Missing or invalid values leave the
shared client unset, and event and operating-config calls fail closed. Builder
errors also leave the client unset. There is no unbounded fallback.

The deployment config is `/home/nathanael/.config/deadlock-bots/bot.toml`, as
specified by `service/systemd/deadlock-bot-rust.service.d/20-release.conf`.
Its values were set to 5000 and 15000 milliseconds without a deploy or restart.
The repository example includes the same values.

The reported impact needs qualification: `post_event` already sets a request
timeout, including the 120-second rank-check and invite paths. Those requests
were already bounded, and their overrides remain effective. The new default
protects requests made through the shared client without an explicit deadline.
Regression tests exercise a stalled body and prove that a longer request
deadline overrides the client default.

Commit b43c538b on the supporter branch supplied the explicit builder timeout
pattern. Its supporter client also rejects missing timeout configuration and
does not fall back to `Client::new()`. The Steam implementation follows that
configuration contract and preserves request overrides.

## Other clients without builder deadlines

These remain listed for follow-up, as permitted by the implementation brief.
Locations below refer to the audited baseline.

| Location | Existing protection or exposure |
| --- | --- |
| `dl-bridges/src/steam_operating.rs:139` | Now uses both configured Steam deadlines and keeps its request override. |
| `dl-bot/src/modglue.rs:1899` | Evidence download uses separate outer timeouts for headers and body reads. No client deadline. |
| `dl-bot/src/modglue.rs:4567` | Discord multipart message POST has no client or request timeout. |
| `dl-bot/src/modglue.rs:4621` | Discord multipart message PATCH has no client or request timeout. |
| `dl-bot/src/turnierglue.rs:128` | Shared tournament HTTP client has no client or request timeout. |
| `dl-bot/src/vanity.rs:46` | Vanity worker client has no client or request timeout. |
| `dl-bot/src/vanity.rs:83` | Member directory worker client has no client or request timeout. |
| `dl-bot/src/serversync.rs:937` | Production server-sync client has no client or request timeout. |
| `dl-bot/src/serversync.rs:957` | Test constructor also uses an unbounded client. |
| `dl-bot/src/mcp.rs:1442` | Test-only HTTP client has no deadline. |
| `dl-bot/src/mcp/self_invite.rs:637` | Test-only HTTP client has no deadline. |

Two builders have deadlines on successful construction but lose them on failure
because `.unwrap_or_default()` creates an unbounded client:

* `dl-bridges/src/twitch.rs:127`, configured total timeout.
* `dl-bot/src/serversync/twitch_link.rs:42`, named request timeout.

The other builders found in `dl-bot` are bounded: MCP uses 60 seconds,
`twitch_invite_sync` uses 15 seconds, and `scrim_adapter` uses `RELAY_TIMEOUT`
in production and explicit timeouts in tests. These builders do not set a
separate connection timeout; their total deadlines also bound connection time.
The supporter implementation from b43c538b is not present in the audited main
baseline.

## Verification

* `cargo fmt -p dl-core -p dl-bridges -- --check` passed.
* `cargo-slot test -p dl-bridges -p dl-core --lib` passed the Steam crate
  suite, including stalled headers, stalled bodies, longer request overrides
  and fail-closed configuration cases. Core checks cover TOML projection and
  range validation.
* `cargo-slot clippy -p dl-bridges -p dl-core --all-targets --no-deps -- -D warnings`
  failed on five existing `double_must_use` diagnostics in `matcher.rs` and
  three existing test `unwrap_used` diagnostics in `steam.rs`. None intersect
  the changed lines; these unrelated warnings remain untouched.
* Self-review against origin/main found no added literal client deadline,
  code comment or production `.unwrap()`. Existing per-request overrides
  remain in place as requested.
