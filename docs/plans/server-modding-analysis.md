# Server modding at the level of client modding: analysis and plan

Status: analysis (read-only WP, 2026-10-22); the foundation work package is built (see "Status per work package" in section 7), the rest is not. It answers the request: mods are classified client, server or both; a server tells the launcher which mods a player needs; the launcher downloads and switches them (with a setting); and server-side modding is brought to the level of client modding.

Sources read: [../guides/modding.md](../guides/modding.md), [../../modules/client-hook/docs/mod-manifest.md](../../modules/client-hook/docs/mod-manifest.md), the client-hook crates (`poe-hook-mod-api`, `poe-hook-mods`, `poe-hook-deploy`, `examples/sample-mod`), the launcher (`src-tauri/src/mods.rs`, `Mods.svelte`, [../guides/launcher.md](../guides/launcher.md)), the server crates and [../architecture/instance-world.md](../architecture/instance-world.md).

## 1. Summary

- **Client modding is a real, tested system**: a manifest spec, a loader that maps native DLLs in a stored order, a versioned C ABI, dependency/conflict/version checks, a launcher that installs (folder or zip), enables, orders and removes mods, an install tool for developers and a sample mod. Its gaps: no hooks in the ABI (a mod finds code with `sig_addr` and patches by itself), `[data]` overlays are parsed but not applied, no unload/hot reload, no mod download, no hashes or signatures.
- **Server modding does not exist as a system.** Three loose pieces exist: a file-override API in `poe-data` that nothing calls (not wired to config), one hard-coded command registry (a `const` slice), and config/game-data driven behaviour. The "hook bus (`poe-mod-api`)" is only a planned tracker row. There is no mod folder, manifest, loader, API crate, versioning, ordering, enable/disable or distribution for the server.
- **The server cannot yet offer rich events.** Entities are players only (no NPCs, monsters or loot), so "entity spawn" and "item drop" hooks have nothing to attach to. Realistic first events: server start/stop, login/logout, chat and commands, instance create/close, area enter/leave, tick, movement input.
- **Recommended design** (details in 4 and 5): one `mod.toml` for both sides with `side = "client" | "server" | "both"`; one package (a zip with the mod folder at its root, the same zip the launcher installs today) that may carry both halves; a server mod loader with the same folder/`state.toml`/plan logic as the client's; a **data-only tier first**, then a **WebAssembly code tier** (wasmtime, JSON messages, deny-by-default permissions, fuel limits) behind a `ModRuntime` trait; the server advertises its required client mods in `GET /api/v1/server` and serves the zips itself (plus optional URLs); the launcher compares, asks, downloads, verifies the SHA-256 and writes `state.toml` per server, with a three-level setting (Off / Ask / Auto).
- **Security headline**: auto-downloading a mod that contains a DLL from a server is remote code execution on the player's PC. The hash from the same server over the same channel proves nothing about safety. The design therefore asks once per (server, mod) for anything with code, never auto-installs native code without that approval, and requires https (or loopback) for code downloads.

## 2. Client modding today

### 2.1 Capabilities

| Area | What exists | Where |
|---|---|---|
| Package | A folder `poe-hook-mods/<id>/` next to `PathOfExileSteam.exe` with `mod.toml`, an optional entry DLL, optional `data/`. A zip with `mod.toml` at its root (or inside exactly one top folder) is the distribution format; the launcher unpacks it with entry, size (4 GiB) and path/symlink limits. | `mod-manifest.md`, `launcher/src-tauri/src/mods.rs` (`install_zip`, `install_dir`) |
| Manifest | `manifest_version = 1`, `id`, `name`, semver `version`, metadata; `[game] versions, hook_api`; `[load] after, before, conflicts`; `[dependencies] id = semver req`; `[entry] dll`; `[data] tables, opcodes, config_defaults`. Unknown keys are ignored and logged. | `poe-hook-mods/src/manifest.rs` |
| Enable and order | `poe-hook-mods/state.toml` (`enabled = [...]`, load order). Launcher: dependencies enabled first (semver check), conflicts and cycles refused, topological order with the user's arrangement for free choices, *Auto order*, move up/down, remove. The manifest is never modified. | `ModStore::{enable,disable,move_to,auto_order,remove}` |
| Loading | `version.dll` loader, once per process, after the core is up: reads `state.toml`, validates each mod (id, manifest, folder name, ABI) **before mapping its DLL**, checks dependencies and conflicts, `LoadLibraryExW`, optional `poe_mod_abi_version`, `poe_mod_init(&HookApi)`. Every failure is a log line and a skip. | `poe-hook/src/mods.rs`, logic in `poe-hook-mods/src/plan.rs` |
| ABI | `poe-hook-mod-api` (dependency-free, `repr(C)`, `HOOK_API_VERSION = 1`): `HookApi` (144 bytes) with `abi_version`, `size`, `ctx`, `mod_id/mod_dir/exe_dir`, `game_version`, `build_revision`, `build_verified`, and functions `log`, `read_only`, `sig_addr`, `feature_ok`, `field`. Appending fields needs no bump (mods read `size`); anything else bumps. A layout test pins the struct. | `poe-hook-mod-api/src/lib.rs` |
| Lifecycle | `poe_mod_init` only. No shutdown, no unload, no hot reload of mods (the debug core itself is hot-reloadable). | spec "Open points" |
| Versioning | mod `version` semver; `game.versions` exact strings (launcher refuses without override, loader warns); `game.hook_api` must equal the loader's ABI (older and newer both refused with a reason); `dependencies` semver. | `compat()`, `check_abi()` |
| Safety | In official mode (or an already set read-only latch) no mod loads and `state.toml` is not even read; mods must watch `HookApi::read_only`. A mod is native code in the game process: the loader and launcher cannot make it safe (documented trust note). The launcher checks manifest and paths only. | `modding.md`, `mod-manifest.md` |
| Developer tooling | `cargo xtask deploy-mod [dir] [--dll --disable --no-build]`, `deploy-client --sample-mod`, `revert-client`; sample mod `examples/sample-mod` (Rust cdylib). | `poe-hook-deploy/src/mods.rs` |
| Tests | Manifest parser, state, load plan (order, skipping, official mode), ABI check, dependency/conflict set, installer, sample manifest validity; launcher unit tests for the store. | spec "Tests" |

What a client mod can actually do is limited by the ABI: it gets logging, the latch, the signature table (addresses of known game functions for this build) and struct offsets. Hooking, patching and reading game memory is the mod's own job with the addresses it was given. There are no events, packet hooks or asset-override hooks in ABI 1.

### 2.2 Packaging and distribution today

- The only distribution format is a zip (or folder) the user already has; the Mods page has *Install folder* and *Install zip* (file pickers). There is **no download** of mods, no index, no hash, no signature, no mod-level update check.
- The launcher's store holds **one version per id** in `<install>/poe-hook-mods/<id>/`; `state.toml` is global for the install (the install is per game version). Nothing is per server.
- The launcher already has the building blocks for downloads: it fetches the hook (`hook.rs`, GitHub releases `hook-v*`, `release.json`, file list check) and server packages (`.sha256` first, archive hash must match, size/link-checked unpacking), and it refuses to send anything to a non-loopback plain `http://` server before the user agrees (`plain_http_ok`).
- Duplicate code: the launcher has its own manifest parser and order resolver in `mods.rs` (1319 lines) next to the loader-side `poe-hook-mods`. Both implement the same spec; the open point "the launcher accepts a DLL mod without `hook_api`" is a symptom of that.

## 3. Server modding today

### 3.1 What exists

| Item | Reality |
|---|---|
| `GameFiles::push_override(dir)` (`crates/poe-data/src/files.rs`) | An API: directories searched before Bundles2, later wins, game tables are read by column name so a replaced `.datc64` flows through. **Nothing calls it**: no config key, no startup wiring, no test outside the crate. `modding.md` lists it as "exists"; it is a capability of the library, not a feature a server owner can use. |
| Command registry (`servers/instance-server/src/commands.rs`, run by `exec.rs`) | One registry for chat `!name` and the overlay form, with role check, log and audit, also used by the web API through `AdminControl`. It is a `const COMMANDS: &[Spec]` of `&'static str` fields: closed at compile time, so no mod can add a command. |
| Config-driven behaviour (`config/server.toml`, `poe-config`) | Ports, leagues, inventories, start areas and start tiles, innate skills, `[movement]` tuning, chat channel, spawn rules, accounts, web, security. This is the "data-only mod" surface that exists, but only as the single operator config, not as layered, switchable packages. |
| Traits | `Store`, `AccountStore`, `TicketRegistry`, `LoginSessions`, `InstanceDirectory`, `AdminControl`, `GridSource`/`Walkable`, plus `Files`/`EdgeView` inside `poe-world`. They separate infrastructure (storage, coordination, terrain). There is **no event, plugin or extension trait**. |
| Game data | `poe-data` reads tables by column name; movement and appearance get skill ids, buff rows, speeds from tables (`player.rs::sim_config`). Server logic is largely data-driven, which is what makes data mods feasible. |
| Packaging | `poe-server` release packages (`server-v*`, tar.gz/zip, `release.json`, `config/server.example.toml`, `versions/`, opcodes, schema, web admin). No `mods/` directory in the package. |

### 3.2 What does not exist

No mod folder, manifest, `state.toml`, loader, API crate (`poe-mod-api` is only in the tracker and `crates.md` as "planned"), event bus, versioning or compatibility check, load order, enable/disable, hot reload, distribution, or advertisement of mods in `GET /api/v1/server` (`ServerInfo` has name, version, cluster id, online, registration, login host/ports, region, uptime). No sandbox or permission model. No CLI (`poe-server account` exists; `poe-server mod` does not).

### 3.3 The shape of the code that mods would attach to

- **Process model**: one tokio runtime; `login-server`, `instance-server` and the web API in one process (`poe-server`), behind traits so they can be split later.
- **Instance actor** (`actor.rs`): one task per instance owns the `World`, the entity counter and the frame clock; sessions talk to it with `Cmd` messages (`Join`, `Input`, `Leave`, `Teleport`, `SetPaused`) and receive encoded `Frame`s. Sessions never touch the `World`. One `tokio` interval of `[instance] tick_ms` (33 ms) calls `World::tick()`; commands received before the tick are drained first.
- **World** (`poe-game`): knows no sockets or opcodes; typed input in, `WorldEvent`s out; frame clock only, never a wall clock (replay tests rely on it). Entities are players; components are fields (no ECS). `Event` is `#[non_exhaustive]`.
- **Chat and commands** run on the session task (`handle_client_message`, `run_command`); the web API reaches commands through `AdminControl`.
- **Live registry** (`live.rs`): who is online, kick, tell, broadcast, mute; `Social` handles party/trade requests.
- Consequences for mods: shared mutable access to a `World` is impossible by design (and should stay so); anything that changes the simulation has to go through the actor as a message or a hook that the actor calls on its own task; the server is deterministic per frame today and replay tests (`replay_tests.rs`, `poe-game/tests/replay.rs`) must stay bit-exact with mods off.

## 4. Gap list: client versus server modding

| Aspect | Client | Server | Gap |
|---|---|---|---|
| Classification | implicit (all client) | none | no `side` field anywhere |
| Package and folder | `poe-hook-mods/<id>/`, zip | none | need `mods/<id>/` in the server working directory, same zip |
| Manifest | `mod.toml` v1 | none | extend, do not fork (section 5.1) |
| Parser | `poe-hook-mods` + a copy in the launcher | none | one shared parser for loader, launcher, server |
| Loading | DLL loader at client start | none | `ModHost` at server start |
| API/ABI | `poe-hook-mod-api` `HookApi`, `repr(C)`, version 1 | none (`poe-mod-api` planned) | define events, commands, host functions, `server_api` version |
| Events/hooks | none in ABI 1 (patch it yourself) | none | the server can be richer than the client here: it owns the code |
| Lifecycle | init only | none | init, event callbacks, shutdown; hot reload later |
| Versioning/compat | `game.versions`, `hook_api`, semver deps | none | add `server_api`; check against server version and game version |
| Ordering | `state.toml` + topological order | none | reuse `plan` logic |
| Enable/disable | launcher UI, `state.toml` | none | `poe-server mod ...` CLI, web API, web admin, launcher Host page |
| Hot reload | core only; mods no | none | WASM makes it cheap; later |
| Distribution | local zip/folder only | none | server-served zip, URL, optional index (section 6) |
| Integrity | none | none | SHA-256 pin; optional signatures later |
| Safety | official-mode exclusion, latch; native code unsandboxed | none; crash or hang of a native mod would kill every instance | sandbox or permissions for server code |
| Data overlays | parsed, not applied | `push_override` exists but unwired | wire in both (client half needs an asset override hook, client-hook "Planned") |
| Tooling | `deploy-mod`, sample mod | none | `pack-mod`, `deploy-server-mod`, SDK, sample "both" mod |
| Docs/tests | spec, tests | one paragraph | spec, plan tests, sample |

Two structural findings:
1. **"Both" is the common case for gameplay mods.** A server-side change to a game table (a skill, a stat, an area) is only coherent if the client has the same data. The client half of that (a loose-file or table override in the client) is **not implemented** (`[data]` unapplied, "Planned" in `modding.md`). So data mods of kind `both` depend on a client-hook work item that is outside the server loader.
2. **A server-only mod is invisible to players, a client-only mod is invisible to the server.** The new thing is the contract between them: the server states which client halves a player needs.

## 5. Design

### 5.1 Unified classification and manifest

Keep one `mod.toml`. Add (additive, `manifest_version = 2`; parsers accept 1 and 2, refuse greater):

```toml
manifest_version = 2
id = "faster-rolls"
name = "Faster rolls"
version = "1.0.0"
side = "both"                 # "client" | "server" | "both"; absent (v1 manifests) = "client"

[game]
versions = ["0.5.5e"]
hook_api = 1                  # client half (as today)
server_api = 1                # server half (new)

[entry]
dll = "client/faster_rolls.dll"        # client half (unchanged key)

[server]                      # server half (new)
runtime = "wasm"              # "data" (no code) | "wasm"
module = "server/faster_rolls.wasm"
permissions = ["chat", "players:read", "storage"]   # deny by default

[data]                        # as today, now with a side-aware meaning
tables = ["data/tables"]
config_defaults = "data/defaults.toml"

[[commands]]                  # data-tier command (alias/text), no code
name = "rollinfo"
help = "Shows the roll tuning."
reply = "Roll distance is 1.5x on this server."
```

Rules: `side = "client"` needs `[entry] dll` or `[data]`; `server` needs `[server]` or server-applicable `[data]`; `both` needs both halves. A DLL mod must state `hook_api` (close the open point). The client loader and launcher ignore `[server]`; the server ignores `[entry]`. A v1-only launcher or loader rejects a server-only v2 manifest (it has no `entry`/`data` it understands), which fails closed. Dependencies keep their meaning across halves. Mixed versions are impossible inside one package, which removes the "client half 1.2, server half 1.3" class of bug.

**Shared parser**: extend `poe-hook-mods` (already used by the loader, `poe-hook-deploy`, xtask; the launcher nests the client-hook submodule) with `Side`, `[server]`, `[[commands]]`, and make the launcher use it instead of its own copy. Renaming it to a neutral name can wait.

**Package format** (specify now; today it is only implied): a zip with the mod folder at its root, named `<id>-<version>.zip`, `mod.toml` at the root (or inside exactly one top folder, as today), no symlinks, no absolute or `..` paths, a `.sha256` sidecar file. The SHA-256 of the whole zip is the identity used for advertisement and download verification.

### 5.2 Server mod layout and loader

```
<server working directory>/
  mods/
    state.toml          enabled = ["faster-rolls", "motd"]   (load order)
    faster-rolls/
      mod.toml, server/faster_rolls.wasm, client/faster_rolls.dll, data/...
    packages/           optional: the zips the server offers to players (served as is)
```

- Same discovery, validation, dependency/conflict/ordering code as the client (`poe-hook-mods::plan`, generalised over the side).
- `[mods]` section in `config/server.toml`: `dir = "mods"`, `enabled = true`, `serve_client_mods = true`, `required = [...]` overrides, runtime limits (fuel per call, memory, call time budget). All limits are configuration, not constants.
- Offline CLI `poe-server mod list|install <zip|dir>|enable|disable|order|remove|pack`, like `poe-server account`; later web API endpoints and a web-admin page; the launcher's Host page can install into the hosted server's folder.
- Startup log lines parallel the client's (`mods: loaded x 1.0.0 (3 ms)`, `skipped y: built for server API 2, this server offers 1`).
- Failure handling: a mod that fails validation, init or traps repeatedly is disabled with a logged reason; the server keeps running. Server mods do not run for other clients' sessions beyond what the API allows.

### 5.3 Tier options

| | (a) Native cdylib (`libloading`, C ABI like `HookApi`) | (b) WebAssembly (wasmtime or wasmi) | (c) Embedded script (Lua via `mlua`, or Rhai) | (d) Data-only | (e) Combination |
|---|---|---|---|---|---|
| Parity with client mechanism | highest (same style, could reuse the ABI crate pattern) | different mechanism, same package/manifest/lifecycle | different mechanism | n/a | |
| Effort in this codebase | S-M: loader and ABI are small | M-L: runtime embedding, host functions, SDK, limits | M: smaller than WASM, sandbox hardening needed | S-M: wiring existing pieces | |
| Isolation | none: a crash, hang or leak takes down every instance, login and web API | strong: traps are errors, fuel/epoch/memory limits, no ambient file/network | medium: needs a stripped stdlib and an instruction hook; still in-process | strongest (no code) | |
| Cross-platform | one build per OS and per Rust/ABI version; Linux `.so` and Windows `.dll` both needed in the zip; breaks on compiler mismatch unless the ABI is pure C | one artifact for Linux and Windows | one source for both | n/a | |
| Untrusted mods | unacceptable | acceptable with permissions | acceptable only if hardened and audited | safe | |
| Authoring | Rust only in practice, unsafe FFI | any language with a wasm target; Rust SDK crate gives safe wrappers; binary is not auditable by reading | easiest to write and to audit by reading (source ships); no compile step | TOML/JSON only | |
| Hot reload | unload is unsafe in Rust | easy: new instance, old dropped | easy | restart or re-read | |
| Determinism | not enforceable | high: no clock or random unless the host gives one | medium (iteration order, number formatting need care) | trivial | |
| Build/CI cost | none new | wasmtime adds a large dependency, longer builds and about 10-15 MB to the binary; wasmi is pure Rust, small, slower | vendored C Lua needs a C compiler on both CI runners (cargo-xwin already used); Rhai is pure Rust | none | |
| Risk | highest | medium (new dependency, API design is the real work) | medium | lowest | |

Verdict. Client parity means the same **manifest, package, lifecycle, ordering, compatibility, distribution and safety rules**, not the same mechanism: the client uses native DLLs because it has to live inside someone else's process; the server owns its code and can do better. Recommendation: **(e) data-only tier (d) first, then WebAssembly (b) as the single code tier**, behind a `ModRuntime` trait so a Lua runtime could be added later on the same event/command model. Native server mods are not recommended; if ever needed they can be a third `runtime = "native"` that is refused unless the owner sets an explicit config flag, because it cannot be sandboxed.

### 5.4 Data-only tier

Works entirely with things the server already has, plus wiring:
- `[data] tables`/file overlays: each enabled mod's `data/files/` tree is pushed with `GameFiles::push_override` in load order (later wins). This activates the existing API. Tables the client also reads make the mod `both`.
- `config_defaults`: a TOML layer *under* `config/server.toml` (mods supply defaults, the operator's file wins), merged per section with provenance logged. Covers leagues, start areas, start tiles, player innate skills, chat defaults, movement tuning.
- `[[commands]]`: declarative commands (text reply, alias to existing commands, simple argument substitution) once the command registry is dynamic.
- `opcodes`: server-side opcode table additions (only meaningful with a protocol change; keep parsed but not applied initially).
- Constraints: validated against the schema of the target (unknown keys are errors for server data, since silent typos are worse than for a client), size-limited, paths confined to the mod folder, applied at start (restart to change). A table override changes `GameData` for every instance; a data mod cannot make the server panic because the data layer already returns errors.

### 5.5 WebAssembly tier

**Boundary.** A core wasm module (no WASI, no component model initially) exporting `poe_mod_init`, `poe_mod_abi_version`, `poe_mod_event(ptr, len)`, and importing host functions in the module `poe` (`log`, `now_frame`, `random_u64` from a seeded host RNG, `players`, `send_chat`, `kv_get/kv_set`, `world_query` ...). Messages are JSON (or postcard) in linear memory: any language works, it is easy to debug, and appending fields is compatible. `server_api` is an integer like `HOOK_API_VERSION`: removing or changing meaning bumps it; adding imports or event fields does not (a missing import fails instantiation with a clear reason, the manifest check comes first). WIT/component model can replace the byte format later without changing the manifest. A Rust `poe-mod-sdk` crate offers the safe wrappers, mirroring `poe-hook-mod-api`.

**Events and hooks, ordered by what the server can support today:**

| Event | Kind | Available now? | Notes |
|---|---|---|---|
| `server_start`, `server_stop` | notify | yes | after data and config load |
| `player_login`, `player_logout` | notify | yes | session start/end (`run_session`), account and character names |
| `chat_message` | pre (may cancel or rewrite) and post | yes | in `handle_client_message`; local and global chat |
| `command` | handler | after the registry is dynamic | the mod registers `Spec`s; reuses role check, audit, overlay |
| `instance_created`, `instance_closed` | notify | yes | `InstanceDirectory` and the actor lifecycle |
| `area_enter`, `area_leave` | notify | yes | actor `Join`/`Leave` |
| `tick` (per instance) | notify, budgeted | yes | actor tick; strict fuel budget, instance-scoped state |
| `movement_input` | pre (veto/modify) | yes, high care | hook before `World::input`; must stay deterministic and cheap |
| `world` mutation (teleport, set speed, send message) | via returned commands | yes | applied by the host after the callback |
| `entity_spawn`, `item_drop`, `monster_*` | notify/pre | **no** | the server has players only; add the events when the systems exist |

**World access without sharing the world.** Mods never get a reference to `World`. Callbacks receive a small read-only snapshot as part of the event (frame, entity ids, cells) and answer with **commands** (`Teleport`, `SendChat`, `SetStat`, `Kick`...), which the host validates and applies on the owning task after the callback returns (command-queue pattern; no re-entrancy, no locks held across the guest). Longer queries go through host functions that read from the snapshot taken at the start of the tick.

**Threading.** Two scopes, because a wasmtime `Store` is single-threaded and the actors run concurrently:
1. *Server scope* (start/stop, login/logout, chat, commands, instance lifecycle): one mod task owning one instance of each mod; events arrive by channel, pre-hooks on the session path wait for the answer with a timeout and then default to "allow".
2. *Instance scope* (tick, join/leave, movement): each actor owns its own instance of each enabled mod (state is per instance by construction, and shared state goes through the host `kv` store). Calls run synchronously on the actor task inside the frame budget, so the clock never waits on a slow mod beyond the configured limit; an overrunning call is aborted, logged, and counts toward disabling that mod for the instance.

**Determinism and performance.** No clock, no OS randomness, no file or network access unless a permission grants it through a host function. Frame number and seeded RNG are provided. The replay tests run with mods off and, as a regression, with a no-op mod on (output must be byte-identical). Per-call fuel and memory limits and a per-tick time budget are configured in `[mods]`. Mods that need nothing but a few events cost microseconds per tick; the budget protects the frame loop from the rest.

**Security for untrusted mods.** The server owner installs mods, but some will come from strangers. WASM gives memory isolation; the manifest's `permissions` are shown on install and enforced by which host functions are linked (a mod without `storage` has no `kv_*` import available). No WASI. Storage is a per-mod namespace in the existing database. A trap or panic is contained; N failures in a window disable the mod. Native tier stays off (above). The web API and admin commands are not reachable from mods except through the declared command permission.

**Cross-platform.** One `.wasm` runs on both server builds. wasmtime needs Cranelift on x86_64 Linux and Windows (both release targets); `wasmi` is the fallback if the build cost is unacceptable (slower interpreter, the same API through the trait).

### 5.6 How a server advertises its mods

Add to the public `GET /api/v1/server` (no auth, like the rest of `ServerInfo`; the launcher already calls it) an additive field:

```json
"mods": {
  "server_api": 1,
  "client_mods": [
    { "id": "faster-rolls", "version": "1.0.0", "side": "both",
      "sha256": "…", "size": 48210,
      "required": true,
      "hook_api": 1, "game_versions": ["0.5.5e"],
      "sources": [ { "type": "server", "path": "/api/v1/mods/faster-rolls/1.0.0.zip" },
                   { "type": "url", "url": "https://example.invalid/faster-rolls-1.0.0.zip" } ],
      "name": "Faster rolls", "authors": ["…"], "description": "…" }
  ],
  "server_only_count": 3
}
```

- Only `client` and `both` mods that the operator enabled are listed (`required = true` by default for `both`; the operator can mark a client mod `optional` in `[mods]`). Server-only mods are not named by default (the operator may expose a list, `[mods] list_server_mods`, as information; the count is harmless).
- `GET /api/v1/mods/{id}/{version}.zip` serves the zip from `mods/packages/` (or packs the mod folder deterministically at startup and caches it); the response sets `Content-Length`, the launcher enforces a maximum size. Unauthenticated like `/server`, rate-limited.
- `sha256` is computed by the server at startup. Old launchers ignore the unknown field; old servers have no `mods` key, which the launcher treats as "no requirements".
- This needs a small, additive change in `crates/poe-web-api` (dto, openapi regeneration), `poe-config` (`[mods]`), and a mod-store accessor in `poe-cluster`/`poe-server`, after the current edits in those crates land.

### 5.7 Launcher flow (mod sync)

1. **Select/check a server** (existing quiet check at start and *Check*): read `mods`. If absent or empty: nothing to do. In *Official (read-only)* mode: nothing, mods never load.
2. **Compare** with the install's mod store: missing, present with other version/hash, present and matching, locally enabled but conflicting, locally enabled and not in the server's set, incompatible (`game_versions` without the selected version, `hook_api` above what the installed hook offers, `side` mismatch). The Servers page shows "This server needs N mods" with a table (name, version, side, state, size, source, author).
3. **Prompt**: *Download and enable* / *Not now* / *Details*. Anything with a DLL shows the author, hash and source and the warning that it runs inside the game process. The decision is remembered per (server, mod id) as an approval (hash recorded).
4. **Download**: sources in order (server, then URL) under the same plain-http rule as every other request to the server; the zip's SHA-256 must equal the advertised one or it is deleted. Native-code mods are only downloaded over `https://` or loopback, or after an explicit extra confirmation. Unpacking reuses `install_zip` with its limits.
5. **Store**: because two servers may need different versions of one id, keep a launcher-side cache `mods-cache/<id>/<version>/` and **materialise** the chosen version into `poe-hook-mods/<id>/` when the set is applied. The loader contract (`poe-hook-mods/<id>/` and `state.toml`) does not change.
6. **Switch**: each server profile gets a mod set (the enabled ids and order, stored in the profile, no secrets). On Play the launcher resolves dependencies/order with the existing resolver, writes `state.toml` and materialises versions, then deploys the hook as today. Auto mode does this without asking; Ask mode asks when the plan changes.
7. **Setting** (global default, overridable per server): `Mod sync: Off | Ask | Auto`.
   - *Off*: only show the requirement; Play warns if required mods are missing but does not block.
   - *Ask* (default): prompt on a new or changed requirement, then remember the approval.
   - *Auto*: apply approved mods and their updates silently; **first approval of a mod that contains code is always asked**; data-only mods (no DLL) are auto-approved.
8. **Play** blocks (with a clear message and a *Fix* button) only when a *required* mod is missing/incompatible and sync is not Off.
9. **Optional later**: the hook reports its loaded mods to the server over the existing HMAC side channel and the server can refuse a login with mismatched required mods (`[mods] enforce_client_mods`). This is a consistency check, not anti-cheat: a modified client can lie. It needs a `HookApi` append and a side-channel message.

### 5.8 Where mods come from

| Source | Pros | Cons |
|---|---|---|
| The server serves the zip (primary) | works for private/unlisted servers; exact version/hash match; no extra infrastructure; the owner controls what players run | server bandwidth; trust is in the server (a hostile server ships hostile code: hence the approval and https rules) |
| A URL in the advertisement (GitHub release asset, any host) | offloads bandwidth; mod authors can publish releases; hash still pins the content | URL rot; more hosts to trust |
| A mod index repository (`index.json` of id, versions, sha256, url, author, reviewed flag) | discovery; curated "reviewed" marks; dependency resolution across servers; lets a server list just `{id, version}` | a repository and review process to maintain; deferred until there are mods worth indexing |
| Local zip/folder (exists) | works offline, developer flow | manual |

Recommendation: server-served zips plus optional URLs now, the index later. Signatures (ed25519 by the mod author, trusted keys in the launcher) are the proper answer to "who made this" and are listed as hardening, not v1.

### 5.9 Versioning and compatibility checks

| Dimension | Field | Checked by | When |
|---|---|---|---|
| Manifest | `manifest_version` | parsers | install/load |
| Game build | `game.versions` | launcher (override), server (refuse), hook (warn) | install, enable, start |
| Client ABI | `game.hook_api` (required for DLL mods) | launcher against the installed hook's offered ABI, loader | enable, Play, load |
| Server API | `game.server_api` | server against `SERVER_API_VERSION` | load |
| Mod version | semver; server pins an exact version + hash | launcher | sync |
| Dependencies/conflicts | semver reqs, ids | shared plan logic | enable, load |
| Hook version | the installed hook release | launcher (needs the hook release's offered ABI recorded in `release.json`) | Play |

## 6. Open questions for the user

Each is written to be asked as a multiple-choice question.

**Q1. What should the first server code tier be?**
- A) Data-only mods only for now (config/table/command overrides), code tier later.
- B) Data-only first, then WebAssembly as the one code tier (recommended).
- C) Data-only first, then Lua scripts as the code tier.
- D) Native Rust dynamic libraries, mirroring the client's DLL mods.
Recommendation: B. It gives sandboxing, one artifact for Linux and Windows and hot reload; a native tier cannot be made safe for mods from other people. If easy authoring matters more than the sandbox, C is the alternative and fits behind the same trait.

**Q2. One package for both halves, or separate client and server mods?**
- A) One `mod.toml` and one zip with `side = "client" | "server" | "both"` (recommended).
- B) Separate packages and manifests per side, linked by a naming convention.
Recommendation: A; versions cannot drift apart, the launcher and server share one parser and one install path.

**Q3. Where should the launcher download required mods from?**
- A) From the server itself (the advertisement carries a path to a zip), optionally with extra URLs (recommended).
- B) From a central mod index repository only.
- C) From GitHub release URLs only.
- D) All of the above, with the server's advertisement choosing.
Recommendation: A now (works for private servers, no new infrastructure), add an index later if mods become shareable between servers.

**Q4. What does the "auto download and switch" setting do by default, and what does it do for mods that contain code?**
- A) Three levels Off / Ask / Auto, default Ask; in Auto data-only mods are automatic, the first approval of a mod with a DLL is always asked, approved updates are automatic (recommended).
- B) Auto means everything is automatic, including code from a server you just added.
- C) Always ask for code, never offer Auto for it.
Recommendation: A. Code from a server is remote code execution on the player's PC; one explicit approval per (server, mod) keeps Auto usable.

**Q5. What happens to a player's own mods when a server's set is applied?**
- A) Each server profile has its own mod set; the server's required mods are added, the player's other mods stay off unless the player turns them on for that server (recommended).
- B) One global set; the server's mods are layered on top and conflicting ones are disabled for the session.
- C) Ask every time the plans differ.
Recommendation: A; predictable, and a "both" mod that changes data cannot be broken by an unrelated local mod.

**Q6. Should a server verify that the client really runs the required mods?**
- A) Advisory only: the launcher enforces, the server trusts the client (v1, recommended).
- B) The hook reports its loaded mods over the side channel and the server refuses login on a mismatch (later phase).
- C) Never.
Recommendation: A now, B as an optional phase; neither is anti-cheat.

**Q7. How strict should the permission model for server WASM mods be?**
- A) Deny by default; the manifest declares permissions (chat, players:read, storage, commands); the owner sees and confirms them on install (recommended).
- B) Every mod gets the full API; the owner is expected to trust it.
Recommendation: A; the cost is small and it is the only thing that makes installing a stranger's mod reasonable.

**Q8. How soon is hot reload needed?**
- A) Restart to change mods in v1; hot reload of WASM mods later (recommended).
- B) Hot reload from the start (an admin command and the web admin).
Recommendation: A; a restart-only first version is much simpler to test, and instance state of reloaded mods needs the storage design first.

**Q9. Where should the new code live?**
- A) Server loader and `poe-mod-api`/SDK in this repo (`crates/`), manifest parsing extended in `poe-hook-mods` of the client-hook repo (recommended).
- B) A new repository for the SDK, templates and a mod index.
- C) Everything in this repo, including the manifest crate (the client-hook and launcher would then depend on it).
Recommendation: A for now; B when there are external mod authors.

**Q10. Does the server pin exact versions?**
- A) The advertisement pins an exact version and hash; the launcher keeps several versions in its cache (recommended).
- B) The server gives a semver range and the launcher takes the newest matching.
Recommendation: A; the server owner decides what players run and the hash is meaningful.

## Decisions (user, 2026-10-22)

These override the recommendations above where they differ.

| # | Question | Decision |
|---|---|---|
| Q1 | First server code tier | **Native Rust libraries** (cdylib with a versioned C ABI, like the client mods), plus data-only mods. No WebAssembly/Lua tier for now. |
| Q2 | Packaging | One `mod.toml` and one zip per mod with `side = "client" \| "server" \| "both"`. |
| Q3 | Where the launcher gets required mods | From the server itself (`/api/v1/mods/{id}/{version}.zip`, optional extra URLs); the Poe2Mods index repo is for the launcher's mod browser. |
| Q4 | Auto download and switch | Setting Off / Ask / Auto; **Auto means everything**, code mods included, even from a newly added server (no extra prompt in Auto). |
| Q5 | Player's own mods | Per-server mod sets: required mods added, other mods off on that server unless the player enables them there. |
| Q6 | Server checks client mods | **From the start**: the hook reports its loaded mods at login and the server refuses a mismatch with its required set. A server config option decides whether extra client-side-only mods are allowed. |
| Q7 | Guarding native server mods | **Just load them**: every mod in the server's mods folder (enabled in state.toml) is loaded; the owner is responsible. ABI and game-version checks still refuse incompatible mods. |
| Q8 | Hot reload | **Never**: changing server mods always needs a restart. |
| Q9 | Code home | **A new repo for the SDK now** (mod API crates, templates, examples); the server loader stays in this repo; the shared manifest parser stays in poe-hook-mods. |
| Q10 | Version pinning | Exact version + SHA-256 in the server's advertisement; the launcher keeps several versions cached. |

## 7. Phased implementation plan

Two streams run in parallel after WP-0: **M** (manifest, advertisement, launcher sync) and **S** (server loader). A joint acceptance closes both.

Sequencing note: other work is currently editing `modules/client-hook`, `modules/launcher`, `crates/poe-web-api`, `crates/poe-config`, `config/` and `servers/poe-server`; the WPs below that touch those start after those edits are committed.

| WP | Stream | Content | Files | Tests and acceptance |
|---|---|---|---|---|
| **WP-0 Spec** | both | Freeze: manifest v2 (`side`, `[server]`, `[[commands]]`, `server_api`), package format and naming, advertisement schema, sync states, `[mods]` config keys, event/command list for server API 1. Answers to Q1-Q10 recorded. | `docs/architecture/server-mods.md` (new), `modules/client-hook/docs/mod-manifest.md`, `docs/guides/modding.md`, tracker | Review by the user; links check. |
| **WP-M1 Shared manifest** | M | `poe-hook-mods`: `Side`, server sections, v1/v2 acceptance, `hook_api` required for DLL mods; launcher drops its own parser and uses it; launcher shows a `side` badge and refuses to install a pure server mod into the client store with a clear message; `deploy-mod` honours `side`. | `modules/client-hook/poe-hook-mods`, `poe-hook-deploy/src/mods.rs`, `modules/launcher/src-tauri/src/mods.rs`, `Mods.svelte`, `types.ts` | Parser tests for v1/v2, all `side` values, fail-closed on unknown versions; launcher tests unchanged for old mods; sample-mod manifest still valid. |
| **WP-M2 Advertisement** | M | `[mods]` config; server mod store accessor (scan `mods/`, `side`, enabled set, sha256); `mods` in `GET /server`; `GET /api/v1/mods/{id}/{version}.zip` with limits; OpenAPI regeneration; web-admin type regeneration (read-only list). Works with a static client-mod directory before the server loader exists. | `crates/poe-config`, `crates/poe-web-api` (`dto.rs`, `public.rs`, `openapi.rs`, `lib.rs`), `servers/poe-server`, `config/server.example.toml`, `docs/architecture/web-api.md`, `openapi.json` | Web API tests: absent mods, listing, hash equals file, ranged/size-limited download, 404 for server-only mods; config parse test. |
| **WP-M3 Launcher sync engine** | M | Mod cache by id/version, plan computation, download + verify, materialise, per-profile mod sets, approvals store, settings `mod_sync` (global and per server), `state.toml` write at Play, https rule for code. | `modules/launcher/src-tauri/src/{mods.rs,modsync.rs,profiles.rs,settings.rs,play.rs,commands.rs,api.rs}` | Rust tests against a mock server: missing/outdated/matching/conflict/extra/incompatible, wrong hash deleted, plain-http gating, switching between two servers with different versions of one mod, official mode untouched, Off/Ask/Auto behaviours. |
| **WP-M4 Launcher UI** | M | Servers page requirement panel, prompt dialog with details and the code warning, Mods page origin/side badges, Settings control, Play-blocked message with *Fix*. | `modules/launcher/src/pages/{Servers,Mods,Settings}.svelte`, `lib/mods.ts`, `ipc.ts`, `mock.ts` | vitest for the state mapping; launcher README/guide update. |
| **WP-S1 Server mod store and plan** | S | `mods/` + `state.toml`, plan (order, deps, conflicts, `game.versions`, `server_api`), `poe-server mod list/install/enable/disable/order/remove`, startup log lines, `ModHost` skeleton with `ModRuntime` trait and the data runtime. | new `crates/poe-mod-host`, `servers/poe-server/src/{main.rs,mods_cli.rs}`, `crates/poe-config` | Plan tests mirror the client's; CLI tests on a temp dir; package unzip uses the shared limits. |
| **WP-S2 Data tier** | S | Wire `GameFiles::push_override` from enabled mods; `config_defaults` layering under `server.toml`; dynamic command registry (replace the `const COMMANDS` slice) and declarative `[[commands]]`; overlay wire includes mod commands. | `crates/poe-data`, `crates/poe-config`, `servers/instance-server/src/{commands.rs,exec.rs,lib.rs}`, `servers/poe-server` | A sample data mod changes a table and a config default and adds `!motd`; tests: override order, operator config wins, command appears in chat and in the overlay list; replay tests unchanged. |
| **WP-S3 Server API 1 and WASM runtime (server scope)** | S | New `crates/poe-mod-api` (event, command and host-function types, `SERVER_API_VERSION`, layout/JSON schema tests); wasmtime runtime behind `ModRuntime`; permissions; limits; events `server_start/stop`, `player_login/logout`, `chat_message`, `command`, `instance_created/closed`; host functions `log`, `players`, `send_chat`, `kv_*`, `now_frame`, `random_u64`; failure isolation and disabling. | `crates/poe-mod-api`, `crates/poe-mod-host`, `servers/instance-server/src/{lib.rs,live.rs,exec.rs}`, `crates/poe-store` (kv table migration), `crates/poe-config` | Runtime tests with WAT modules: a trap, an infinite loop (fuel), memory limit, missing permission, wrong `server_api`, cancelled chat message; Linux and Windows build check (cargo-xwin). |
| **WP-S4 Instance scope** | S | Per-actor mod instances; `area_enter/leave`, `tick` (budgeted), `movement_input` pre-hook; command-queue application of mod commands (teleport, message, kick); read-only snapshot in events. | `servers/instance-server/src/actor.rs`, `crates/poe-mod-host`, `crates/poe-game` (only if a hook needs a seam) | Actor tests with fake sessions: a mod teleports on enter; an overrunning tick is aborted; **replay tests byte-identical with a no-op mod loaded**. |
| **WP-S5 SDK, sample, packaging** | S+M | `poe-mod-sdk` guest crate; `examples/sample-server-mod` and a sample `both` mod (client DLL logs + server wasm logs) in one folder; `cargo xtask pack-mod` (deterministic zip + `.sha256`), `deploy-server-mod`; docs for authors. | `crates/poe-mod-sdk`, `modules/client-hook/examples/`, `xtask`, `docs/guides/modding.md`, `docs/guides/writing-server-mods.md` | Pack then install via both the CLI and the launcher; zip is byte-identical across two packs. |
| **WP-S6 Admin surface** | S | Web API list/enable/disable/reorder, web-admin Mods page, launcher *Host a server* page installs into the hosted server; reload by restart. | `crates/poe-web-api`, `modules/web-admin`, `modules/launcher` (host page) | API tests, vitest, admin role checks. |
| **WP-J Joint acceptance** | both | Server with the sample `both` mod advertises it; the launcher prompts, downloads, verifies, switches; the client log shows `mods: loaded` and the server log shows the server half receiving `player_login`. Unattended in-game test on the Agents desktop (no official server). | `docs/guides/setup.md`, tracker | Recorded run; a negative test with a wrong hash. |
| **Later** | | hot reload (WASM); `enforce_client_mods` report from the hook; ed25519 signatures and a trusted-key list; mod index repository; Lua runtime; server events for NPCs, monsters and loot when those systems exist; client `[data]` overlays (asset override hook) to make table mods of kind `both` real. | | |

### Status per work package (updated 2026-10-22)

The decisions changed the plan: the code tier is native Rust (no WASM), and a new SDK repository exists. "WP-F" is the foundation work package that came out of that; it delivers WP-0 (spec), the parser half of WP-M1, the package tooling and the ABI/SDK half of WP-S3/S5. Later rows still read as written (S3/S4 become native-library loading instead of wasmtime).

| WP | Status |
|---|---|
| WP-0 Spec | **Done in WP-F**: manifest v2 and package format specified in `modules/client-hook/docs/mod-manifest.md`; the server API in the SDK's `poe-mod-api` and `docs/abi-stability.md`; guide `docs/guides/modding.md`. Advertisement schema, sync states and `[mods]` config keys are still as in sections 5.6 and 5.7 (frozen when WP-M2/M3 start). |
| WP-F Foundation (this) | **Done**: manifest v2 in the shared parser (`side`, `[client]`, `[server]` with per-platform libraries, `[[commands]]`, fail-closed v1 readers, cross-side dependencies and conflicts); client loader refuses server-only mods and ignores the server half; `deploy-mod` honours `side`; package format, `verify_package` (poe-hook-mods) and the reproducible packer (`poe-mod pack`); new public repo **Poe2ModSDK** with `poe-mod-api` (server ABI 1), `poe-mod-sdk`, `poe-mod`, three examples, docs and CI (Linux and Windows). |
| WP-M1 Shared manifest | **Parser done** (WP-F). Open: the launcher drops its own parser and uses `poe-hook-mods`, shows a `side` badge and refuses to install a pure server mod into the client store. |
| WP-M2 Advertisement | **Done** (server mod host WP): `[mods]` config, `mods` array in `GET /api/v1/server` (the plain array the launcher consumes; final shape in [../architecture/web-api.md](../architecture/web-api.md#mods), which differs from the sketch in 5.6: `code`, `sources`, no `server_api`/`server_only_count`), `GET /api/v1/mods/{id}/{version}.zip` (ETag = SHA-256), the login check of the client's reported mods (Q6) with the hook-side report. Packages are verified with `verify_package_bytes` and packed from the folder when missing. |
| WP-M3 / M4 Launcher sync and UI | Open. Needs the hook release's offered ABI in `release.json`; uses `verify_package` before installing. |
| WP-S1 Server mod store and plan | **Done**: `crates/poe-mod-host` (`mods/`, `state.toml`, plan with game-version, `server_api`, dependency and conflict checks through `poe-hook-mods`), `poe-server mod list/install/enable/disable/verify`. No `order`/`remove` subcommands (edit `state.toml`, delete the folder). See [../architecture/mod-host.md](../architecture/mod-host.md). |
| WP-S2 Data tier | Open (`push_override` wiring, `config_defaults` layering, dynamic command registry and `[[commands]]`). |
| WP-S3 / S4 Server API and loader | **Done**: `libloading` loader with version/game-version checks, `register_command` into the dynamic registry, events (server start/stop, join/leave, command, instance created/closed, tick) with read-only snapshots delivered from the instance actors, commands applied on the actor task / through a sink, failure counting and disabling, shutdown; frames byte-identical with a no-op mod. The real SDK `hello-server` library is loaded in a test. `instance created/closed` mean the instance's actor started/ended. |
| WP-S5 SDK, sample, packaging | **Done in WP-F** (SDK repo, templates, examples, packer); a deploy-to-server command comes with S1. |
| WP-S2 Data tier | **Mostly done**: `[[commands]]` (reply/alias, role) in the dynamic registry and `[server] tables` via `GameFiles::push_override`; `config_defaults` layering and server `opcodes` are open (logged as not yet supported). |
| WP-S6 Admin surface | **Done** (read-only): `GET /api/v1/admin/mods`, the web admin's Mods page with a restart-required notice, `poe-server mod ...` for changes (no HTTP endpoint changes mods: restart-only by decision Q8). |
| WP-J Joint acceptance | Server side tested without the game (packed SDK examples installed in a temp server, advertised, downloaded with the right hash, `!hello` through the real library); the launcher flow and an in-game run are for the joint test. |

Parallelism: WP-M1 and WP-S1 can start together after WP-0 (M1 is in the client-hook and launcher repos, S1 in this repo); WP-M2 needs only the `[mods]` config and a directory scan, not the loader; WP-M3/M4 can be built against a mock server before WP-M2 exists; WP-S2 and WP-S3 are independent of M. Effort (agent sessions, rough): WP-0 S, M1 M, M2 M, M3 L, M4 M, S1 M, S2 M, S3 L, S4 M, S5 M, S6 M, J S.

### Risks

- The wasmtime dependency (build time, binary size, pinned nightly toolchain compatibility); mitigation: the `ModRuntime` trait and `wasmi` as a fallback, a spike at the start of WP-S3.
- Dynamic command registry touches `exec.rs` and `lib.rs`, which are hot files; keep the change mechanical and covered by the existing command tests.
- The word "required" is a promise only the launcher keeps; document that a modified client can ignore it.
- Table overrides without the client half are a trap for operators; the loader warns when a server-only mod overrides a table the client also reads (a list of client-read tables comes from game data).
- Hook mod ABI has no `server_api`-style negotiation of the hook's offered ABI visible to the launcher today; WP-M3 needs `release.json` to state it (the client-hook repo).
