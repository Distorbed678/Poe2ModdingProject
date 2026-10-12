# Milestones

| # | Goal | Status |
|---|---|---|
| M0 | Foundation: repo, tools, workspace, config, data access, client DLL proxy, deploy tooling | in progress |
| M1 | Login → character select/creation, league selection | – |
| M2 | Instance server transfer, first zone loads with the player | – |
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
- [ ] Verified: the client loads the DLL and connects to the local login server
