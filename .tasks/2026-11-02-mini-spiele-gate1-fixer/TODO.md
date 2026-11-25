# Mini games P3: merge gate round 1 fixer, state

Stopped by the usage limit hook before commit. This is a separate file because the brief forbids editing the task TODO in `.tasks/2026-11-01-mini-spiele/`.

## State

Worktree `/home/nathanael/.worktrees/tb-mini-spiele-p3`, branch `feat/mini-spiele-p3`. The fix is in the working tree, NOT committed and NOT pushed.

Changed files, both under `bot/dashboard_v2/src/minigames/components/features/flashcards/`:
- `use-flashcard-deck.ts`: `liveSerial` and `answeredSerial` refs guard `choose`; delayed advances run the latest `moveOn` through a ref and compute the exclusion when they fire.
- `FlashcardGame.tsx`: number keys moved into `FlashcardOptions` and disabled while exiting; `InertWhileExiting` makes the exiting card inert.

## Evidence so far

- `/tmp/p3-tools/gate1-before.log`: 33 failures on the baseline build, findings 1 and 3 reproduced on all four decks, plus number keys answering the hidden card and three clicks in one tick recording 3 answers.
- `/tmp/p3-tools/gate1-after.log`: 73 ok, 0 failures on the rebuilt preview in Moli.
- `tsc -b` and `eslint src/minigames tests/miniSpiele.test.ts`: clean. `tests/miniSpiele.test.ts`: 20 of 20 pass.
- `/tmp/p3-tools/run-regress.sh` (existing heroes, decks, effects flows): no FAIL at 1440 and 390. Servers and Moli stopped, ports free.

## Next steps

1. Look at a few screenshots in `/tmp/p3-tools/shots-gate1/` (for example `p3-heroes-wrong-de-1440.png`, `p3-heroes-wrong-de-390.png`) to confirm the `contents` wrapper left the layout unchanged. Not looked at yet.
2. Read the full diff once more against classes A and B of the brief.
3. Stage only the two files, add a new commit (no amend), English message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
4. In a plain Bash call on its own: `git push origin feat/mini-spiele-p3`.
5. Report SHA, change, before and after evidence, anything unverified, in under 150 words.
