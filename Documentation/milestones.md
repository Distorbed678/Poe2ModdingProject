# Milestones

| # | Goal | Status |
|---|---|---|
| M0 | Foundation: repo, tools, workspace, config, data access, client DLL proxy, deploy tooling | done |
| M1 | Login → character select/creation, league selection | done (league label colour TBD) |
| M2 | Instance server transfer, first zone loads with the player | in progress |
| M3 | Movement (WASD and click-to-move) | – |
| M4 | Server-side terrain generation (reimplement the client generator) for collision | – |
| M5 | Zone/instance transfer | – |
| M6 | Inventory and items | – |
| M7 | Monsters and skills | – |
| M8+ | To be decided (parties, chat, multiplayer, mod runtimes) | – |

## M0 checklist
- [x] git repo, `.gitignore`, `CLAUDE.md`
- [x] Tools: poe_data_tools CLI, dat-schema, LibGGPK3, cargo-xwin, reference repos
- [x] `poe-config`, `poe-net` (codec, cipher trait, connection), `poe-data` (Bundles2 + datc64 + schema)
- [x] Login and instance listeners that hexdump incoming bytes
- [x] `poe-hook` version.dll proxy with logging and a crash logger
- [x] `cargo xtask build-dll / deploy-client / revert-client / run-client`
- [x] Verified 2026-10-12: the DLL loads under Proton, the client reads our gateway entry and connects to 127.0.0.1:20481, and the server logs the first message (see protocol/login.md)

## M1 result (2026-10-12)
These work end to end:
- Key exchange (DLL `crypto_bypass`).
- Authentication: SQLite account keyed by the machine hash.
- The first-character cutscene.
- Character creation: start state → class from game data, persisted in SQLite, slot limit from config.
- The character select screen, showing class, area and league.
- Standard and Hardcore leagues from `server.toml`.

Open items:
- The league label is grey on the select screen.
- The meaning of the remaining entry fields.
- Error codes for refused character creation.

## M2: status and next steps (paused 2026-10-12)
Implemented, **not yet tested live**:
1. `PlayCharacter` (C→S `0x0e`: `u8 flags, varuint n, (u32 char_index, u32 account_id)[n]`) triggers a ticket from `poe-cluster::MemoryTickets` (60 s, single use, 64-byte random keys).
2. The server then sends `InstanceDetails` (S→C `0x14`):
   - `astring host, u16 port, u32 (ticket), u16 area HASH16, u32, 64-byte keys, 8 bytes, u8`;
   - host and port come from `[instance] public_host/public_port`, the area from the character's `area`.
3. The instance server (port 6112) logs the client's first bytes raw, and decrypted with each pending ticket's client→server keys.

To resume:
- Run `cargo run -p poe-server` and `cargo xtask run-client`, log in, click Play on a character.
- Check the server log for the instance connection and its first message (data/server-run.log).
- Then reverse engineer the client's first instance message and the server's area/entity messages:
  - The instance packet factories still need to be found. `tools/re/find_opcode_stores.py` shows clusters at 0x14215xxxx–0x14218xxxx.
  - Look for "Generating level N area" in `GameFiles/logs/Client.txt`.
