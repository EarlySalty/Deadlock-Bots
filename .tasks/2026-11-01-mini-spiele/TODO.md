# Mini-Spiele: state at quota stop, 2026-11-02

Order on resume: A, then B, then C. The P3 worker overwrote the earlier TODO; section A restores what it held.

## A. Function proof after deploy, P2 SHA 67c72d556 (integrator, ticket 27)

Done:
1. `deploy-twitch-release --pruefen` exit 0, NRestarts 0, dashboard `journalctl -p err` since 12:49:00 empty.
4. Clip `https://assets-bucket.deadlock-api.com/assets-api-res/sounds/abilities/abrams/a2_charge/abrams_a2_charge_wall_impact.mp3`: HTTP 200, `audio/mpeg`, CORS `*`.

2. Deployed release bundle under the live CSP in Moli: `proof/p2-live1440.txt` and `proof/p2-live390.txt` each 183 checks ok, 0 failed, 0 page or console problems, 0 aborted requests. Hub, trivia, higher or lower, item stats and guess sound in DE and EN. Sound clip loaded as blob, play accepted, no media error.
3. German dashboard default offers "Extended Magazine", switch to Deutsch offers "Erweitertes Magazin", guess and choice survive reload. Same evidence.
6. Worktree and local branch `feat/mini-spiele-p2` removed; proof shots looked at, four kept in `proof/p2-live-shots/`.

Limits of this proof: the 390 run hides invisible values through injected CSS because of the Moli layout stall below 640 px. Moli has no audio. The page behind real login was not driven.

Found by looking: in item stats at 1440 the check mark overlaps the end of "Vitalität" (`proof/p2-live-shots/p2-item-stats-de-result-1440.png`). Cosmetic, folded into the P3 brief.

Open:
5. Only the user can prove audible sound and guess item on a real phone. Ask once.

## B. P3 Flashcards: finish the build

Next action: start a fresh worker (Opus 5.5, effort medium) in the P3 worktree with section "P3 Flashcards" below as its brief, because the first worker stopped on quota. Do not message the old agent `a4dae5fcdf0a69c99` if it has been idle over an hour. Worktree state at quota stop: uncommitted, based on origin/main 67c72d556, which is still current main.

Then, as integrator: judge acceptance, `gate-review --repo /home/nathanael/.worktrees/tb-mini-spiele-p3 --base origin/main`, plain `git push origin HEAD:main` from the worktree, `deploy-queue submit --sha <full> --thread 74718eb2-e674-4143-8e13-4bb07bedbe96 --task /home/nathanael/repos/Deadlock-Bots/.tasks/2026-11-01-mini-spiele`, then end the turn. No CSP change expected. Hub proof then expects 8 tiles. Clean up after the proof.

User change requests, 2026-11-02 15:00, folded into P3:
1. Remove the footer "Spiele und Spieldaten von deadlock-api.com (MIT-Lizenz)". `src/minigames/LICENSE` stays.
2. Share buttons copy a link `https://dead-lock.de/twitch/mini-spiele/ergebnis?...` that opens a public result overview. The route is public in Rust, no backend, data in the URL, no answers in it. CSP already matches `/twitch/mini-spiele/*`.
3. Fix label overlap in the hint panel ("ANFANGSBUCHSTABE" runs into its value).
4. "Schläger" is our own translation; the API only has `brawler`. Hero types stay English (Assassin, Brawler, Marksman, Mystic). Other game terms in hints follow the game terms setting.
Live proof must cover: link opened logged out on dead-lock.de returns 200 and shows the overview; `/twitch/mini-spiele` still redirects to login.

## C. After P3 is live

1. Brain knowledge page in `Deadlock-Docs` or `Deadlock-2nd-Brain`. Test a real question by DM to the concierge.
2. Check Huelle (`xxhuelle`, display "Huelle", Discord linked) is an active partner, then send through `dl-bot` (DMs reach Concierge and Brain, `rust/crates/dl-community/src/concierge.rs:5009`) the approved text:
   "Hey Huelle, dein Wunsch ist umgesetzt: Im Dashboard gibt es jetzt den Bereich „Mini-Spiele“ mit Deadlockdle und mehr. Du findest ihn links in der Seitenleiste oder direkt hier: https://deutsche-deadlock-community.de/twitch/mini-spiele\nTeste gerne direkt und sag Bescheid, wenn dir etwas auffällt."
3. Still unproven: watcher sending a DM on a real upstream change; page behind real login and silhouette filter (user was asked to look, no answer yet). The user sets the inbox entry to done.
4. Offer as a separate question: Rust defaults use non-resolving `assets.deadlock-api.com` (`tb-engagement/src/deadlock_wiki.rs:199`, `tb-dashboard-api/src/handlers/overlay.rs:30`, `tb-config/src/dashboard_options.rs:168`); 3 moderate Dependabot alerts; standalone `tb-stt-server` not updated by the deploy.

# P3 Flashcards: handed over

State 2026-11-02: finished and pushed, not merged, not deployed. Next step is the integrator: `gate-review`, merge, deploy, live proof.

Branch `feat/mini-spiele-p3`, commit `e1110ffd5c446cbded4346043fe2c83d0de3bdcf`, based on origin/main 67c72d556. Worktree /home/nathanael/.worktrees/tb-mini-spiele-p3.

Checks: `tsc -b`, `vite build`, `eslint .` (0 errors, 5 existing warnings), `npm test` 530 of 535 with the same five failing tests as the base commit, clippy for `tb-dashboard-api` clean, `twitch_paths` 7 of 7, `ops/tests/dead_lock_website_caddy.test.mjs` 2 of 2.

Proof in `proof/`: `p3-log-1440.txt`, `p3-log-390.txt`, `p3-edge-values.txt`, `p3-run.mts`, `p3-*.png`. The single FAIL in `p3-log-1440.txt` is a case-sensitive regex in the runner, fixed and passed on rerun in the same log.

Open for the integrator:
1. Moli stalls below 640 px on the hint value `Charge Delay: -1.0s`. The base commit stalls too. The 390 runs inject `dl dd{display:block}`, so the grid `dd` was not rendered at 390 for those values. Check once in a real browser.
2. Result page has no language switch and dead-lock.de defaults to English without a stored language. German friends see English unless `?lang=de`. Product decision.
3. Not following game terms: trivia hero stat labels, NPC names in sentences, `{value} Seelen`, `Stufe {tier}`, higher or lower stat words.
4. Live proof still due: link opened logged out on dead-lock.de returns 200 and shows the overview; `/twitch/mini-spiele` still redirects to login. Caddy paths and CSP already cover the route, no Caddy change.
5. Clipboard is stubbed in the share test.

# Resume state 2026-11-02 (usage limit, gate round 1 fix uncommitted)

- Fix for gate round 1 is UNCOMMITTED in worktree `/home/nathanael/.worktrees/tb-mini-spiele-p3`: `FlashcardGame.tsx`, `use-flashcard-deck.ts`. Fixer state and evidence: `/home/nathanael/repos/Deadlock-Bots/.tasks/2026-11-02-mini-spiele-gate1-fixer/TODO.md` (before 33 failures, after 73 ok, 0 failures; tsc, eslint, 20/20 tests).
- Integrator read the diff against classes A and B: no objection. Screenshots in `/tmp/p3-tools/shots-gate1/` still not viewed (`contents` wrapper layout check).
- Commit 36ba68c96 is local only; origin/feat/mini-spiele-p3 is at e1110ffd5.
- origin/main moved to af16b8fc6 (was 67c72d556): merge origin/main into the branch before the gate, no rewrite of the published commit.

Next:
1. View 2 screenshots, stage the two files by literal path, new commit, English message with Co-Authored-By line.
2. Publish the branch to origin in a plain standalone git call from the worktree.
3. `gate-review --repo /home/nathanael/.worktrees/tb-mini-spiele-p3 --base origin/main` in the background (round 2). User has been told about the round 1 BLOCK.
4. On ALLOW: retro ledger line, `ledger-themes.py`, merge to main, `deploy-queue submit` (thread 74718eb2-e674-4143-8e13-4bb07bedbe96), end turn. On BLOCK: gate-learning, fresh fixer.
