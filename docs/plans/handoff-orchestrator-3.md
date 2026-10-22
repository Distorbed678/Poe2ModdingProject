# Handoff: orchestrator 3 (2026-10-22)

For a new orchestrator. Read [CLAUDE.md](../../CLAUDE.md), your MEMORY.md, [docs/overview.md](../overview.md) (which repo does what), then this file. Plan 2 ([orchestration-2026-10b.md](orchestration-2026-10b.md)) is done apart from the user gates below; the work since then followed the user's requests directly (launcher and web admin redesign, mod system, repo split). The previous handoff, [handoff-orchestrator-2.md](handoff-orchestrator-2.md), has the plan-2 history.

## How to work (what worked)
- Each work package (WP) goes to a Sonnet subagent: `Agent(model: "sonnet", effort: "high", subagent_type: "general-purpose")` in the background. The prompt has the goal, the files to read first, the user's decisions, the hard rules, the other agents' file sets, acceptance checks, "docs in the same change", and the report wanted.
- Agents normally do not commit. The orchestrator reviews, runs the checks, commits inside the submodule first, pushes, then bumps the pointer in the hub. Exceptions given explicitly in a prompt: release agents may commit, push and tag in the one repo they release; agents that create a new repo's content may push there.
- Checks before a commit: hub `cargo build && cargo test`, `cargo xtask tracker --check`, `cargo xtask versions --check`, `python3 scripts/check-links.py`; server `cargo xtask server test` (or `cargo test` in modules/server) and `cargo xtask openapi --check`; `cargo xtask build-dll` when the hook changed; launcher `npm run check`, `npm test`, src-tauri `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; web admin `npm run check`, `npm test`, `npm run build`. Watch CI with `gh run list/watch` after each push.
- Parallel agents share one working tree: give each a disjoint file set; when a file holds hunks of two WPs, stage only one WP's files or hunks. Never `git stash`.
- Only one agent at a time uses the game client.
- Never put passwords or tokens in a prompt. Agents create throwaway accounts with `poe-server account create <name> --password-stdin` and keep secrets in git-ignored runtime/.
- Questions and suggestions to the user go through the AskUserQuestion tool (the user asked for this), with the recommended option first.

## User preferences learned
- Steam may be started without asking; never log in to official servers; official captures are the user's.
- The KWin "Agents" desktop rules are **off** (the user allowed it; sprint-end movement checks also need them off). Agents must not turn them on (one did; it was reverted). No Spectacle screenshots; visual checks with headless browsers into the scratchpad; never open the user's browser.
- Usage limits: when the user's usage runs out, agents stop mid-work. On resume, send each agent a message to continue (the user said fresh agents and handoffs are not needed for that). Check `git status` first: twice an agent's uncommitted edits were gone after a limit; tell the agent so it redoes them.
- **Permission classifier:** in auto mode it blocks starting new agents (even with the user's OK). Ask the user to switch to manual mode, then start them.
- The user wants regular status updates when an agent finishes, and wants to approve plans and options before larger changes.

## Repos (all public, all submodules of the hub under modules/)
| Repo | Hub path | Notes |
|---|---|---|
| Poe2ModdingProject (hub) | . | docs, plans, scripts, xtask (dev/RE tooling, `cargo xtask server build/run/test/web-admin`), news.json, tracker |
| Poe2Server | modules/server | the server (servers/*, crates/*, config/), release workflow `server-v*`, pinned `dat-schema-<game>` release, own nested submodules protocol, client-hook, web-admin |
| Poe2Protocol | modules/protocol | opcodes and **versions/** (per-game-build definition) |
| Poe2ClientHook | modules/client-hook | the DLL, mod loader, poe-hook-mods (manifest v2), release `hook-v*` |
| Poe2WebAdmin | modules/web-admin | the web admin SPA; dev builds from the hub's checkout, releases from the server's nested one |
| Poe2Launcher | modules/launcher | Tauri launcher, release `v*`, auto-update (signing secrets set in the repo, key in ~/.tauri/) |
| Poe2ModSDK | modules/mod-sdk | server mod ABI (poe-mod-api), SDK, `poe-mod` tool, tag v0.1.0 |
| Poe2Mods | modules/mod-index | mod index for the launcher's browser (PR-based) |
| Poe2ServerList | modules/server-list | public server list for the launcher's browser (PR-based) |
| Poe2ModdingProject-private-backup | (none) | private, the pre-scrub history; never publish |

Nested submodules stay (each repo builds alone); bump them together with the top-level ones.

## Current releases
- Launcher **v0.4.0** (layout A, Home, server browser, logs, backups, mods and mod sync, hook download, server releases from Poe2Server with fallback to the hub repo, "Server update available" with one-click update: verified download, graceful stop, database backup, config carried over, restart, rollback on failure).
- Client hook **hook-v0.3.0** (manifest v2, mod report at login).
- Server **server-v0.2.1** in Poe2Server (first release there). The old server-v0.1.0/0.2.0 and dat-schema-0.5.5e in the hub repo stay as history; the launcher falls back to them.
- SDK **v0.1.0**.

## What is left
1. Release process for later releases (launcher example): bump tauri.conf.json, package.json, src-tauri/Cargo.toml and the lockfiles, CHANGELOG Unreleased -> version, commit, tag `v<ver>`, watch the workflow. Server: bump modules/server Cargo.toml workspace version, tag `server-v<ver>` in Poe2Server. Hook: bump the client-hook workspace version, tag `hook-v<ver>`. Then bump the pointers in the hub.
2. **F acceptance: the user's end-to-end test** (user): clean launcher profile, install the game (Steam depot), get the hook, Host a server (now from Poe2Server), Play, Open web admin, Restart from the web admin. Steps were given in the conversation; the short form is in [docs/guides/launcher.md](../guides/launcher.md). Expect fixes (first real DepotDownloader run, pause/resume, icon/frame on Wayland, Play with a downloaded hook).
3. **Gate G-PARTY and movement problem 5** (user capture, after the e2e test): before it: no test running, `cargo xtask deploy-client --official` (the Agents rules are already off); afterwards deploy-client back to private. Then implement party/trade accept and decline ([entities.md](../protocol/entities.md), "Party and trade flows (WP C3)") and the sprint end with W held ([movement.md](../protocol/movement.md), problem 5).
4. **Windows** (needs a Windows machine or VM, ask the user): launcher installer + update, Host a server (zip, taskkill), Play, server restart under the launcher.
5. **Smaller open items:** remote players' equipment visuals; dynamic obstacles (entities) in the ported client pathfinder; data-tier mod overlays `config_defaults` and server `opcodes`; class names in the web admin Players page; ban commands in chat/CLI; rate limit on the mod download endpoint.
6. **After that:** the terrain port to all areas ([terrain-all-areas.md](terrain-all-areas.md)), CLAUDE.md's main task; it removes the need for the terrain upload.

## User decisions to keep (2026-10-21/22)
- Audit answers: keep the commit email; keep the HNC key_xor table (gitleaks allowlist); MIT only; official mode public with docs (also a launcher entry "Official (read-only)"); machine-hash Steam login removed; launcher app id `org.poe2modding.launcher`; terrain upload stays on; ConnectEx guard done; private vulnerability reporting on.
- Mods ([server-modding-analysis.md](server-modding-analysis.md), Decisions): native Rust server mods + data-only mods; one package per mod with side client/server/both; the server serves required mods; launcher mod sync Off/Ask/Auto, Auto downloads everything; per-server mod sets; login mod check from the start with `[mods] allow_extra_client_mods`; mods just load (no install guard); never hot reload; exact version + SHA-256 pinning; SDK in its own repo.
- Launcher: layout A (Home, Servers, Host, Library, Settings); square window frame (rounded corners blanked the window in a nested KWin; untried on the real desktop); "Minimize while the game runs" interpreted as minimize on start, restore on exit (the user's exact meaning was not confirmed).
- Web admin: launcher style, Stop/Restart with countdown, live charts, Players with bans, config editor (on for admins by default), scheduled tasks (restart only so far), MOTD.
- Repo split: server in Poe2Server with history, versions in Poe2Protocol, all repos as submodules in modules/, nested submodules kept, launcher server update = notify + one click.

## State left behind
- GameFiles/: private mode, standalone login, hook DLLs from client-hook 723de35 era (deployed by the server-mods agent); `hello-both` test mod installed but disabled, `sample-mod` enabled.
- The user's launcher data (~/.local/share/poe2-launcher) has server-v0.1.0 installed from an early test (old web admin): launcher 0.4.0 offers the update to server-v0.2.1 on the Host page; that is the first real run of the update flow (only mock-tested so far).
- Agents desktop rules: off.
