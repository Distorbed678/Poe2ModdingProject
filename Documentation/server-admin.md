# Server commands, admins and instances

## Commands

One registry (`servers/instance-server/src/commands.rs`) defines every server command: name,
group, whether it needs an admin, typed arguments and help. The same commands work in two
places:

- **Chat**: `!name args` (prefix `[commands] prefix`; `/` stays with the client's own
  commands). Replies come back as chat lines from `<Server>`. Needs the client's chat message
  (`0x0016`) decoded; see [protocol/instance.md](protocol/instance.md).
- **Overlay**: the DLL's Commands tab builds a form per command from the registry, which the
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

A transfer is a real hand-off, as on official servers: the instance server issues a ticket for
the target instance and asks the account's login connection (`LoginSessions::transfer`) to send
`InstanceDetails`; the client then connects to that instance and loads its area. (The client
keeps its login connection open while in game, which is where official hand-offs arrive.) A
second AreaInfo on one connection would crash the client, so transfers always reconnect.

Forced layouts: an instance with a topology or orientation sends them in AreaInfo's first
stat record (flags0 bit 7; `map_force_topology` = 1-based topology, `map_force_orientation` =
orientation + 1), and the server-side terrain check uses the same choice. Not verified in game
yet (which of the two records the generator reads is a guess: the first).

## Overlay tabs (our server only)

Hidden while the read-only latch is on (official servers), like the Server and Desync tabs.

- **Commands**: every registry command as a form, grouped; results below.
- **Instances**: the server's instances (id, area, seed, layout, players, time until an empty
  one closes) with Join and Close, and a new-instance form: area picker (search, kind and act
  filters, recent picks; every area from `poe-hook-data/areas.tsv`, written by
  `cargo xtask dll-data` or `deploy-client`), seed (random or server-picked), topology and
  orientation (natural or forced), with "Transfer there" and "Create without transferring".
