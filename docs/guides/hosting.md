# Hosting: letting other players connect

Analysis of 2026-10-17. Goal: several people on different machines play on one server and see
each other, as on official servers. Nothing here is built yet unless it says so; today only a
client on the same machine as the server works. The two other halves of the plan are in
[../architecture/accounts.md](../architecture/accounts.md) (who is logging in) and
[../architecture/instance-world.md](../architecture/instance-world.md) (the shared world).

## Short answer

Chat, the cluster plumbing (tickets, login sessions, transfers, instance directory) and
per-account storage already work for many accounts. Three things stop real multiplayer:

1. **A remote client cannot log in at all.** The DLL treats every non-loopback server as
   official. It refuses the connection (`block_external`), or, if allowed, turns on the
   read-only latch, which disables the crypto bypass.
2. **Several parts of the server only work when the client runs on the same machine.** These
   are the spawn point, the walkability grid, position sync and the debug link.
3. **There is no shared world.** Each connection is a private simulation with the player as
   entity 1 (see [instance-world.md](../architecture/instance-world.md)).

Already in place: the login and instance servers bind `0.0.0.0` and hold many concurrent
connections (one tokio task each).

## Connecting from another machine

| What | Where | Problem | Fix |
|---|---|---|---|
| External connections refused | `modules/client-hook/poe-hook/src/guard.rs` `on_connect` | Private mode blocks every non-127.x address, so a friend can't reach your IP. | A `trusted_servers` list (IP:port) in `poe-hook.toml`, written by deploy. Only listed addresses count as "ours". Everything else stays blocked or latches read-only as today, so the official read-only rule is unchanged. |
| Read-only latch disables the crypto bypass | `crypto_bypass.rs:69`, `guard.rs` | Any external connection latches read-only, so login to a remote custom server fails. | Same list: a trusted address does not latch. |
| "Custom server" detection | `ui/mod.rs` `custom_server()`, `abi.rs` `Connection.external` | The overlay hides server features on any external connection. | Use the trusted list instead of the loopback test. |
| Advertised addresses | `config/server.toml` `[network] public_host` = `127.0.0.1` | The gateway entry and InstanceDetails send loopback, so a remote client connects to itself. | Set them to the LAN/WAN address. Optionally, a per-client address (LAN vs internet) based on where the login came from. |
| Client setup for friends | `cargo xtask deploy-client` ([setup.md](setup.md)) | It reads this repo's `server.toml` and needs our GameFiles copy. | `cargo xtask client-pack --host <ip>`: a zip with `version.dll`, the core, `poe-hook.toml` (trusted list, host) and `gateway_list.txt`, and no game files. Each player uses their own legal install; it works on Windows natively and under Proton. |
| Ports | `config/server.toml` | TCP 20481 (login), TCP 6112 (instance), UDP 6112 (reports/debug link). | Document port forwarding. Bind is already `0.0.0.0`. |
| Encryption | `login-server/src/handshake.rs` | With the bypass, keys come from `SHA-512(server public value)`, which is sent in clear. Anyone on the path can decrypt the session. This is fine on a LAN but not secure over the internet. | Later: the DLL swaps the client's baked-in DSA public key for ours and keeps verification on, so the server does real DH and signs it ([../re/crypto.md](../re/crypto.md)). |

## Same-machine dependencies

| Dependency | Where | Why it exists | Multiplayer fix |
|---|---|---|---|
| **Spawn position** from `dumps/0.5.5e/terrain/layouts/terrain_layout_<hash>.tsv` | `instance-server/src/spawn.rs` (`[spawn] layout_dir`), `lib.rs:240` | The server can't generate terrain for every area yet, so the DLL exports the client's layout to disk and the server reads it. | Short term: the DLL sends the layout over the network (one upload per tile hash, cached on the server), so whoever generates an area first supplies it for everyone. Long term: server-side generation (`[terrain] generate`, [../status/terrain.md](../status/terrain.md)). |
| **Walkability grid** `dumps/0.5.5e/terrain/grids/terrain_grid_<hash>.bin` | `spawn.rs` `client_grid`, `player.walkable` | Same reason; the grid places a restored position (the server has no collision of its own). | Same as above. |
| **Position sync** (UDP `PSY2`) | `modules/client-hook/poe-hook-core/src/position_sync.rs`, `instance-server/src/sync.rs` | Server movement only approximates the client's, so the client reports where it really is. The DLL sends only to a loopback connection, and the server accepts only loopback senders. Reports are keyed by the client's TCP port only. | Allow trusted remote senders. Key by (IP, port) or, better, by a session token (below). This matters more with others watching, because positions are broadcast. Long term: exact server movement and collision, and drop the bridge ([../protocol/movement.md](../protocol/movement.md)). |
| **Debug link** (`PDC1`/`PDB1`: overlay commands, Instances page, registry) | `sync.rs` `listen`, `devlink.rs` | Loopback only. Identity is "which TCP port says it's mine", so any local process can claim any session's port. | Authenticate datagrams with a per-session token: HMAC over the datagram with a key from the instance ticket keys, which both sides know (the DLL already sees them through the crypto bypass). Then remote admins can use the overlay. Chat `!` commands already work remotely ([server-admin.md](server-admin.md)). |
| Dev globals | `[dev] movement_trace`, `sweep`, `known_seeds`, `POE_NPC`, `POE_FORCE` env | One file or setting for the whole server. | Fine for dev. Tag trace lines with the account, and keep these off on a shared server. |

## Suggested order

1. **Remote connect:** DLL trusted-server list, advertised host config, `client-pack`, port docs. With that, a friend can log in and play alone in a shared instance, and chat works.
2. **Layout upload over the network:** spawn and walkability for remote clients. Accept position reports from trusted remote peers, keyed by (IP, port).
3. **Capture (user, official, read-only):** a town with other players moving around, plus one joining and leaving, to get the remote-player AddObject and the action broadcasts.
4. **Instance actor and entity IDs:** refactor `run_session` into session ↔ instance-actor messages, with unique entity IDs and one clock ([instance-world.md](../architecture/instance-world.md)).
5. **Remote player add/remove and action relay.** At this point two players see each other moving.
6. **Accounts:** real login names and tokens, duplicate-login kick, admin by name ([accounts.md](../architecture/accounts.md)).
7. **Debug-link tokens:** remote admin overlay, and closing the port-spoofing hole.
8. **Later:** parties (instance ownership), trade, real crypto, server-side movement.
