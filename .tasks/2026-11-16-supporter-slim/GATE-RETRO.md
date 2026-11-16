# Supporter P1 slim: round-four review

Four counted gate rounds without ALLOW (95, 54, 261, 394 blocking lines). This file records the triage of round 4 and the one fixer pass that follows it.

## REAL

- `panel_apply_lock` was held while `fetch_live_guild_model` awaited plain serenity Http without a time limit (Regelwerk publish, Welcome apply in `serversync.rs`).
- `read_optional_banner` returned an empty banner as present and attached it to the Support, Regelwerk, FAQ, Rang-Guide and Voice-UX cards; the Welcome banner and divider readers did the same.
- `link_outcome` in `supporter/links.rs` logged "Steam Bot" for errors on the Twitch connect path.
- `supporter/roles.rs` tests repeated the same member JSON fixture.
- `.tasks/2026-11-08-support-panel/README.md` restated bounds as literals and described placeholder rules that did not match the empty tables of `assets/support_texts.toml`.

## TRIVIA

The 59 NITs and the unit-table lines that follow from the classes above. They are dropped in this pass and not chased line by line. Twins outside the feature are restored to origin/main and listed in `claude-config/orchestrierung/OFFENE-URSACHEN.md`.

## Second opinion

Fable: narrow the diff, restore unrelated files, fix only supporter/panel defects, no reword.
