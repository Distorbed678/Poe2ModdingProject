# Side channel (client DLL and our instance server)

Everything the client DLL says to our instance server outside the game protocol, and what the server answers: per-frame **position reports** and the **debug link** (overlay commands and state) over UDP, and the **terrain upload** and the **mod report** over TCP. WP3.4 replaced the old unauthenticated datagrams (`PSY1`/`PSY2`/`PDC1`/`PDB1`, loopback only, keyed by TCP port) with the format below. There is no fallback to the old formats.

Code: `modules/client-hook/poe-hook-shared/src/sidechannel.rs` (framing, keys, replay window, upload payload; one implementation used by both sides), DLL `poe-hook-core/src/{side,position_sync,devlink}.rs`, server `servers/instance-server/src/{sync,terrain_store}.rs`.

## Session keys
Both sides hold the instance session's 64-byte **key material**: the server issues it in the ticket (`Ticket::keys`, random per hand-off) and sends it to the client in `InstanceDetails` (login stream, encrypted); the client passes it to the instance cipher builder `0x14212aa90` (`rdx` = `InstanceDetails + 0x38`), which the DLL hooks ([../re/crypto.md](../re/crypto.md#side-channel-keys)). Every transfer to another instance is a new ticket, so a new session.

From the material `m` (HMAC-SHA256 with `m` as the key):

| Value | Derivation |
|---|---|
| session id (`u64`) | first 8 bytes of `HMAC(m, "poe2-side/v1/session")`, little-endian |
| client-to-server key | `HMAC(m, "poe2-side/v1/c2s")` (32 bytes) |
| server-to-client key | `HMAC(m, "poe2-side/v1/s2c")` |

The labels are unrelated to how the material is used for the Salsa20 streams and the HNC channel, so the keys are independent of them. The session id is sent in clear in every frame (it only names the session); the keys never leave either side. The server registers the session when it redeems the ticket (`ClientReports::register`) and forgets it when the connection ends.

## Frame
All integers little-endian.

| Offset | Size | Field |
|---|---|---|
| 0 | 3 | magic `PSS` |
| 3 | 1 | version (`1`); any other value is rejected |
| 4 | 1 | kind |
| 5 | 1 | flags (0) |
| 6 | 8 | session id |
| 14 | 8 | counter |
| 22 | n | payload |
| 22 + n | 16 | tag: first 16 bytes of `HMAC-SHA256(key, bytes[0 .. 22 + n])` |

The key is the client-to-server key for kinds 1, 2, 4, 5 and 6 and the server-to-client key for kind 3, so a frame cannot be reflected back at its sender (the receiver also rejects a kind that travels the wrong way). The tag is compared in constant time.

| Kind | Name | Way | Transport | Payload |
|---|---|---|---|---|
| 1 | Report | C to S | UDP | 26 bytes: `u16 flags` (actor `+0x2a2`), `u32 frame`, `f32 x`, `f32 y` (grid cells), `u16 action`, `u16 action2`, `i16 cursor x`, `i16 cursor y`, `u16 window width`, `u16 window height` |
| 2 | Command | C to S | UDP | text: a server command line (the overlay's, [../guides/server-admin.md](../guides/server-admin.md)); `registry` asks for the command registry |
| 3 | Debug | S to C | UDP | text lines `key<TAB>value` (simulation state), `cmds`/`instances` lists, `ack<TAB>command<TAB>result`; at most about 3000 bytes |
| 4 | Upload | C to S | TCP | see [terrain upload](#terrain-upload) |
| 5 | UploadOffer | C to S | TCP | `u32 tile hash` |
| 6 | ModReport | C to S | TCP | the loaded client mods, see [Mod report](#mod-report) |

### Counters and replay protection
Each sender numbers the frames of a stream from 1, rising by one. The receiver keeps a 64-wide sliding window per stream (`ReplayWindow`): a counter is accepted once if it is above the highest seen, or inside the window and not seen yet; counter 0, duplicates and anything older than 64 behind the top are rejected. The window moves **only after the tag verified**, so a forged frame cannot advance it.

Streams: the client's UDP stream (kinds 1 and 2, shared counter), the client's upload stream (kinds 4, 5 and 6, its own counter: an upload over a slow link takes longer than 64 report frames), and the server's debug stream (kind 3; the DLL keeps the window). A new session starts every stream at 1.

### Server rules
- Datagrams arrive on the instance bind address, UDP, same port number as the TCP listener. No source address filter (remote clients work); identity is the session id plus the tag.
- A datagram is dropped, without any reply, if it is malformed or has another version, names an unknown session, fails the tag, travels the wrong way, has an unexpected kind, or replays a counter. The first rejection and every thousandth are logged (`side-channel datagram rejected`, with the reason).
- Debug frames go to the source address of the session's latest **authentic** datagram (NAT rebinding follows; spoofed packets cannot redirect it).
- Reports never move the player; only the drift checker reads them ([movement.md](movement.md#drift-checker)).

## Terrain upload
Over the instance TCP port, no extra port. The server tells an upload from a game connection by peeking the first four bytes: `PSUP` is not a game opcode (a `Hello` starts with its u16 opcode).

```
client                                         server
PSUP u32 len  frame(UploadOffer, hash)   ->
                                         <-    PSUR u8 status u16 len text
PSUP u32 len  frame(Upload, payload)     ->    (only after status 0)
                                         <-    PSUR u8 status u16 len text
```

The reply is not authenticated (it only tells the client what to do). Statuses: `0` send it / stored, `1` already have it (`cached` or `being uploaded` by another session), `2` rejected (reason in the text), `3` authentication failed (unknown session, bad tag, replay). The server closes the connection after any non-zero status and after the data.

Upload payload: `u32 tile hash`, `u8 encoding` (1: zlib), `u32 layout bytes`, `u32 grid bytes` (both as sent), the zlib-compressed layout TSV ([../architecture/client-dll.md](../architecture/client-dll.md#layout-export-format)), the zlib-compressed grid (`PWG1`). Example: the Riverbank layout is 924 KiB and its grid 14 MiB, and the upload is 237 KiB.

Server checks (`terrain_store.rs`), all before anything is written:
- `[spawn.upload] enabled`, `[spawn] layout_dir` set; the header's length at most `max_bytes` (checked before the body is read) and the session id known before the body is read at all.
- Frame authentication and the upload counter; an upload needs an accepted offer of the same hash from the same session.
- Decompression limited to `max_layout_bytes` / `max_grid_bytes` (no decompression bombs).
- The layout is UTF-8, every tile line parses, the size is 1..2048 in both axes, every tile is inside it; the grid is a `PWG1` file of exactly `23 * w` by `23 * h` cells.
- Tile hash consistency: the hash must equal the one the session reported in `TerrainHashes` (the game stream is authenticated by its own encryption). An upload that arrives before the hash report is held in memory and committed when the hash matches (dropped when it does not). So a session can only fill the cache slot of its own area. The hash cannot be recomputed from the exported layout, which lacks tile heights and orientations; a client that lies about the content of its own area's layout still poisons that slot (the price of trusting any client that is allowed to play).
- Existing cache entries are never replaced. Files are written as `grids/terrain_grid_<hash>.bin` first, then `layouts/terrain_layout_<hash>.tsv`, each through a temporary file and a rename, so the layout's presence means both exist.

Spawn waits for the layout in `TerrainStore::wait_layout`: `[spawn.upload] wait_ms` normally (a local DLL writes the file at once), `wait_upload_secs` once an upload of that hash was announced by any session; it wakes on the store's notification and also polls the directory, so locally written files work as before.

The DLL (`side.rs`) uploads from a background thread after its layout export, only when a session exists and `trust::side_channel_allowed(read_only, server)` holds, and retries the connection three times. If the server answers `1` it stops.

## Mod report
`Kind::ModReport` (6): the client mods the hook has loaded, sent once per instance session so a server that requires client mods can check them at login ([../architecture/mod-host.md](../architecture/mod-host.md#the-login-check-of-the-clients-mods)). Over the instance TCP port with the same framing as the upload (`PSUP`, `u32 length`, one sealed frame), counted in the upload stream. The server answers `PSUR` with status `0` ("stored") or `2` (the payload does not decode); an unknown session, a bad tag or a replayed counter is `3` as for uploads.

Payload (little-endian), `poe_hook_shared::modreport`:

| Field | |
|---|---|
| `u8 format` | `1`; anything else is rejected |
| `u16 count` | at most 64 |
| per mod: `u8 id length`, id, `u8 version length`, version (UTF-8) | the mod's `id` and `version` |
| `[u8; 32]` | SHA-256 of the installed `mod.toml`, byte for byte |
| `u8 has_dll`, then `[u8; 32]` when 1 | SHA-256 of the entry DLL; 0 for a data-only mod |

An empty list (`count` 0) is a valid report: the hook runs, no mod is loaded. A client without a report is an old hook (or one that never reached the server). The hook's core sends it from a background thread as soon as the loader has finished loading mods and the connection to a trusted server is known (retrying for about a minute), only when a side-channel session exists, so never in official mode, never while read-only, never to an untrusted address. The server keeps the newest report per session (`ClientReports::store_mod_report`) and `InstanceServer::client_mod_verdict` waits for it up to `[mods] report_wait_secs`. A consistency check, not anti-cheat.
