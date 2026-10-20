# Milestones

| # | Goal | Status |
|---|---|---|
| M0 | Foundation: repo, tools, workspace, config, data access, client DLL proxy, deploy tooling | done |
| M1 | Login → character select/creation, league selection | done (league label colour TBD) |
| M2 | Instance server transfer, first zone loads with the player | done (2026-10-13) |
| M3 | Movement: WASD, click-to-move and dodge roll; save area and position on exit | done (2026-10-14) |
| M3.5 | Fixes before the terrain milestone: movement lag, full dodge roll, sprint, second-character creation | done (2026-10-14), sprint partial |
| M4 | Debug suite: in-game imgui overlay (Vulkan) in the client DLL, packet capture and export for our and official servers (read-only), protocol coverage, inspectors, hot reload | done (2026-10-14); official sessions captured 2026-10-14/15; overlay reorganised into a sidebar 2026-10-17 |
| M5 | Server-side terrain generation (reimplement the client generator) for collision | in progress: 25 of 33 test vectors match (towns, G1_1-3 and G1_6-9 fully) ([terrain-status.md](terrain-status.md)); off by default (`[terrain] generate`) |
| M6 | Zone/instance transfer | dev transfers done (2026-10-17): instance directory, transfers, forced layouts, Instances page ([server-admin.md](server-admin.md)); waypoints and area transitions open |
| M7 | Inventory and items | – |
| M8 | Monsters and skills | – |
| M9+ | To be decided (parties, multiplayer, mod runtimes) | chat channels and server commands done early (2026-10-17); multiplayer gap analysis in [multiplayer.md](multiplayer.md) |

## Status 2026-10-17
- **Server tools:** one command registry for chat (`!` prefix) and the overlay, admins from `[admin] accounts`, audit log, `who`/`kick`/`mute`/`announce` ([server-admin.md](server-admin.md)).
- **Instances and transfers (M6, dev side):** `poe_cluster::InstanceDirectory` (area, seed, forced topology/orientation, players, idle expiry). A transfer ends the instance session; the client returns to character selection on its open login connection and the login server sends the pending InstanceDetails. Forced layouts go in AreaInfo's stat records and were verified against the client's tile hash. Still open: waypoints/area transitions, and transfers without the character-select step (needs an official capture of a zone change).
- **Chat:** C→S `0x16` ChatSend and S→C `0x18` ChatMessage decoded; global/trade by channel number, local, whispers, mutes.
- **Terrain (M5):** see [terrain-status.md](terrain-status.md); the server can generate per instance and compare hashes (`[terrain] generate`, `!terrain`).
- **Overlay:** sidebar navigation, Ctrl+P palette, opcode hiding in the Packets page, our-server pages hidden on official servers ([client-dll.md](client-dll.md)).

## M4 scope: Debug suite (agreed 2026-10-14)
Renamed from terrain generation, which moves to M5 (later milestones shift by one). Goal: fewer workarounds, everything the DLL knows visible in game and in files.
- **Overlay:** Dear ImGui drawn in the game's Vulkan present (our own hook; no DX switch). Insert toggles it (configurable).
- **All DLL diagnostics** in imgui tabs *and* in log files (`GameFiles/logs/poe-hook/<session>/`).
- **Packets:** plaintext captured inside the client (no cipher breaking) with exact message boundaries, for the login and instance connections, on our and on official servers. Names and decoded fields from `protocol/0.5.5e/*.toml` / `poe-proto`; same export format for both so sessions can be diffed. Secrets (Steam ticket, session tokens, key material) are redacted in exports.
- **Official servers (rule change 2026-10-14):** allowed, **read-only**. The GameFiles copy runs them (`deploy-client --official`); the Steam install stays untouched. A latch turns off everything that changes the game or talks to our server (crypto bypass, position sync, auto keys, input injection, memory writes) once a non-local server is seen; the overlay shows `OFFICIAL: read-only`.
- **Tabs:** Log, Packets, Protocol coverage (per opcode; official vs ours), Game log (`Client.txt`), Player inspector, Entity browser + memory watches, Timing graphs (frame time, packet arrival, playback backlog), Features (runtime toggles).
- **Bug-report snapshot:** a hotkey saves the last ~30 s of packets, logs, graphs and a screenshot into one folder.
- **Hot reload:** `version.dll` becomes a small loader that keeps the persistent hooks (network guard, Vulkan, window procedure); the debug core (`poe_hook_core.dll`) can be rebuilt and reloaded from the overlay without restarting the game.

### M4 status (2026-10-14)
- Done: loader/core split with hot reload, read-only latch and official deploy, dear-imgui-rs overlay (Status, Log, Packets, Coverage, Timing, Game log, Player, Entities, Watches, Features), packet capture with redaction and files, snapshots, `scripts/re/packets_diff.py`. Details in [client-dll.md](client-dll.md).
- Official sessions: captured 2026-10-14 and 2026-10-15 (read-only); compared in [protocol/official-comparison.md](protocol/official-comparison.md).
- Found with the minimap (2026-10-14), for M5: the player walked onto a cell where grid 0 (`terrain+0xd0`) is 0 while grids 1-3 are 5; grid 1 shows one connected open area around the player. The server's spawn code treats grid 0 non-zero as walkable; grid 1 is the likelier walkability grid. Check before building collision.
- M5 tooling (2026-10-15, [re/terrain.md](re/terrain.md)): area export with hash self-check (tile and doodad hashes recomputed from the export match the client on every area tried), generator trace (every RNG draw and phase), walkability probe, generator-object dump, and a seed sweep (server `[instance] sweep` + `scripts/re/sweep_drive.py`).
- Found by the capture, left for later (server unchanged on request): the instance opening is two messages, `0x000f` Hello (u32) and `0x000e` (12 bytes, u32 ticket, u32, u8); the server reads them as one.

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
- **Seeds**: some random seeds fail client-side generation ("Mismatching ground types"). Until M5 the server picks seeds the client is known to accept (`instance.known_seeds`, from live logs), and otherwise a random one.
- Spawn: a dev bridge until M5. The DLL exports the client's tile layout and cell grids; the server picks the start feature (`[spawn.start_tiles]`) and snaps to the nearest walkable cell.

Known gaps (later milestones):
- Movement: done in M3 (see below).

## M3 status (2026-10-14)
All items work; details are in [protocol/movement.md](protocol/movement.md).
- **WASD:** `0x63` → `0x64`. The server walks the same way, planning paths toward the WASD target.
- **Click-to-move:** `0x1a5` UseSkill (Move) → `0x1c8` StartAction. It needs the client's click-to-move input mode.
- **Dodge roll:** characters get `DodgeRollPlayer` (`[player] innate_skills`) in their Actor skill list, with its stats. Space sends UseSkill, answered with StartAction. The server simulates the measured roll.
- **Pause:** `0x1ff` also pauses the server simulation.
- **Save area and position:** area, terrain seed and cell are saved when the session ends and restored on the next login (same layout, same cell).
- **Known limit:** near walls the server's approximate pathfinding can differ from the client's by a few cells. The client then snaps to the server's position on the next input. An exact pathfinding port comes with the server-side terrain (M5).

## M3.5 status (2026-10-14)
Details are in [protocol/movement.md](protocol/movement.md) and [protocol/login.md](protocol/login.md).
- **Lag:** the client has no lag compensation to enable (the world is always lockstep). The lag came from the server: a slow A* (about 108 ms per frame in a debug build, now under 1 ms), client batches lost to the undecoded `0x1a8`, and snapping where the server's path differed from the client's. Fixed with the new A*, the `0x1a8` / `0x1aa` / `0x1cb` steering messages, and position sync (the DLL reports the client's position, a dev bridge until M5). Run the server with `--release` for play.
- **Dodge roll:** input during a roll is held until the roll ends, instead of cutting it off, so the roll finishes.
- **Sprint:** `0x2e6` / `0x2e7` are decoded and echoed. The client sprints briefly after a roll with Space held. A full sprint needs the buff system (M8): setting `is_sprinting` as a plain stat made the client drop its movement after the roll.
- **Follow-up fixes (2026-10-14, after play-testing):**
  - **Held click-to-move hitch:** a held button repeats UseSkill every ~70 ms (target flag `0x10`); the server answered each with a new StartAction, so the client restarted its Move and lost most of every other step. `0x1cb` cannot re-aim a Move, so a held click now walks like WASD (`0x64` toward the cursor) and ends with a Move to the last cursor point.
  - **Stutter when turning:** the client puts the player on the centre of the cell in `0x64`/`0x1c8` in the frame the turn applies. The cell was one frame old (every turn jerked back by a step); now the server predicts the client's position for that exact frame and picks the cell whose centre is one normal step along the new heading. Rapid-turn test: off-length turn frames down from 13 to 3 of 64. A movement trace (`runtime/movement-trace.tsv`, `scripts/re/movement_trace.py`) records play sessions for this kind of analysis. Play-testing chose `[instance] turn_cell = "natural"` (the cell the client reaches anyway); right after a turn, positions are predicted from that turn rather than from the last two reports (which a placement distorts), cutting placements more than 0.75 cells off from 18 to 3 in a recorded session.
  - **Teleports / invisible walls** after the collision removal: when reports were missing, the server's wall-less simulation drifted and the client snapped at 46+ cells. Reports now go only to the instance port, and once they arrive the server never moves the player on its own.
  - **Server collision removed** at the user's request: no walkability grid, A* or wall sliding in the server simulation until M5 (the grid only places a restored position).
  - **Exit to log in screen** went to character selection: the client sends `0x355` + `0x1b` and shows the log-in screen only when its login connection is closed. The server now decodes both and closes the login connection on `0x1b`.
  - WASD measured smooth (steady 33 ms frames, constant 1.23-cell steps). The remaining unevenness comes from the client itself (sliding along walls, corners).
- **Facing the cursor** (2026-10-15): the player faces the cursor like on the official servers (not while walking by click-to-move or rolling). The client does not do this by itself here and sends no cursor data, so the DLL reports the cursor and the server sets the facing with `0x1df` (locked).
- **Second character:** the create screen shows a league step for second and later characters. It was empty because Hardcore was sent as a separate league. It is now a variant of Standard (entry +0x80 = base league id, +0xe1 bit 1), shown as the Hardcore checkbox on the Standard banner. The create result is opcode `0x13` (the server sent `0x16`, a different message, so the screen stayed open after Continue). New characters are stored with their start area (`[player] start_area`). A new character now goes straight into the game.

## M3 scope (agreed 2026-10-14)
- WASD movement (client `0x63`).
- Click-to-move.
- Dodge roll.
- Save each character's area and position on exit, and restore them on the next login.
- Pause (`0x1ff`) also stops the server simulation.
- Proper start positions for areas other than the Riverbank (waypoints, area transitions) come with M5/M6.

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
