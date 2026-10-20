# Official servers vs ours

Comparison of the debug DLL's captures of official sessions (2026-10-14: login, character
delete/create, the Riverbank intro, moving, sprinting, a bow pickup, three kills) with our
server's traffic, and what was changed to match. Tools: the DLL's Packets/Protocol/Coverage tabs,
`modules/protocol/poe-proto/examples/check_capture.rs` (decodes every captured message with our layouts;
`--dump <channel> <in|out> <0xop>` prints them) and `scripts/re/packets_diff.py`.
Run it inside the Poe2Protocol submodule (it is not a member of the main workspace): `cd modules/protocol && cargo run -p poe-proto --example check_capture -- opcodes/0.5.5e <packets.jsonl> [--show N]`, with captures from `dumps/0.5.5e/captures/`.

## Layout check
Every official message we have a decoder for decodes without errors or leftover bytes, except
two whose richer forms we do not support (our server sends the simple forms): `AreaInfo`
optional blocks and `AddInventory` items.

## Fixed (2026-10-14/15)
| What | Official | Ours before | Now |
|---|---|---|---|
| Local player index | 1 (`LocalPlayers` entry, `PlayerEntity 01 01`, first byte of `AddInventory`); the client copies it into its messages (`0062 01 ..`, `0063 01 ..`) | 0 | 1 |
| Inventories at entry | 118 inventories (ids 1-94, 122-144) with real sizes and a flags byte (0x00/01/03/04/60/61/65) | ~120 from `Inventories.dat`, mostly 1x1, flags 0 | the official table, `modules/protocol/opcodes/0.5.5e/inventories.tsv`. **Without ids 122-144 the client never starts the world** (found while bisecting) |
| `MoveDirectionUpdate` (`0x64`) | header (entity, entity, 9 = Actor component, 0), last byte `0xf1` | header (entity, 0, 0, 0), last byte 0 | official |
| Cursor facing | client `0x0062` AimTarget (mode 2 = cell), server echoes `0x01dc` SetAimTarget one frame later | DLL cursor report + SetOrientation lock | official. The client only sends cells in WASD input mode and while the Player component is in combat stance, which official servers set with a `0x02f9` ComponentDelta on the first Move; ours does it on the first movement or skill. In click-to-move mode the client sends no AimTarget, so the DLL path stays for it |
| `0x0062` size | 4, 8 or 12 bytes by mode | fixed 4 in `client_body_sizes` (would misread the batch) | decoded by mode |
| Login `0x003c` | four zero u32 before the key-exchange reply | not sent | sent |
| Login `0x0017` | u16 `0x0014` right after `InstanceDetails` | not sent | sent 2026-10-15, removed 2026-10-17: it makes the client `shutdown()` its login connection, which our transfers need (`[login] close_after_handoff`) |
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
   `0x142123a2c`, monsters milestone); `0x0066` = skill bar (`u8` local, `bool`, `u8` count,
   64-byte records: `u32` skill id, ..., reader `0x142153de0`; skills milestone); `0x005a` = `u32`
   + 22 bytes kept on the session (+0x2318, all zero officially); `0x00c6` = a social-list entry
   (account name). Others: `0x0021` (u16 id list), `0x00c6` (account name `Name#1234`),
   `0x0043`, `0x0045`, `0x018b`, `0x018c`, `0x0229`, `0x0226`, `0x0237`, `0x0066` (389 bytes,
   looks like the skill bar), `0x003a`, `0x02db`, `0x00c4`, `0x02d8`, `0x0325` (5.5 KB),
   `0x005a`, `0x0329`, `0x0320`, `0x031d`, `0x0271`, `0x00de` (per-inventory data),
   `0x02a6`, `0x0352`, `0x0358`, `0x0222`.
3. **AreaInfo** 221 vs 39 bytes: the layout is now known (instance.md); missing are the
   object-type preload list (needs the area's spawns), a u16 key list, scalar bytes and the
   optional blocks.
4. **CharacterList**: experience (+0x40) is now sent. Still missing for characters that have
   played: `unk_51` 0x40, the 91-byte gear blob (component 8 delta format) and the stat pairs.
5. Skills, monsters, items, sprint buff: later milestones; the captures have the sequences.
