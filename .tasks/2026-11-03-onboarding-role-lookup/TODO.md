# P8 onboarding role lookup: state and next steps

Worktree: /home/nathanael/.worktrees/bots-p8/Deadlock-Bots, branch feat/onboarding-role-lookup (base origin/main b47f06c0). Nothing committed yet.

## Done (uncommitted, not built yet)
- dl-community/src/onboarding.rs: ROLE_LFG_PING_ID -> ROLE_CUSTOM_GAMES_ID (1407086020331311144), ROLE_CUSTOM_GAMES_PING_ID -> ROLE_FUNNY_CUSTOM_PING_ID (1407085699374649364), new ROLE_TURNIER_TEILNEHMER_ID (1566909672676663447).
- dl-bot/src/journeyglue.rs: journey labels "lfg_ping" -> "custom_games", "custom_games_ping" -> "funny_custom_ping" (only a write-only analytics tag in activity.journey_user_state.weiche_choice; no reader matches on the value).
- dl-bot/src/serversync.rs: new interest_prompt "Worauf hast du Lust?" (Scrims = SCRIM_SIGNUP_ROLE_ID, Lustige Custom Games = ROLE_CUSTOM_GAMES_ID, Turniere = ROLE_TURNIER_TEILNEHMER_ID, emojis from dl_voice::router::ROUTER_EMOJI_*), resolved by ID via require_role_id_by_id; inserted as prompt index 1; ping_prompt reduced to Patchnotes, Spielersuche, Streams; outdated "drei Prompts" comment removed.

## Next
1. Update serversync tests (~line 9640 onwards): onboarding_model must contain roles with the three real IDs (1520849762851618817, 1407086020331311144, 1566909672676663447); drop 5005/5006 entries; prompt count 3 -> 4, ping index 1 -> 2, rank index 2 -> 3; ping_options[4] Streams -> [2]; onboarding_ping_rollen_matching test uses prompts[2]; put-payload test rank prompt index 3; apply-revalidation test index. Add assertion for interest prompt role IDs.
2. rules.rs:2308 Sammelpunkt exception: also cover the exact role name "Custom Games" (normalized exact match, not substring). Check rules.rs:98/103 (Events & Turniere aliases) and :4499 test fixture "Grind Custom Ping".
3. docs/onboarding-redesign/phase1-rechte-soll-modell.ENTWURF.md:29 and :211 mention Funny/Grind Custom Ping: update or report.
4. From rust/: cargo-slot fmt --all -- --check; cargo-slot clippy -p dl-bot -p dl-community -p dl-server-as-code --all-targets --jobs 4 -- -D warnings; cargo-slot test -p dl-bot -p dl-community -p dl-server-as-code --jobs 4.
5. Commit (English message), load rolle-merge-schleuse, gate-review --repo <worktree> --base origin/main, fix BLOCKs, push branch only (separate plain git push call; the gate hook blocks commands containing the word push inside heredocs/python).

## Open items for the report
- ROLE_STREAMER_PARTNER_ID = 1411798947936342097 is actually "Spieler Suche Ping" (journey label "streamer_partner" is wrong). Not changed, report.
- Proposal JSON has 5 in_onboarding prompts; the code comment said Discord caps at 4 (TOO_MANY_ONBOARDING_PROMPTS, verified 2026-07-15). The code builder publishes 4 (drops "Willst du Starthilfe?"). Report as risk for the manual PUT.
- Scrims option is Variante A; Variante B (no Scrims) is the user's decision.
