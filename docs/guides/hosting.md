# Hosting: letting other players connect

How to run the server so people on other machines can play on it, and what each of them needs. Status: a remote client can log in with an account and password (or a launcher token), needs no shared disk (terrain upload), its side channels are authenticated, and all players in an instance share one world and see each other (verified with two real clients on a LAN, [below](#local-two-client-test)). What is still rough is listed under [Not done yet](#not-done-yet). The related designs: [../architecture/accounts.md](../architecture/accounts.md) (who is logging in), [../architecture/instance-world.md](../architecture/instance-world.md) (the shared world), [../protocol/side-channel.md](../protocol/side-channel.md) (the DLL's side channel).

## Ports

| What | Default | Protocol | Who connects | Forward it? |
|---|---|---|---|---|
| Login server | `[network.login] bind` = `0.0.0.0:20481` | TCP | every client (gateway entry), also the in-game transfers | yes |
| Instance server | `[network.instance] bind` = `0.0.0.0:6112` | TCP | every client (game connection, handed over by the login server) and the DLL's terrain upload | yes |
| Side channel | same port number as the instance bind | **UDP** | the client DLL: position reports and the debug link (overlay, commands) | yes, UDP as well as TCP |
| Web API and web admin | `[web] bind` = `127.0.0.1:8080` | TCP (HTTP) | admins, the launcher's account pages | no, unless behind TLS ([below](#web-api)) |

There is nothing else: no extra port for the terrain upload (it shares the instance TCP port), and the web API is optional. The servers bind `0.0.0.0` and take many concurrent connections.

## Server setup

Edit `config/server.toml` (or copy it and start with `POE_CONFIG=<file> cargo run -p poe-server`):

```toml
[network]
# The address players type: your LAN address for a LAN party, your public IP or DNS name over the internet.
public_host = "203.0.113.7"

[network.login]
bind = "0.0.0.0:20481"
# public_port = 30481      # only when the forwarded port differs from the bind port

[network.instance]
bind = "0.0.0.0:6112"
public_port = 6112         # the port the login server tells clients to connect to
```

- `public_host` is used in three places, all derived from this one value: the gateway entry that `cargo xtask deploy-client` writes (`gateway_list.txt`), the host in `InstanceDetails` (login server to client), and the DLL's `trusted_servers` list (below). Never leave it at `127.0.0.1` for remote players; the server logs a warning at startup if it listens beyond loopback but advertises a loopback address.
- Behind port forwarding the ports the client sees can differ from the bind ports: `[network.instance] public_port` (also where the DLL sends its side channel) and `[network.login] public_port` (optional, defaults to the bind port). Forward the same external port to the internal one for both TCP and UDP of the instance port.
- Names work for `public_host`: the client resolves it, and `deploy-client` resolves it to IPv4 when it writes `trusted_servers` (the DLL only reads IP literals). Over the internet prefer a fixed address, or redeploy when it changes.
- A player on the same LAN as the server connecting through the public address needs NAT loopback (hairpin) on the router, or the LAN address in their own deploy (`public_host` is read from the `server.toml` they deploy with).

### Terrain (`[spawn]`, `[spawn.upload]`)
The server cannot generate terrain for every area yet, so it needs the layout and cell grid the client generated. A remote client sends them itself: after generating an area its DLL offers the tile hash over the instance TCP port, and uploads (zlib-compressed, about a quarter of a megabyte for the Riverbank) if the server does not have it. The server validates the upload and caches it per tile hash in `[spawn] layout_dir` (default `dumps/0.5.5e/terrain/{layouts,grids}`); the first client to generate an area supplies it for everyone, later clients are told the server has it. Local files in the same directory keep working (a client on the server machine writes them directly).

- `[spawn.upload] enabled = true`; `max_bytes`, `max_layout_bytes`, `max_grid_bytes` cap an upload as sent and after decompression; `wait_ms` is how long a spawn waits for a layout nobody announced, `wait_upload_secs` how long once an upload was announced (raise it for slow uplinks).
- Without a layout the player spawns at the origin, which is outside the walkable area on most maps. If a remote player spawns wrongly, look for `terrain upload cached` and `spawn from client layout` lines in the server log.
- Uploads need a trusted connection and the DLL; the player must not be on an official server. See [Security notes](#security-notes) for what the upload does and does not prove.

### Firewall
Open the ports of the table (inbound), for TCP and UDP of the instance port:

```
# ufw
ufw allow 20481/tcp
ufw allow 6112/tcp
ufw allow 6112/udp
# firewalld
firewall-cmd --permanent --add-port=20481/tcp --add-port=6112/tcp --add-port=6112/udp && firewall-cmd --reload
# Windows
New-NetFirewallRule -DisplayName "PoE2 server" -Direction Inbound -Protocol TCP -LocalPort 20481,6112 -Action Allow
New-NetFirewallRule -DisplayName "PoE2 server UDP" -Direction Inbound -Protocol UDP -LocalPort 6112 -Action Allow
```

On a router, forward the same three (TCP 20481, TCP 6112, UDP 6112) to the server machine. Cloud firewalls and security groups need the same three rules. Blocked UDP does not stop play: only the position reports (used by the drift checker) and the overlay's debug link stop working.

Check from outside: `nc -vz <host> 20481`, `nc -vz <host> 6112`; the UDP port answers nothing by design (datagrams without a valid tag are dropped silently). The server log shows `client position reports started` and `side-channel session` lines when a client gets through.

### Accounts
Create the players' accounts on the server: the offline `account create <name> <password>` ([setup.md](setup.md#accounts-and-characters-offline), other account commands: [server-admin.md](server-admin.md#accounts)), or turn on `[accounts] open_registration` and use the web API's register endpoint. Keep `[accounts] dev_machine_hash_login = false` on a shared server: that Steam-login shortcut creates an account for anyone without a password.

## What a remote player needs

1. **Their own legal install** of Path of Exile 2 0.5.5e (the files are not distributed). They also need Steam running when they start the client (without it the client exits at `Unable to init Steam API`).
2. **The client hook** (`version.dll`, `poe_hook.dll`, `poe_hook_core.dll`, `poe-hook.toml`) in the game folder, with a gateway entry and a trust list that point at your server. Either:
   - **the launcher** (`modules/launcher`, [launcher.md](launcher.md)): installs 0.5.5e from Steam depots, deploys the hook and takes the server's web API address (so the web API must be reachable for it, behind TLS outside a trusted network); or
   - **`cargo xtask deploy-client`** from a checkout of this repository, with a `config/server.toml` that has your `public_host` and ports (copy yours, or edit just `[network]`), and the client files in `GameFiles/`: `cargo xtask build-dll` (once), then `cargo xtask deploy-client --no-build --login-mode standalone`. It writes the gateway entry `public_host:login port`, `login_mode`, `position_sync_port` and `trusted_servers = ["<public_host>:<login port>", "<public_host>:<instance port>"]`. Add more servers by hand in `GameFiles/poe-hook.toml`, e.g. `"203.0.113.7:*"`.
3. **Start the client** (`cargo xtask run-client`, or Steam with the deployed folder) and log in with the account name and password ([setup.md](setup.md#remember-me-token-launcher-auto-login)).

What the trusted entries change in the DLL: the connections are not blocked, the read-only latch stays off, the login crypto bypass runs, and the side channel (position reports, debug link, terrain upload) goes to them. Official servers, the shop and telemetry stay refused or read-only exactly as before, and no config key can trust anything in official mode ([../architecture/client-dll.md](../architecture/client-dll.md#login-mode-and-trusted-servers)). A client deployed with `--official` never sends anything to any server of ours.

The player does not need the server's `dumps/`, `GameFiles/Bundles2` copy or any shared folder.

## Security notes

- **Side channel.** Position reports, the overlay's debug link and the terrain upload are authenticated per session: the DLL and the server derive an HMAC-SHA256 key from the instance session's ticket keys (the 64 bytes the login server gives only to that client), every frame carries the session id, a counter and a 16-byte tag, and the server drops datagrams without a valid tag, from an unknown session, or with a replayed counter. Another player, or anyone who can send UDP to the server, cannot move a session's reports, inject overlay commands (which can be admin commands) or redirect its debug state. Details: [../protocol/side-channel.md](../protocol/side-channel.md). There is no unauthenticated fallback.
- **What it does not protect.** The frames are authenticated, not encrypted: someone on the network path can read positions and the debug state, though not forge them. The game stream itself is encrypted, but with the crypto bypass its keys derive from a value the server sends in clear ([../re/crypto.md](../re/crypto.md)), so on the open internet someone on the path can decrypt a session, and with the instance key material, its side channel keys. It is fine on a LAN or for friends over a VPN; for the open internet use a VPN (WireGuard or similar) until the real key exchange is restored.
- **The terrain upload** is accepted only from a session that redeemed a ticket, for the tile hash that session reported, and is size-limited, decompression-limited and format-checked. It cannot overwrite an existing cache entry. A player who is allowed to play can still upload a wrong layout for their own area (the hash cannot be recomputed from the export), so a wrong cache entry is a server-side file to delete: `dumps/0.5.5e/terrain/layouts/terrain_layout_<hash>.tsv` and the grid beside it.
- **Web API.** `[web] bind` defaults to loopback because passwords travel in request bodies and the admin endpoints act on the whole server. To use it from other machines put a TLS reverse proxy (Caddy, nginx) in front, keep the bind on loopback, forward `/api/` and the WebSocket upgrade, and set `[web] cors_origins` only for the origins that need it. Do not forward port 8080 to the internet without TLS. `[accounts] open_registration` stays off unless you want strangers to create accounts.
- **Admins.** The overlay's server commands and `!` chat commands need the admin role on the account (`account role <name> admin`); nobody is an admin by default except the dev machine-hash account `Exile1`.
- **Dev settings.** `[dev] known_seeds`, `sweep`, `movement_trace` and the `POE_*` environment variables are one-per-server development settings; keep them off on a shared server. `[dev] drift_check` is harmless.

## Local two-client test
`scripts/test-mp.sh` runs two real game clients against one server on one machine (Steam must be running). Both connect through the machine's LAN IP, not 127.0.0.1, so the trusted-remote path of the DLL runs (the deploy puts the LAN IP into `trusted_servers`), and each logs in as its own account (`mp1`, `mp2`) on the standalone login with a remember-me token (web API launcher login, then `POST /game-token`).

Commands (`setup`, `server`, `clients`, `keys`, `shot`, `stop`): [setup.md](setup.md#two-client-test-lan).

- **Setup details.** The second client uses its own Proton prefix and its own input file, window title ("PoE2 MP <n>") and session folder; the windows default to 1280x720 so both fit on screen. All of it and the environment variables: [setup.md](setup.md#two-client-test-lan).
- **Characters.** One Ranger per account in `G1_town`, made offline. Both start on the town's start cell; since WP C2 the second one is put on the nearest free walkable cell at least `[spawn] player_spacing` (8) cells away, so they no longer stand on each other (the first run of the test, before WP C2, had both on (379, 356)).

Results (2026-10-21, client 0.5.5e, two clients, one `G1_town` instance, evidence from F8 screenshots, server log and each client's `packets.jsonl`):

| Item | Result | Evidence |
|---|---|---|
| Token login of both accounts through the LAN IP (trusted remote) | pass | `authenticated ... token_login=true`; DLL log `trusted servers: loopback, <LAN>:20481, <LAN>:6112`, no latch |
| Both in one instance, see each other | pass (the remote tint is gone since WP C2, see the re-check below) | entities 12000 and 12001, `players=2`; each client received two `AddObject`s; in the F8 shots each client draws the other character (before WP C2 the remote one looked darker and orange) |
| Walk and roll of one reach the other | pass | client 2's `packets.jsonl` has `0x0064` (header `(12000, 12000, 9, 0)`), `0x02e7`, `0x01c8`, `0x01ae`, `0x01b1` with client 1's entity; the remote character is drawn at the walked position. Roll animation of the remote: see the WP C2 re-check below |
| Chat | pass | `text:` + Enter in client 2: `chat from=Mp2Ranger`, the line "Mp2Ranger: hello-from-two" is on client 1's screen |
| Transfer gives the leave sequence | pass | `POST /admin/players/Mp2Ranger/transfer {"area":"G1_1"}`: client 1 got `0x0271`, `0x01c5`, `0x01c7`, `0x02fa` for entity 12001 in one frame; Mp2Ranger entered G1_1 (alone, pause request applied, then released) |
| Duplicate login kicks the first session | pass | client 2 closed, then logged in as `mp1`: `account logged in again ...: disconnecting the player`, `player left the instance ... Mp1Ranger`, the new session enters as entity 12002 at the saved cell |

**Town freeze (fixed 2026-10-21).** The first runs of this test showed WASD, clicks and a roll doing nothing in town while Escape worked. The cause was not the window mode: our `InitialState` always had `flags = 7`, whose bits 0/1 start the client's world paused (`+0x868/+0x869`, entities update with `dt = 0`; the town looked frozen). In a zone the client then sends `PauseRequest(1)` and `PauseRequest(0)` itself and the server releases the pause; **in a town it sends none** (official town capture: `InitialState` flags `04`, no `PauseRequest`), so the pause was never released. The server now sends flags `04` for towns and hideouts. The earlier single-client smoke test of the instance actor had run in a zone.

**WP C2 re-check (run 2026-10-21, commit 80e7de6).** Changes since the table above: spawn spreading, the remote blob's Life section, Player flags, derived stats and weapon set stat sets, owner-only stats left out of relayed batches ([../protocol/entities.md](../protocol/entities.md#remote-player-look-wp-c2)). Plan and result, with `scripts/test-mp.sh` as in the table (`setup`, `server`, `clients`; Mp2Ranger had been moved to `G1_town` in `runtime/server-mp.sqlite` first, the earlier transfer test had left it in `G1_1`):

| Step | Result | Evidence |
|---|---|---|
| 1. Both in `G1_town`, spawn spread | pass | server log: `spawn cell taken: standing on a free cell nearby player=Mp2Ranger wanted=(379, 356) cell=(379, 348)`, then `player joined ... entity=12001 players=2`; Mp1Ranger (entity 12000) stands on (379, 356), the cells are exactly 8 apart (`player_spacing`). F8 shots of both clients show two characters side by side (`runtime/shots/wpc2/c1-two-chars.png`, `c2-two-chars.png`, git-ignored copies) |
| 2. Remote look | pass: no remote tint | the same character looks the same as local and as remote: Mp2 is the lower, firelit (orange) figure in both clients' shots (local in client 2, remote in client 1), Mp1 the paler upper one in both. Mean RGB of the white top (10x10 px): Mp1 local 108/80/71, remote 113/81/73; Mp2 local 191/163/149, remote 174/140/124 (about 10 % darker, a little redder, on one of the two; the cameras differ). The orange of the lower figure comes from the town's fire, the local one has it too. Nothing like the dark, orange model of the first run |
| 3. Bisect (`[dev] remote_blob_omit`, `remote_animated_path`) | not needed | the condition (tint remains) did not hold, so none of the six runs was made and the defaults stay. The switches remain for a later look; the one measurable residual (Mp2 remote about 10 % darker than Mp2 local) is within the view-to-view lighting difference and was not chased. Equipment visuals are still missing on remote players |
| 4. Roll | pass | see below |
| 5. `stop` | pass | clients killed, server stopped, `deploy-client --login-mode steam` put `GameFiles/poe-hook.toml` back to `mode = "private"`, `login_mode = "steam"`, loopback trust, the `poe-hook.*.keys` files are gone |

Roll check, client 2's `packets.jsonl` for client 1's entity 12000, repeated 5 times (standing rolls, one with `D` held, one steered with the cursor 700 px to the right of the character). Every roll arrives in this order, in one frame: `SprintState` (1), `StartAction` (flags `0x410`), `ActionEvent`, `StatsUpdate` with one stat (`is_dodge_rolling` = 1, key 19305), `UseSkillAck`, `StatsUpdate` with **two** stats (`virtual_heavy_stun_threshold_+%_final` 17555 and `virtual_light_stun_threshold_+%_final` 19070, both 100; `stun_threshold_+%_final_while_performing_action` is not there), later `SprintState` (0), `SkillInputEndAck` and the reset `StatsUpdate` (the same three keys back to 0, `is_dodge_rolling` last). That is the official remote order ([entities.md](../protocol/entities.md#remote-roll-check-wp-c2)). The steered roll also relays `UpdateSkillTargetAck` while the cursor target moves.

The remote roll animates. In the shots of client 2 a few frames apart (about 240 ms, `runtime/shots/wpc2/c2-remote-roll-mid.png`, `c2-remote-roll-mid2.png`) the remote character of the steered roll is first standing, then tumbling in the roll pose far from where it stood, then up again at the new place. Standing rolls without a steering cursor do not move the character, and in the frames of both the remote and the local character there is no visible roll pose, so for animation checks use a roll with the cursor away from the character (`inmove:<x>,<y>`, then `indown:0x20`, `0x20`, `wait:250`, `inup:0x20`). The distance of the roll was not checked against the official value (movement work, WP C1).

Test tooling notes from this run: while client 2 takes shots back to back, client 1 processes its keys file late or not at all until the shots end (the rolls then all came at the end of the shot series, the roll and its reset in one frame). A pattern that worked: write client 2's keys (`wait:200`, then `0x77 wait:80` repeated), wait 0.4 s, then write client 1's keys. A 12-shot series then straddles the roll (the roll started 3 ms after the first shot). `GameFiles/logs/poe-hook/<session>/packets.jsonl` times (`t`, ms since the epoch) can be compared with the shot files' modification times.

## Web API
`[web]` is optional ([../architecture/web-api.md](../architecture/web-api.md)). The game itself never needs it, but the launcher does (account login and register, `GET /server`, `POST /game-token`), so a server for launcher users has to expose it through a TLS proxy as described in the security notes. It listens on `127.0.0.1:8080` by default.

## Not done yet

- **Remote players, rough edges.** Players in one instance share one world (instance actor, WP3.6) and see each other ([Local two-client test](#local-two-client-test)). Open: no equipment on remote players (the tint is gone, [re-check](#local-two-client-test)), and no party, trade or combat sync between players yet. Players no longer spawn on one cell (`[spawn] player_spacing`).
- **Real login crypto.** The bypass keys derive from a public value; fine on a LAN, not over the open internet ([../re/crypto.md](../re/crypto.md)). A later step: the DLL swaps the client's baked-in DSA public key for ours and keeps verification on.
- **Bundled hook DLLs.** The launcher still needs the hook DLLs from a local build of the client hook (its settings point at them); a ready package of the hook with a `poe-hook.toml` for friends does not exist yet ([launcher.md](launcher.md#not-done)).
- **Per-client address.** One `public_host` serves everyone; LAN players and internet players of the same server need NAT loopback or their own `server.toml` for deploy.
