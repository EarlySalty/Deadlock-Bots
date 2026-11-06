# TODO: Brain ticket capability (paused 2026-11-06, Brain gate cap reached)

## State
- Brain: branch fix/brain-ticket-capability-20261106 pushed, head 78f67554 (worktree /home/nathanael/.worktrees/brain-ticket-capability-20261106). Contains origin/main.
  - Gate rounds 1 to 4 BLOCK, each finding fixed: uncited acknowledgements, digit-free bypass, static ticket_process evidence without text guard, injected evidence counted as tool evidence, banter/image/capability routes on tickets.
  - Round 5 fix 78f67554: on ticket queries every static-only citation set (capability, attached_image, ticket_process in any combination) must pass ticket_process_accepted. fmt and clippy clean, brain_contracts 128 and brain_kernel 107 tests pass.
  - gate-review now answers "BLOCK: Gate-Deckel" (BLOCK_ROUNDS_MAX = 4) without reviewing. Needs a user decision per docs/merge-gate-zweiphasig.md. The branch counter expires 72 h after its last change.
- Bots: branch fix/brain-ticket-capability-20261106 head 44b43385, gate ALLOW (nits only). Pins Brain rev 7af8a54e; must be bumped to the merged Brain main SHA and re-gated.

## Next
1. After the user's gate decision: gate-review Brain until ALLOW, then `git -C /home/nathanael/.worktrees/brain-ticket-capability-20261106 push origin HEAD:main`.
2. Bump Bots brain-client/brain-contracts rev to the merged Brain SHA, commit, push, gate-review Bots, merge.
3. Deploy Brain via release wrapper, add "discord_ticket" to the first bot_task_consumers entry in /home/nathanael/.config/deadlock-brain/brain-serve.json, restart brain-serve.
4. Deploy Bots /opt/deadlock/bots/current release, restart deadlock-bot-rust.service.
5. Live proof: purpose bot_task:ticket in Brain logs and shadow posts in log channel 1374364800817303632.
6. Clean up branches and worktrees.
