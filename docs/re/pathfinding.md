# The client's pathfinder (0.5.5e Hotfix 1)

Result of the movement-path work of 2026-10-21 (Ghidra, plus a DLL hook that logs the plans the client makes for the local player). Short version: **the client plans every walk itself, with a bounded jump point search over its 4-bit walkability grid**, for click-to-move, WASD, rolls and sprints, and the server now runs a port of it (`modules/server/crates/poe-game/src/clientpath.rs`). Addresses are Ghidra addresses of build 0.5.5e Hotfix 1 ([addresses.md](addresses.md#pathfinder-2026-10-21)); the movement spec that uses it is [../protocol/movement.md](../protocol/movement.md#the-clients-pathfinder).

## Where it is used
| Caller | What it plans |
|---|---|
| `Pathfinding_StepAlongPath` (`0x141dcc110`) / path request `0x141dcc6a0` | click-to-move and the Move action's walk. The request clamps the vector to the goal to 126 cells (`0x42fc0000`, squared limit `0x4840`) and calls the search with the goal as target. A plan that ends short of the goal is made again the next step |
| `Actor_StartWasdWalk` (`0x141e4fb40`) and the Actor tick (`0x141e42a00`) | WASD and the sprint stream: goal = the entity's cell + `floor(vector * 0.092)` (`0x142193270`), re-aimed by a timer at Actor `+0x3dc` (0.05 s for the player, 0.25 s for others; 2 frames of 33 ms in the logs). While a sprint's `0x64` stream runs, every frame |
| the Move/roll action handlers | rolls: the goal is the roll's remaining distance ahead, re-planned every frame (a straight leg in the open) |

Plans are only made for the local player's own movement; other entities use the same code. Nothing here is server-side or sent: the server only sends the direction/target, the client plans.

## The search (`0x141ed0510`, wrapper `0x141ece1d0`, A* fallback inside)
Signature (as logged by the hook): `(goal*, entity, params*, ?, out path*, out count*, out flag*, ...)`; `params` holds `min_dist`, `max_dist` (wanted distance to the goal; 0, 0 = the goal cell), `radius` (stop that far from the goal, -1 none) and flags. Returns whether the goal was reached; the path is written goal-first, start last, at most 0x88 cells.

- **Grid.** Terrain `+0xd0/+0xe8/+0x100/+0x118` are four 4-bit layers (`ceil(w/2)` bytes per row; the DLL exports them as `terrain_grid_<hash>.bin`, [../architecture/client-dll.md](../architecture/client-dll.md)). The search uses layer 0 (`FUN_141d0df50`): value 0 = wall, 1..4 = distance-to-wall ramp, 5 = clear (read as 255). A cell is **blocked when `level >= value`**; level -1 turns the test off.
- **Level (clearance).** `FUN_141dc7d00(Positioned + 0x1e8)`: the larger of the object sizes `+0x1f8` (`Positioned.object_size`, 0 for characters) and `+0x1f9` (`secondary_object_size`, 4) when the extra flags are set, else `+0x1f8`. A roll/sprint start (`FUN_141b77110`) sets `+0x1fc..+0x200` so that the level is 4, and the end (`FUN_141b773a0`) clears the flag at `+0x1ff`. So **from the roll's sprint until the sprint ends, cells with value <= 4 are walls**: the sprint keeps about 5 cells off every wall (cells with value 5), while walking and rolling use level 0 and hug walls. This is the "10 cells clearance" of the old problem 2. If the start cell itself is a wall at level > 0, the search is made again at level 0.
- **Window.** 272 x 272 cells (`0x110`) around the start (node array at `0x1445d8970`, 16 bytes per node: `u16` stamp, `u8` flags, ...). Cells outside the terrain, or outside the ellipse with foci start/goal and focal sum `L = clamp(2 * |start-goal|, 20, 136)` beyond radius `0.433 * L` of the midpoint, count as walls.
- **Costs.** Straight step 1, diagonal 1.4142 (`g`). `h` = distance to the goal after the min/max/radius adjustment plus `0.001 * |cross(goal - start, cell - goal)|` (prefers the straight line). Open list: `std::push_heap`/`pop_heap` on `f = g + h` (MSVC implementation: ties are decided by its sift order, so the port reproduces it exactly).
- **Jump point search with bounded jumps.** Expanding a node jumps along its direction (and the two forced-neighbour directions) until a forced neighbour, the goal, an obstacle or the jump limit; diagonals first check both straights. The limit is `min(cap, 3 + 5 * max(0, dot(unit_dir, cell - goal) / (max_dist + radius + h)) + 0.2 * (g + h - h_start))` (times 0.7071 for diagonals), `cap = clamp(0.33 * h_start, 5, 22)`: nodes end up a few cells apart and the open list is small. The start node expands in all 8 directions.
- **Result.** If no node reaches `h = 0`, the node with the lowest `h` is the end. The path is rebuilt through the parents, keeping only cells where line of sight (Bresenham over the grid, blocked cells and window edge stop it) to the last kept cell ends; a second pass takes windows of 5 and drops the cells between two visible ones.
- **Start cell.** The client plans from its own grid cell, which is not the cell of the position but a digital line that trails it by up to half a cell (see below).

## Planning cell
The client's grid cell (Positioned `+0x444/+0x448`) is not `floor(world / 10.8696)`: it advances along its current line (`FUN_141ec86a0`, `FUN_141ec8cb0`) one whole cell once the world position is a cell away from the line's start, so while moving it trails the position by up to half a cell in the movement direction. Plans start from that cell. The port reproduces it with `trunc(position - 0.5 - line_start)` (66 % of 643 logged plans start in exactly the cell it gives, 45 % in the cell of the position); where it lands in a wall the cell of the position is used.

## Evidence
- DLL hook `path_log` (`modules/client-hook`, `[features] path_log` or `POE_HOOK_PATH_LOG=1`, signature `path_plan`): one `PATHPLAN` line per plan of the local player with goal, start, the 7 params, the level bytes and the waypoints. 541 distinct plans from walks, sprints (level 4), rolls and clicks around obstacles and diagonal walls on `MapRiverside`/`G1_1` are the fixture `modules/server/crates/poe-game/tests/fixtures/clientpath_plans.json`; the port returns **536 of 541 exactly**, the other 5 differ in one waypoint next to the start (the last line-of-sight test of an entity already moving along its line, not ported). Test: `cargo test -p poe-game --test clientpath` (needs the area's grid in `dumps/0.5.5e.hotfix1/terrain/grids-keep/` or `grids/`, skips otherwise).
- Positions: the client's per-frame positions follow the logged paths at walking speed with a mean error of 0.013 cells over 246 plan intervals (7 walk cases; position of the frame after the plan plus the first two legs).
- A Python prototype of the same search was run against the logs first (441 of 476 exact, then the level-0 fallback and the 5 known deviations).

## Not ported / open
- Dynamic obstacles (other entities, `FUN_141ed1da0` / `FUN_141ed2330` mark them as walls in the window) and the second grid layers.
- The A* fallback branch (`0x141ece1d0`) used for monsters/pets and when the main search is switched off.
- The exact line-of-sight variant used when the entity is already moving along a line (5 of 541 plans).
- The values `min_dist`, `max_dist`, `radius` other than the player's (0, 0, -1) — monsters keep distance to targets.
