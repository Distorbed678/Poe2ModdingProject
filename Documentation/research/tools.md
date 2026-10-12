# Tool survey (2026-10)

| Tool | Use in project | Notes |
|---|---|---|
| poe_data_tools (adamthedash), Rust, Apache-2.0 | **Library in `poe-data`**, CLI in Tools/ | PoE2 0.x support. Pure-Rust Oodle (`oozextract`). Parsers for arm/tgt/tgm/tsi/et/gft/ecf. Needs nightly. |
| dat-schema (poe-tool-dev) | Runtime schema | `schema-poe2.min.json`, with 0.5.5 additions. Many PoE2 columns unnamed. |
| LibGGPK3 (aianlinb), C#, AGPL | Tools/ GUI only | Bundle browser and extractor. Never linked. |
| pathofexile-dat / poe-dat-viewer (SnosMe) | Optional | Web viewer, handy for naming unknown columns. |
| annalithic/poeformats, C# | Reference | `.tgr` (Graph.cs) and `.tdt` parsers; needed for terrain (M4). |
| KiwiNet (Crypto137), C#, AGPL | Reference only | PoE1 0.8.8 emulator. |
| exileSniffer (ncatlin) | Reference only | PoE1 3.x protocol dissector, 2018. |
| retour | Rejected | libudis86 C dependency fails under clang-cl. Replaced by `ilhook`. |
