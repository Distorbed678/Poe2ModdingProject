# Server commands, admins and instances

## Commands

One registry (`servers/instance-server/src/commands.rs`) defines every server command: name,
group, whether it needs an admin, typed arguments and help. The same commands work in two
places:

- **Chat**: `!name args` (prefix `[commands] prefix`; `/` stays with the client's own
  commands). Replies come back as chat lines from `<Server>`.
- **Overlay**: the DLL's Commands page builds a form per command from the registry, which the
  server sends over the authenticated debug link (`cmds` lines in debug frames; the overlay
  asks with a `registry` command frame, [../protocol/side-channel.md](../protocol/side-channel.md)).
  Results arrive as `ack` lines. Commands added to the registry show up in the overlay by
  themselves.

Execution is in `exec.rs` (`InstanceServer::run_command`). Admin commands need the **admin role**
of the account (checked when the command runs; see Accounts below) and are appended to
`[commands] audit_log` (`unix ms, account, command, result`; passwords are written as `***`).

| Command | Admin | What it does |
|---|---|---|
| `help [command]` | | the command list, or one command's usage |
| `where` | | your instance, area, seed and cell |
| `who` | yes | online players and their instances |
| `kick <player>` | yes | disconnects a player |
| `mute <player> [minutes]` / `unmute <player>` | yes | stops / restores a player's chat |
| `announce <message>` | yes | a `<Server>` chat line to everyone |
| `instances` | yes | every instance (id, area, seed, layout, players) |
| `instance_new <area> [seed] [topology] [orientation]` | yes | creates an instance without entering it |
| `transfer <area> [seed] [topology] [orientation]` | yes | creates an instance and moves you into it |
| `join <instance>` | yes | moves you into an existing instance |
| `close <instance>` | yes | closes an instance; its players are disconnected |
| `teleport <x> <y>` | yes | moves you to a grid cell |
| `account <action> [name] [value]` | yes | accounts, see below |
| `terrain` | yes | generates this instance's terrain on the server and logs the comparison |
| `area_info <area> <seed>` | yes | research: a second AreaInfo on the connection (the client may crash) |

`seed` takes `random` (the server picks), `topology` and `orientation` take `natural` (the
client's own choice). Any area in `WorldAreas` works, not only act 1. Topology numbers are
1-based in the area's topology list; orientations are checked against that topology's allowed
ones.

## Web API

The same commands are available over HTTP for the web admin site: `kick`, `mute`/`unmute`, `announce`, instance create/close, player transfer and the `account` actions are endpoints of `poe-web-api` that run the registry command as the logged-in web admin (role checked at the time, audit lines tagged `<admin> (web)`, passwords redacted). Endpoints, authentication and the live log: [../architecture/web-api.md](../architecture/web-api.md). Setup in short: make an admin offline ([setup.md](setup.md#accounts-and-characters-offline)), start `poe-server`, `POST /api/v1/login` on `[web] bind` (default `127.0.0.1:8080`), and send the token as `Authorization: Bearer`. `[accounts] open_registration = true` lets `POST /api/v1/register` create player accounts; server commands always can.

## Web admin

A browser UI for the same commands in the launcher's look (dark or light theme, sidebar navigation, a bottom tab bar on narrow screens): dashboard with the server control, live logs, players online (kick, mute, transfer), instances, accounts, characters and an announce dialog. It is the Svelte app in the submodule `modules/web-admin` (repo Poe2WebAdmin); how it is built is in [../architecture/web-api.md](../architecture/web-api.md#the-web-admin-spa).

Build it, point `[web] static_dir` at `modules/web-admin/dist`, restart `poe-server` and open <http://127.0.0.1:8080> (the `[web] bind` address): [setup.md](setup.md#web-admin-build). Only accounts with the `admin` role get in (make one offline, [setup.md](setup.md#accounts-and-characters-offline)); the session lasts 12 hours. The moon/sun button in the top bar switches the theme (saved in the browser; the system preference decides until you choose).

### Stop and restart the server

The dashboard's **Server** card has **Restart** and **Stop** buttons (admins only, like the whole site). Each opens a confirmation dialog with an optional message for the players and a countdown in seconds (0 to 3600; quick choices Now, 30 s, 1 min, 5 min, 10 min). The players in game get a `<Server>` chat line when the countdown starts and again at 10 min, 5 min, 2 min, 1 min, 30 s, 10 s and 5 s before the end, and a last one at the end, each with your message; then they are disconnected (their characters are saved) and the server stops or restarts. A banner under the top bar of every page shows the countdown with a **Cancel** button (the players are told) until the last announcement. After a restart the page notices that the server is back, keeps you signed in (the session is in the database) and says so. A stop leaves the server off: the page shows "The server has stopped" and reconnects by itself if somebody starts it again. The same endpoints work with `curl` (`POST /api/v1/admin/server/stop` and `/restart`, body `{"delay_secs": 60, "message": "new build"}`, [../architecture/web-api.md](../architecture/web-api.md#stop-and-restart)); every request, cancellation and announcement is in the audit file as `<admin> (web)`.

Ctrl+C and SIGTERM (systemd stop, the launcher's Stop) shut the server down the same way, at once and without announcements: players are disconnected, sessions saved, exit code 0.

**How a restart brings the server back** (`[server] restart_mode` in `config/server.toml`):

| `restart_mode` | After the shutdown | Use it |
|---|---|---|
| `"exec"` (default) | the process starts itself again with the same binary, arguments, environment and working directory (Unix: `exec`, the pid stays; Windows: a new process is started and the old one exits) | `cargo run`, a terminal, tmux, a systemd unit without special settings on Linux |
| `"exit"` | the process exits with **code 75** and something else starts it again | systemd (`RestartForceExitStatus=75`, [hosting.md](hosting.md#running-it-permanently)), the launcher's Host a server page (see below), any supervisor that restarts on an exit code |

A stop exits with 0, so a supervisor that restarts only on failure leaves a stopped server stopped. A supervisor that restarts on every exit (`Restart=always`) also restarts a stop; use `on-failure` as in the hosting guide. Without a supervisor and with `"exit"` a restart request simply ends the process.

**Hosted through the launcher:** the launcher starts `poe-server` as a child process. On Linux and macOS the default `exec` mode works unchanged (same pid, same pipes). On Windows `exec` mode makes the launcher see an exit, so the launcher should write `restart_mode = "exit"` into the config it generates and start the server again when it exits with code 75. That launcher change is not done yet: until then, with a launcher-hosted server on Windows a restart from the web admin ends the server (the launcher shows it as exited) and the host page's Start brings it back.

The log page shows what the server's `[logging] filter` lets through (to see `debug` events, enable them there). For development the Vite dev server proxies `/api` to `[web] bind` ([setup.md](setup.md#web-admin-build)).

## Accounts

Admins are accounts with the `admin` role; there is no admin list in the config. Model, hashing and tokens: [../architecture/accounts.md](../architecture/accounts.md).

| Command | What it does |
|---|---|
| `account create <name> <password>` (CLI: `account create <name> --password-stdin`) | new player account. The server hashes the password the way the client does (SHA-256 of the UTF-16LE text), then argon2id. Minimum 8 characters, username 3-64 of `A-Za-z0-9_.@+-` |
| `account role <name> <player\|admin>` | sets the role; takes effect on the next command |
| `account disable <name>` / `account enable <name>` | a disabled account cannot log in; disabling revokes its saved logins and ends its sessions |
| `account passwd <name> <password>` (CLI: `account passwd <name> --password-stdin`) | new password; revokes the account's saved logins (Remember Me, launcher, web) |
| `account list` | every account: id, role, state, last login (`no password (dev)` marks accounts made by the removed machine-hash login; nobody can log in to one until `account passwd`) |

An admin cannot demote or disable themselves.

**Passwords on the command line.** On the CLI give the password of `create` and `passwd` with `--password-stdin`: `poe-server account create alice --password-stdin` reads one line from standard input (e.g. `read -rs pw; printf '%s\n' "$pw" | poe-server account create alice --password-stdin`, or a pipe from a password manager). Only the trailing newline is removed (spaces stay part of the password), an empty line is refused, and the value is never echoed or logged. The flag works anywhere among the words, only with these two subcommands, and not together with a password argument. The old form `account create <name> <password>` still works but is discouraged: every program on the machine can read a command line in the process list (and your shell history keeps it). The launcher's "Host a server" setup uses the stdin form.

Passwords typed in chat travel over the game connection and appear on screen: prefer the offline form for real passwords. `poe-server account ...` runs against the database, also while the server is up, and works for the first admin; it and the offline `poe-server character create|list` (test setups, `scripts/test-mp.sh`) are in [setup.md](setup.md#accounts-and-characters-offline).

Password attempts are rate limited (`[security]`: 5 failures per account name and 20 per IP in 5 minutes lock the key for 5 minutes, in game and on the web; saved-token logins are not affected; a restart clears locks; details: [../architecture/accounts.md](../architecture/accounts.md#password-attempt-limits)). Set a limit to 0 to switch it off, e.g. for a test script that fails logins on purpose.

A client whose exe is not a known build of `[game] version` (its `client_hash` is not in `versions/<version>.toml` `[client] hashes`) is refused at login with the client's own "There has been a patch that you need to update to" popup. `[security] allow_unknown_client = true` lets any build in (logged); a test config made without a version file checks nothing. Nothing is refused while the version file's `hashes_confirmed` is `false` ([new-game-version.md](new-game-version.md#what-the-server-does-with-the-clients-version)).

A second login of an account ends the first one (login connection and game). Existing databases: accounts made by the removed machine-hash login (null password) cannot log in until `account passwd` gives one a password, and an existing `Exile1` was made an admin by the database migration (a fresh database promotes nobody).

## Instances

`poe_cluster::InstanceDirectory` (in-process `MemoryInstances`) holds every instance: area,
seed, forced topology and orientation, the players in it and since when it is empty.

- Entering from character selection: towns and hideouts share one kept instance; other
  areas join the area's newest open instance, else get a new one.
- The first player decides an instance's seed (spec seed, else saved location, known seeds,
  random); later players get the same one.
- Empty instances close after `[instance] idle_minutes` (towns and hideouts stay in the directory; their simulation (the instance actor) also ends after that time and restarts with the next player).
- A closed instance disconnects its players.
- All players in an instance share one simulation and see each other ([instance-world.md](../architecture/instance-world.md#the-instance-actor-wp36)). The escape menu pauses the world only for a lone player.
- `instances` shows each instance's player count and names (written by its actor).

## Transfers

Verified 2026-10-17 (`G1_1` to `G1_2` with a forced topology 3, orientation 0; the client's
tile hash 1655364279 equals the server's forced generation):

1. The instance server issues a ticket for the target instance and hands it to the account's
   login connection (`LoginSessions::transfer`), which keeps it as pending.
2. It ends the client's instance session. The client goes back to character selection on its
   still-open login connection and asks for the character list.
3. The login server answers with the list and the pending `InstanceDetails`; the client
   connects to the new instance and loads its area.

What does not work: `InstanceDetails` while the client is in an instance (received, ignored),
and a second AreaInfo on one connection (crash). The login connection must stay open, so the
server does not send login `0x0017` after the hand-off (`[network.login] close_after_handoff = false`;
why: [../protocol/login.md](../protocol/login.md#hand-off-to-the-instance-server)). How official
servers reconnect for later transfers is still to be captured.

Forced layouts: an instance with a topology or orientation sends them in AreaInfo's first stat
record (flags0 bit 7; `map_force_topology` = 1-based topology, `map_force_orientation` =
orientation + 1). Verified: the client generates that layout.

## Chat

C→S `0x0016` ChatSend (`u8`, wstring text, `u8`) carries the line as typed. Lines starting with
the command prefix run as commands (replies as `<Server>` chat lines); others are routed by
their prefix: `#` global and `$` trade to everyone on the same channel number (from the
client's join requests, `/global N`), none = local (same instance), `@name text` whispers.
Party (`%`) and guild (`&`) answer "not supported yet". Muted players get "You are muted."

## Overlay pages (our server only, SERVER group)

Hidden while the read-only latch is on (official servers), like the Sync state and Desync pages.

- **Commands**: every registry command as a form, grouped; results below.
- **Instances**: the server's instances (id, area, seed, layout, players, time until an empty
  one closes) with Join and Close, and a new-instance form: area picker (search, kind and act
  filters, recent picks; every area from `poe-hook-data/areas.tsv`, written by
  `cargo xtask dll-data` or `deploy-client`), seed (random or server-picked), topology and
  orientation (natural or forced), with "Transfer there" and "Create without transferring".
