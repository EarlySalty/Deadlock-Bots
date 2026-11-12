# Steam timeout configuration correction

1. Mechanism: the saved gate log `/tmp/steam-timeout-gate.log` rejects the literal connect timeout introduced by 80b643f8.
2. Why possible: the constructor had no timeout configuration inputs and the first fix added constants instead of following the supporter contract.
3. Why unnoticed: request timing regressions exercised stalls and overrides, but did not cover missing or invalid settings.
4. Twins: both Steam constructors and the operating-config builder now enforce configured limits. Remaining unrelated clients are recorded in `docs/internal/http-client-timeout-audit.md`.

Both new config fields: absent, empty, malformed, zero, overflow and values above 60000 milliseconds disable the client or fail TOML validation. Boundary values 1 and 60000 are accepted. The runtime projection and both constructors use the same range. Request overrides remain unchanged.

The deployment input `/home/nathanael/.config/deadlock-bots/bot.toml` contains 5000 and 15000 milliseconds. No deploy or restart was performed.

Verification: cargo fmt passed for dl-core and dl-bridges. The final cargo-slot unit run passed 61 dl-bridges tests and 44 dl-core tests, with 2 ignored core tests. Strict clippy reported only 5 existing matcher double_must_use diagnostics and 3 existing Steam test unwrap_used diagnostics; none are on changed lines. Full log: `/tmp/steam-timeout-fix-tests-final.log`; clippy log: `/tmp/steam-timeout-fix-clippy.log`.

Self-review: no new literal client timeout, code comments or production unwrap calls. Existing default, rankcheck, invite and operating-config request overrides remain unchanged.
