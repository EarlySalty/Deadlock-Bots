# Steam bridge HTTP timeout audit

Audited on 2026-11-12 against origin/main at 92d9654c, covering Rust sources in
`rust/crates/dl-bridges` and `rust/bin/dl-bot`.

## Mechanism and fix

`SteamBotClient::new` used `reqwest::Client::new()`. The resolved reqwest
0.12.28 source initializes both `connect_timeout` and `timeout` to `None` in
`async_impl/client.rs`. Its builder documentation states that the total deadline
covers connection establishment through response body completion and defaults
to no timeout. A request without an override therefore had no client deadline.

The Steam builder now sets named constants for a 5-second connection timeout
and the existing 15-second default total timeout. Initialization failure stops
construction with a specific error, preserving the previous constructor's
fail-fast behavior. There is no fallback to an unbounded client.

The reported impact needs qualification: `post_event` already sets a request
timeout, including the 120-second rank-check and invite paths. Those requests
were already bounded, and their overrides remain effective. The new default
protects requests made through the shared client without an explicit deadline.
Regression tests exercise a stalled body and prove that a longer request
deadline overrides the client default.

Commit b43c538b on the supporter branch supplied the explicit builder timeout
pattern. Its supporter client also rejects missing timeout configuration and
does not fall back to `Client::new()`. This fix uses named constants because the
Steam constructor has no timeout configuration parameter.

## Other clients without builder deadlines

These remain listed for follow-up, as permitted by the implementation brief.
Locations below refer to the audited baseline.

| Location | Existing protection or exposure |
| --- | --- |
| `dl-bridges/src/steam_operating.rs:139` | Builder has no deadline, but its request sets a 10-second timeout. |
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

* `cargo fmt -p dl-bridges -- --check` passed after formatting the crate.
* `cargo-slot test -p dl-bridges --lib` passed all 60 tests, including both
  deadline regressions.
* `cargo-slot clippy -p dl-bridges --all-targets -- -D warnings` failed on
  existing dependency warnings in `dl-broker` and `dl-changelog`.
* The narrower `cargo-slot clippy -p dl-bridges --all-targets --no-deps -- -D warnings`
  also failed: five existing `double_must_use` diagnostics in `matcher.rs`
  and three existing test `unwrap_used` diagnostics in `steam.rs`. The final
  run reported no diagnostics in the new code.
* `cargo fmt --all -- --check` found existing formatting differences outside
  this patch. The touched crate's formatting check passes.
