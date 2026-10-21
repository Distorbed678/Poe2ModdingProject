# Captures: official servers vs ours, and the live client log

## Official servers vs ours

Comparison of the debug DLL's captures of official sessions (2026-10-14: login, character
delete/create, the Riverbank intro, moving, sprinting, a bow pickup, three kills) with our
server's traffic, and what was changed to match. Tools: the DLL's Packets/Protocol/Coverage tabs,
`modules/protocol/poe-proto/examples/check_capture.rs` (decodes every captured message with our layouts;
`--dump <channel> <in|out> <0xop>` prints them) and `scripts/re/packets_diff.py`.
Run it inside the Poe2Protocol submodule (it is not a member of the main workspace): `cd modules/protocol && cargo run -p poe-proto --example check_capture -- opcodes/0.5.5e <packets.jsonl> [--show N]`, with captures from `dumps/0.5.5e/captures/`.

Compare a capture with one from our server: `third_party/pyenv/bin/python scripts/re/packets_diff.py <official>/packets.jsonl <ours>/packets.jsonl [--channel instance] [--seq 40]`. How a capture is made: [../guides/setup.md](../guides/setup.md#official-servers-read-only), [../architecture/client-dll.md](../architecture/client-dll.md#packet-capture-packets-packetsrs).

## Layout check
Every official message we have a decoder for decodes without errors or leftover bytes, except
two whose richer forms we do not support (our server sends the simple forms): `AreaInfo`
optional blocks and `AddInventory` items.

## Movement capture 2026-10-20
`dumps/0.5.5e/captures/official/2026-10-20_18-41-14_336/`: a read-only official session that followed a checklist for the movement gaps (standing and chained rolls, taps around the sprint threshold, stops during a sprint, attacks during rolls, sprints into walls, click-to-move and held clicks with rolls, an input-mode toggle). The DLL's movement log was on, so `poe-hook.log` has the client's position per frame (`frame #n ... world ... step ... grid`, same frame numbers as the capture) and the client's action starts/ends (`action start slot 0: DodgeRoll`). `check_capture` decodes every movement message with 0 errors and 0 leftover bytes (only `AddInventory` items mismatch, as before). Results: [../protocol/movement.md](../protocol/movement.md) (index at "Session 2026-10-20"), fixtures `crates/poe-game/tests/fixtures/` (`scripts/re/movement_fixtures.py`).

## Town capture with other players 2026-10-20
| Session | What | Results |
|---|---|---|
| `dumps/0.5.5e/captures/official/2026-10-20_21-51-49_336/` (read-only official; 29,412 instance messages) | The Riverbank cleared, then Clearfell Encampment for 160 s with three other players (WASD and click-to-move, waypoint, stash, NPCs). Party invites (one cancelled by us), a whisper and a cancelled trade request. The DLL's `actions` log is on (action start/end with the client's frame number: client frame + 4527 = capture frame in the town; no per-frame positions) | Remote players' `AddObject`, removal, relayed actions, party and trade: [../protocol/entities.md](../protocol/entities.md#other-players-add-actions-remove-party-and-trade-wp35). New decoders in `poe-proto` (`player_blob.rs`, `remote.rs`) decode every message of the session without errors or leftover bytes except `AddInventory` items, `BuffAdded` flag `0x10` and `SetAimTarget` kind 1 (a `u32 entity` after `x, y`); anonymised fixtures in `modules/protocol/poe-proto/tests/data/`. Names and chat in the capture are not copied anywhere |

Click-to-move in this session (local player, `f8667-9183`, 7 presses and 189 held repeats): two single-click walks reached their target with no message at arrival (`f8988` 44.6 cells in 34 frames, `f9027` 32 cells in 26 frames: 1.2-1.3 cells per frame; the next click's `StartAction` starts exactly at the previous target); a held-click's server restarts (`StartAction` Move, flags 0, header `(self, 9, n)`) came 13-17 frames and 13.6-17.7 cells apart. These findings are not yet merged into [../protocol/movement.md](../protocol/movement.md) (gap 2).

## Fixed (2026-10-14/15)
| What | Official | Ours before | Now |
|---|---|---|---|
| Local player index | 1 (`LocalPlayers` entry, `PlayerEntity 01 01`, first byte of `AddInventory`); the client copies it into its messages (`0062 01 ..`, `0063 01 ..`) | 0 | 1 |
| Inventories at entry | 118 inventories (ids 1-94, 122-144) with real sizes and a flags byte (0x00/01/03/04/60/61/65) | ~120 from `Inventories.dat`, mostly 1x1, flags 0 | the official table, `modules/protocol/opcodes/0.5.5e/inventories.tsv`. **Without ids 122-144 the client never starts the world** (found while bisecting) |
| `MoveDirectionUpdate` (`0x64`) | header (entity, entity, 9 = Actor component, 0), last byte `0xf1` | header (entity, 0, 0, 0), last byte 0 | official |
| Cursor facing | client `0x0062` AimTarget (mode 2 = cell), server echoes `0x01dc` SetAimTarget one frame later | DLL cursor report + SetOrientation lock | official. The client only sends cells in WASD input mode and while the Player component is in combat stance, which official servers set with a `0x02f9` ComponentDelta on the first Move; ours does it on the first movement or skill. In click-to-move mode the client sends no AimTarget, so the DLL path stays for it |
| `0x0062` size | 4, 8 or 12 bytes by mode | fixed 4 in `client_body_sizes` (would misread the batch) | decoded by mode |
| Login `0x003c` | four zero u32 before the key-exchange reply | not sent | sent |
| Login `0x0017` | u16 `0x0014` right after `InstanceDetails` | not sent | sent 2026-10-15, removed 2026-10-17: it makes the client `shutdown()` its login connection, which our transfers need (`[network.login] close_after_handoff`) |
| `AddObject` header (player) | (0xffffffff, 0, 0) | (0, 0xff, 0) | official; verified with and without the first-visit intro (the old (0, 0, 0) crash does not come back) |
| AreaInfo league (+0x38) | the character's league | empty | the character's league |
| InitialState | flags 7, four stash tabs "1".."4", character flags `68 40 00`, player entry `0x40`/`7` | flags 0, no tabs, `00 40 00`, `0`/`0` | official (`[accounts] stash_tabs`) |
| Entry order | `LocalPlayers`, `PlayerEntity`, inventories | `PlayerEntity` after the inventories | official |

## Open (by priority)
1. **Player component deltas at entry**: official servers send flag words (`0x0002`, 5 entries;
   key 0x08 added after the first Move) and combat stance 1, then stance 0 with `+0x3e7 = 1`;
   after kills, deltas with mask `0x0201` (experience 17, 34, 51 and +0x209 set). Ours sends only the
   stance.
2. **Messages official servers send at entry that we do not** (all decoded sizes in the
   session's `protocol.tsv`). Known so far: `0x0021` = MonsterVarieties rows to preload (`u8`,
   `u16` count, `u16`, count x `u16` row, count x `u8`; session dispatcher `0x142121f70` case
   `0x142123a2c`, needed for monsters); `0x0066` = skill bar (`u8` local, `bool`, `u8` count,
   64-byte records: `u32` skill id, ..., reader `0x142153de0`; needed for skills); `0x005a` = `u32`
   + 22 bytes kept on the session (+0x2318, all zero officially); `0x00c6` = a social-list entry
   (account name). Others: `0x0021` (u16 id list), `0x00c6` (account name `Name#1234`),
   `0x0043`, `0x0045`, `0x018b`, `0x018c`, `0x0229`, `0x0226`, `0x0237`, `0x0066` (389 bytes,
   looks like the skill bar), `0x003a`, `0x02db`, `0x00c4`, `0x02d8`, `0x0325` (5.5 KB),
   `0x005a`, `0x0329`, `0x0320`, `0x031d`, `0x0271`, `0x00de` (per-inventory data),
   `0x02a6`, `0x0352`, `0x0358`, `0x0222`.
3. **AreaInfo** 221 vs 39 bytes: the layout is now known ([../protocol/instance.md](../protocol/instance.md)); missing are the
   object-type preload list (needs the area's spawns), a u16 key list, scalar bytes and the
   optional blocks.
4. **CharacterList**: experience (+0x40) is now sent. Still missing for characters that have
   played: `unk_51` 0x40, the 91-byte gear blob (component 8 delta format) and the stat pairs.
5. Skills, monsters, items, the sprint buff: later work; the captures have the sequences.

## Live client log (`Client.txt` from real play, read only)

The user's log from official sessions (rules and location: [../../CLAUDE.md](../../CLAUDE.md)) shows what a correct session looks like.
- `scripts/re/clientlog_entries.py`: per-entry timelines (area, seed, first log lines with ms offsets).
- `scripts/re/clientlog_terrain.py`: (area, level, seed, tile hash, doodad hash) test vectors into `dumps/0.5.5e/terrain/vectors/terrain-vectors.tsv`. That gives 1,744 vectors across 260 areas, for the terrain generator ([terrain/README.md](terrain/README.md)); towns and hideouts always use seed 1 (details there).
- A live `G1_1` entry follows the same client log order as ours up to `[SCENE] Set Source [The Riverbank]`. Live then shows `Joined guild …` and `InstanceClientSetSelfPartyInvitationSecurityCode = 0` (both server messages we don't send), `Tried to SetExtraActivationRangeAroundTile() after level generation was complete` (client-side noise, see [terrain/README.md](terrain/README.md#client-side-facts-offline-analysis-2026-10-13)), and the washed-up intro dialogue ("Wounded Man: By the First Ones! You're alive!").
- Log lines of interest: `Connected to <host> in Nms.` is the login connection; `Got Instance Details from login server` the hand-off; `Connecting to instance server at <ip>:<port>`; `Generating level N area "<AreaId>" with seed <u32>` the client generating its terrain.
