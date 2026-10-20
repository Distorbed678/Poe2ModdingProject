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

Execution is in `exec.rs` (`InstanceServer::run_command`). Admin commands are checked against
`[admin] accounts` (account names, e.g. `Exile1`) and appended to `[commands] audit_log`
(`unix ms, account, command, result`).

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
| `turn_cell`, `turn_timing`, `face_cursor` | yes | movement switches (see `[instance]`) |
| `terrain` | yes | generates this instance's terrain on the server and logs the comparison |
| `area_info <area> <seed>` | yes | research: a second AreaInfo on the connection (the client may crash) |

`seed` takes `random` (the server picks), `topology` and `orientation` take `natural` (the
client's own choice). Any area in `WorldAreas` works, not only act 1. Topology numbers are
1-based in the area's topology list; orientations are checked against that topology's allowed
ones.

## Instances

`poe_cluster::InstanceDirectory` (in-process `MemoryInstances`) holds every instance: area,
seed, forced topology and orientation, the players in it and since when it is empty.

- Entering from character selection: towns and hideouts share one kept instance; other
  areas join the area's newest open instance, else get a new one.
- The first player decides an instance's seed (spec seed, else saved location, known seeds,
  random); later players get the same one.
- Empty instances close after `[instances] idle_minutes` (towns and hideouts stay).
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
server does not send login `0x0017` after the hand-off (`[login] close_after_handoff = false`;
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
