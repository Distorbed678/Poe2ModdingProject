# Multiplayer: what is in place and what is missing

Analysis of 2026-10-17. Goal: several people on different machines play on one server and
see each other, as on official servers.

## Short answer

Chat, the cluster plumbing (tickets, login sessions, transfers, instance directory) and
per-account storage already work for many accounts. Three things stop real multiplayer:

1. **A remote client cannot log in at all.** The DLL treats every non-loopback server as
   official. It refuses the connection (`block_external`), or, if allowed, turns on the
   read-only latch, which disables the crypto bypass.
2. **Several parts of the server only work when the client runs on the same machine.** These
   are the spawn point, the walkability grid, position sync and the debug link.
3. **There is no shared world.** Each connection is a private simulation with the player as
   entity 1. Two players in the "same" instance share only chat and the player count; they
   cannot see each other.

## 1. Connecting from another machine

| What | Where | Problem | Fix |
|---|---|---|---|
| External connections refused | `client/poe-hook/src/guard.rs` `on_connect` | Private mode blocks every non-127.x address, so a friend can't reach your IP. | A `trusted_servers` list (IP:port) in `poe-hook.toml`, written by deploy. Only listed addresses count as "ours". Everything else stays blocked or latches read-only as today, so the official read-only rule is unchanged. |
| Read-only latch disables the crypto bypass | `crypto_bypass.rs:69`, `guard.rs` | Any external connection latches read-only, so login to a remote custom server fails. | Same list: a trusted address does not latch. |
| "Custom server" detection | `ui/mod.rs` `custom_server()`, `abi.rs` `Connection.external` | The overlay hides server features on any external connection. | Use the trusted list instead of the loopback test. |
| Advertised addresses | `config/server.toml` `[login] public_host`, `[instance] public_host` = `127.0.0.1` | The gateway entry and InstanceDetails send loopback, so a remote client connects to itself. | Set them to the LAN/WAN address. Optionally, a per-client address (LAN vs internet) based on where the login came from. |
| Client setup for friends | `cargo xtask deploy-client` | It reads this repo's `server.toml` and needs our GameFiles copy. | `cargo xtask client-pack --host <ip>`: a zip with `version.dll`, the core, `poe-hook.toml` (trusted list, host) and `gateway_list.txt`, and no game files. Each player uses their own legal install; it works on Windows natively and under Proton. |
| Ports | — | TCP 20481 (login), TCP 6112 (instance), UDP 6112 (reports/debug link). | Document port forwarding. Bind is already `0.0.0.0`. |
| Encryption | `login-server/src/handshake.rs` | With the bypass, keys come from `SHA-512(server public value)`, which is sent in clear. Anyone on the path can decrypt the session. This is fine on a LAN but not secure over the internet. | Later: the DLL swaps the client's baked-in DSA public key for ours and keeps verification on, so the server does real DH and signs it. |

## 2. Same-machine dependencies

| Dependency | Where | Why it exists | Multiplayer fix |
|---|---|---|---|
| **Spawn position** from `dumps/0.5.5e/terrain/layouts/terrain_layout_<hash>.tsv` | `instance-server/src/spawn.rs` (`[spawn] layout_dir`), `lib.rs:240` | The server can't generate terrain yet (M5), so the DLL exports the client's layout to disk and the server reads it. | Short term: the DLL sends the layout over the network (one upload per tile hash, cached on the server), so whoever generates an area first supplies it for everyone. Long term: server-side generation (`[terrain] generate`, terrain-status.md). |
| **Walkability grid** `dumps/0.5.5e/terrain/grids/terrain_grid_<hash>.bin` | `spawn.rs` `client_grid`, `player.walkable` | Same reason; collision and pathing use the client's grid. | Same as above. |
| **Position sync** (UDP `PSY2`) | `client/poe-hook-core/src/position_sync.rs`, `instance-server/src/sync.rs` | Server movement only approximates the client's, so the client reports where it really is. The DLL sends only to a loopback connection, and the server accepts only loopback senders. Reports are keyed by the client's TCP port only. | Allow trusted remote senders. Key by (IP, port) or, better, by a session token (below). This matters more with others watching, because positions are broadcast. Long term: exact server movement and collision (M5), and drop the bridge. |
| **Debug link** (`PDC1`/`PDB1`: overlay commands, Instances page, registry) | `sync.rs` `listen`, `devlink.rs` | Loopback only. Identity is "which TCP port says it's mine", so any local process can claim any session's port. | Authenticate datagrams with a per-session token: HMAC over the datagram with a key from the instance ticket keys, which both sides know (the DLL already sees them through the crypto bypass). Then remote admins can use the overlay. Chat `!` commands already work remotely. |
| Dev globals | `[instance] movement_trace`, `sweep`, `known_seeds`, `POE_NPC`, `POE_FORCE` env | One file or setting for the whole server. | Fine for dev. Tag trace lines with the account, and keep these off on a shared server. |

## 3. Account and identity

| What | Where | Problem | Fix |
|---|---|---|---|
| Account = machine hash | `login-server/src/session.rs:247` (`auth.machine_hash`) | Two people on one PC share an account. The hash is sent by the client, so anyone can claim anyone's account. Account names are `Exile<id>`, not chosen. | Real login: the DLL (or a small launcher) supplies a username and password or token. Options: (a) does this exe still have the standalone email/password login path? (Ghidra, open question); (b) the DLL puts a token from `poe-hook.toml` into the auth packet's unused bytes; (c) the Steam ticket. GGG's key encrypts it, so we can't verify it; at best it identifies the account and doesn't authenticate it. |
| Admins | `[admin] accounts = ["Exile1"]` | Tied to auto-generated names. That's fine for now, but the names change if the DB is reset. | Name accounts at registration. Admin list by name or by an `admin` column in the DB. |
| Same account twice | `poe-cluster` `MemorySessions` | A second login replaces the first's transfer channel; nothing kicks the old session. | Official behaviour: the new login kicks the old one. Do the same. |
| Character names | `poe-store` | Already unique (case-insensitive) across accounts. | — |
| Account settings | `[accounts] flags`, `full_access`, `stash_tabs` | Global for every account. | Per-account columns (stash tabs come with the stash milestone). |

## 4. Shared world (the main work)

**Today:**
- `run_session` (`instance-server/src/lib.rs`) owns one `Player` with `PLAYER_ENTITY` = 1.
- It runs its own frame clock (`SimulationTick` every `tick_ms`) and sends only that player's AddObject.
- An "instance" in `MemoryInstances` is metadata: area, seed, layout and player count.

**Needed:**
- **Instance actor:** one task per instance owning the world. That means entities with unique IDs, one frame clock and the tick loop. Sessions forward client input to it and receive per-client output. The current per-session `Player` logic (`player.rs`) moves into the actor as one entity among many.
- **Entity IDs per instance:** `0x0024 PlayerEntity` already tells each client which entity is its own (`u8 local player index, u32 entity id`), so the protocol supports this. InitialState's PlayerState is matched by player id, which still needs checking for several players.
- **Remote players:**
  - Send an AddObject for other players' characters (class object type plus a blob as seen by others) when they enter, or come into range.
  - Remove them when they leave.
  - Forward their actions (StartAction, movement updates, aim, skill use) with their entity ID.
  - We don't know yet how the official server encodes a *remote* player (which components, and whether the Player component carries less). **This needs an official capture with other players nearby** (town is enough, read-only as always).
- **Visibility / interest management:** which entities each client gets (whole instance for small areas, range-based later). Official servers send removes when you move away.
- **Lockstep:** the client runs in lockstep with the server's frames. With several clients, all must follow the instance's frame clock. Each client's inputs arrive on that clock and are echoed to the others in the right frame. Our movement replies are already frame-based; the open question is how the client treats other entities' actions arriving late.
- **Movement authority:** with the position-sync bridge, the server trusts each client's reported position. Others see whatever that client reports. That's acceptable for friends, not for anti-cheat. Exact server-side movement needs terrain (M5).
- **Shared instances by default:** `play_character` finds or creates per area. Towns and hideouts are persistent, and areas are already found by spec. Official behaviour is per-party instances for zones and shared towns, so that needs parties.

## 5. Already in place

- Login and instance servers bind `0.0.0.0` and hold many concurrent connections (one tokio task each).
- Accounts, characters and per-account character lists in SQLite. Character names are globally unique.
- Tickets with random keys per hand-off. Resume tickets. Login sessions per account (logout, transfers).
- Instance directory with find/create/join/leave/close/idle expiry and persistent towns.
- Live registry of online players (`live.rs`): who, kick, tell, broadcast, mute.
- Chat: global and trade by channel number, local by instance, whispers, `!` commands with admins, audit log.
- Transfers between instances (character-select hop).

## Suggested order

1. **Remote connect:** DLL trusted-server list, advertised host config, `client-pack`, port docs. With that, a friend can log in and play alone in a shared instance, and chat works.
2. **Layout upload over the network:** spawn and walkability for remote clients. Accept position reports from trusted remote peers, keyed by (IP, port).
3. **Capture (user, official, read-only):** a town with other players moving around, plus one joining and leaving, to get the remote-player AddObject and the action broadcasts.
4. **Instance actor and entity IDs:** refactor `run_session` into session ↔ instance-actor messages, with unique entity IDs and one clock.
5. **Remote player add/remove and action relay.** At this point two players see each other moving.
6. **Accounts:** real login names and tokens, duplicate-login kick, admin by name.
7. **Debug-link tokens:** remote admin overlay, and closing the port-spoofing hole.
8. **Later:** parties (instance ownership), trade, real crypto, server-side movement (M5).

## Open questions

- Does `PathOfExileSteam.exe` keep a usable email/password login (standalone client path)?
- What does a remote player's AddObject blob contain compared with the local one?
- How does the client handle InitialState and PlayerState with several players?
