# Mini-Spiele tab in the Twitch dashboard

Goal: logged-in streamers open "Mini-Spiele" in the dashboard sidebar and play Deadlockdle and Flashcards in German and English, following the dashboard language switch, using the public Deadlock API directly from the browser. A Rust watcher reports new upstream versions of the copied frontend to the owner as a Discord DM (`deliver_notice` in `rust/bin/tb-bot/src/minigames_upstream.rs`).

Upstream: github.com/deadlock-api/deadlock-api, website/, MIT, pinned at fadfbdd66beed55ec9787da5fc896db4b1cd5b4a.
Out of scope: Guess the Rank (needs upstream server functions, its database and clips), viewer chat games, own game backend.

Acceptance: `/twitch/mini-spiele` is live on deutsche-deadlock-community.de behind the existing partner gate, every game is playable in both languages on real API data with no CSP errors, and the watcher posts exactly one notice per new upstream head that touches the copied paths.

Packages: P1 tab, routing, CSP, hub, guess hero/item/ability. P2 trivia, item stats, higher or lower, guess sound. P3 flashcards. P4 upstream watcher.
Integrator and deploy: main session, thread 74718eb2-e674-4143-8e13-4bb07bedbe96.
