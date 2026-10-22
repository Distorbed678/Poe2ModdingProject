# Hosting: letting other players connect

How to run the server so people on other machines can play on it, and what each of them needs. It covers getting the server (a release package or a source build), the ports and the firewall, the config, running it permanently, a TLS proxy for the web API, backups and updates, logs, security and troubleshooting. Status: a remote client can log in with an account and password (or a launcher token), needs no shared disk (terrain upload), its side channels are authenticated, and all players in an instance share one world and see each other (verified with two real clients on a LAN, [below](#local-two-client-test)). What is still rough is listed under [Not done yet](#not-done-yet). The related designs: [../architecture/accounts.md](../architecture/accounts.md) (who is logging in), [../architecture/instance-world.md](../architecture/instance-world.md) (the shared world), [../protocol/side-channel.md](../protocol/side-channel.md) (the DLL's side channel). Players install and start the client with [client.md](client.md).

**This is an unofficial hobby server.** You need your own legal copy of the game on the server machine (the server reads game tables from it); no game files are distributed with the server. Hosting is at your own risk and responsibility, see the [disclaimer](../../README.md#disclaimer).

## Overview: pick a way

| Way | For | Where |
|---|---|---|
| **The launcher** | Someone who wants a server on their own PC without a terminal | [Host through the launcher](#host-through-the-launcher) |
| **A release package** | A machine you manage (a home server, a VPS) | [Getting a server release](#getting-a-server-release) |
| **From source** | Developers | [Running from source](#running-from-source), [setup.md](setup.md) |

Checklist for any of them: (1) a legal 0.5.5e client install on the server machine, (2) a config with your `public_host` ([Server setup](#server-setup)), (3) the ports open and forwarded ([Ports](#ports), [Firewall](#firewall)), (4) accounts ([Accounts](#accounts)), (5) the web API behind TLS if anyone outside a trusted network uses it ([TLS reverse proxy](#tls-reverse-proxy-for-the-web-api)), (6) the players' clients pointing at your address ([What a remote player needs](#what-a-remote-player-needs), [client.md](client.md)).

## Host through the launcher

The launcher's **Host a server** page does all of this for you on the machine you play on: it downloads the server release for your game version (SHA-256 checked) or takes a package you already have, asks for the name, ports, public host, registration and an admin account, writes `config/server.toml` from the example, starts and stops the server and shows its log. Details and what it does with your answers: [launcher.md](launcher.md#host-a-server). The launcher uses plain http for the web API, so the ports and TLS advice below still apply to anyone outside your own network.

## Getting a server release
Instead of building from source, take a ready package from the repository's GitHub releases (tags `server-v<version>`, built by CI; how they are made: [releasing-server.md](releasing-server.md)). Each release has, per platform, `poe-server-v<version>-game-<game version>-<linux|windows>-x86_64.tar.gz` (Linux) or `.zip` (Windows), a `.sha256` file next to it and a combined `SHA256SUMS`. The game version in the name must be the client build your players have. Until the repositories are public the releases are visible only to people with access to the repository.

```sh
sha256sum -c poe-server-v0.1.0-game-0.5.5e-linux-x86_64.sha256    # Windows: certutil -hashfile <zip> SHA256
tar xzf poe-server-v0.1.0-game-0.5.5e-linux-x86_64.tar.gz && cd poe-server-v0.1.0-game-0.5.5e-linux-x86_64
cp config/server.example.toml config/server.toml
# edit config/server.toml: game_dir (your own game install, the folder with Bundles2), [network] public_host,
# and static_dir = "web-admin" to serve the web admin
./poe-server account create <name> --password-stdin   # type the password, Enter; poe-server.exe on Windows
./poe-server account role <name> admin
./poe-server
```

The package mirrors the repository layout, because every path in `config/server.toml` is relative to the folder you start `poe-server` from: the binary, `config/server.example.toml`, `versions/` (the supported client build), `modules/protocol/opcodes/` (the message tables), `third_party/dat-schema/` (the community data schema, the exact file the build is checked against), `web-admin/` (the web admin site, served on `[web] bind`), `README.md`, `HOSTING.md` (this guide) and `release.json` (server version, game version, commit). **No game files are included**: `game_dir` must point at a legal install of the client (`Bundles2` is read from it). The default config puts the database at `runtime/server.sqlite` (the server creates the folder). Always start the server from the package folder (a service must set its working directory to it). From here the rest of this guide applies.

The example config listens on all interfaces (`0.0.0.0`) and has `open_registration = false` and the web API on loopback; check it against [Security notes](#security-notes) before you open the ports.

## Running from source
```sh
git clone --recursive https://github.com/Distorbed678/Poe2ModdingProject.git && cd Poe2ModdingProject
third_party/fetch-tools.sh          # tools and the pinned nightly toolchain, see setup.md
cp config/server.example.toml /path/to/my-server.toml     # edit it; do not use the developer config for others
POE_CONFIG=/path/to/my-server.toml cargo run --release -p poe-server
```
The committed `config/server.toml` listens on loopback only, which is right for a checkout and wrong for a hosted server: start from `config/server.example.toml`. Details and the developer config: [setup.md](setup.md).

## Ports

| What | Default | Protocol | Who connects | Forward it? |
|---|---|---|---|---|
| Login server | `[network.login] bind` = `0.0.0.0:20481` (example config; the committed dev config uses `127.0.0.1`) | TCP | every client (gateway entry), also the in-game transfers | yes |
| Instance server | `[network.instance] bind` = `0.0.0.0:6112` (same) | TCP | every client (game connection, handed over by the login server) and the DLL's terrain upload | yes |
| Side channel | same port number as the instance bind | **UDP** | the client DLL: position reports and the debug link (overlay, commands) | yes, UDP as well as TCP |
| Web API and web admin | `[web] bind` = `127.0.0.1:8080` | TCP (HTTP) | admins, the launcher's account pages | no, unless behind TLS ([below](#tls-reverse-proxy-for-the-web-api)) |

There is nothing else: no extra port for the terrain upload (it shares the instance TCP port), and the web API is optional. Connections that have not logged in yet are limited ([Security notes](#security-notes)).

## Server setup

Edit `config/server.toml` (a copy of `config/server.example.toml`; or keep the file anywhere and start with `POE_CONFIG=<file>`):

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
- Names work for `public_host`: the client resolves it, and `deploy-client` resolves it to IPv4 when it writes `trusted_servers` (the DLL only reads IP literals). Over the internet prefer a fixed address, or redeploy when it changes. Names under the official game domains are refused for the trust list.
- A player on the same LAN as the server connecting through the public address needs NAT loopback (hairpin) on the router, or the LAN address in their own deploy (`public_host` is read from the `server.toml` they deploy with).
- `[game] game_dir` is the folder with `Bundles2` of the server machine's own client install; `[game] version` selects `versions/<version>.toml` (the supported build).
- `[cluster] id` is part of every login token; change it to log everyone out.
- `[database] path` is the SQLite file (accounts, characters, tokens).

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
Create the players' accounts on the server: the offline `account create <name> --password-stdin` ([setup.md](setup.md#accounts-and-characters-offline), other account commands: [server-admin.md](server-admin.md#accounts)), or turn on `[accounts] open_registration` and use the web API's register endpoint (the launcher's "Create account" does that; the web API must be reachable, behind TLS). Make yourself an admin with `account role <name> admin`. There is no password-less login; the Steam `0x0005` login is always refused.

## Running it permanently

The server is one process with no daemon mode; stop it with Ctrl+C or SIGTERM (players are disconnected and their sessions saved, the database is consistent after every write, exit code 0). Admins can also stop and restart it from the web admin's dashboard, with a countdown announced in game ([server-admin.md](server-admin.md#stop-and-restart-the-server)). Run it from the package folder.

**Linux (systemd).** A unit for a dedicated user, saved as `/etc/systemd/system/poe-server.service` (adapt the paths):

```ini
[Unit]
Description=PoE2 private server
After=network-online.target
Wants=network-online.target

[Service]
User=poe
WorkingDirectory=/opt/poe-server
ExecStart=/opt/poe-server/poe-server
Environment=RUST_LOG=info
# A crash restarts the server. The web admin's "Restart" (with `[server] restart_mode = "exit"`
# in config/server.toml) exits with code 75: count it as a normal exit and restart on it.
# The web admin's "Stop" and `systemctl stop` exit 0 and stay stopped.
Restart=on-failure
RestartForceExitStatus=75
SuccessExitStatus=75
RestartSec=2
# Optional hardening: the server only writes below runtime/ and (terrain cache) dumps/.
NoNewPrivileges=true
ProtectSystem=strict
ReadWritePaths=/opt/poe-server/runtime /opt/poe-server/dumps
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

`systemctl daemon-reload && systemctl enable --now poe-server`; the log is `journalctl -u poe-server -f`. Set `restart_mode = "exit"` in `[server]` so the web admin's Restart button hands the restart to systemd (exit code 75, `RestartForceExitStatus=75` above); with the default `"exec"` the server restarts itself inside the same service (the pid stays, so systemd does not notice), which works too. `systemctl restart poe-server` is a plain SIGTERM stop (graceful, 5 s drain) and start. Create accounts with the service stopped or running (`sudo -u poe /opt/poe-server/poe-server account create ...` from the same folder; the commands work against the database directly).

**Windows.** Run `poe-server.exe` from the package folder in a terminal, or register it as a scheduled task that starts at boot with "Start in" set to the folder (`schtasks /Create /TN poe-server /SC ONSTART /RU <user> /TR "cmd /c cd /d C:\poe-server && poe-server.exe >> runtime\server.log 2>&1"`). A service wrapper such as NSSM works the same way (application folder as the working directory); with a wrapper or task that restarts the program when it exits, set `[server] restart_mode = "exit"` (code 75) so the web admin's Restart is handled by it, because Windows has no `exec` and the default mode starts a new process and exits 0.

**Anywhere.** `tmux`/`screen` is fine for a few friends. Whatever you use, the working directory must be the folder with `config/`, `versions/` and `modules/`.

## TLS reverse proxy for the web API

The web API and the web admin carry passwords and admin actions, and the server has no TLS of its own. Keep `[web] bind` on loopback and publish it through a reverse proxy that terminates TLS. Three things matter: forward `/api/` and the WebSocket upgrade, send the real client address, and set `[security] trust_forwarded_for = true` **only** when the proxy is the only way to reach the port. With it on, the server uses the **last** `X-Forwarded-For` entry (the one your proxy appended), so a client that prepends its own entries cannot choose the address the login limiter sees. Without it, every web client appears as the proxy's address and one attacker can lock the web login for everyone (the game login on port 20481 is unaffected, it sees real addresses).

Caddy (`Caddyfile`; TLS certificates are obtained automatically for a public DNS name):

```
play.example.org {
    reverse_proxy 127.0.0.1:8080
}
```

Caddy appends the client address to `X-Forwarded-For` and passes WebSockets through by itself.

nginx (inside a `server { listen 443 ssl; ... }` block with your certificate):

```nginx
location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;   # appends the real peer: what trust_forwarded_for reads
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;                         # the live log WebSocket
    proxy_set_header Connection $connection_upgrade;                # with: map $http_upgrade $connection_upgrade { default upgrade; "" close; }
}
```

Then `[security] trust_forwarded_for = true`, and in the launcher add the server as `https://play.example.org`. The server also sends security headers (CSP, `nosniff`, no referrer, no framing) with the web admin. `[web] cors_origins` stays empty unless another site must call the API from a browser. Do not forward port 8080 itself.

## Backups, updates and maintenance

- **What to back up:** `runtime/server.sqlite` (accounts, tokens, characters) and your `config/server.toml`. The terrain cache in `dumps/<version>/terrain/` is rebuilt from clients and does not need a backup. Back up the database while the server runs with `sqlite3 runtime/server.sqlite ".backup 'backup-$(date +%F).sqlite'"`, or copy the file while the server is stopped. Passwords are stored as argon2id hashes, tokens only as SHA-256; still treat the file as private.
- **Updating the server:** stop it, unpack the new release next to the old one (or over it), keep `config/server.toml` and `runtime/`, start it. Database changes are applied automatically at start-up (migrations). Compare `config/server.example.toml` with your config for new keys; unknown or missing keys have defaults, a removed key is ignored. A release names the client build it supports in its file name: when GGG ships a new client build, wait for a release for it and tell players not to update the game until then ([new-game-version.md](new-game-version.md)).
- **Updating players:** the hook DLLs and the gateway entry are deployed by the launcher or `deploy-client`; a new server release that changes the supported build needs a new launcher/hook, see [client.md](client.md).
- **Resetting:** to log everyone out change `[cluster] id`; to lock an account `account disable <name>`; to remove a bad terrain cache entry delete its `terrain_layout_<hash>.tsv` and the grid beside it (see [Security notes](#security-notes)).

## Logs and monitoring

- The server logs to standard output. The level is `[logging] filter` (`tracing` syntax, default `info,poe_net=debug`) and `RUST_LOG` overrides it. Useful lines: `login server listening`, `authenticated`, `ticket redeemed`, `player joined ... players=N`, `terrain upload cached`, `side-channel session`, and warnings such as `too many connections before login: refused`.
- Admin commands (kick, announce, role changes) are appended to `[commands] audit_log` (time, account, command, result).
- The web admin's **Logs** page shows the live log; the dashboard shows players online and instances. `curl http://127.0.0.1:8080/api/v1/server` returns the realm name, game version, players online, uptime and the addresses the launcher uses.
- Nothing is sent anywhere: no telemetry, no update checks.

## What a remote player needs

1. **Their own legal install** of Path of Exile 2 0.5.5e (the files are not distributed). They also need Steam running when they start the client (without it the client exits at `Unable to init Steam API`).
2. **The client hook** (`version.dll`, `poe_hook.dll`, `poe_hook_core.dll`, `poe-hook.toml`) in the game folder, with a gateway entry and a trust list that point at your server. Either:
   - **the launcher** (`modules/launcher`, [launcher.md](launcher.md)): installs 0.5.5e from Steam depots, deploys the hook and takes the server's web API address (so the web API must be reachable for it, behind TLS outside a trusted network); or
   - **`cargo xtask deploy-client`** from a checkout of this repository, with a `config/server.toml` that has your `public_host` and ports (copy yours, or edit just `[network]`), and the client files in `GameFiles/`: `cargo xtask build-dll` (once), then `cargo xtask deploy-client --no-build --login-mode standalone`. It writes the gateway entry `public_host:login port`, `login_mode`, `position_sync_port` and `trusted_servers = ["<public_host>:<login port>", "<public_host>:<instance port>"]`. Add more servers by hand in `GameFiles/poe-hook.toml`, e.g. `"203.0.113.7:*"`.
3. **Start the client** (`cargo xtask run-client`, or Steam with the deployed folder) and log in with the account name and password ([setup.md](setup.md#remember-me-token-launcher-auto-login)).

What the trusted entries change in the DLL: the connections are not blocked, the read-only latch stays off, the login crypto bypass runs, and the side channel (position reports, debug link, terrain upload) goes to them. Official servers, the shop and telemetry stay refused or read-only exactly as before, and no config key can trust anything in official mode ([../architecture/client-dll.md](../architecture/client-dll.md#login-mode-and-trusted-servers)). A client deployed with `--official` never sends anything to any server of ours.

The player does not need the server's `dumps/`, `GameFiles/Bundles2` copy or any shared folder.

## Security notes

- **Pre-login limits.** Idle or slow sockets cannot pin the game ports: before a client has logged in (login server) or shown a ticket (instance server), one message must arrive complete within 15 s, the login must finish within 60 s, at most 64 KiB are buffered for an incomplete message, and at most 256 such connections exist in total and 16 per client IP. All are config values in `[network.login]` and `[network.instance]` (`pre_auth_*`, `max_pre_auth_*`; `0` turns one off, [accounts.md](../architecture/accounts.md#limits-before-the-login)). Players behind one NAT share the per-IP limit: raise `max_pre_auth_per_ip` for a LAN party behind one address. This is a protection against careless load, not a DDoS defence: put a firewall or your host's filtering in front for that.
- **Side channel.** Position reports, the overlay's debug link and the terrain upload are authenticated per session: the DLL and the server derive an HMAC-SHA256 key from the instance session's ticket keys (the 64 bytes the login server gives only to that client), every frame carries the session id, a counter and a 16-byte tag, and the server drops datagrams without a valid tag, from an unknown session, or with a replayed counter. Another player, or anyone who can send UDP to the server, cannot move a session's reports, inject overlay commands (which can be admin commands) or redirect its debug state. Details: [../protocol/side-channel.md](../protocol/side-channel.md). There is no unauthenticated fallback.
- **What it does not protect.** The frames are authenticated, not encrypted: someone on the network path can read positions and the debug state, though not forge them. The game stream itself is encrypted, but with the crypto bypass its keys derive from a value the server sends in clear ([../re/crypto.md](../re/crypto.md)), so on the open internet someone on the path can decrypt a session, and with the instance key material, its side channel keys. It is fine on a LAN or for friends over a VPN; for the open internet use a VPN (WireGuard or similar) until the real key exchange is restored.
- **The terrain upload** is accepted only from a session that redeemed a ticket, for the tile hash that session reported, and is size-limited, decompression-limited and format-checked. It cannot overwrite an existing cache entry. A player who is allowed to play can still upload a wrong layout for their own area (the hash cannot be recomputed from the export), so a wrong cache entry is a server-side file to delete: `dumps/0.5.5e/terrain/layouts/terrain_layout_<hash>.tsv` and the grid beside it. If you only host for people you do not trust, set `[spawn.upload] enabled = false` and run one client on the server machine that writes the layouts directly; remote players then spawn from the cached layouts only.
  **Decision (audit S-08, accepted):** the upload stays **on** by default, because remote players need it to spawn correctly. The trade-off: a modified client can plant a well-formed but wrong layout for its own area hash, and later players of that area spawn by it (it cannot overwrite an existing entry, cannot touch other areas, and does not reach other players' game state). It goes away once the server generates every area itself ([terrain status](../status/terrain.md)); until then, delete a bad cache entry or turn the upload off as described above.
- **Web API.** `[web] bind` defaults to loopback because passwords travel in request bodies and the admin endpoints act on the whole server. To use it from other machines put a TLS reverse proxy in front ([above](#tls-reverse-proxy-for-the-web-api)). Do not forward port 8080 to the internet without TLS. `[accounts] open_registration` stays off unless you want strangers to create accounts. The log WebSocket authenticates with a single-use ticket, not with the session token in the URL.
- **Admins.** The overlay's server commands and `!` chat commands need the admin role on the account (`account role <name> admin`); nobody is an admin by default. (A database that was created before the account roles existed may have an account `Exile1` (made by the removed machine-hash login) promoted by the migration; check `account list`.)
- **Password logins** are rate limited per account name and per client IP (`[security]`, [accounts.md](../architecture/accounts.md#password-attempt-limits)). Someone who knows an account name can lock that name for the lockout time by failing on purpose; the owner can still use a saved token.
- **Dev settings.** `[dev] known_seeds`, `sweep`, `movement_trace`, `debug_link`, `position_sync` and the `POE_*` environment variables are development settings; the committed config and the developer config (`runtime/server.dev.toml`, [setup.md](setup.md#developer-config)) are not for a shared server. The example config is the template. `[dev] drift_check` is harmless.

## Troubleshooting

| Symptom | Likely cause | What to do |
|---|---|---|
| Server log: `[network] public_host = "127.0.0.1" is a loopback address` | You listen beyond loopback but advertise localhost | Set `public_host` to the LAN/public address or name |
| Client cannot reach the realm / "Local Server" never answers | Port 20481 not open or forwarded; wrong `public_host` in the player's gateway entry | `nc -vz <host> 20481` from outside; check the player's `gateway_list.txt` (written by the launcher or `deploy-client`) |
| Login works, then the client hangs on loading or disconnects | Port 6112 TCP blocked, or `public_port` differs from the forwarded port | `nc -vz <host> 6112`; set `[network.instance] public_port` to the external port |
| Player spawns in a wrong place / at the origin | The server has no terrain layout for the area (uploads off, or UDP/TCP upload failed, or no client generated it yet) | Look for `terrain upload cached` and `spawn from client layout` lines; see [Terrain](#terrain-spawn-spawnupload) |
| Overlay shows no state, no position drift | UDP 6112 blocked | Forward UDP too; play is not affected |
| Launcher: "cannot reach the server" | The web API is on loopback or not behind TLS where the launcher expects it | [TLS reverse proxy](#tls-reverse-proxy-for-the-web-api); `curl https://<host>/api/v1/server` |
| "You have attempted to log in too often" / HTTP 429 | The rate limiter (5 failures per account or 20 per IP in 5 minutes) | Wait for the lockout (`login_lockout_secs`), or restart the server to clear the in-memory locks; behind a proxy set `trust_forwarded_for` |
| Client shows "There has been a patch that you need to update to" | The client's exe is not a known build of `[game] version` | Use the supported build; to try another one `[security] allow_unknown_client = true` ([new-game-version.md](new-game-version.md)) |
| Server will not start: `... is not a supported build` / version file errors | `[game] version` has no `versions/<version>.toml` in the folder you started from | Start from the package folder; check `[game] version` |
| `Too many connections before login: refused` in the log | More than 16 unauthenticated connections from one address, or 256 in all | Normal under a flood; raise `max_pre_auth_per_ip` for many players behind one NAT |

## Local two-client test
`scripts/test-mp.sh` runs two real game clients against one server on one machine (Steam must be running). Both connect through the machine's LAN IP, not 127.0.0.1, so the trusted-remote path of the DLL runs (the deploy puts the LAN IP into `trusted_servers`), and each logs in as its own account (`mp1`, `mp2`) on the standalone login with a remember-me token (web API launcher login, then `POST /game-token`); the random passwords of the two accounts are in `runtime/test-accounts/` (git-ignored, never printed).

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
`[web]` is optional ([../architecture/web-api.md](../architecture/web-api.md)). The game itself never needs it, but the launcher does (account login and register, `GET /server`, `POST /game-token`), so a server for launcher users has to expose it through a TLS proxy ([above](#tls-reverse-proxy-for-the-web-api)). It listens on `127.0.0.1:8080` by default.

## Not done yet

- **Remote players, rough edges.** Players in one instance share one world (instance actor, WP3.6) and see each other ([Local two-client test](#local-two-client-test)). Open: no equipment on remote players (the tint is gone, [re-check](#local-two-client-test)), and no party, trade or combat sync between players yet. Players no longer spawn on one cell (`[spawn] player_spacing`).
- **Real login crypto.** The bypass keys derive from a public value; fine on a LAN, not over the open internet ([../re/crypto.md](../re/crypto.md)). A later step: the DLL swaps the client's baked-in DSA public key for ours and keeps verification on.
- **Bundled hook DLLs.** The launcher still needs the hook DLLs from a local build of the client hook (its settings point at them); a ready package of the hook with a `poe-hook.toml` for friends does not exist yet ([launcher.md](launcher.md#not-done)).
- **Per-client address.** One `public_host` serves everyone; LAN players and internet players of the same server need NAT loopback or their own `server.toml` for deploy.
- **Untested here:** the systemd unit, the Windows scheduled task and the nginx/Caddy snippets are standard patterns written for this guide and have not been run against a release package yet.
