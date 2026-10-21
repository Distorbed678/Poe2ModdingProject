# Handoff: orchestrator of plan 2 (2026-10-21)

For a new orchestrator that continues [orchestration-2026-10b.md](orchestration-2026-10b.md). Read CLAUDE.md, MEMORY.md, the plan, then this file.

## How the last orchestrator worked
- Each work package (WP) goes to a Sonnet subagent: `Agent(model: "sonnet", effort: "high", subagent_type: "general-purpose")`, run in the background. The prompt has the goal, files to read first, the hard rules, the parallel agents' file sets, the acceptance criteria, and "update the docs in the same change, don't commit, report the files changed".
- Agents don't commit. The orchestrator reads the diff and runs `cargo build && cargo test`, plus the submodule tests, `cargo xtask build-dll` when the DLL changed, `python3 scripts/check-links.py` and `cargo xtask tracker --check`. It then commits the submodule first, pushes, bumps the pointer and commits the main repo.
- Parallel agents share one working tree. When a file holds hunks of two WPs, stage only one WP's hunks: `git diff <file> > x.diff`, keep the wanted hunks, `git apply --cached --recount`. Never `git stash` in the shared tree.
- Code agents run in parallel. **Only one agent at a time uses the game client** (GameFiles and the test prefixes). Code agents report an in-game plan, and the orchestrator schedules it.
- Never put passwords or tokens in an agent prompt: the permission classifier blocked one as data exfiltration. Agents read test accounts from the docs or create a throwaway account with `poe-server account create`.
- After each verified step, run the tests again before committing. Flipping `hashes_confirmed` broke a test once, fixed in 2529124.

## User preferences (also in memory)
- The game, the launcher and any browser started by agents appear only on the KWin desktop "Agents": `scripts/agent-desktop.sh on|off|status`. The rules also make those windows never take focus; without that, the game pulled the user's view to its desktop. A visible browser is only the `poe-agent-browser` Firefox instance (setup.md). Never use `xdg-open` or the launcher's "Open web admin" into the user's browser.
- Steam may be used and started without asking. Never log in to official servers.
- When usage is low, the user asks for a graceful stop: agents stop at a safe point and write `docs/plans/handoff-<wp>.md`. On resume, start fresh agents from the handoffs. Don't resume old agent ids.
- Monitor quietly. Report to the user when a WP finishes, at a gate, or on a failure.

## Status
| WP | State | Commits |
|---|---|---|
| A1 assessment | done | 31e2b4f ([../re/update-compatibility.md](../re/update-compatibility.md)) |
| A2 signatures | done, verified in game | client-hook d8b06a3, main ffda417 |
| A3/A4 check-build, versions | done, verified in game: client_hash = exe sha256, `hashes_confirmed = true`, unknown build gets code 8, broken table fails closed | main 0ac816b, a2eb526; protocol 457c1a6; launcher d39c7b0, 0391a68 |
| B1 login RE | done (gate G-LOGIN not needed) | 5ca0bec |
| B2 login and accounts | done, verified in game; fix: close the login connection after a refusal | 5c35884, 5f36335; protocol aeca943 |
| C1 movement gaps #11/#12 | code done (sprint-end coast, roll facing fallback); **in-game drift check was running at the stop**, see [handoff-c1.md](handoff-c1.md) | c947a15 |
| C2 multiplayer polish | done, verified with two clients (spawn 8 cells apart, no tint, remote roll) | 80e7de6, 5e349b8; protocol 76c28cd |
| C3 party and trade | code done: invite/cancel, trade request/cancel. Invitee side derived from Ghidra (UNVERIFIED). Accept/decline only logged. **Two-client check not run** | 2b5cc48; protocol c845efd |
| C4 mod loader | done, sample mod loads in game | 224f5a5; client-hook 2d4d0a9; launcher 6954269 |
| Phase D-F | not started | |

## Next steps
1. **C1:** start a fresh agent from [handoff-c1.md](handoff-c1.md). It finishes the in-game drift cases (click-to-move first, then `use_wasd_to_move=true` in the test prefix's `poe2_production_Config.ini` for the WASD cases, then restore it), then updates movement.md and the tracker's Dodge roll and Sprint notes. Then commit.
2. **C3 two-client check:** the plan is in docs/protocol/entities.md, "Party and trade flows (WP C3)", run with `scripts/test-mp.sh`. If the invitee's popups don't appear, the two derivations are the first suspects: party role 0 with `unk_0` = the invitee's index, and `TradeState.unk_c = 0x32`.
3. **Gate G-PARTY (user):** the capture checklist was given to the user and is in entities.md. Before the user captures: `scripts/agent-desktop.sh off`, `cargo xtask deploy-client --official`, and no test running. Afterwards: `deploy-client` back to private, rules `on`. Then implement accept and decline.
4. **Phase D (public release):** D1 audit (read-only, all 5 repos and their full history), D2 fix and scrub (user confirmation before any force-push), D3 guides and hygiene, D4 go public (user gate). Inputs for D1:
   - The test credentials (mp1/mp2, b2test, phasea) are in docs and runtime only. Check the history for them.
   - `dumps/` and `runtime/` are git-ignored; check the history anyway.
   - Poe2Launcher CI uses deploy keys and secrets (HOOK_DEPLOY_KEY, PROTOCOL_DEPLOY_KEY); Poe2ClientHook CI uses SUBMODULE_TOKEN. Both go away in D3.
5. **Phases E and F** follow the plan.

## State left behind
- `GameFiles/`: private mode. The login mode is `steam`, left by test-mp.sh restore. The current DLL is from client-hook 2d4d0a9, with `sample-mod` enabled. For the standalone login: `cargo xtask deploy-client --no-build`.
- KWin "Agents" desktop rules: on.
- Dev databases: `runtime/server.sqlite` (accounts b2test, phasea, mp1, ...) and `runtime/server-mp.sqlite`. Test passwords are not in this file.
- Open items, from CLAUDE.md "Open elsewhere":
  - movement (C1 in-game);
  - party/trade replies (G-PARTY);
  - remote equipment visuals;
  - the server walks about 1% slower than the client (`dt` 0.033 vs `speed/30`), noted by C1;
  - early roll steps deviate up to about 11° (the client's own path planning).
