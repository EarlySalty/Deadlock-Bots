# P1 proof

All browser runs used Moli (`moli serve --layout --image --font`) against the real `api.deadlock-api.com`, driven by the scripts in `proof/`. `npm run dev:preview` served the app.

- `proof/run.mts`, output in `proof/result.txt`: 78 `ok` lines, 2 `FAIL` lines, 0 page or console errors. Each of the three modes ran twice: start in one language, wrong guess, reload, language switch mid-puzzle, reload, right guess, reload, switch again. It also covers a five-attempt loss in guess item plus reload, and the same daily answer ID in both languages.
- The 2 `FAIL` lines are the silhouette filter check. Moli drops `filter: url(...)` from CSSOM: `probe.style.filter = 'url(#x)'` reads back empty while `blur(2px)` reads back intact (`proof/dbg.mts`). The silhouette and warp effects of guess hero are therefore not visually verified. The filter elements exist in the DOM with the upstream values. Check them once in a real browser after deploy.
- `proof/nav.mts`, output in `proof/nav-result.txt`: tile, back link and previous day navigation, archive badge, unknown `spiel`, future `date`. It also ran the preview build served with the `@dashboard_paths` CSP from `hosts/v50671/Caddyfile:263` (`proof/csp-server.mjs`): API fetch and asset image loaded, no console errors.
- Moli reports a 0x0 box for some absolutely positioned or animated elements, so `proof/run.mts` waits for `attached`, submits by Enter or a dispatched click, and switches language by a dispatched click. Screenshots show the dropdown and result panel rendered.
- Screenshots at 1440 and 390, German and English, were inspected by eye: hub, result of every mode, hero mid-puzzle. They are not committed.

# Fix round proof

Browser run with Moli against the real `api.deadlock-api.com`, driven by a one-off Playwright script over CDP that stays out of the branch to keep the delta under the gate size limit. Output in `proof/fix-result.txt`. It used the preview build, because the Moli page hung on `page.goto` once `page.addInitScript` was registered. The clipboard is a stub installed after load, so the copied text is the exact value handed to `navigator.clipboard.writeText`; a real clipboard was not exercised.

- Guess item: wrong guess in German, language switch to English, reload, right guess, reload. All checks `ok`, 0 page or console errors.
- Share text of the mode: `Deadlockdle #225: Item 2/5` plus grid. Hub share text: `Deadlockdle #225`, blank line, one row per game. Neither holds a URL. The hub text used seeded win states in localStorage.
- Real asset lists, both languages, all three pools: 0 duplicate names and 0 empty names on 2026-11-01 (`proof/fix-result.txt`). The id comparison therefore guards a case the current data does not contain; `tests/miniSpiele.test.ts` covers it with constructed names.
- Watcher tests: `cargo-slot test -p tb-bot` refuses the whole workspace while ionos is down (`spawns_test_postgres` in `cargo-slot`, exit 78). The test binary came from `cargo-slot build -p tb-bot --tests` and ran directly with no database variable set: `tb_bot-* minigames_upstream`, 12 passed, 1 ignored. `head_is_marked_only_after_a_successful_dm_and_only_once` printed `SKIP: TB_TEST_DATABASE_URL not set`, so its database path did not run.
- Live check: `tb_bot-* minigames_upstream::tests::live_check_against_github --exact --ignored --nocapture` with the real manifest and pin, no DM possible on that path: head `44451cedba275db0ea4251311f98a936436cd0da`, `NoWatchedChange`, 0 changed files.

