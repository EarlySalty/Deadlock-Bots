# Package 10 (P8): onboarding interest prompt with roles by ID

Port of the onboarding work saved before the sweep of 25.11.2026:
`sicherung/sweep-20261125/bots-p8-wip` @ `3be6f774` (tracked diff on
`feat/onboarding-role-lookup`, base `b47f06c0`) and
`sicherung/sweep-20261125/bots-p8-untracked` @ `2299000e` (its TODO). Work branch
`nexusnzt/10-discord-onboarding` from `origin/main`.

## Goal

The native onboarding built by `serversync` asks four questions: the welcome
switch, a new interest question "Worauf hast du Lust?" (Scrims, Lustige Custom
Games, Turniere), the reduced ping question (Patchnotes, Spielersuche, Streams)
and the carried-over rank question. The interest roles are resolved by their
Discord IDs, not by name search, so a renamed or duplicated role name cannot
point the option at the wrong role.

## Boundaries

- No guild apply, no preview against the live guild, no Discord calls. The
  apply stays a manual owner step.
- Variant A (Scrims option present) as saved in the WIP. Variant B is an open
  owner decision.
- The ping and welcome questions keep their exact name resolution.
- The draft doc `docs/onboarding-redesign/phase1-rechte-soll-modell.ENTWURF.md`
  is not edited (see open items).

## Acceptance criterion

`cargo test -p dl-bot -p dl-server-as-code` passes with tests that pin: four
prompts in the order above, the three interest options carrying exactly
`SCRIM_SIGNUP_ROLE_ID`, `ROLE_CUSTOM_GAMES_ID` and `ROLE_TURNIER_TEILNEHMER_ID`,
a blocker naming the missing role ID when one is absent (also when a role with
the same name but another ID exists), the apply revalidation catching a stale
interest role ID, and the Sammelpunkt exception keeping the `Custom Games`
overwrite only for that exact role name.

## Changes over the WIP

- `serversync` tests updated to the four-prompt layout (the WIP left them
  red), plus the missing-role and revalidation cases for the interest prompt.
- `dl-server-as-code/src/rules.rs`: the Sammelpunkt custom-ping exception also
  keeps the overwrite of the role named exactly `Custom Games` (normalized
  exact match). Without it, a server-as-code derive drops that role's
  Sammelpunkt overwrite from the desired model, and an apply would remove the
  access.

## Open items (owner decisions, not changed here)

- Scrims option: variant A (in the interest question) or variant B (without).
- `Funny Custom Ping` (1407085699374649364) is no longer offered in onboarding;
  it stays as a journey label and keeps its Sammelpunkt overwrite.
- `ROLE_STREAMER_PARTNER_ID` = 1411798947936342097 is the live role
  "Spieler Suche Ping"; its journey label `streamer_partner` is wrong. Not
  renamed, because the label is written to analytics.
- The saved proposal JSON has five in-onboarding prompts, Discord accepts four
  (`TOO_MANY_ONBOARDING_PROMPTS`). The builder publishes four and drops
  "Willst du Starthilfe?". A manual PUT of the proposal would fail.
- Draft doc lines 29 and 211 still name Funny/Grind Custom Ping only. Whether
  the live role "Custom Games" is the renamed Grind role needs the live role
  list, which is not reachable from here.
- `WELLE2B_PING_ROLE_TEMPLATE_CANDIDATES` still lists `Custom Games`; it only
  picks a template role for creating missing ping roles. Checked, unchanged.

## State

26.11.2026: code ready, not yet built or tested. The toolchain (caddy, node, Rust,
Docker) is missing on the machine of this session; installing it waits for the
owner's OK. No pull request before the checks of section 12 and the review pass.
