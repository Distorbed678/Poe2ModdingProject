# Handoff 15: after R1.5 (2026-10-29)

Start here. Read [CLAUDE.md](../../CLAUDE.md) (hard rules), [handoff-agent-7.md](handoff-agent-7.md)
section 2 and 3 (the user's preferences, commit rules, CPU limits; still valid), then the roadmap
[roadmap-2026-11.md](roadmap-2026-11.md) and your memory. R1 is in [handoff-agent-14.md](handoff-agent-14.md).

## State
- **R1.5 (launcher and web admin cleanup, Hotfix 1 only): DONE and released**, signed off by the
  user after an in-game test of the Create account button. Plan and every decision:
  [r1.5-ui-cleanup.md](r1.5-ui-cleanup.md).
  - **0.5.5e is gone** everywhere: version files, signature table, `opcodes/0.5.5e.hotfix1`, RE
    addresses in docs and comments are Hotfix 1 addresses (moved with `scripts/re/convert_addrs.py`),
    `dumps/0.5.5e` merged into `dumps/0.5.5e.hotfix1` (terrain, captures, Ghidra exports). The Ghidra
    project still holds the unused `/PathOfExileSteam.exe` (0.5.5e) program; the user may delete it.
  - **Launcher v0.6.0**: several hosted servers (`servers/<id>`), Servers tabs My servers / Browse
    public / Host, Manage (Overview with usage, Settings in folding sections, Program, Backups),
    delete with the name typed, no account creation, Home chooser, News feed, Library and Settings
    reworked.
  - **server-v0.4.0** (Latest): `GET/POST /setup` (first admin, loopback only), `POST /admin/accounts`,
    `GET /local/metrics`, network rates in the metrics (`poe_net::stats`), web admin with register
    and first-run pages and calmer pages.
  - **Launcher v0.7.0** (follow-up the same evening, the user's request): **no launcher login**.
    Accounts belong to each server; Play starts the game at its own login screen pointed at the
    selected server, and the player signs in there. Rows offer *Web admin* and *Create account*
    (server pages in the browser); the plain-http confirmation is gone (the launcher sends no
    secrets). The first-admin notice follows the server's `GET /setup`.
  - **hook-v0.5.0**: Hotfix 1 only; the login screen's Create account button opens the server's
    register page (`register_url`, `ShellExecuteW` hook; never in official mode).
- Nothing of ours runs; `runtime/client.lock` absent; the Agents-desktop rules are on. `GameFiles/`
  has the hook built from `2376961` (same code as hook-v0.5.0) with `register_url` for the dev server.

## Next: R2, server-side collision
Unchanged from [handoff-agent-14.md](handoff-agent-14.md#next-r2-server-side-collision-roadmap-section-r2)
(grid exports at scale, fix differences, live movement test, `generated_grid` on by default).
Paths there that named `dumps/0.5.5e/...` are now `dumps/0.5.5e.hotfix1/...`.

## Lessons from this session
- **Never `pkill -f` or `pgrep -f` with a pattern that is in your own command line**: it kills the
  shell running it (exit 144, twice). Find the PID first (`ps -eo pid,comm,args`), then `kill <pid>`.
- `node_modules` of the launcher and the web admin can be stale after a pull (a missing
  `@fontsource-variable/inter`): `npm ci` first.
- Screenshots without a visible browser: `runtime/ui-rework/shot/shot.mjs` (launcher `?mock`, web
  admin `FIXTURES=./admin-fixtures.mjs`, now with `SIGNEDOUT=1` and `SETUP=1`).
- Poe2Server is not `cargo fmt`-clean as a whole and its CI does not check it; the hook, launcher and
  mod SDK CIs do (`cargo fmt` before pushing there).
