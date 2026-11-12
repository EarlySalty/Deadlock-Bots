# HTTP deadline contract

`dl_core::bot_http::Client::builder` requires an explicit `Deadlines` value.
It performs no process config lookup. Components without an override pass the
catalog defaults. Steam alone reads the optional Steam keys through its injected
lookup. Gateway HTTP providers derive deadlines from their own
`dl-ai-gateway.toml` provider `request_timeout_ms`; they never load `bot.toml`.
The same library constructors work in dl-knowledge, dl-verbinder, dl-web and
dl-brain-feeder without assuming a config file name or working directory.

`settings_catalog::http_defaults` owns the default connection budget (5000 ms),
unspecified request budget (15000 ms), provider default (60000 ms), ingest budget
(30000 ms), invite and rankcheck budget (120 s), sync budget (180 s), operating
config budget (10 s), and maximum operation budget (3600000 ms). Steam settings
remain bounded to 1 through 120000 ms, with connect no greater than total after
applying defaults. Missing Steam settings select defaults. Empty, zero,
non-numeric, overflow and inverted values are rejected. Catalog null resets
remove the optional value and restore the default, so missing keys do not block
unrelated dashboard edits.

Explicit component and request budgets replace the unspecified request budget.
They must be positive and at most the catalog maximum operation budget. They
are never shortened by Steam settings. Invite retains its compile-time contract
above the external Steam worst case of 90 s. Rankcheck retains 120 s; rank,
subrank and friends sync retain 180 s. Operating-config GET and PATCH retain
10 s. Connection setup remains bounded. The request deadline also covers body
consumption. Request building and sending share the same finishing function.

Client creation failures retain their cause in subsequent request errors.
Cloning a failed request preserves its initialization error rather than reporting
missing metadata. AI error classification distinguishes initialization/config
failure from transport failure. Scrim initialization has no production expect.
Steam panel and admin failures share the same user message; dashboard linking
no longer promises that a permanent failure will recover in a minute.

The construction guard covers dl-bot, dl-bridges, dl-voice, dl-ai, dl-community
and dl-brain, including image fetching and API ingest. It rejects raw client
constructors, grouped client imports, convenience GETs, glob imports and reqwest
aliases. The ignored dl-knowledge live golden test also constructs a bounded
client. Other binaries with already bounded raw clients remain outside this
guard's scope.

## Deployment order

Deploy binaries before adding keys. `BridgeOptions` has `deny_unknown_fields`,
so older binaries reject the optional Steam fields. Missing keys work through
catalog defaults; no live config change is needed at all. Keep the live file
unchanged for deployment and rollback compatibility. The integrator removed the
premature live keys from earlier rounds. This fixer changes only repository
examples and documentation, performs no deployment and makes no claims about
current service health.

Earlier branch commits describe superseded bounds and override semantics.
The current contract is 1 through 120000 ms for optional Steam settings,
independent named operation budgets up to the catalog maximum, and defaults for
missing values. History is preserved because this task forbids force pushes.

## Verification

Use cargo-slot for workspace tests and clippy. Cargo runs tests from each package
directory. Tests construct explicit deadlines and require no cwd workaround or
live config. See `http-client-timeout-round4-review.md` for findings and results.
