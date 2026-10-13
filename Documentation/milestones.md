# Milestones

| # | Goal | Status |
|---|---|---|
| M0 | Foundation: repo, tools, workspace, config, data access, client DLL proxy, deploy tooling | done |
| M1 | Login → character select/creation, league selection | done (league label colour TBD) |
| M2 | Instance server transfer, first zone loads with the player | in progress (game screen reached) |
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

## M2: status and next steps (2026-10-13)
Working end to end: Play → `InstanceDetails` → Hello/EnableEncryption → AreaInfo (client generates the area and reports terrain hashes) → LocalPlayers → AddInventory ×120 → PlayerEntity → AddObject (player) → ticks → InitialState. The client shows the game screen with the HUD (life orb, skill bar, "WASD to move").

Open items:
- **Spawn position**: Positioned sends grid (0, 0), so the camera sits in fog outside the map. We need a walkable start position, either from server-side terrain (M4) or from the client's generated terrain (DLL dump).
- The mana orb is empty, so the Life pool order (life/mana/ES) is still a guess.
- The Actor section is undecoded (zeros).
- The post-area messages are still behind `POE_EXPERIMENT`; they should become the default flow.
- Rendering: the 3D view is uniform grey fog with a black vertical bar that scales with zoom; the minimap shows terrain around the player. Ruled out: missing simulation ticks (now sent every `tick_ms`), death state, spawn outside the map (the Riverbank terrain is 69×69 tiles, logged by the DLL probe). Next test: `POE_AREA=G1_town` to see whether the problem is specific to the Riverbank.
- Client input seen: `0x63` WASD direction (`u8, varint x, varint y, 3 bytes`), `0x1c` about 10 Hz (heartbeat?, no reply sent yet).
