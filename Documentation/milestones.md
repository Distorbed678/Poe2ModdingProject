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
Working end to end: Play → `InstanceDetails` → Hello/EnableEncryption → AreaInfo (client generates the area and reports terrain hashes) → LocalPlayers → AddInventory ×120 → PlayerEntity → AddObject (player) → ticks every `tick_ms` → InitialState. The client shows the game screen with the HUD.

Done:
- **Class and default stats** (verified in the character panel, Ranger level 1: Str 7, Dex 15, Int 7, Life 28, Mana 34, Evasion 5%, mana regeneration 1.4/s):
  - the object type is the class (`Characters.Id`, e.g. `Metadata/Characters/Dex/DexFour`);
  - the Stats component carries the merged object-template defaults (`poe_data::template`, `Character.ot`) plus the class's base attributes, life and mana and the per-level stats (`poe_game::stats::character_defaults`), keyed by `Stats.dat` row + 1;
  - Player sends the class byte (`PlayerClass` +0x158) and Animated the class's `.ao`.
- **Spawn on walkable ground**: dev bridge until M4 (the DLL exports the client's tile layout and cell grids; the server picks the start feature and snaps to the nearest walkable cell).
- **New characters wash up** in the intro area (Player flags bit 0) once; `characters.intro_done` is stored.
- Towns and hideouts use seed 1 (matches live tile hashes).
- Mana pool order fixed (mana orb full).

Open (blocking M2's "player is visible"):
- **The player body is not drawn.** The entity exists, is awake (+0x8d bit 2), is updated every simulation frame (~30 Hz), has rendering enabled (Render +0x39a = 0) and its light is drawn. Forcing the AO scene node visible (`0x1416a9810`) changes nothing. See [re/findings-2026-10-13.md](re/findings-2026-10-13.md#player-model-not-drawn).
- **Grey view on a fresh client.** The first area entry of a client process shows grey with a black vertical bar; entering again in the same process (after a reconnect) shows the area normally. A spawn delay does not help.
- The post-area messages are still behind `POE_EXPERIMENT`; they should become the default flow.
- Client input seen: `0x63` WASD direction (`u8, varint x, varint y, 3 bytes`), `0x1c` about 10 Hz (heartbeat?, no reply sent yet).
