//! Terrain research tools for the server-side generator (M5). Everything here only reads game
//! memory, so it also runs on official servers (read-only).
//!
//! - **Area export** (`[features] area_export`): when the client's generator driver
//!   (`0x141ffe940`) returns, the area is written to `<session>/areas/<n>_<hash16>_<seed>/`:
//!   `meta.txt`, `layout.tsv` (every tile record with the fields the tile hash covers, checked
//!   by recomputing the hash), `grids.bin` (the four cell grids), `entities.tsv` (objects the
//!   generator created: doodads and tile entities, with path, cell, world position, facing and
//!   components). "Export area" in the Minimap tab writes it again with the server's entities
//!   and the walkability probe added.
//! - **Walkability probe**: the cells the player stood on, with each grid layer's value there
//!   (`probe.tsv`); a walkability layer is never 0 under the player.
//! - **Generator trace** (`[features] generator_trace` or `POE_HOOK_GEN_TRACE`): every draw from
//!   the terrain random number generator (MSVC `std::mt19937` in thread-local storage, seeded by
//!   `0x141f9c020`; raw draws `0x140a58a70`, bounded draws `0x141f0d7f0`) and every generator
//!   phase, in call order, as `generator_trace.tsv`. Diffing it against the server's port finds
//!   the first divergence.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use ilhook::x64::{HookType, Registers};
use poe_hook_shared::{
    mem::{read, readable, wide_string},
    module::{Module, stack_refs},
};

/// Generator driver: `(generator)`; generator +0x28 terrain, +0x30 generation state (+4 seed),
/// +0x38 world.
const DRIVER: usize = 0x141ffe940;
/// Seeds the thread-local terrain RNG: `(u32 seed, u8 disabled)`.
const SEED_RNG: usize = 0x141f9c020;
/// `std::mt19937` next value: `(state)` (state +0 index, +4.. words).
const MT_NEXT: usize = 0x140a58a70;
/// MSVC `uniform_int` helper: `(&{state, bits, mask}, n)` -> value in `0..n`.
const UNIFORM: usize = 0x141f0d7f0;
/// `TerrainRng_Below(n)` / `TerrainRng_Range(a, b)`: logged with their exact call site.
const RNG_BELOW: usize = 0x141f9c270;
const RNG_RANGE: usize = 0x141f9c340;
/// Room pass (`0x141f8e6c0`), right after the tile-set lookup: rax = the candidate list
/// (begin, end; 0x18-byte entries: tile resource, u8 orientation, i32 weight at +0x10; +0x18
/// total weight). Logged in the generator trace.
const ROOM_CANDIDATES: usize = 0x141f8ed45;
/// Place a tile: `(terrain context, x, y, &tile, u8 orientation, i32 height, u8)`.
const PLACE_TILE: usize = 0x141f9e2e0;
/// De-repetition pass `(ctx, region, ...)`: region +0x58..+0x60 lists room placements
/// (0x38 bytes, +0 room resource); room data (+0x28) +0xe8 is a (w-1)×(h-1) grid of resource
/// pointers at interior lattice points (pieces touching a set point are not re-picked).
const DEREPEAT: usize = 0x141fdd7c0;
/// Graph node placement (`0x1420c87a0`: context, result out, node): the room or tile picked
/// for a room node and its position (result +0x70).
const NODE_PLACE: usize = 0x1420c87a0;
/// Room/tile candidates of a node (`0x1420c76d0`: list out, node, room type, key, ..., ...):
/// 0x68-byte entries (name view +0/+8, variant byte +0x10, size +0x31/+0x32, weight +0x60).
const NODE_CANDIDATES: usize = 0x1420c76d0;
/// Edge routing (`0x1420cd460`: router, edge) → bool: the edge's ends (+0x20/+0x28 nodes,
/// position +0x4c), end offsets (+0x70/+0x78) and internal points (+8..+0x10) afterwards.
const ROUTE_EDGE: usize = 0x1420cd460;
/// Virtual-edge splitting (`0x1420bc400`: layout (+0x20 graph), ...): the graph afterwards.
const SPLIT_VIRTUAL: usize = 0x1420bc400;
/// Edge smoothing after routing (`0x1420bd650`: layout, flag): the graph afterwards.
const SMOOTH_EDGES: usize = 0x1420bd650;
/// Arc at a virtual edge's start / end (`0x1420d1b70` / `0x1420d2470`: ctx, edge) and corner
/// rounding (`0x1420b2f60`: ctx, edge, from end).
const ARC_START: usize = 0x1420d1b70;
const ARC_END: usize = 0x1420d2470;
const ROUND_CORNERS: usize = 0x1420b2f60;
/// Edge tile placement stage (`0x1420b6950`: layout ...): the graph on entry.
const EDGE_TILES: usize = 0x1420b6950;
/// Steps between edge tiles and the fill pass that edit the corner lattice (ground index,
/// height, flags per tile corner): snapshots before each (`lattice` trace lines).
const LATTICE_STEPS: &[(usize, &str, usize)] = &[
    (0x1420b94d0, "borders", 0x28),
    (0x1420c0a60, "flood", 0x8),
    (0x1420b9820, "heights", 0x28),
    (0x141f92ce0, "92ce0", 0x0),
    (0x1420bdae0, "bdae0", 0x28),
    (0x1420be360, "be360", 0x28),
    (0x141f93060, "fill", 0x8),
];
/// Tile set lookup by key (`0x141d0a4c0(tile set, key, feature)`), traced for edge tiles.
const TILE_LOOKUP: usize = 0x141d0a4c0;
/// Edge-tile path crossings `0x1420b7490(_, out, &tiles, size)`: logs the path's tile list.
const PATH_CROSSINGS: usize = 0x1420b7490;
/// Generator vfunc +0x38: adds a created object (doodad or tile entity) to the world:
/// `(generator, entity, region)`.
const ADD_OBJECT: usize = 0x141ff7770;
/// Inside the per-tile doodad pass (`0x141ff3310`), right after the doodad's path hash: eax =
/// path hash, [rsp+0x70]/[rsp+0x74] = placed x/y (cells), [rsp+0x84] = angle, r15 = the tile's
/// doodad entry (+0x38: the art resource). The doodad hash is the wrapping sum of
/// `y + path hash + (x << 16)` over these.
const DOODAD_HASH_SITE: usize = 0x141ff3643;
/// Positioned component's initial-state reader (identifies the component by its vtable).
const POSITIONED_READER: usize = 0x141dd8820;
/// Generator phases (driver order); names are provisional.
const PHASES: &[(usize, &str)] = &[
    (0x141ffe7f0, "prepare"),
    (0x141ffcad0, "phase_cad0"),
    (0x141ffc360, "phase_c360"),
    (0x141ffdab0, "phase_dab0"),
    (0x141ffde60, "phase_de60"),
    (0x141ffb930, "phase_b930"),
    (0x141d0e5f0, "terrain_e5f0"),
    (0x141ffab20, "vt50_ab20"),
    (0x141ff5550, "doodads"),
    (0x141ff3310, "tile_doodads"),
    (0x141ff3eb0, "vt28_3eb0"),
    (0x141ff4730, "vt58_4730"),
    (0x141ff4b50, "doodads_post"),
    (0x141ffe0a0, "heights_post"),
    (0x141d141d0, "tile_hash"),
];
/// Tile records: terrain +0x18 width, +0x20 height (in tiles, 23 cells each), +0x28 records
/// (0x38 bytes, row-major).
const TILE_RECORD: usize = 0x38;
const TRACE_LINES_MAX: usize = 3_000_000;

static MAIN: OnceLock<Module> = OnceLock::new();
static EXPORT: AtomicBool = AtomicBool::new(false);
static TRACE: AtomicBool = AtomicBool::new(false);
/// Thread running the generator (0: none); trace hooks only record on it.
static GEN_THREAD: AtomicU32 = AtomicU32::new(0);
static TRACE_SEQ: AtomicU64 = AtomicU64::new(0);
static AREA_COUNT: AtomicUsize = AtomicUsize::new(0);
static TRACE_BUF: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Objects the generator added in the current generation: (entity, region argument).
/// Per generator thread (world-map panels generate in parallel).
static GEN_OBJECTS: Mutex<Option<HashMap<u32, Vec<(usize, usize)>>>> = Mutex::new(None);
static CURRENT: Mutex<Option<Area>> = Mutex::new(None);
/// Doodads that entered the doodad hash: (x, y, angle, path hash, path).
type Doodad = (i32, i32, f32, u32, String);
static DOODADS: Mutex<Option<HashMap<u32, Vec<Doodad>>>> = Mutex::new(None);

/// The calling thread's entry of a per-thread buffer, taken out.
fn take_mine<T>(m: &Mutex<Option<HashMap<u32, Vec<T>>>>) -> Vec<T> {
    m.lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .remove(&thread_id())
        .unwrap_or_default()
}

fn push_mine<T>(m: &Mutex<Option<HashMap<u32, Vec<T>>>>, v: T) {
    m.lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .entry(thread_id())
        .or_default()
        .push(v);
}

#[derive(Clone)]
struct Area {
    dir: PathBuf,
    world: usize,
    /// Cells the player stood on.
    visited: HashSet<(i32, i32)>,
    objects: Vec<(usize, usize)>,
}

pub fn init(main: &Module, export: bool, trace: bool) -> Result<(), String> {
    MAIN.set(*main).ok();
    EXPORT.store(export, Ordering::Relaxed);
    TRACE.store(trace, Ordering::Relaxed);
    let hook = |ghidra: usize, kind: HookType, user: usize| {
        crate::hooks::install(main.from_ghidra(ghidra), kind, user)
            .map_err(|e| format!("hook {ghidra:#x}: {e}"))
    };
    hook(DRIVER, HookType::Retn(on_driver), 0)?;
    hook(ADD_OBJECT, HookType::JmpBack(on_add_object), 0)?;
    hook(DOODAD_HASH_SITE, HookType::JmpBack(on_doodad), 0)?;
    if trace {
        hook(SEED_RNG, HookType::JmpBack(on_seed), 0)?;
        hook(MT_NEXT, HookType::Retn(on_mt_next), 0)?;
        hook(UNIFORM, HookType::Retn(on_uniform), 0)?;
        hook(RNG_BELOW, HookType::JmpBack(on_rng_call), 0)?;
        hook(ROOM_CANDIDATES, HookType::JmpBack(on_candidates), 0)?;
        hook(PLACE_TILE, HookType::JmpBack(on_place), 0)?;
        hook(DEREPEAT, HookType::JmpBack(on_derepeat), 0)?;
        hook(NODE_PLACE, HookType::Retn(on_node_place), 0)?;
        hook(NODE_CANDIDATES, HookType::Retn(on_node_candidates), 0)?;
        hook(ROUTE_EDGE, HookType::Retn(on_route_edge), 0)?;
        hook(SPLIT_VIRTUAL, HookType::Retn(on_split_virtual), 0)?;
        hook(SMOOTH_EDGES, HookType::Retn(on_smooth_edges), 0)?;
        hook(ARC_START, HookType::Retn(on_edge_step), 0)?;
        hook(ARC_END, HookType::Retn(on_edge_step), 1)?;
        hook(ROUND_CORNERS, HookType::Retn(on_edge_step), 2)?;
        hook(EDGE_TILES, HookType::JmpBack(on_edge_tiles), 0)?;
        hook(TILE_LOOKUP, HookType::JmpBack(on_tile_lookup), 0)?;
        hook(PATH_CROSSINGS, HookType::JmpBack(on_path_crossings), 0)?;
        for (i, (addr, _, _)) in LATTICE_STEPS.iter().enumerate() {
            hook(*addr, HookType::JmpBack(on_lattice_step), i)?;
        }
        hook(RNG_RANGE, HookType::JmpBack(on_rng_call), 1)?;
        for (i, (addr, _)) in PHASES.iter().enumerate() {
            hook(*addr, HookType::JmpBack(on_phase), i)?;
        }
    }
    crate::hooks::spawn(probe_loop);
    Ok(())
}

fn thread_id() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}

fn on_generator_thread() -> bool {
    GEN_THREAD.load(Ordering::Relaxed) == thread_id()
}

fn ghidra(addr: usize) -> usize {
    MAIN.get()
        .filter(|m| m.contains(addr))
        .map_or(addr, |m| m.to_ghidra(addr))
}

/// The first game-code return addresses on the stack (Ghidra addresses), for context.
fn callers(rsp: usize, n: usize) -> String {
    let Some(m) = MAIN.get() else {
        return String::new();
    };
    stack_refs(m, rsp, 24)
        .into_iter()
        .take(n)
        .map(|a| format!("{:x}", m.to_ghidra(a)))
        .collect::<Vec<_>>()
        .join(",")
}

fn trace_line(kind: &str, caller: &str, args: &str, result: &str) {
    let mut buf = TRACE_BUF.lock().unwrap();
    if buf.len() >= TRACE_LINES_MAX {
        return;
    }
    let seq = TRACE_SEQ.fetch_add(1, Ordering::Relaxed);
    buf.push(format!("{seq}\t{kind}\t{caller}\t{args}\t{result}"));
}

type DriverFn = unsafe extern "win64" fn(usize) -> usize;

unsafe extern "win64" fn on_driver(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_driver");
    let r = unsafe { &*regs };
    let generator = r.rcx as usize;
    let world = read::<usize>(generator + 0x38).unwrap_or(0);
    let seed = read::<usize>(generator + 0x30)
        .and_then(|s| read::<u32>(s + 4))
        .unwrap_or(0);
    take_mine(&GEN_OBJECTS);
    take_mine(&DOODADS);
    // The trace follows playable areas only (world-map panels generate in parallel).
    let kind = read::<usize>(world + 0x98)
        .and_then(|a| read::<u32>(a + 0x1b4))
        .unwrap_or(0);
    let traced = !matches!(kind, 0x20 | 0x21 | 0x23);
    if traced && TRACE.load(Ordering::Relaxed) {
        TRACE_BUF.lock().unwrap().clear();
        TRACE_SEQ.store(0, Ordering::Relaxed);
        GRID_DUMPED.store(false, Ordering::Relaxed);
        trace_line("begin", "", &format!("seed={seed}"), "");
    }
    if traced {
        GEN_THREAD.store(thread_id(), Ordering::Relaxed);
    }
    let f: DriverFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(generator) });
    if traced {
        GEN_THREAD.store(0, Ordering::Relaxed);
    }
    let objects = take_mine(&GEN_OBJECTS);
    let doodads = take_mine(&DOODADS);
    if EXPORT.load(Ordering::Relaxed) && world != 0 {
        export_area(world, seed, objects, doodads, traced);
    }
    ret
}

unsafe extern "win64" fn on_add_object(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_add_object");
    let r = unsafe { &*regs };
    push_mine(&GEN_OBJECTS, (r.rdx as usize, r.r8 as usize));
}

unsafe extern "win64" fn on_doodad(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_doodad");
    let r = unsafe { &*regs };
    let rsp = r.rsp as usize;
    let (Some(x), Some(y)) = (read::<i32>(rsp + 0x70), read::<i32>(rsp + 0x74)) else {
        return;
    };
    let angle = read::<f32>(rsp + 0x84).unwrap_or(0.0);
    let path = read::<usize>(r.r15 as usize + 0x38)
        .map(resource_path)
        .unwrap_or_default();
    push_mine(&DOODADS, (x, y, angle, r.rax as u32, path));
}

unsafe extern "win64" fn on_seed(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_seed");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    trace_line(
        "seed",
        &callers(r.rsp as usize, 3),
        &format!("{} disabled={}", r.rcx as u32, r.rdx as u8),
        "",
    );
}

/// `below`/`range` entry: the exact caller (return address) and arguments.
unsafe extern "win64" fn on_rng_call(regs: *mut Registers, range: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_rng_call");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    let ret = read::<usize>(r.rsp as usize).map_or(0, ghidra);
    let args = if range == 1 {
        format!("{} {}", r.rcx as u32, r.rdx as u32)
    } else {
        format!("{}", r.rcx as u32)
    };
    trace_line(
        if range == 1 { "range" } else { "below" },
        &format!("{ret:x}"),
        &args,
        "",
    );
}

unsafe extern "win64" fn on_derepeat(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_derepeat");
    if !on_generator_thread() {
        return;
    }
    let region = unsafe { &*regs }.rdx as usize;
    let (Some(b), Some(e)) = (read::<usize>(region + 0x58), read::<usize>(region + 0x60)) else {
        return;
    };
    let mut p = b;
    while p < e {
        if let Some(room) = read::<usize>(p).filter(|&r| r != 0) {
            let path = resource_path(room);
            let data = read::<usize>(room + 0x28).unwrap_or(0);
            let (w, h) = (
                read::<u8>(data + 0x138).unwrap_or(0) as usize,
                read::<u8>(data + 0x139).unwrap_or(0) as usize,
            );
            let grid = read::<usize>(data + 0xe8).unwrap_or(0);
            let mut points = Vec::new();
            for y in 1..h.max(1) {
                for x in 1..w.max(1) {
                    let at = grid + ((y - 1) * (w - 1) + (x - 1)) * 8;
                    if let Some(v) = read::<usize>(at).filter(|&v| v != 0) {
                        points.push(format!(
                            "{x},{y}:{}",
                            resource_path(v).rsplit('/').next().unwrap_or("")
                        ));
                    }
                }
            }
            let pos: [i32; 4] = read(p + 8).unwrap_or_default();
            trace_line(
                "derepeat_room",
                "",
                &format!("{path} {w}x{h} at {pos:?}"),
                &points.join(" "),
            );
        }
        p += 0x38;
    }
}

unsafe extern "win64" fn on_place(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_place");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    let tile = read::<usize>(r.r9 as usize)
        .map(resource_path)
        .unwrap_or_default();
    let rsp = r.rsp as usize;
    let o = read::<u8>(rsp + 0x28).unwrap_or(0xff);
    let height = read::<i32>(rsp + 0x30).unwrap_or(0);
    let ret = read::<usize>(rsp).map_or(0, ghidra);
    trace_line(
        "place",
        &format!("{ret:x}"),
        &format!("{} {} o{o} h{height}", r.rdx as i32, r.r8 as i32),
        &tile,
    );
}

unsafe extern "win64" fn on_candidates(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_candidates");
    if !on_generator_thread() {
        return;
    }
    let list = unsafe { &*regs }.rax as usize;
    let (Some(b), Some(e)) = (read::<usize>(list), read::<usize>(list + 8)) else {
        return;
    };
    let total = read::<i32>(list + 0x18).unwrap_or(0);
    let mut out = Vec::new();
    let mut p = b;
    while p < e && out.len() < 64 {
        let tile = read::<usize>(p).map(resource_path).unwrap_or_default();
        let o = read::<u8>(p + 8).unwrap_or(0);
        let w = read::<i32>(p + 0x10).unwrap_or(0);
        out.push(format!(
            "{}:o{o}:w{w}",
            tile.rsplit('/').next().unwrap_or("")
        ));
        p += 0x18;
    }
    trace_line(
        "candidates",
        "141f8ed45",
        &format!("total={total}"),
        &out.join(" "),
    );
}

/// Dumps the corner lattice of a map view (`[map, x0:i32, y0:i32, .., w @+0x18, h @+0x1c]`;
/// map +0x40 row stride, +0x50 3-byte points, +0x68 ground handles) before a step.
unsafe extern "win64" fn on_lattice_step(regs: *mut Registers, step: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_lattice_step");
    if !on_generator_thread() {
        return;
    }
    let (_, name, view_at) = LATTICE_STEPS[step];
    let r = unsafe { &*regs };
    let view = read::<usize>(r.rcx as usize + view_at).unwrap_or(0);
    let map = read::<usize>(view).unwrap_or(0);
    let (x0, y0) = (read::<i32>(view + 8).unwrap_or(0), read::<i32>(view + 0xc).unwrap_or(0));
    let (w, h) = (read::<u32>(view + 0x18).unwrap_or(0), read::<u32>(view + 0x1c).unwrap_or(0));
    let stride = read::<u64>(map + 0x40).unwrap_or(0) as usize;
    let data = read::<usize>(map + 0x50).unwrap_or(0);
    let (gb, ge) = (read::<usize>(map + 0x68).unwrap_or(0), read::<usize>(map + 0x70).unwrap_or(0));
    let mut grounds = Vec::new();
    let mut q = gb;
    while q < ge && grounds.len() < 256 {
        let h = read::<usize>(q).unwrap_or(0);
        let p = if h == 0 { "-".to_string() } else { resource_path(h) };
        grounds.push(p.rsplit('/').next().unwrap_or("").to_string());
        q += 8;
    }
    trace_line("lattice", name, &format!("view {x0},{y0} {w}x{h}"), &grounds.join(","));
    if w > 1024 || h > 1024 || stride == 0 {
        return;
    }
    for y in 0..=h as usize {
        let mut row = Vec::with_capacity(w as usize + 1);
        for x in 0..=w as usize {
            let at = data + (((y0 as usize + y) * stride) + x0 as usize + x) * 3;
            let b: [u8; 3] = read(at).unwrap_or_default();
            row.push(format!("{}.{}.{}", b[0], b[1] as i8, b[2]));
        }
        trace_line("lattice", name, &format!("row {y}"), &row.join(" "));
    }
}

/// A tile key (`0x4e` bytes): edges[4], size, grounds[4] (at +0x22), heights, exits and
/// virtual exits (4 bytes each from +0x42).
unsafe extern "win64" fn on_path_crossings(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_path_crossings");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    let v = r.r8 as usize;
    let (b, e) = (read::<usize>(v).unwrap_or(0), read::<usize>(v + 8).unwrap_or(0));
    let mut tiles = Vec::new();
    let mut q = b;
    while q < e && tiles.len() < 4096 {
        let t: [i32; 2] = read(q).unwrap_or_default();
        tiles.push(format!("{},{}", t[0], t[1]));
        q += 8;
    }
    trace_line("path_tiles", "1420b7490", &format!("size {} n {}", r.r9, tiles.len()), &tiles.join(" "));
}

unsafe extern "win64" fn on_tile_lookup(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_tile_lookup");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    let key = r.rdx as usize;
    let name = |h: usize| -> String {
        if h == 0 {
            "-".into()
        } else {
            let p = resource_path(h);
            p.rsplit('/').next().unwrap_or("").to_string()
        }
    };
    let edges: Vec<String> = (0..4).map(|k| name(read::<usize>(key + k * 8).unwrap_or(0))).collect();
    let grounds: Vec<String> = (0..4).map(|k| name(read::<usize>(key + 0x22 + k * 8).unwrap_or(0))).collect();
    let size: [u8; 2] = read(key + 0x20).unwrap_or_default();
    let bytes: [u8; 12] = read(key + 0x42).unwrap_or_default();
    let ret = read::<usize>(r.rsp as usize).map_or(0, ghidra);
    trace_line(
        "tile_key",
        &format!("{ret:x}"),
        &format!("size {:?} h {:?} x {:?} v {:?}", size, &bytes[0..4], &bytes[4..8], &bytes[8..12]),
        &format!("e {} g {}", edges.join(","), grounds.join(",")),
    );
}

/// A `(wchar*, len)` string view.
fn view(addr: usize) -> String {
    let (Some(p), Some(n)) = (read::<usize>(addr), read::<usize>(addr + 8)) else {
        return String::new();
    };
    if n == 0 || n > 512 || !readable(p, n * 2) {
        return String::new();
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p as *const u16, n) })
}

fn node_desc(node: usize) -> String {
    let x = read::<i32>(node + 0x4c).unwrap_or(0);
    let y = read::<i32>(node + 0x50).unwrap_or(0);
    let o = read::<u8>(node + 0x40).unwrap_or(0xff);
    let room = msvc_wstring(node + 0x20).unwrap_or_default();
    format!("node ({x},{y}) {room:?} o{o}")
}

type NodePlaceFn = unsafe extern "win64" fn(usize, usize, usize) -> usize;

unsafe extern "win64" fn on_node_place(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_node_place");
    let r = unsafe { &*regs };
    let (ctx, out, node) = (r.rcx as usize, r.rdx as usize, r.r8 as usize);
    let f: NodePlaceFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(ctx, out, node) });
    if on_generator_thread() {
        let x = read::<i32>(out + 0x70).unwrap_or(0);
        let y = read::<i32>(out + 0x74).unwrap_or(0);
        let variant = read::<u8>(out + 0x10).unwrap_or(0xff);
        trace_line(
            "node_place",
            "1420c87a0",
            &node_desc(node),
            &format!("{} v{variant} at {x},{y}", view(out)),
        );
    }
    ret
}

type NodeCandidatesFn = unsafe extern "win64" fn(usize, usize, usize, usize, usize, usize) -> usize;

unsafe extern "win64" fn on_node_candidates(
    regs: *mut Registers,
    original: usize,
    _: usize,
) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_node_candidates");
    let r = unsafe { &*regs };
    let rsp = r.rsp as usize;
    let a5 = read::<usize>(rsp + 0x28).unwrap_or(0);
    let a6 = read::<usize>(rsp + 0x30).unwrap_or(0);
    let (list, node) = (r.rcx as usize, r.rdx as usize);
    let (a3, a4) = (r.r8 as usize, r.r9 as usize);
    let f: NodeCandidatesFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(list, node, a3, a4, a5, a6) });
    if on_generator_thread() {
        let (b, e) = (
            read::<usize>(list).unwrap_or(0),
            read::<usize>(list + 8).unwrap_or(0),
        );
        let mut out = Vec::new();
        let mut p = b;
        while p < e && out.len() < 64 {
            out.push(format!(
                "{}:v{}:{}x{}:w{}",
                view(p),
                read::<u8>(p + 0x10).unwrap_or(0),
                read::<u8>(p + 0x31).unwrap_or(0),
                read::<u8>(p + 0x32).unwrap_or(0),
                read::<i32>(p + 0x60).unwrap_or(0)
            ));
            p += 0x68;
        }
        // The node's key edges (key +0, four resources) and slot -> side (a5, four u32), and
        // each slot edge's internal points (edge +8..+0x10, (x, y) i32 pairs).
        let mut key = Vec::new();
        for i in 0..4 {
            let e = read::<usize>(a4 + i * 8).unwrap_or(0);
            key.push(if e == 0 {
                "-".to_string()
            } else {
                resource_path(e).rsplit('/').next().unwrap_or("").to_string()
            });
        }
        let sides: [u32; 4] = read(a5).unwrap_or_default();
        let mut slots = Vec::new();
        for i in 0..4 {
            let e = read::<usize>(node + 0x58 + i * 8).unwrap_or(0);
            if e == 0 {
                continue;
            }
            let (pb, pe) = (read::<usize>(e + 8).unwrap_or(0), read::<usize>(e + 0x10).unwrap_or(0));
            let mut pts = Vec::new();
            let mut q = pb;
            while q < pe && pts.len() < 16 {
                let xy: [i32; 2] = read(q).unwrap_or_default();
                pts.push(format!("{},{}", xy[0], xy[1]));
                q += 8;
            }
            let from = read::<usize>(e + 0x20).unwrap_or(0) == node;
            slots.push(format!("s{i}{}[{}]", if from { ">" } else { "<" }, pts.join(" ")));
        }
        trace_line(
            "node_key",
            "1420c76d0",
            &format!("key {} sides {:?}", key.join("|"), sides),
            &slots.join(" "),
        );
        trace_line(
            "node_candidates",
            "1420c76d0",
            &format!("{} total={}", node_desc(node), read::<i32>(list + 0x18).unwrap_or(0)),
            &out.join(" "),
        );
    }
    ret
}

fn edge_points(e: usize) -> String {
    let (pb, pe) = (read::<usize>(e + 8).unwrap_or(0), read::<usize>(e + 0x10).unwrap_or(0));
    let mut pts = Vec::new();
    let mut q = pb;
    while q < pe && pts.len() < 64 {
        let xy: [i32; 2] = read(q).unwrap_or_default();
        pts.push(format!("{},{}", xy[0], xy[1]));
        q += 8;
    }
    pts.join(" ")
}

type RouteFn = unsafe extern "win64" fn(usize, usize) -> usize;

/// Dumps a layout graph (nodes, then edges) as trace lines of `kind`.
fn dump_graph(kind: &str, graph: usize) {
    let vec = |at: usize| -> Vec<usize> {
        let (b, e) = (read::<usize>(at).unwrap_or(0), read::<usize>(at + 8).unwrap_or(0));
        let mut out = Vec::new();
        let mut q = b;
        while q < e && out.len() < 4096 {
            out.push(read::<usize>(q).unwrap_or(0));
            q += 8;
        }
        out
    };
    let nodes = vec(graph + 0x28);
    let edges = vec(graph + 0x10);
    let node_id = |n: usize| nodes.iter().position(|&x| x == n).map_or(-1, |i| i as i64);
    let edge_id = |e: usize| edges.iter().position(|&x| x == e).map_or(-1, |i| i as i64);
    for (i, &n) in nodes.iter().enumerate() {
        let pos: [i32; 2] = read(n + 0x4c).unwrap_or_default();
        let slots: Vec<i64> = (0..4)
            .map(|k| read::<usize>(n + 0x58 + k * 8).unwrap_or(0))
            .map(|e| if e == 0 { -1 } else { edge_id(e) })
            .collect();
        let flags: [u8; 4] = read(n + 0x88).unwrap_or_default();
        trace_line(
            kind,
            "node",
            &format!("{i} {},{} slots {:?} h {} flags {:?}", pos[0], pos[1], slots, read::<i32>(n + 0x48).unwrap_or(0), flags),
            "",
        );
    }
    for (i, &e) in edges.iter().enumerate() {
        let from = node_id(read::<usize>(e + 0x20).unwrap_or(0));
        let to = node_id(read::<usize>(e + 0x28).unwrap_or(0));
        let et = read::<usize>(e + 0x38).map(resource_path).unwrap_or_default();
        let vends: [i32; 8] = read(e + 0x4c).unwrap_or_default();
        let vflags: [u8; 3] = read(e + 0x48).unwrap_or_default();
        let off: [i32; 4] = read(e + 0x70).unwrap_or_default();
        let width: [i32; 2] = read(e + 0x80).unwrap_or_default();
        let split = read::<u8>(e + 0x6d).unwrap_or(0);
        // The `.et` data's flag and number bytes (+0x40..+0x50).
        let etdata: [u8; 16] = read::<usize>(e + 0x38)
            .and_then(|h| read::<usize>(h + 0x28))
            .and_then(|d| read(d + 0x40))
            .unwrap_or_default();
        trace_line(
            kind,
            "edge",
            &format!(
                "{i} {from}->{to} {} vflags {:?} vends {:?} off {:?} width {:?} split {split} et {:?}",
                et.rsplit('/').next().unwrap_or(""),
                vflags,
                vends,
                off,
                width,
                etdata
            ),
            &edge_points(e),
        );
    }
}

type LayoutFn = unsafe extern "win64" fn(usize, usize) -> usize;

unsafe extern "win64" fn on_split_virtual(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_split_virtual");
    let r = unsafe { &*regs };
    let (layout, a2) = (r.rcx as usize, r.rdx as usize);
    let f: LayoutFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(layout, a2) });
    if on_generator_thread() {
        dump_graph("graph_after_split", read::<usize>(layout + 0x20).unwrap_or(0));
    }
    ret
}

unsafe extern "win64" fn on_smooth_edges(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_smooth_edges");
    let r = unsafe { &*regs };
    let (layout, a2) = (r.rcx as usize, r.rdx as usize);
    let f: LayoutFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(layout, a2) });
    if on_generator_thread() {
        dump_graph(&format!("graph_after_smooth{}", a2 as u8), read::<usize>(layout + 0x20).unwrap_or(0));
    }
    ret
}

type EdgeStepFn = unsafe extern "win64" fn(usize, usize, usize) -> usize;

/// Logs an edge before and after a smoothing step.
unsafe extern "win64" fn on_edge_step(regs: *mut Registers, original: usize, which: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_edge_step");
    let r = unsafe { &*regs };
    let (ctx, edge, a3) = (r.rcx as usize, r.rdx as usize, r.r8 as usize);
    let traced = on_generator_thread();
    let ends = |e: usize| {
        let p = |n: usize| -> [i32; 2] { read(n + 0x4c).unwrap_or_default() };
        let from = p(read::<usize>(e + 0x20).unwrap_or(0));
        let to = p(read::<usize>(e + 0x28).unwrap_or(0));
        format!("{},{}->{},{}", from[0], from[1], to[0], to[1])
    };
    let before = traced.then(|| (ends(edge), edge_points(edge)));
    let f: EdgeStepFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(ctx, edge, a3) });
    if let Some((e0, p0)) = before {
        let name = ["arc_start", "arc_end", "round"][which];
        let off: [i32; 4] = read(edge + 0x70).unwrap_or_default();
        trace_line(
            name,
            &format!("{a3}"),
            &format!("{e0} off {off:?} before [{p0}] after {}", ends(edge)),
            &edge_points(edge),
        );
    }
    ret
}

unsafe extern "win64" fn on_edge_tiles(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_edge_tiles");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    // `0x1420b6950(&state)`: state +0x30 is the layout (its +0x20 the graph).
    let layout = read::<usize>(r.rcx as usize + 0x30).unwrap_or(0);
    dump_graph("graph_before_tiles", read::<usize>(layout + 0x20).unwrap_or(0));
}

static GRID_DUMPED: AtomicBool = AtomicBool::new(false);

unsafe extern "win64" fn on_route_edge(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_route_edge");
    let r = unsafe { &*regs };
    let (router, edge) = (r.rcx as usize, r.rdx as usize);
    let before = on_generator_thread().then(|| edge_points(edge));
    if before.is_some() && (std::env::var_os("POE_HOOK_ROUTE_GRIDS").is_some() || !GRID_DUMPED.swap(true, Ordering::Relaxed)) {
        // The router's tile grid before the first edge: cost, uses, occupied (+0x28, 12 bytes).
        let w = read::<u32>(router + 0x40).unwrap_or(0);
        let h = read::<u32>(router + 0x44).unwrap_or(0);
        let base = read::<usize>(router + 0x28).unwrap_or(0);
        let mut cells = Vec::with_capacity((w * h) as usize);
        for i in 0..(w * h) as usize {
            let c: [i32; 3] = read(base + i * 12).unwrap_or_default();
            cells.push(format!("{}:{}:{}", c[0], c[1], c[2] & 0xff));
        }
        trace_line("route_grid", "", &format!("{w} {h}"), &cells.join(" "));
    }
    let f: RouteFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe { f(router, edge) });
    if let Some(before) = before {
        let pos = |n: usize| {
            let x = read::<i32>(n + 0x4c).unwrap_or(0);
            let y = read::<i32>(n + 0x50).unwrap_or(0);
            format!("{x},{y}")
        };
        let from = read::<usize>(edge + 0x20).unwrap_or(0);
        let to = read::<usize>(edge + 0x28).unwrap_or(0);
        let off: [i32; 4] = read(edge + 0x70).unwrap_or_default();
        let width: [u32; 2] = read(edge + 0x80).unwrap_or_default();
        let et = read::<usize>(edge + 0x38)
            .map(resource_path)
            .unwrap_or_default();
        trace_line(
            "route_edge",
            "1420cd460",
            &format!(
                "{} {}->{} off {:?} width {:?} ok={} before [{}]",
                et.rsplit('/').next().unwrap_or(""),
                pos(from),
                pos(to),
                off,
                width,
                ret as u8,
                before
            ),
            &edge_points(edge),
        );
    }
    ret
}

unsafe extern "win64" fn on_phase(regs: *mut Registers, index: usize) {
    let _guard = crate::hook_guard!("terrain_tools::on_phase");
    if !on_generator_thread() {
        return;
    }
    let r = unsafe { &*regs };
    let (addr, name) = PHASES[index];
    trace_line(
        "phase",
        &callers(r.rsp as usize, 2),
        &format!("{name} {addr:x}"),
        &format!("rdx={:x} r8={:x}", r.rdx, r.r8),
    );
}

type MtFn = unsafe extern "win64" fn(usize) -> u32;

unsafe extern "win64" fn on_mt_next(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_mt_next");
    let r = unsafe { &*regs };
    let state = r.rcx as usize;
    let f: MtFn = unsafe { std::mem::transmute(original) };
    let index = read::<u32>(state).unwrap_or(0);
    let v = guard.original(|| unsafe { f(state) });
    if on_generator_thread() {
        let ret = read::<usize>(r.rsp as usize).map_or(0, ghidra);
        trace_line(
            "mt",
            &format!("{ret:x}"),
            &format!("state={state:x} i={index}"),
            &v.to_string(),
        );
    }
    v as usize
}

type UniformFn = unsafe extern "win64" fn(usize, u32) -> u64;

unsafe extern "win64" fn on_uniform(regs: *mut Registers, original: usize, _: usize) -> usize {
    let mut guard = crate::hook_guard!("terrain_tools::on_uniform");
    let r = unsafe { &*regs };
    let (engine, n) = (r.rcx as usize, r.rdx as u32);
    let f: UniformFn = unsafe { std::mem::transmute(original) };
    let v = guard.original(|| unsafe { f(engine, n) });
    if on_generator_thread() {
        let state = read::<usize>(engine).unwrap_or(0);
        let ret = read::<usize>(r.rsp as usize).map_or(0, ghidra);
        trace_line(
            "uniform",
            &format!("{ret:x}|{}", callers(r.rsp as usize, 3)),
            &format!("n={n} state={state:x}"),
            &(v as u32).to_string(),
        );
    }
    v as usize
}

/// MSVC `std::wstring` at `addr` (buffer or pointer +0, size +0x10, capacity +0x18).
fn msvc_wstring(addr: usize) -> Option<String> {
    let len = read::<usize>(addr + 0x10)?;
    let cap = read::<usize>(addr + 0x18)?;
    if len > 1024 || cap < len {
        return None;
    }
    let data = if cap > 7 { read::<usize>(addr)? } else { addr };
    if !readable(data, len * 2) {
        return None;
    }
    let units = unsafe { std::slice::from_raw_parts(data as *const u16, len) };
    Some(String::from_utf16_lossy(units))
}

/// A resource's path (resource +8 is its `std::wstring` path; tile objects and object types).
fn resource_path(resource: usize) -> String {
    msvc_wstring(resource + 8)
        .or_else(|| read::<usize>(resource + 8).and_then(|p| wide_string(p, 300)))
        .unwrap_or_default()
}

const M: u32 = 0x5bd1e995;

fn mix(k: u32) -> u32 {
    let k = k.wrapping_mul(M);
    (k ^ (k >> 24)).wrapping_mul(M)
}

fn finish(h: u32) -> u32 {
    let h = (h ^ (h >> 13)).wrapping_mul(M);
    h ^ (h >> 15)
}

/// The client's string hash (`0x140419970`): MurmurHash2 over UTF-16 code units, two at a time.
pub fn hash_utf16(units: &[u16], seed: u32) -> u32 {
    let mut h = (units.len() as u32 * 2) ^ seed;
    let mut chunks = units.chunks_exact(2);
    for c in &mut chunks {
        let k = c[0] as u32 | (c[1] as u32) << 16;
        h = h.wrapping_mul(M) ^ mix(k);
    }
    if let [u] = chunks.remainder() {
        h = (*u as u32 ^ h).wrapping_mul(M);
    }
    finish(h)
}

/// MurmurHash2 over bytes (`0x142194f90`).
pub fn hash_bytes(data: &[u8], seed: u32) -> u32 {
    let mut h = data.len() as u32 ^ seed;
    let mut chunks = data.chunks_exact(4);
    for c in &mut chunks {
        h = h.wrapping_mul(M) ^ mix(u32::from_le_bytes([c[0], c[1], c[2], c[3]]));
    }
    let rest = chunks.remainder();
    if !rest.is_empty() {
        if rest.len() == 3 {
            h ^= (rest[2] as u32) << 16;
        }
        if rest.len() >= 2 {
            h ^= (rest[1] as u32) << 8;
        }
        h = (rest[0] as u32 ^ h).wrapping_mul(M);
    }
    finish(h)
}

/// One tile's contribution to the tile hash (`0x141d141d0`).
fn tile_hash_entry(x: u32, y: u32, height: i16, orientation: u8, path_hash: u32) -> u32 {
    let mut h = (mix(height as i32 as u32) ^ 0x9bfb_c07e).wrapping_mul(M);
    h = (mix(orientation as u32) ^ h).wrapping_mul(M);
    h = (mix(x) ^ h).wrapping_mul(M);
    h = mix(y) ^ h;
    finish(h.wrapping_mul(M) ^ mix(path_hash))
}

struct Tiles {
    w: usize,
    h: usize,
    records: usize,
}

fn tiles(terrain: usize) -> Option<Tiles> {
    let w = read::<u32>(terrain + 0x18)? as usize;
    let h = read::<u32>(terrain + 0x20)? as usize;
    let records = read::<usize>(terrain + 0x28)?;
    (w > 0 && h > 0 && w * h < 1_000_000 && readable(records, w * h * TILE_RECORD))
        .then_some(Tiles { w, h, records })
}

/// `layout.tsv` and the recomputed tile hash.
fn layout(t: &Tiles) -> (String, u32) {
    let mut out = String::from(
        "# x\ty\ttile path\tsub-tile index\theight(+0x30)\torientation(+0x36)\tskip(+0x34,+0x35)\tnibbles(+0x32,+0x33)\tflags(+0x37)\tlist(+0x10) entries\traw +0x10..+0x38\n",
    );
    // A tile object's sub-tile array starts at its lowest sub-tile pointer.
    let mut starts: HashMap<usize, usize> = HashMap::new();
    let record = |x: usize, y: usize| t.records + (y * t.w + x) * TILE_RECORD;
    for y in 0..t.h {
        for x in 0..t.w {
            if let (Some(q0), Some(o)) =
                (read::<usize>(record(x, y)), read::<usize>(record(x, y) + 8))
                && q0 != 0
            {
                let e = starts.entry(o).or_insert(q0);
                *e = (*e).min(q0);
            }
        }
    }
    let mut hashes = Vec::new();
    let mut path_cache: HashMap<usize, (String, u32)> = HashMap::new();
    // The client hashes column by column.
    for x in 0..t.w {
        for y in 0..t.h {
            let r = record(x, y);
            let q0 = read::<usize>(r).unwrap_or(0);
            let object = read::<usize>(r + 8).unwrap_or(0);
            let height = read::<i16>(r + 0x30).unwrap_or(0);
            let bytes: [u8; 8] = read(r + 0x30).unwrap_or_default();
            let (orientation, skip) = (bytes[6], (bytes[4], bytes[5]));
            let (path, path_hash) = path_cache
                .entry(object)
                .or_insert_with(|| {
                    let p = if object != 0 {
                        resource_path(object)
                    } else {
                        String::new()
                    };
                    let units: Vec<u16> = p.encode_utf16().collect();
                    let hp = hash_utf16(&units, 0xf3_6b12);
                    (p, hp)
                })
                .clone();
            if q0 != 0 && skip == (0, 0) {
                hashes.push(tile_hash_entry(
                    x as u32,
                    y as u32,
                    height,
                    orientation,
                    path_hash,
                ));
            }
            let sub = starts
                .get(&object)
                .map_or(0, |&s| q0.saturating_sub(s) / 0x38);
            let list = match (read::<usize>(r + 0x10), read::<usize>(r + 0x18)) {
                (Some(b), Some(e)) if e >= b => (e - b) / 16,
                _ => 0,
            };
            let raw: [u8; 0x28] = read(r + 0x10).unwrap_or([0; 0x28]);
            let raw: String = raw.iter().map(|b| format!("{b:02x}")).collect();
            let _ = writeln!(
                out,
                "{x}\t{y}\t{path}\t{sub}\t{height}\t{orientation}\t{},{}\t{},{}\t{:#x}\t{list}\t{raw}",
                skip.0, skip.1, bytes[2], bytes[3], bytes[7]
            );
        }
    }
    let bytes: Vec<u8> = hashes.iter().flat_map(|h| h.to_le_bytes()).collect();
    (out, hash_bytes(&bytes, 0xf3_6b12))
}

/// An object's Positioned data: grid cell, world position, facing.
fn positioned(entity: usize) -> Option<((i32, i32), (f32, f32), f32)> {
    let m = MAIN.get()?;
    let reader = m.from_ghidra(POSITIONED_READER);
    let comps = crate::entities::components(entity);
    let pos = comps.into_iter().find(|&c| {
        read::<usize>(c)
            .and_then(|vt| read::<usize>(vt + 0x98))
            .is_some_and(|f| f == reader)
    })?;
    Some((
        (read::<i32>(pos + 0x444)?, read::<i32>(pos + 0x448)?),
        (read::<f32>(pos + 0x490)?, read::<f32>(pos + 0x494)?),
        read::<f32>(pos + 0x44c)?,
    ))
}

fn entity_line(source: &str, entity: usize, region: usize) -> Option<String> {
    let resource = read::<usize>(entity + 8)?;
    let path = resource_path(resource);
    let id = read::<u32>(entity + 0x88).unwrap_or(0);
    let (cell, world, facing) = positioned(entity)
        .map(|(c, w, f)| {
            (
                format!("{}\t{}", c.0, c.1),
                format!("{:.1}\t{:.1}", w.0, w.1),
                format!("{f:.4}"),
            )
        })
        .unwrap_or_else(|| ("\t".into(), "\t".into(), String::new()));
    let comps: Vec<String> = crate::entities::component_names(entity)
        .into_iter()
        .map(|(_, n)| n)
        .collect();
    Some(format!(
        "{source}\t{id}\t{path}\t{cell}\t{world}\t{facing}\t{region:#x}\t{}",
        comps.join(",")
    ))
}

fn entities_tsv(objects: &[(usize, usize)], with_server: bool) -> String {
    let mut out = String::from(
        "# source\tid\tpath\tgrid x\tgrid y\tworld x\tworld y\tfacing\tregion\tcomponents\n",
    );
    for &(e, region) in objects {
        if let Some(l) = entity_line("generator", e, region) {
            out += &l;
            out.push('\n');
        }
    }
    if with_server {
        for e in crate::entities::list() {
            if let Some(l) = entity_line("server", e.ptr, 0) {
                out += &l;
                out.push('\n');
            }
        }
    }
    out
}

fn write(dir: &std::path::Path, name: &str, data: impl AsRef<[u8]>) {
    if let Err(e) = std::fs::write(dir.join(name), data) {
        log::error!("area export {name}: {e}");
    }
}

fn export_area(
    world: usize,
    seed: u32,
    objects: Vec<(usize, usize)>,
    doodads: Vec<Doodad>,
    traced: bool,
) {
    let Some(terrain) = crate::dump_types::terrain(world) else {
        log::warn!("area export: no terrain object");
        return;
    };
    let area_row = read::<usize>(world + 0x98).unwrap_or(0);
    let hash16 = read::<u16>(area_row + 0x2a).unwrap_or(0);
    let kind = read::<u32>(area_row + 0x1b4).unwrap_or(0);
    let n = AREA_COUNT.fetch_add(1, Ordering::Relaxed);
    let dir = crate::host::session_dir()
        .join("areas")
        .join(format!("{n:02}_{hash16:04x}_{seed}"));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::error!("area export: {e}");
        return;
    }
    let tile_hash = read::<u32>(world + 0x114).unwrap_or(0);
    let doodad_hash = read::<u32>(world + 0x118).unwrap_or(0);
    let mut meta = format!(
        "area_hash16\t{hash16:#06x}\nseed\t{seed}\narea_kind\t{kind:#x}\nworld_4b0\t{}\nworld_4b4\t{}\ntile_hash\t{tile_hash}\ndoodad_hash\t{doodad_hash}\n",
        read::<u32>(world + 0x4b0).unwrap_or(0),
        read::<u32>(world + 0x4b4).unwrap_or(0),
    );
    if let Some(t) = tiles(terrain) {
        let (text, computed) = layout(&t);
        // The client stores 1 when the hash comes out 0.
        let ok = computed == tile_hash || (computed == 0 && tile_hash == 1);
        let _ = write!(
            meta,
            "size_tiles\t{}\t{}\ncomputed_tile_hash\t{computed}\t{}\n",
            t.w,
            t.h,
            if ok { "match" } else { "MISMATCH" }
        );
        if !ok {
            log::warn!(
                "area export: recomputed tile hash {computed} != client {tile_hash} (layout fields incomplete?)"
            );
        }
        write(&dir, "layout.tsv", text);
        if let Some(grids) = crate::dump_types::grid_bytes(terrain, t.w, t.h) {
            write(&dir, "grids.bin", grids);
        }
    }
    let computed = doodads.iter().fold(0u32, |h, &(x, y, _, ph, _)| {
        h.wrapping_add(y as u32)
            .wrapping_add(ph)
            .wrapping_add((x as u32) << 16)
    });
    let _ = writeln!(
        meta,
        "computed_doodad_hash\t{computed}\t{}",
        if computed == doodad_hash {
            "match"
        } else {
            "MISMATCH"
        }
    );
    let mut text = String::from("# x\ty\tangle\tpath hash\tpath\n");
    for (x, y, a, h, p) in &doodads {
        let _ = writeln!(text, "{x}\t{y}\t{a:.4}\t{h}\t{p}");
    }
    write(&dir, "doodads.tsv", text);
    write(&dir, "meta.txt", &meta);
    write(&dir, "entities.tsv", entities_tsv(&objects, false));
    if traced && TRACE.load(Ordering::Relaxed) {
        let buf = std::mem::take(&mut *TRACE_BUF.lock().unwrap());
        let mut text = String::from("# seq\tkind\tcaller(s)\targs\tresult\n");
        for l in &buf {
            text += l;
            text.push('\n');
        }
        write(&dir, "generator_trace.tsv", text);
        log::info!("generator trace: {} lines", buf.len());
    }
    log::info!(
        "area exported to {} ({} generator objects, tile hash {tile_hash}, doodad hash {doodad_hash})",
        dir.display(),
        objects.len()
    );
    // World-map panels (kinds 0x20/0x21/0x23) are generated too, but the player is not in them.
    if matches!(kind, 0x20 | 0x21 | 0x23) {
        return;
    }
    finish_current();
    *CURRENT.lock().unwrap() = Some(Area {
        dir,
        world,
        visited: HashSet::new(),
        objects,
    });
}

/// Writes the walkability probe of the current area.
fn write_probe(area: &Area) {
    let Some(terrain) = crate::dump_types::terrain(area.world) else {
        return;
    };
    let layers: Vec<_> = (0..4).map(crate::minimap::Grid::current).collect();
    let mut zeros = [0usize; 4];
    let mut out = String::from("# grid x\tgrid y\tlayer0\tlayer1\tlayer2\tlayer3\n");
    let mut cells: Vec<_> = area.visited.iter().copied().collect();
    cells.sort();
    for (x, y) in cells {
        let mut line = format!("{x}\t{y}");
        for (i, g) in layers.iter().enumerate() {
            let v = g
                .as_ref()
                .and_then(|g| g.get(x as i64, y as i64))
                .map_or(-1, |v| v as i32);
            if v == 0 {
                zeros[i] += 1;
            }
            let _ = write!(line, "\t{v}");
        }
        out += &line;
        out.push('\n');
    }
    let _ = terrain;
    let summary = format!(
        "# {} cells visited; cells with value 0 per layer: {zeros:?} (a walkability layer has none)\n",
        area.visited.len()
    );
    write(&area.dir, "probe.tsv", summary + &out);
    log::info!(
        "walkability probe: {} cells, zero cells per layer {zeros:?}",
        area.visited.len()
    );
}

/// Ends the current area: its probe is written (called when the next area generates).
fn finish_current() {
    if let Some(area) = CURRENT.lock().unwrap().take()
        && !area.visited.is_empty()
    {
        write_probe(&area);
    }
}

/// "Export area" (Minimap tab): writes the current area again with the server's entities and
/// the probe so far. False when no area was exported yet.
pub fn export_now() -> Option<PathBuf> {
    let area = CURRENT.lock().unwrap().clone()?;
    write(&area.dir, "entities.tsv", entities_tsv(&area.objects, true));
    write_probe(&area);
    Some(area.dir)
}

/// Samples the player's cell for the walkability probe; the probe file is rewritten every
/// 30 s while it grows.
fn probe_loop() {
    let mut ticks = 0u32;
    let mut written = (PathBuf::new(), 0usize);
    while crate::hooks::sleep(Duration::from_millis(100)) {
        ticks += 1;
        let player = crate::dump_types::player();
        if player != 0
            && let Some((cell, _, _)) = positioned(player)
            && let Some(area) = CURRENT.lock().unwrap().as_mut()
        {
            area.visited.insert(cell);
        }
        if ticks % 300 == 0 {
            let area = CURRENT.lock().unwrap().clone();
            if let Some(area) = area
                && (area.dir != written.0 || area.visited.len() > written.1)
                && !area.visited.is_empty()
            {
                written = (area.dir.clone(), area.visited.len());
                write_probe(&area);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur_matches_reference() {
        // MurmurHash2("", seed 0) = 0; four bytes exercise one block.
        assert_eq!(hash_bytes(&[], 0), 0);
        assert_eq!(hash_bytes(b"abcd", 0), murmur2_reference(b"abcd", 0));
        assert_eq!(hash_bytes(b"abcdefg", 7), murmur2_reference(b"abcdefg", 7));
    }

    /// The textbook MurmurHash2.
    fn murmur2_reference(data: &[u8], seed: u32) -> u32 {
        let mut h = seed ^ data.len() as u32;
        let mut i = 0;
        while data.len() - i >= 4 {
            let mut k = u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
            k = k.wrapping_mul(M);
            k ^= k >> 24;
            k = k.wrapping_mul(M);
            h = h.wrapping_mul(M);
            h ^= k;
            i += 4;
        }
        let rest = &data[i..];
        match rest.len() {
            3 => {
                h ^= (rest[2] as u32) << 16;
                h ^= (rest[1] as u32) << 8;
                h ^= rest[0] as u32;
                h = h.wrapping_mul(M);
            }
            2 => {
                h ^= (rest[1] as u32) << 8;
                h ^= rest[0] as u32;
                h = h.wrapping_mul(M);
            }
            1 => {
                h ^= rest[0] as u32;
                h = h.wrapping_mul(M);
            }
            _ => {}
        }
        h ^= h >> 13;
        h = h.wrapping_mul(M);
        h ^ (h >> 15)
    }
}
