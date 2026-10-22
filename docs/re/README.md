# Reverse engineering

How the client is analysed and where the results are written down. The rules (static analysis only, official servers read-only, never the Steam install) are in [../../CLAUDE.md](../../CLAUDE.md).

**Scope of what is committed.** Everything under `docs/re/` is research about the 0.5.5e client: addresses, struct offsets, opcode meanings, formats and function names, written in our own words. No decompiler output, game code, assets, bundles or data tables are pasted here or anywhere in the repositories (the datamined dumps stay in the git-ignored `dumps/<version>/`). The addresses are for build 0.5.5e only and change with every client update ([update-compatibility.md](update-compatibility.md)). The project is unofficial and not affiliated with Grinding Gear Games ([disclaimer](../../README.md#disclaimer)).

## Reverse engineering access
- **Ghidra MCP** (`mcp__ghidra__*`). Call `connect_instance("Poe2ModdingProject")` first; `/PathOfExileSteam.exe` (image base 0x140000000) is open and analyzed.
  - Use targeted calls (`search_strings`, `get_xrefs_to`, `decompile_function`).
  - Huge functions (packet factories) give 20k+ token decompiles, so prefer `disassemble_function` on ranges or search by constant.
  - Rename functions and add comments in Ghidra as understanding grows, and record stable findings in [addresses-0.5.5e.md](addresses-0.5.5e.md).
- **x64dbg / dynamic debugging**: not installed. **Ask the user** when it would help, for example runtime values such as keys, crash triage, or verifying a hook.
- The client DLL can log plaintext packets (trace feature) for analyzing our own sessions: [../architecture/client-dll.md](../architecture/client-dll.md#packet-capture-packets-packetsrs).

## Offline scripts (`scripts/re/`, no Ghidra needed)
Run them with the project's Python environment, which `third_party/fetch-tools.sh` creates ([../guides/setup.md](../guides/setup.md#tools)): `third_party/pyenv/bin/python scripts/re/<script>.py <args>`. Addresses are Ghidra virtual addresses in hex; `pe.py` holds the shared PE-reading helpers. Most scripts read `GameFiles/PathOfExileSteam.exe`.

| Script | Use |
|---|---|
| `disasm.py <start_va> <length>` | Disassemble a range |
| `find_disp.py <disp> [min max]`, `find_imm.py <imm> [min max]` | Find instructions using a struct-field displacement or a 32-bit immediate |
| `sig_gen.py <spec> [exe]` | Generates `modules/client-hook/signatures/<version>.toml` from a spec of Ghidra addresses (`sig_spec_0.5.5e.py`): unique wildcarded function prefixes, anchored site/ref patterns, struct-offset captures. Verify with `cargo run --release -p poe-hook-sig --example resolve -- <exe>` in `modules/client-hook` |
| `version.py` | (module) The client version the scripts work on: the newest `modules/protocol/versions/<version>.toml`, or `$POE_VERSION` |
| `find_switches.py <func_start> <length>` | List the jump-table switches in a function and the opcodes each case handles; prints the arguments `dump_factory.py` needs (base, count, dword and byte table RVAs, default case) |
| `case_strings.py <func_start> <len> [window]` | Strings referenced near each switch case |
| `dump_factory.py <base> <count> <table_rva> <default_va> [out.json] [--bytes <byte_table_rva>]` | Dump a packet factory (opcode → case, size, vtable, deserializer) into `dumps/<version>/ghidra/`; for the 0.5.5e instance factory (`FUN_142170380`): `dump_factory.py d 35f 217305c 142173040`; the login factory (`0x14214fa60`, index table): `dump_factory.py 0 47 215011c 142150103 dumps/0.5.5e/ghidra/login_factory_s2c.json --bytes 2150190`. The dumps are the input of the opcode drift baseline (`cargo xtask check-build <exe> --write-fingerprints`, [../guides/new-game-version.md](../guides/new-game-version.md)) |
| `find_opcode_stores.py`, `find_packet_senders.py <vt_lo> <vt_hi>`, `find_stack_packets.py <min> <max> [opcode…]` | Find packet constructors and senders (opcode stored at object +8) |
| `packet_layout.py <opcode…>` | Recover a packet's layout from its serializer and deserializer; the vtable-to-opcode map is in `dumps/0.5.5e/ghidra/packet_classes_0.5.5e.json` |
| `export_symbols.py` | Build the DLL's symbol table (`modules/client-hook/poe-hook-core/assets/symbols.tsv`) from Ghidra's named functions (exported into `dumps/<version>/ghidra/`; the version is the newest `modules/protocol/versions/*.toml`, see `version.py`) and the address tables in [addresses-0.5.5e.md](addresses-0.5.5e.md); rerun it after documenting new addresses |
| `packets_diff.py`, `clientlog_entries.py`, `clientlog_terrain.py` | Compare captures, and read the live client log: [captures.md](captures.md) |
| `movement_sequences.py list\|events\|scen\|never <session>` | Movement and skill message sequences in a capture, with frame offsets against `0x2f4` ticks: [../protocol/movement.md](../protocol/movement.md) |
| `movement_trace.py` | Movement hitches in a server movement trace: [../protocol/movement.md](../protocol/movement.md#movement-trace) |
| `sweep_drive.py`, `send_dev.py` | Drive a terrain seed sweep, send a server command over the debug link: [terrain/README.md](terrain/README.md#tools), [../guides/setup.md](../guides/setup.md#unattended-client-tests) |

## Where findings go
| Topic | File |
|---|---|
| Function and struct addresses (Ghidra) | [addresses-0.5.5e.md](addresses-0.5.5e.md) |
| What breaks on a new client build, ranked, with proposed fixes (signatures, `modules/protocol/versions/<ver>.toml`, `check-build`) | [update-compatibility.md](update-compatibility.md) |
| Which Winsock APIs the client uses to connect (what the DLL's network guard hooks) | [network-apis.md](network-apis.md) |
| Login key exchange, Salsa20, the HNC side channel, the DLL bypass | [crypto.md](crypto.md) |
| Steam vs email/password login, the `0x0003` credentials packet, Remember Me tokens, forcing the standalone screen | [standalone-login.md](standalone-login.md) |
| Terrain generator (`.tgr`, `.dgr`, RNG, hashes) | [terrain/](terrain/README.md) |
| Official sessions vs ours, the live client log | [captures.md](captures.md) |
| Game files, `.datc64` tables, the dumps folder | [datamining.md](datamining.md) |
| Wire formats | [../protocol/](../protocol/framing.md) |
