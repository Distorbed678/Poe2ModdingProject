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
