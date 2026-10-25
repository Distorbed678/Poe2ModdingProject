# Handoff: orchestrator 6 (2026-10-24)

For a new orchestrator. Read [CLAUDE.md](../../CLAUDE.md), your MEMORY.md, [docs/overview.md](../overview.md), [handoff-orchestrator-3.md](handoff-orchestrator-3.md) (repos, releases, how to work), [handoff-orchestrator-4.md](handoff-orchestrator-4.md) (terrain method, regression gate, user decisions), [handoff-orchestrator-5.md](handoff-orchestrator-5.md) (waves 4 and 5, working-method additions, the wave 6 plan), then this file. Everything in those files is still valid; this one only adds the state of wave 6.

## State
- Pushed and green: Poe2Server `b021b44` (wave 5 merged), Poe2ClientHook `811a3a7`, launcher `v0.4.1`, hub with handoff 5.
- Terrain: the port matches **196,094 of 218,705** client layouts (baseline `runtime/forced-baseline.tsv`); matrix 3286 match / 877 partial / 306 fail / 39 untested.
- **Wave 6 started and was stopped at a safe point** at the user's request (not merged). Worktrees `runtime/wt/{a,b,c}`, branches `terrain6-a/b/c` from Poe2Server `b021b44`; brief `runtime/wave6/README.md`; job lists `runtime/wave6/<x>-fails.tsv`, gate inputs `<x>-gate-in.tsv`, `match-sample-in.tsv`; client requests in `runtime/wave6/{b,c}/client-requests.txt` (answers in `client-results.txt`). Each agent wrote `runtime/wave6/<x>/handoff.md` for a fresh agent.
- Wave 6 split (user-approved, same as handoff 5): **A** `.dgr` wrong output and the only client user (serves B's and C's client requests first); **B** `.dgr` errors, G3_1 node-tile settle, G1/G4_WorldMap corridor bends, MapKarst pairing plugin, G4_13; **C** league stone-circle jobs (inside A's areas), HideoutLimestone, G3_town, Design, P2_1, the collision grid.

## How to resume wave 6
1. Start-up checks from handoff 4 (gh auth, git and submodule status, CI, versions/tracker/links, no leftover processes). Then per worktree: `git -C runtime/wt/<x> log --oneline b021b44..HEAD`, `git -C runtime/wt/<x> status --short --ignore-submodules=all -- crates`, `git -C runtime/wt/<x> diff --stat b021b44 HEAD -- modules` (must be empty).
2. The user prefers **fresh agents built from the handoffs** (no re-caching of old agents). One Sonnet agent per worktree (`model: "sonnet"`, `effort: "high"`, general-purpose, background); the prompt names the brief, its `runtime/wave6/<x>/handoff.md`, what is not gated yet (gate it first), and the next steps from the handoff. If the auto-mode classifier blocks a start, ask the user to switch to manual mode.
3. When the agents are done: merge all three branches into a merge worktree (`git -C modules/server worktree add ../../runtime/wt/m -b terrain6-merge main`, symlink `modules/*`), resolve conflicts keeping both sides where two agents extended the same code, build, `cargo test --workspace`, then **one full gate** on the merged code (`POE_FORCED_BUDGET=60 nice -n 19 <forced_verify> dumps/0.5.5e/terrain/batch/full/batch-results.tsv --out runtime/wave6/gate/forced-w6.tsv --threads 8`, about 1.5-2 h), compare with the baseline (no `match` may turn into a fail; bisect on lost jobs if any). Then: fast-forward Poe2Server `main`, bump nested submodules, push; merge the result into `dumps/0.5.5e/terrain/vectors/forced-results.tsv`, new baseline, `cargo xtask terrain-matrix && cargo xtask tracker`, update the tracker row and `docs/status/terrain.md`, commit the agents' docs sections, push the hub; remove the worktrees and branches.

## Reminders
- CPU limits (`nice -n 19`, 4 jobs/threads per agent); orphaned processes survive editor restarts.
- The pre-commit hook in Poe2Server's common git dir refuses `modules/` changes on `terrain*` branches.
- Ghidra must be open for the Ghidra MCP; subagents load its tools with ToolSearch.
- KWin Agents desktop: `scripts/agent-desktop.sh on` after a reboot (the user wants the game client there).
- Waiting on the user: `.vscode/settings.json` commit or not; the small open items ([open-items.md](open-items.md)); G-PARTY capture; Windows tests. After wave 6: fresh-seed client batch, WP7.13, WP7.15, then offer a server release.
