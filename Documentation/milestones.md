# Milestones

| # | Goal | Status |
|---|---|---|
| M0 | Foundation: repo, tools, workspace, config, data access, client DLL proxy, deploy tooling | done |
| M1 | Login → character select/creation, league selection | done (league label colour TBD) |
| M2 | Instance server transfer, first zone loads with the player | done (2026-10-13) |
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

## M2 result (2026-10-13)
Play → `InstanceDetails` → Hello/EnableEncryption → AreaInfo (the client generates the area and reports terrain hashes) → LocalPlayers → AddInventory ×120 → PlayerEntity → AddObject (player) + frame marker → InitialState → a frame marker every `tick_ms`. Verified on a fresh client:
- **Town** (`G1_town`): the Ranger stands in Clearfell Encampment, the area banner shows, and the town is fully lit and animated.
- **New character** in the Riverbank (`G1_1`): the character lies washed up under the intro tree and the "Select a movement type" prompt appears. Returning characters spawn standing.
- **Class and default stats** (character panel, Ranger level 1): Str 7, Dex 15, Int 7, Life 28, Mana 34, Evasion 5%, mana regeneration 1.4/s. The object type is the class (`Characters.Id`). Stats come from the merged object template (`Character.ot`) plus the class's base attributes and per-level stats (`poe_game::stats`). Player sends the class byte, and Animated the class's `.ao`.

Key findings:
- **Frame length**: InitialState's `frame_ms` (world +0x11c) sets the length of each simulation frame; with 0, nothing animates or fades in, so no models are drawn and the camera breaks on a fresh client. See [protocol/instance.md](protocol/instance.md).
- **Frame ordering**: the player's AddObject is scheduled after all components (+0x18 = 0xff), else the washed-up intro adds the player twice and crashes.
- **Seeds**: some random seeds fail client-side generation ("Mismatching ground types"). Until M4 the server picks seeds the client is known to accept (`instance.known_seeds`, from live logs), and otherwise a random one.
- Spawn: a dev bridge until M4. The DLL exports the client's tile layout and cell grids; the server picks the start feature (`[spawn.start_tiles]`) and snaps to the nearest walkable cell.

Known gaps (later milestones):
- Movement: client input `0x63` (WASD direction) and clicks are not answered yet (M3).
- Heartbeat `0x1c` (about 10 Hz) is not answered; the client stays connected anyway.
- Returning characters start at the intro start feature; under the intro tree they are hidden by its canopy. Proper start positions (waypoints, area transitions) come with M4/M5.
- Server-sent NPCs need their StateMachine/quest states (see [protocol/entities.md](protocol/entities.md)).
