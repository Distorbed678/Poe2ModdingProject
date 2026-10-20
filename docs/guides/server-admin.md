# Server commands, admins and instances

## Commands

One registry (`servers/instance-server/src/commands.rs`) defines every server command: name,
group, whether it needs an admin, typed arguments and help. The same commands work in two
places:

- **Chat**: `!name args` (prefix `[commands] prefix`; `/` stays with the client's own
  commands). Replies come back as chat lines from `<Server>`.
- **Overlay**: the DLL's Commands page builds a form per command from the registry, which the
  server sends over the debug link (`cmds` datagrams; the overlay asks with `registry`).
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

The same commands are available over HTTP for the web admin site: `kick`, `mute`/`unmute`, `announce`, instance create/close, player transfer and the `account` actions are endpoints of `poe-web-api` that run the registry command as the logged-in web admin (role checked at the time, audit lines tagged `<admin> (web)`, passwords redacted). Endpoints, authentication and the live log: [../architecture/web-api.md](../architecture/web-api.md). Setup in short: make an admin offline (below), start `poe-server`, `POST /api/v1/login` on `[web] bind` (default `127.0.0.1:8080`), and send the token as `Authorization: Bearer`. `[accounts] open_registration = true` lets `POST /api/v1/register` create player accounts; server commands always can.

## Accounts

Admins are accounts with the `admin` role; there is no admin list in the config. Model, hashing and tokens: [../architecture/accounts.md](../architecture/accounts.md).

| Command | What it does |
|---|---|
| `account create <name> <password>` | new player account. The server hashes the password the way the client does (SHA-256 of the UTF-16LE text), then argon2id. Minimum 8 characters, username 3-64 of `A-Za-z0-9_.@+-` |
| `account role <name> <player\|admin>` | sets the role; takes effect on the next command |
| `account disable <name>` / `account enable <name>` | a disabled account cannot log in; disabling revokes its saved logins and ends its sessions |
| `account passwd <name> <password>` | new password; revokes the account's saved logins (Remember Me, launcher, web) |
| `account list` | every account: id, role, state, last login (`no password (dev)` marks machine-hash accounts) |

An admin cannot demote or disable themselves. Passwords typed in chat travel over the game connection and appear on screen: prefer the offline form for real passwords. It runs against the database, also while the server is up, and works for the first admin:

```
cargo run -p poe-server -- account create alice "a long password"
cargo run -p poe-server -- account role alice admin
cargo run -p poe-server -- account list
```

A second login of an account ends the first one (login connection and game). Existing databases: accounts made by the machine-hash login keep working on `[accounts] dev_machine_hash_login = true` (null password; `account passwd` gives one a password), and `Exile1` was made an admin by the database migration.

## Instances

`poe_cluster::InstanceDirectory` (in-process `MemoryInstances`) holds every instance: area,
seed, forced topology and orientation, the players in it and since when it is empty.

- Entering from character selection: towns and hideouts share one kept instance; other
  areas join the area's newest open instance, else get a new one.
- The first player decides an instance's seed (spec seed, else saved location, known seeds,
  random); later players get the same one.
- Empty instances close after `[instance] idle_minutes` (towns and hideouts stay).
- A closed instance disconnects its players.

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
