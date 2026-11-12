# Round 3 evidence correction

Round 3 was BLOCK. Its prior disposition table and green-test claim are
withdrawn: those binaries were executed from the repository root, which hid the
shared client's dependency on `config/bot.toml`. That was not a successful
standard workspace test run. The earlier live-config statements also described
an unsafe deployment order. The integrator removed those keys.

Round 4 removes the shared builder's global config lookup, restores operation
budgets, supplies missing-key defaults, removes the scrim startup panic and fixes
the remaining image, golden-test, operating-config and message twins.
See `http-client-timeout-round4-review.md` and `http-client-timeout-audit.md`.
