# Documentation index

A private server for Path of Exile 2 (client **0.5.5e**), written in Rust. Start with [setup.md](setup.md), then read [architecture.md](architecture.md).

| Document | Contents |
|---|---|
| [setup.md](setup.md) | Toolchain, tools, building, deploying to `GameFiles/`, running client and server |
| [architecture.md](architecture.md) | Crates, servers, data flow, design rules |
| [milestones.md](milestones.md) | Roadmap and current status |
| [modding.md](modding.md) | Server and client modding hooks (planned and existing) |
| [datamining.md](datamining.md) | Reading Bundles2 and `.datc64` tables, the schema, useful tables |
| [protocol/official-comparison.md](protocol/official-comparison.md) | Official servers vs ours: what differs, what was fixed, what is open |
| [client-dll.md](client-dll.md) | The client DLLs: version.dll loader, hot-reloadable debug core, modes (read-only), features and config |
| [protocol/framing.md](protocol/framing.md) | Wire format: encodings, framing, encryption |
| [protocol/login.md](protocol/login.md) | Login protocol messages |
| [protocol/instance.md](protocol/instance.md) | Instance protocol: opening sequence, area info, inventories, spawn |
| [protocol/entities.md](protocol/entities.md) | Entity initial-state blobs (player components, actor skills) |
| [protocol/movement.md](protocol/movement.md) | Lockstep movement: WASD, click-to-move, skills and dodge roll |
| [re/crypto.md](re/crypto.md) | Login key exchange, Salsa20 key/IV layout, the DLL bypass |
| [re/findings-2026-10-13.md](re/findings-2026-10-13.md) | Offline analysis: live-log comparison, client packet classes, rendering/spawn notes |
| [re/addresses-0.5.5e.md](re/addresses-0.5.5e.md) | Reverse-engineering findings (Ghidra addresses) for 0.5.5e |
| [research/poe1-prior-art.md](research/poe1-prior-art.md) | What public PoE1 research says about the protocol |
| [research/tools.md](research/tools.md) | Survey of datamining and RE tools |
