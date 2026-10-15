# Official servers vs ours

Comparison of the debug DLL's captures of official sessions (2026-10-14: login, character
delete/create, the Riverbank intro, moving, sprinting, a bow pickup, three kills) with our
server's traffic, and what was changed to match. Tools: the DLL's Packets/Protocol/Coverage tabs,
`crates/poe-proto/examples/check_capture.rs` (decodes every captured message with our layouts;
`--dump <channel> <in|out> <0xop>` prints them) and `tools/re/packets_diff.py`.

## Layout check
Every official message we have a decoder for decodes without errors or leftover bytes, except
three whose richer forms we do not support (our server sends the simple forms): `AreaInfo`
optional blocks, `InitialState` 0x68-byte records, `AddInventory` items.

## Fixed (2026-10-14/15)
| What | Official | Ours before | Now |
|---|---|---|---|
| Local player index | 1 (`LocalPlayers` entry, `PlayerEntity 01 01`, first byte of `AddInventory`); the client copies it into its messages (`0062 01 ..`, `0063 01 ..`) | 0 | 1 |
| Inventories at entry | 118 inventories (ids 1-94, 122-144) with real sizes and a flags byte (0x00/01/03/04/60/61/65) | ~120 from `Inventories.dat`, mostly 1x1, flags 0 | the official table, `protocol/0.5.5e/inventories.tsv`. **Without ids 122-144 the client never starts the world** (found while bisecting) |
| `MoveDirectionUpdate` (`0x64`) | header (entity, entity, 9 = Actor component, 0), last byte `0xf1` | header (entity, 0, 0, 0), last byte 0 | official |
| Cursor facing | client `0x0062` AimTarget (mode 2 = cell), server echoes `0x01dc` SetAimTarget one frame later | DLL cursor report + SetOrientation lock | both: AimTarget/SetAimTarget implemented; the DLL path stays until the client sends AimTarget cells (see open items) |
| `0x0062` size | 4, 8 or 12 bytes by mode | fixed 4 in `client_body_sizes` (would misread the batch) | decoded by mode |
| Login `0x003c` | four zero u32 before the key-exchange reply | not sent | sent |
| Login `0x0017` | u16 `0x0014` right after `InstanceDetails` | not sent | sent |
| `AddObject` header (player) | (0xffffffff, 0, 0) | (0, 0xff, 0) | official; verified with and without the first-visit intro (the old (0, 0, 0) crash does not come back) |
| Entry order | `LocalPlayers`, `PlayerEntity`, inventories | `PlayerEntity` after the inventories | official |

## Open (by priority)
1. **Cursor reports.** Our client sends `0062 01 01` (mode 1) but never mode 2 cells. On official
   servers mode 2 starts right after the server's answer to the first skill use: `0x02f9`,
   `0x02a6`, `0x01ec`, `SkillStatsUpdate` x4, `StatsUpdate`, `StartAction`, `0x01ca`, `0x01ae`,
   `0x01b2`, `0x02f9`. One of these enables it.
2. **Messages official servers send at entry that we do not** (all decoded sizes in the
   session's `protocol.tsv`): `0x0021` (u16 id list), `0x00c6` (account name `Name#1234`),
   `0x0043`, `0x0045`, `0x018b`, `0x018c`, `0x0229`, `0x0226`, `0x0237`, `0x0066` (389 bytes,
   looks like the skill bar), `0x003a`, `0x02db`, `0x00c4`, `0x02d8`, `0x0325` (5.5 KB),
   `0x005a`, `0x0329`, `0x0320`, `0x031d`, `0x0271`, `0x00de` (per-inventory data),
   `0x02a6`, `0x0352`, `0x02f9`, `0x0358`, `0x0222`.
3. **AreaInfo** 221 vs 39 bytes, **InitialState** 121 vs 29 bytes (richer forms to decode).
4. **CharacterList**: `unk_51` (64), `unk_40` (85) and a 92-byte appearance/gear blob; league
   flag row of the current league.
5. Skills, monsters, items, sprint buff: later milestones; the captures have the sequences.
