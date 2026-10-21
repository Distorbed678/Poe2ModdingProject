# Hosting: letting other players connect

How to run the server so people on other machines can play on it, and what each of them needs. Status after WP3.4: a remote client can log in (WP3.3), needs no shared disk (terrain upload) and its side channels are authenticated. What is still missing is the shared world: every connection is its own simulation, players only share chat and the instance list (see [Not done yet](#not-done-yet)). The related designs: [../architecture/accounts.md](../architecture/accounts.md) (who is logging in), [../architecture/instance-world.md](../architecture/instance-world.md) (the shared world), [../protocol/side-channel.md](../protocol/side-channel.md) (the DLL's side channel).

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
Create the players' accounts on the server: `cargo run -p poe-server -- account create <name> <password>` ([server-admin.md](server-admin.md)), or turn on `[accounts] open_registration` and use the web API's register endpoint. Keep `[accounts] dev_machine_hash_login = false` on a shared server: that Steam-login shortcut creates an account for anyone without a password.

## What a remote player needs

1. **Their own legal install** of Path of Exile 2 0.5.5e (the files are not distributed). They also need Steam running when they start the client (without it the client exits at `Unable to init Steam API`).
2. **The client hook** (`version.dll`, `poe_hook.dll`, `poe_hook_core.dll`, `poe-hook.toml`) in the game folder, with a gateway entry and a trust list that point at your server. Either:
   - **the launcher** (`modules/launcher`, WP5): installs 0.5.5e from Steam depots, deploys the hook and asks for the server address; or
   - **`cargo xtask deploy-client`** from a checkout of this repository, with a `config/server.toml` that has your `public_host` and ports (copy yours, or edit just `[network]`), and the client files in `GameFiles/`: `cargo xtask build-dll` (once), then `cargo xtask deploy-client --no-build --login-mode standalone`. It writes the gateway entry `public_host:login port`, `login_mode`, `position_sync_port` and `trusted_servers = ["<public_host>:<login port>", "<public_host>:<instance port>"]`. Add more servers by hand in `GameFiles/poe-hook.toml`, e.g. `"203.0.113.7:*"`.
3. **Start the client** (`cargo xtask run-client`, or Steam with the deployed folder) and log in with the account name and password ([setup.md](setup.md#remember-me-token-launcher-auto-login)).

What the trusted entries change in the DLL: the connections are not blocked, the read-only latch stays off, the login crypto bypass runs, and the side channel (position reports, debug link, terrain upload) goes to them. Official servers, the shop and telemetry stay refused or read-only exactly as before, and no config key can trust anything in official mode ([../architecture/client-dll.md](../architecture/client-dll.md#login-mode-and-trusted-servers)). A client deployed with `--official` never sends anything to any server of ours.

The player does not need the server's `dumps/`, `GameFiles/Bundles2` copy or any shared folder.

## Security notes

- **Side channel (WP3.4).** Position reports, the overlay's debug link and the terrain upload are authenticated per session: the DLL and the server derive an HMAC-SHA256 key from the instance session's ticket keys (the 64 bytes the login server gives only to that client), every frame carries the session id, a counter and a 16-byte tag, and the server drops datagrams without a valid tag, from an unknown session, or with a replayed counter. Another player, or anyone who can send UDP to the server, cannot move a session's reports, inject overlay commands (which can be admin commands) or redirect its debug state. Details: [../protocol/side-channel.md](../protocol/side-channel.md). There is no unauthenticated fallback.
- **What it does not protect.** The frames are authenticated, not encrypted: someone on the network path can read positions and the debug state, though not forge them. The game stream itself is encrypted, but with the crypto bypass its keys derive from a value the server sends in clear ([../re/crypto.md](../re/crypto.md)), so on the open internet someone on the path can decrypt a session, and with the instance key material, its side channel keys. It is fine on a LAN or for friends over a VPN; for the open internet use a VPN (WireGuard or similar) until the real key exchange is restored.
- **The terrain upload** is accepted only from a session that redeemed a ticket, for the tile hash that session reported, and is size-limited, decompression-limited and format-checked. It cannot overwrite an existing cache entry. A player who is allowed to play can still upload a wrong layout for their own area (the hash cannot be recomputed from the export), so a wrong cache entry is a server-side file to delete: `dumps/0.5.5e/terrain/layouts/terrain_layout_<hash>.tsv` and the grid beside it.
- **Web API.** `[web] bind` defaults to loopback because passwords travel in request bodies and the admin endpoints act on the whole server. To use it from other machines put a TLS reverse proxy (Caddy, nginx) in front, keep the bind on loopback, forward `/api/` and the WebSocket upgrade, and set `[web] cors_origins` only for the origins that need it. Do not forward port 8080 to the internet without TLS. `[accounts] open_registration` stays off unless you want strangers to create accounts.
- **Admins.** The overlay's server commands and `!` chat commands need the admin role on the account (`account role <name> admin`); nobody is an admin by default except the dev machine-hash account `Exile1`.
- **Dev settings.** `[dev] known_seeds`, `sweep`, `movement_trace` and the `POE_*` environment variables are one-per-server development settings; keep them off on a shared server. `[dev] drift_check` is harmless.

## Web API
`[web]` is optional ([../architecture/web-api.md](../architecture/web-api.md)). Players never need it for the game. It listens on `127.0.0.1:8080`. Expose it only through a TLS proxy as described in the security notes.

## Not done yet

- **A shared world.** Each connection still runs its own simulation with the player as entity 1; two players in the "same" instance share only chat, the player count and the instance list. The remote-player protocol (WP3.5, needs an official capture), the instance actor (WP3.6) and the two-client test (WP3.7) come next.
- **Real login crypto.** The bypass keys derive from a public value; fine on a LAN, not over the open internet ([../re/crypto.md](../re/crypto.md)). A later step: the DLL swaps the client's baked-in DSA public key for ours and keeps verification on.
- **`client-pack`.** A zip with the hook and a ready `poe-hook.toml` for friends (no repository checkout) is the launcher's job (WP5).
- **Per-client address.** One `public_host` serves everyone; LAN players and internet players of the same server need NAT loopback or their own `server.toml` for deploy.
