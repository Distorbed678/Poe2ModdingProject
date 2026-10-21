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

## Status (updated 2026-10-21, evening)
| WP | State | Commits |
|---|---|---|
| A1-A4, B1, B2, C2, C4 | done, verified in game | see git log before 05922af |
| C1 movement | done except problem 5 (sprint end with W held: client coasts then restarts itself; official answer uncaptured). Problems 1, 3, 4 solved: WASD tests need `user_input_mode=wasd` + `last_selected_KBM_input_mode=wasd`; sprint-end checks need the Agents desktop rules OFF | 86d0e88, 593a790, f420e3d |
| C3 party and trade | verified with two clients (invite/cancel, trade request/cancel; crash "Invalid inventory id" fixed with the two trade AddInventory). Accept/decline only logged: gate G-PARTY | 93a2d18 |
| D1 audit | done: [public-release-audit.md](public-release-audit.md), questions 1-15 for the user | ab89ae4 |
| D2 code fixes | done for everything that needs no user answer (S-01 safe committed config + `scripts/dev-config.py`, S-02 pre-auth limits, S-07, ...) | 61f1603 |
| D2 history scrub | done: rewritten main history in a NEW repo `Poe2ModdingProject`; old repo renamed `Poe2ModdingProject-private-backup` (private); modules needed no rewrite | new history head a0f1456 |
| D3 hygiene | done: CONTRIBUTING, SECURITY, licences, templates, disclaimers, client guide, hosting guide, main CI; 2026-10-21: CI switched to plain public checkouts, deploy keys and SUBMODULE_TOKEN deleted | 61f1603, launcher b70a3c5 |
| D4 go public | **user gate** | |
| E1/E2 release CI, updater | done, workflow never run. Signing key in `~/.tauri/poe2-launcher.key{,.pub,.password}`; user must create `TAURI_SIGNING_PRIVATE_KEY(_PASSWORD)` | launcher a02901d, main 5237edb |
| E3 test releases | after D4 | |
| F1 server release CI, F3 server side | done, workflow never run (tag `server-v*`) | 619cc1b, web-admin 1544232 |
| F2/F3 launcher | done, tested against a real package (backend tests) | launcher b70a3c5, main 343187c |

## Next steps
1. User decisions: audit questions (docs/plans/public-release-audit.md), the launcher signing secrets, `account create --password-stdin` (an agent for it was blocked by the auto-mode classifier; the launcher creates the admin via a private loopback start + POST /register meanwhile).
2. G-PARTY capture (plus a W-held sprint end for problem 5): before it, no test running, `cargo xtask deploy-client --official`; afterwards deploy-client back to private. The Agents rules are OFF since 2026-10-21 (user allowed it).
3. D2 scrub -> D4 public -> D3 CI switch to public checkouts -> E3 releases v0.1.0/v0.1.1 -> launcher end-to-end with a Steam depot download (Secret Service/KWallet is on now).
4. Repo-wide `cargo fmt` in the main repo is still pending (CI does not check it there).

## State left behind
- `GameFiles/`: private mode, login_mode steam; the DLL there is still client-hook 2d4d0a9 (newer builds not deployed).
- KWin "Agents" rules: off.
- An old `stash@{0}` in the main repo holds the first C1 diagonal-cursor attempt (superseded by f420e3d); the user decides whether to drop it.
