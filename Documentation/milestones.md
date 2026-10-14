# Milestones

| # | Goal | Status |
|---|---|---|
| M0 | Foundation: repo, tools, workspace, config, data access, client DLL proxy, deploy tooling | done |
| M1 | Login → character select/creation, league selection | done (league label colour TBD) |
| M2 | Instance server transfer, first zone loads with the player | done (2026-10-13) |
| M3 | Movement: WASD, click-to-move and dodge roll; save area and position on exit | done (2026-10-14) |
| M3.5 | Fixes before M4: movement lag, full dodge roll, sprint, second-character creation | done (2026-10-14), sprint partial |
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
- Movement: done in M3 (see below).

## M3 status (2026-10-14)
All items work; details are in [protocol/movement.md](protocol/movement.md).
- **WASD:** `0x63` → `0x64`. The server walks the same way, planning paths toward the WASD target.
- **Click-to-move:** `0x1a5` UseSkill (Move) → `0x1c8` StartAction. It needs the client's click-to-move input mode.
- **Dodge roll:** characters get `DodgeRollPlayer` (`[player] innate_skills`) in their Actor skill list, with its stats. Space sends UseSkill, answered with StartAction. The server simulates the measured roll.
- **Pause:** `0x1ff` also pauses the server simulation.
- **Save area and position:** area, terrain seed and cell are saved when the session ends and restored on the next login (same layout, same cell).
- **Known limit:** near walls the server's approximate pathfinding can differ from the client's by a few cells. The client then snaps to the server's position on the next input. An exact pathfinding port comes with the server-side terrain (M4).

## M3.5 status (2026-10-14)
Details are in [protocol/movement.md](protocol/movement.md) and [protocol/login.md](protocol/login.md).
- **Lag:** the client has no lag compensation to enable (the world is always lockstep). The lag came from the server: a slow A* (about 108 ms per frame in a debug build, now under 1 ms), client batches lost to the undecoded `0x1a8`, and snapping where the server's path differed from the client's. Fixed with the new A*, the `0x1a8` / `0x1aa` / `0x1cb` steering messages, and position sync (the DLL reports the client's position, a dev bridge until M4). Run the server with `--release` for play.
- **Dodge roll:** input during a roll is held until the roll ends, instead of cutting it off, so the roll finishes.
- **Sprint:** `0x2e6` / `0x2e7` are decoded and echoed. The client sprints briefly after a roll with Space held. A full sprint needs the buff system (M7): setting `is_sprinting` as a plain stat made the client drop its movement after the roll.
- **Follow-up fixes (2026-10-14, after play-testing):**
  - **Held click-to-move hitch:** a held button repeats UseSkill every ~70 ms (target flag `0x10`); the server answered each with a new StartAction, so the client restarted its Move and lost most of every other step. `0x1cb` cannot re-aim a Move, so a held click now walks like WASD (`0x64` toward the cursor) and ends with a Move to the last cursor point.
  - **Stutter when turning:** the client puts the player on the centre of the cell in `0x64`/`0x1c8` in the frame the turn applies. The cell was one frame old (every turn jerked back by a step); now the server predicts the client's position for that exact frame and picks the cell whose centre is one normal step along the new heading. Rapid-turn test: off-length turn frames down from 13 to 3 of 64. A movement trace (`data/movement-trace.tsv`, `tools/re/movement_trace.py`) records play sessions for this kind of analysis. Play-testing chose `[instance] turn_cell = "natural"` (the cell the client reaches anyway); right after a turn, positions are predicted from that turn rather than from the last two reports (which a placement distorts), cutting placements more than 0.75 cells off from 18 to 3 in a recorded session.
  - **Teleports / invisible walls** after the collision removal: when reports were missing, the server's wall-less simulation drifted and the client snapped at 46+ cells. Reports now go only to the instance port, and once they arrive the server never moves the player on its own.
  - **Server collision removed** at the user's request: no walkability grid, A* or wall sliding in the server simulation until M4 (the grid only places a restored position).
  - **Exit to log in screen** went to character selection: the client sends `0x355` + `0x1b` and shows the log-in screen only when its login connection is closed. The server now decodes both and closes the login connection on `0x1b`.
  - WASD measured smooth (steady 33 ms frames, constant 1.23-cell steps). The remaining unevenness comes from the client itself (sliding along walls, corners).
- **Second character:** the create screen shows a league step for second and later characters. It was empty because Hardcore was sent as a separate league. It is now a variant of Standard (entry +0x80 = base league id, +0xe1 bit 1), shown as the Hardcore checkbox on the Standard banner. The create result is opcode `0x13` (the server sent `0x16`, a different message, so the screen stayed open after Continue). New characters are stored with their start area (`[player] start_area`). A new character now goes straight into the game.

## M3 scope (agreed 2026-10-14)
- WASD movement (client `0x63`).
- Click-to-move.
- Dodge roll.
- Save each character's area and position on exit, and restore them on the next login.
- Pause (`0x1ff`) also stops the server simulation.
- Proper start positions for areas other than the Riverbank (waypoints, area transitions) come with M4/M5.

## M2.5 (2026-10-13, in progress)
- **Start position** (done): the Riverbank start is on the shore beside the intro tree, inside the `Intro_Tree_01` tile. The DLL exports each sub-tile's canonical index (its position in the tile's own orientation). The server finds which of the 8 rotations/mirrors the tile was placed with, then maps the configured point (`[spawn.start_tiles.G1_1] point`, grid cells in the tile's own orientation) onto the map. Verified on a mirrored and a rotated placement.
- **Delete characters** (done): `0x0c` DeleteCharacter → `0x0d` result 0 → a fresh `0x15` CharacterList (see [protocol/login.md](protocol/login.md)).
- **Leaving the game** (done): exit to character selection now uses the resume ticket (`0x173` → `0x174` → login `0x08`), with no "unexpected disconnection". Exit to log in, exit game and closing the window all work without a crash.
- **Heartbeats** (done): the login keep-alive (every 10 s) is answered with `0x01`. The instance frame reports (`0x1c`, about 10 per second) are answered with `0x1d` ping replies carrying idle HNC bytes (HNC keys derived like the client does).
- **Pause** (done): `0x1ff` → `0x1fe` SetPause; the escape menu shows "GAME PAUSED" and the client world pauses.
- **Chat** (done): `0x14c` join requests are answered with `0x14d` (channel `[chat] default_channel`); no more "Timeout while joining … chat".
- **Full access** (done): InitialState CharacterState flags byte 1 bit 6 (`[accounts] full_access`); the escape menu no longer shows "Get Full Access".
- **Play from Steam** (done): non-Steam shortcut instructions in [setup.md](setup.md#playing-from-steam-non-steam-game-shortcut).
- Server-sent NPCs need their StateMachine/quest states (see [protocol/entities.md](protocol/entities.md)).
