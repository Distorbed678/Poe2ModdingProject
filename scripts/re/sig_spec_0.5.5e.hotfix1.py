"""Spec for scripts/re/sig_gen.py: the 0.5.5e.hotfix1 entries of modules/client-hook/signatures/0.5.5e.hotfix1.toml.

Addresses are Ghidra addresses (image base 0x140000000) of client 0.5.5e.hotfix1 (0.5.5e Hotfix 1, revision 326410).
Made from sig_spec_0.5.5e.hotfix1.py with scripts/re/addr_map.py (every address moved by its function's delta). `features` names the
DLL features that are off when the entry does not resolve (see poe-hook-core `start_features`).
For another build, run the generator on that exe with the (moved) addresses.
"""
SPEC_NAME = "sig_spec_0.5.5e.hotfix1.py"

BUILD = dict(
    version="0.5.5e.hotfix1",
    build_revision="326410",
    revision_pattern="25 53 2e 70 6f 65 63 61 70 00 00 00 00 00 00 00 20 <rev:bytes6> 00",
    pe_timestamp=0x6AE12470,
    image_size=0x04CA0000,
    exe_sha256="d46f4c27576122afc41a428eef96b3b4c75592a16827ed8c2204508ab06a455d",
)

DUMP = ["dump_object_types"]
# gen_batch (in-client batch terrain generation) needs everything the area export needs, plus its own.
EXPORT = ["area_export", "gen_batch"]
BATCH = ["gen_batch"]
TRACE = ["generator_trace"]

SIGS = [
    # ---- loader: standalone login ----
    dict(name="client_platform_detect", addr=0x142197010, features=["login_patch"],
         expect="40 55 53", patch="31 c0 c3",
         note="ClientPlatform_Detect; patch = xor eax,eax; ret (platform 0)"),
    dict(name="standalone_patcher_run", addr=0x1401162D0, features=["patcher_skip"],
         expect="48 8b c4", patch="b0 01 c3",
         note="StandalonePatcher_Run; patch = mov al,1; ret (skip the patcher)"),
    dict(name="client_platform_get", addr=0x140109B20, features=["login_cache"],
         note="cached platform getter; anchor of the two cache globals"),
    dict(name="platform_cache_guard", addr=0x144717144, kind="ref", anchor="client_platform_get",
         features=["login_cache"], note="thread-safe-static guard of the cached platform"),
    dict(name="platform_cache_value", addr=0x144717148, kind="ref", anchor="client_platform_get",
         features=["login_cache"], note="the cached platform value (i32)"),
    # ---- crypto bypass and side channel ----
    dict(name="verify_and_derive", addr=0x14219F160, features=["crypto_bypass"],
         note="signature check + key derivation (hooked, replaced for trusted servers)"),
    dict(name="crypto_dh_agree", addr=0x14219EF90, features=["crypto_bypass"],
         note="DH agree + SHA-512 into the session key buffer; anchor of the key buffer offset"),
    dict(name="instance_ciphers", addr=0x14212AC10, features=["side_channel"],
         note="instance packet-cipher builder (out, key material, &is_client)"),
    # ---- packet capture ----
    dict(name="read_message", addr=0x1421C5B90, features=["packets"]),
    dict(name="session_send", addr=0x14211FA00, features=["packets"]),
    dict(name="stream_flush", addr=0x142198110, features=["packets"]),
    dict(name="packet_flush_plaintext", addr=0x142198187, kind="site", anchor="stream_flush",
         features=["packets"], note="after the lock, before the cipher; rbx = stream"),
    dict(name="login_session_loop", addr=0x1420E6D40, features=["packets"],
         note="receive loop of the login session (caller of read_message); its .pdata range gives the channel"),
    dict(name="instance_session_loop", addr=0x142120CF0, features=["packets"],
         note="receive loop of the instance session; its .pdata range gives the channel"),
    dict(name="stream_read", addr=0x142197E80, features=["packets"]),
    dict(name="stream_write", addr=0x142197FB0, features=["packets"]),
    # ---- entity and world research (dump_object_types) ----
    dict(name="sim_frame_processor", addr=0x1421343C0, features=DUMP,
         note="frame processor; unique caller, anchor of object_type_lookup"),
    dict(name="object_type_lookup", addr=0x141693840, kind="ref", anchor="sim_frame_processor",
         features=DUMP, expect_fn=True, note="a clone-like function: found by its unique caller"),
    dict(name="read_initial_state", addr=0x141E712A0, features=DUMP),
    dict(name="path_plan", addr=0x141ED0510, features=["path_log"],
         note="grid pathfinder (JPS-style search, A* fallback): (goal*, entity, params*, ..., out path*, out count*, ...); hooked by path_log to log the player's plans"),
    dict(name="entity_destroy", addr=0x141E717A0, features=DUMP),
    dict(name="actor_sim_handler", addr=0x1421497C0, features=["position_sync", "inspect"],
         note="Actor component simulation handler (0x64 WASD, 0x1c8 start action, ...)"),
    dict(name="roll_end", addr=0x141B76E25, features=["position_sync"],
         note="end of a dodge roll: reads the sprint-key bit"),
    dict(name="noop_reader", addr=0x14010A080, features=["entity_trace"], pdata=False,
         note="shared `ret 0` stub used as the reader of components without initial state"),
    dict(name="entity_update", addr=0x141E71210, features=DUMP),
    dict(name="frame_drain", addr=0x142130840, features=DUMP),
    dict(name="entity_tick", addr=0x141E7D970, features=DUMP),
    dict(name="game_update", addr=0x14167EBC0, features=DUMP),
    dict(name="set_orientation", addr=0x141DDC820, features=DUMP),
    dict(name="load_area_client", addr=0x14167B3F0, features=DUMP),
    dict(name="load_area_headless", addr=0x14212F680, features=DUMP + BATCH,
         note="HeadlessWorld_LoadArea (base world vfunc +0x1e0): the sequence gen_batch repeats; anchor of the world generation fields"),
    dict(name="positioned_reader", addr=0x141DD89A0, features=DUMP + EXPORT),
    dict(name="terrain_construct", addr=0x1415A3960, features=DUMP + EXPORT,
         note="sets the terrain vtable; anchor of terrain_vtable"),
    dict(name="terrain_vtable", addr=0x143385CF0, kind="ref", anchor="terrain_construct",
         features=DUMP + EXPORT, note="identifies the area terrain object inside the world"),
    # ---- entity probe (calls game functions 5 s after the player spawned) ----
    dict(name="probe_anchor", addr=0x140237120, features=["entity_probe"],
         note="calls the render getter, the visibility refresh and the terrain sampler"),
    dict(name="entity_get_render", addr=0x140150CB0, kind="ref", anchor="probe_anchor", index=0,
         features=["entity_probe"], expect_fn=True, note="entity -> Render component by name"),
    dict(name="render_refresh", addr=0x1416A9990, kind="ref", anchor="probe_anchor",
         features=["entity_probe"], expect_fn=True),
    dict(name="terrain_sample", addr=0x141D0D5A0, kind="ref", anchor="probe_anchor",
         features=["entity_probe"], expect_fn=True, note="terrain height at (x, y)"),
    dict(name="positioned_height", addr=0x141DE5510, features=["entity_probe"]),
    # ---- area export ----
    dict(name="terrain_driver", addr=0x141FFEAC0, features=EXPORT),
    dict(name="terrain_add_object", addr=0x141FF78F0, features=EXPORT),
    dict(name="tile_doodads", addr=0x141FF3490, features=EXPORT + TRACE),
    dict(name="doodad_hash_site", addr=0x141FF37C3, kind="site", anchor="tile_doodads",
         features=EXPORT, note="right after a doodad's path hash: eax = hash, r15 = tile entry"),
    # ---- gen_batch: a synthetic world and generator (docs/re/terrain/batch.md) ----
    dict(name="world_construct_client", addr=0x14167D470, features=BATCH,
         note="ClientWorld constructor (this, frame mode u8, flag u8); the batch makes its own worlds"),
    dict(name="terrain_gen_construct", addr=0x141FEFBA0, features=BATCH,
         note="TerrainGen_Construct(generator, world): base generator vtable 0x1434edf80"),
    dict(name="terrain_gen_destroy", addr=0x141FEFFC0, features=BATCH, note="generator destructor"),
    dict(name="terrain_topology_pick", addr=0x141FF9400, features=BATCH,
         note="reads map_force_topology (0xb92) / map_force_orientation (0xb93) from the world stats; anchor of fields"),
    dict(name="world_replace", addr=0x14100B2A0, features=BATCH,
         note="creates the in-game world and destroys the old one through vfunc +0x20 (deleting destructor)"),
    dict(name="vcrt_getptd", addr=0x142C144C8, features=BATCH,
         note="__vcrt_getptd(): the thread's C++ exception state (+0x20/+0x28 exception and context records, +0x58 frame info chain, +0x70 ...); gen_batch restores it after a recovered throw"),
    dict(name="game_alloc", addr=0x142BBA600, features=BATCH, note="the game's allocator: (size) -> block"),
    dict(name="game_free", addr=0x142BBA4E0, features=BATCH, note="the game's free: (block)"),
    dict(name="flat_map_anchor", addr=0x140B59740, features=BATCH,
         note="calls flat_map_set once; anchor of it (the set function itself has no unique prefix)"),
    dict(name="flat_map_set", addr=0x1423BA640, kind="ref", anchor="flat_map_anchor", index=0,
         features=BATCH, expect_fn=True,
         note="sorted (i32 key, i32 value) vector set (this, key, value); the world stats at world +0x128"),
    dict(name="world_areas_table", addr=0x140223710, features=BATCH,
         note="(out) -> reference-counted Data/Balance/WorldAreas.dat table object"),
    dict(name="resource_wait_loaded", addr=0x1421D7070, features=BATCH,
         note="blocks until a resource (state +0x40 == 2) is loaded"),
    dict(name="room_check_user", addr=0x141EFF410, features=BATCH,
         note="Incursion temple room constructor: reads the WorldAreas table object from world +0xa0; anchor of world.area_table"),
    dict(name="table_rows_user", addr=0x141D02D20, features=BATCH,
         note="reads table +0x28 -> (begin, end) of the rows; anchor of table.rows"),
    # ---- generator trace (research, off by default) ----
    dict(name="terrain_seed_rng", addr=0x141F9C1A0, features=TRACE),
    dict(name="mt_next", addr=0x140A58A70, features=TRACE, pdata=False),
    dict(name="uniform_int", addr=0x141F0D970, features=TRACE),
    dict(name="rng_below", addr=0x141F9C3F0, features=TRACE),
    dict(name="rng_range", addr=0x141F9C4C0, features=TRACE),
    dict(name="room_pass", addr=0x141F8E840, features=TRACE),
    dict(name="room_candidates", addr=0x141F8EEC5, kind="site", anchor="room_pass", features=TRACE,
         note="right after the tile-set lookup: rax = the candidate list"),
    dict(name="place_tile", addr=0x141F9E460, features=TRACE),
    dict(name="derepeat", addr=0x141FDD940, features=TRACE),
    dict(name="node_place", addr=0x1420C8920, features=TRACE),
    dict(name="node_candidates", addr=0x1420C7850, features=TRACE),
    dict(name="route_edge", addr=0x1420CD5E0, features=TRACE),
    dict(name="room_fits", addr=0x141F9D380, features=TRACE,
         note="TileKey fits the terrain at a position: (lattice ctx, &pos, key, wild) -> 0 or a failure code; traced for the overlay candidate scan"),
    dict(name="split_virtual", addr=0x1420BC580, features=TRACE),
    dict(name="smooth_edges", addr=0x1420BD7D0, features=TRACE),
    dict(name="arc_start", addr=0x1420D1CF0, features=TRACE),
    dict(name="arc_end", addr=0x1420D25F0, features=TRACE),
    dict(name="round_corners", addr=0x1420B30E0, features=TRACE),
    dict(name="edge_tiles", addr=0x1420B6AD0, features=TRACE),
    dict(name="lattice_borders", addr=0x1420B9650, features=TRACE),
    dict(name="lattice_flood", addr=0x1420C0BE0, features=TRACE),
    dict(name="lattice_heights", addr=0x1420B99A0, features=TRACE),
    dict(name="lattice_92ce0", addr=0x141F92E60, features=TRACE),
    dict(name="lattice_bdae0", addr=0x1420BDC60, features=TRACE),
    dict(name="lattice_be360", addr=0x1420BE4E0, features=TRACE),
    dict(name="lattice_fill", addr=0x141F931E0, features=TRACE),
    dict(name="lattice_round", addr=0x141FD7340, features=TRACE),
    dict(name="round_corner", addr=0x141FD736B, kind="site", anchor="lattice_round", features=TRACE,
         note="after the corner-set call of one levelling step: rsi = &(x, y), [rsp+0x30..0x38] = vector of the tile's set corner heights"),
    dict(name="tile_lookup", addr=0x141D0A640, features=TRACE),
    dict(name="path_crossings", addr=0x1420B7610, features=TRACE),
    dict(name="tdiff_graph", addr=0x141FF0770, features=TRACE),
    dict(name="tdiff_post", addr=0x141FF7B90, features=TRACE),
    dict(name="tdiff_outer_fill", addr=0x141FD6170, features=TRACE),
    dict(name="tdiff_outer_buffer", addr=0x141FD84E0, features=TRACE),
    dict(name="tdiff_grid_fd0", addr=0x141FE1150, features=TRACE),
    dict(name="tdiff_grid_ea0", addr=0x141FE1020, features=TRACE),
    dict(name="tdiff_overlay_scan", addr=0x141FA2560, features=TRACE),
    dict(name="tdiff_placeholder", addr=0x1420A62C0, features=TRACE),
    dict(name="tdiff_preunique", addr=0x14209C2D0, features=TRACE),
    dict(name="tdiff_topology", addr=0x141FF9400, features=TRACE),
    dict(name="strip_grounds", addr=0x141FDBDF4, kind="site", anchor="tdiff_outer_buffer", features=TRACE,
         note="outer buffer row strips (0x141fd84e0), the fill branch: [rsp+0x40] = row y, [rsp+0x44] = walk limit, [rbp+0x10] / rbx = the two grounds compared"),
    dict(name="placeholder_sizes", addr=0x1420A637D, kind="site", anchor="tdiff_placeholder", features=TRACE,
         note="after the door type's size list lookup: rdi = the list (begin, end; 12-byte entries w, h, weight; total weight at +0x18)"),
    dict(name="phase_prepare", addr=0x141FFE970, features=TRACE),
    dict(name="phase_cad0", addr=0x141FFCC50, features=TRACE),
    dict(name="phase_c360", addr=0x141FFC4E0, features=TRACE),
    dict(name="phase_dab0", addr=0x141FFDC30, features=TRACE),
    dict(name="phase_de60", addr=0x141FFDFE0, features=TRACE),
    dict(name="phase_b930", addr=0x141FFBAB0, features=TRACE),
    dict(name="phase_terrain_e5f0", addr=0x141D0E770, features=TRACE),
    dict(name="phase_vt50_ab20", addr=0x141FFACA0, features=TRACE),
    dict(name="phase_doodads", addr=0x141FF56D0, features=TRACE + EXPORT),
    dict(name="phase_vt28_3eb0", addr=0x141FF4030, features=TRACE),
    dict(name="phase_vt58_4730", addr=0x141FF48B0, features=TRACE),
    dict(name="phase_doodads_post", addr=0x141FF4CD0, features=TRACE),
    dict(name="phase_heights_post", addr=0x141FFE220, features=TRACE),
    dict(name="phase_tile_hash", addr=0x141D14350, features=TRACE + EXPORT),
    # ---- test input ----
    dict(name="aim_report_tick", addr=0x14016BA40, features=["auto_keys_focus"],
         note="AimReport_Tick: compares GetForegroundWindow with the game window pointer"),
    dict(name="game_window_ptr", addr=0x1445D8350, kind="ref", anchor="aim_report_tick",
         features=["auto_keys_focus"], note="pointer to the game window handle"),
]

ENT = ["dump_object_types", "area_export", "position_sync"]
PS = ["position_sync"]
INSPECT = ["inspect"]
PKT = ["packets"]

FIELDS = [
    # ---- entity: the component loop of Entity_ReadInitialState ----
    dict(anchor="read_initial_state", features=ENT, items=[
        dict(name="entity.type", disp=0x8, at=0x141E71340),
        dict(name="type.lookup", disp=0x28, at=0x141E71352),
        dict(name="lookup.serialized_count", disp=0x78, at=0x141E71358),
        dict(name="entity.components_begin", disp=0x10, at=0x141E71361),
        dict(name="component.owner", disp=0x8, at=0x141E71369),
        dict(name="component.vt_reader", disp=0x98, at=0x141E71375),
        dict(name="entity.tick_base", disp=0x58, at=0x141E7137F,
             note="the frame-tick hook receives entity + this"),
    ]),
    dict(anchor="entity_update", features=ENT, items=[
        dict(name="entity.components_end", disp=0x18, at=0x141E7121F)]),
    dict(anchor="entity_destroy", features=ENT, items=[
        dict(name="entity.world", disp=0x78, at=0x141E71813)]),
    dict(anchor="entity_destroy", features=ENT, items=[
        dict(name="entity.id", disp=0x88, at=0x141E71859)]),
    dict(anchor="entity_get_render", features=INSPECT, items=[
        dict(name="lookup.names_begin", disp=0x28, at=0x140150CE9),
        dict(name="lookup.names_end", disp=0x30, at=0x140150CF5),
    ]),
    # ---- Positioned ----
    dict(anchor="positioned_reader", features=ENT, items=[
        dict(name="positioned.grid_x", disp=0x444, at=0x141DD8AA2),
        dict(name="positioned.grid_y", disp=0x448, at=0x141DD8AD2),
        dict(name="positioned.facing", disp=0x44C, at=0x141DD8AF9),
    ]),
    dict(anchor="positioned_reader", features=ENT, items=[
        dict(name="positioned.world_x", disp=0x490, at=0x141DD8D99),
        dict(name="positioned.world_y", disp=0x494, at=0x141DD8DC2),
    ]),
    dict(anchor="set_orientation", features=ENT, items=[
        dict(name="positioned.facing_lock", disp=0x1E5, at=0x141DDC82F,
             note="byte; bit 1 = orientation locked"),
        dict(name="positioned.facing_set", disp=0x44C, at=0x141DDC849, agrees_with="positioned.facing"),
    ]),
    dict(anchor="positioned_reader", features=ENT, items=[
        dict(name="world.vt_terrain", disp=0x1A8, at=0x141DD8B4C,
             note="world vtable slot: returns the terrain object")]),
    # ---- Actor ----
    dict(anchor="actor_sim_handler", features=PS + INSPECT, items=[
        dict(name="actor.action0", disp=0x238, at=0x14214A484),
        dict(name="actor.action1", disp=0x240, at=0x14214A49D),
    ]),
    dict(anchor="actor_sim_handler", features=PS + INSPECT, items=[
        dict(name="action.type", disp=0x120, at=0x142149DAD, note="u16 at an action object")]),
    dict(anchor="actor_sim_handler", features=INSPECT, items=[
        dict(name="actor.dir_x", disp=0x400, at=0x142149ABF),
        dict(name="actor.dir_y", disp=0x404, at=0x142149ACA),
    ]),
    dict(anchor="roll_end", features=PS + INSPECT, items=[
        dict(name="actor.sprint_flags", disp=0x2A2, at=0x141B76F03, note="byte; bit 5 = sprint key held")]),
    # ---- world ----
    dict(anchor="frame_drain", features=["dump_object_types"], items=[
        dict(name="world.frame_mode", disp=0x661, at=0x14213086C)]),
    dict(anchor="frame_drain", features=INSPECT, items=[
        dict(name="world.pause", disp=0x868, at=0x1421308F9, note="u16 flags: +0x868 pause, +0x869 next flag")]),
    dict(anchor="frame_drain", features=INSPECT, items=[
        dict(name="world.frame_queue", disp=0x6A8, at=0x142130A11)]),
    dict(anchor="game_update", features=INSPECT, items=[
        dict(name="game.update_accumulator", disp=0x17CC, at=0x14167EBFD)]),
    dict(anchor="load_area_client", features=["area_export", "dump_object_types", "gen_batch"], items=[
        dict(name="world.area", disp=0x98, at=0x14167B40F),
        dict(name="area.kind", disp=0x1B4, at=0x14167B41B),
    ]),
    dict(anchor="load_area_client", features=["area_export", "dump_object_types", "gen_batch"], items=[
        dict(name="world.tile_hash", disp=0x114, at=0x14167B64C)]),
    dict(anchor="load_area_client", features=["area_export", "dump_object_types", "gen_batch"], items=[
        dict(name="world.doodad_hash", disp=0x118, at=0x14167B6EE)]),
    # gen_batch: what a synthetic world needs (docs/re/terrain/batch.md)
    dict(anchor="load_area_client", features=BATCH, items=[
        dict(name="world.stats", disp=0x128, at=0x14167B8DB,
             note="the world's stat map (sorted i32 key/value pairs at +8..+0x10; map_force_* live here)")]),
    dict(anchor="load_area_headless", features=BATCH, items=[
        dict(name="world.gen_flag", disp=0x663, at=0x14212F6B1, note="u8, cleared before a headless generation"),
        dict(name="world.gen_state", disp=0x4B0, at=0x14212F6B8, note="the generation state (u32 +0, seed u32 +4)"),
        dict(name="world.gen_seed", disp=0x4B4, at=0x14212F6BE),
    ]),
    dict(anchor="terrain_topology_pick", features=BATCH, items=[
        dict(name="world.league_mode", disp=0xB8, at=0x141FF960C, note="u32 mode column the topologies are filtered by"),
        dict(name="area.topology_count", disp=0x60, at=0x141FF9497),
        dict(name="area.topology_list", disp=0x68, at=0x141FF94AE, note="16-byte entries"),
    ]),
    dict(anchor="terrain_gen_construct", features=BATCH, items=[
        dict(name="generator.validate", disp=0xE, at=0x141FEFBCC,
             note="u8, 1 after construction: runs the tile map validation pass (0x141ffdc30, throws TerrainGenerationException); the client clears it")]),
    dict(anchor="world_replace", features=BATCH, items=[
        dict(name="world.vt_delete", disp=0x20, at=0x14100B9AB, note="vtable slot: deleting destructor (this, 1)")]),
    dict(anchor="resource_wait_loaded", features=BATCH, items=[
        dict(name="resource.state", disp=0x40, at=0x1421D7082, note="2 = loaded")]),
    dict(anchor="table_rows_user", features=BATCH, items=[
        dict(name="table.rows", disp=0x28, at=0x141D02DC1, note="pointer to (begin, end) of the table rows"),
        dict(name="resource.refs", disp=0x3C, at=0x141D02D64, note="i32 reference count (lock inc / lock dec)")]),
    dict(anchor="room_check_user", features=BATCH, items=[
        dict(name="world.area_table", disp=0xA0, at=0x141EFF515,
             note="reference-counted WorldAreas table object (the world's area row belongs to it); the world's destructor releases it")]),
    # ---- generator and terrain ----
    dict(anchor="terrain_driver", features=EXPORT, items=[
        dict(name="generator.world", disp=0x38, at=0x141FFF022),
        dict(name="generator.terrain", disp=0x28, at=0x141FFF03B),
    ]),
    dict(anchor="terrain_driver", features=EXPORT, items=[
        dict(name="generator.state", disp=0x30, at=0x141FFF081)]),
    dict(anchor="terrain_driver", features=EXPORT, items=[
        dict(name="state.seed", disp=0x4, at=0x141FFED62, note="u32 at the generation state")]),
    dict(anchor="phase_tile_hash", features=EXPORT + DUMP, items=[
        dict(name="terrain.width", disp=0x18, at=0x141D1437C),
        dict(name="terrain.height", disp=0x20, at=0x141D1437F),
    ]),
    dict(anchor="phase_tile_hash", features=EXPORT + DUMP, items=[
        dict(name="terrain.tiles", disp=0x28, at=0x141D143CE)]),
    # ---- network streams ----
    dict(anchor="stream_read", features=PKT, items=[
        dict(name="stream.error", disp=0x252, at=0x142197E9D),
        dict(name="stream.msg_read", disp=0x278, at=0x142197EAA),
    ]),
    dict(anchor="stream_read", features=PKT, items=[
        dict(name="stream.recv_base", disp=0x298, at=0x142197EF3),
        dict(name="stream.recv_end", disp=0x2A0, at=0x142197EFA),
        dict(name="stream.recv_head", disp=0x268, at=0x142197F04),
    ]),
    dict(anchor="stream_write", features=PKT, items=[
        dict(name="stream.send_len", disp=0x258, at=0x142198025)]),
    dict(anchor="stream_write", features=PKT, items=[
        dict(name="stream.send_buf", disp=0x280, at=0x142197FC4)]),
    dict(anchor="stream_flush", features=PKT, items=[
        dict(name="stream.send_sent", disp=0x260, at=0x142198187)]),
    dict(anchor="session_send", features=PKT, items=[
        dict(name="session.stream", disp=0x20F8, at=0x14211FA1A)]),
    # ---- crypto ----
    dict(anchor="crypto_dh_agree", features=["crypto_bypass"], items=[
        dict(name="crypto.key_buffer", disp=0x60, at=0x14219F0F0,
             note="pointer to the 64-byte key buffer inside the crypto session")]),
]
