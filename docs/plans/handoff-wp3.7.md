# Handoff: WP3.7 two-client test (stopped early, 2026-10-20)

## Done
- **client-hook b7579a9** (pushed): `POE_HOOK_KEYS_FILE` (file name next to the exe, or absolute path) selects each process's input file in `poe-hook-core/src/auto_keys.rs`; the DLL logs `input file: ...` at start. The main repo's `modules/client-hook` pointer is NOT bumped yet.
- **Main repo, uncommitted:**
  - `scripts/test-mp.sh` (setup / server / clients / client n / keys n ... / shot n / status / stop / restore). It generates `runtime/server-mp.toml` (LAN IP as `public_host`, own database `runtime/server-mp.sqlite`), creates accounts mp1/mp2 and one character each in `G1_town`, copies prefix 1 to `third_party/proton-prefix-2`, deploys with the LAN trust list, and fetches a fresh remember-me token per launch (web API launcher login, then game-token).
  - `xtask/src/main.rs`: global `--prefix`, new `select-gateway` command; `run-client`, `login-token` and the gateway selection of `deploy-client` take the prefix.
  - `servers/poe-server/src/main.rs`: offline `poe-server character create <account> <name> <start_state> [area] [league]` and `character list [account]`.
- GameFiles is back to the private deploy, `login_mode = "steam"`, loopback trust; `config/server.toml` still has 127.0.0.1. Clients and server are stopped.

## Results so far (acceptance)
- Token login on the standalone path works for both accounts (`authenticated ... token_login=true`), both through 192.168.1.93. DLL log: `trusted servers: loopback, 192.168.1.93:20481, 192.168.1.93:6112`, `connect ... (trusted server)`, no latch. PASS (log-in part of item 1).
- Both characters entered the same `G1_town` instance (445932): entities 12000 and 12001, `players=2`; client 1 received 2 `AddObject`s. Memory: 11 GB used with none, 19 GB with two clients (62 GB total), no problem.
- Item 2 (see each other): UNDECIDED. Both spawn on the same cell (379,356), so the characters overlap in the F8 screenshots; the shots show one character each.
- Items 3-6: NOT TESTED. Blocker: **WASD input does nothing in either client** (`in:0x57:ms` and `indown/wait/inup`, also click-to-move): no `action start ... Move` in the DLL log, no `0x1a5`-type packets out, even with client 2 closed. `Escape` (posted key) does open the pause menu, so UI input works; only the raw-input path fails.
- The only differences from the verified WP2.4 runs: windowed 1280x720 (`borderless_windowed_fullscreen=false`), standalone login, LAN host, `POE_HOOK_WINDOW_TITLE`/`KEYS_FILE` env. I had just restored prefix 1 to 2560x1440 borderless and relaunched client 1 to test whether the window mode is the cause, but was stopped before testing movement.

## Next steps
1. `scripts/test-mp.sh setup; server; client 1`, then `keys 1 inmove:1700,720 indown:0x57 wait:2000 inup:0x57` with the original 2560x1440 borderless config (setup rewrites the resolution to `MP_RES`: run with `MP_RES=2560x1440`, and set `borderless_windowed_fullscreen=true` back by hand or fix the script, which forces `false`). If it moves, the window mode was the cause; then use cursor positions at that resolution (the old tests used `inmove:1700,720`).
2. If it still fails, test the same single client with `login_mode steam` and the committed config (known good setup) to separate a WP3.6 actor regression from the test setup. Compare `Move` action start in the DLL log.
3. Separate the spawn cells (two players on one cell overlap): walk A away first, then F8 from both clients.
4. Then items 3-6 (relay of moves/roll, chat, transfer leave sequence, duplicate login kick); write results to `docs/guides/hosting.md` ("Local two-client test") and `docs/status/tracker.toml` (`cargo xtask tracker`).
5. Document `character create`, `--prefix`, `select-gateway` and `POE_HOOK_KEYS_FILE` in `docs/guides/setup.md` and `server-admin.md`; run `python3 scripts/check-links.py`; bump the client-hook pointer and commit in the main repo.
6. Script bugs to remember: the script's `stop` calls `restore`, which resets prefix 1's window size from `runtime/mp-orig-res.txt`; `set -e` pitfalls were fixed with `n_auth`.
