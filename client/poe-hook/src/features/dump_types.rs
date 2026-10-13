//! Dumps the client's object-type table the first time it is used.
//!
//! `0x1416936c0(registry, out, key)` binary-searches a sorted array of 0x30-byte entries
//! `{u32 key, .., wchar* path at +8}` to resolve the object type key carried by the
//! server's add-object message (0x2f7). The dump (`object_types.txt` next to the exe, one
//! `key<TAB>path` per line) gives the key for each metadata path.
//!
//! It also traces entity deserialization: the components of each entity read by
//! `READ_INITIAL_STATE` and, per component reader, the blob span it consumed. That gives the
//! exact layout of a server-sent blob in one run.

use std::{
    collections::HashSet,
    io::Write,
    path::PathBuf,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use ilhook::x64::{CallbackOption, HookFlags, HookType, Hooker, Registers};

use crate::module::Module;

/// Ghidra address of the object-type lookup (0.5.5e).
const LOOKUP: usize = 0x1416936c0;
/// Entity initial-state reader `(entity, blob)`: logs the entity's components (0.5.5e).
const READ_INITIAL_STATE: usize = 0x141e71120;
const ENTRY_SIZE: usize = 0x30;
/// Shared `ret 0` stub used as the reader of components without initial state.
const NOOP_READER: usize = 0x14010a080;
/// Area loads that generate the terrain and log "Tile hash"/"Doodad hash" (`this` in rcx; the
/// instance path is `0x14167b270`). When one returns, the layout is exported for the server
/// (`terrain_layout_<tile hash>.tsv`).
const LOAD_AREA: [usize; 2] = [0x14167b270, 0x14212f500];
/// Vtable of the area terrain object (size in tiles at +0x18/+0x20, tiles at +0x28).
const TERRAIN_VTABLE: usize = 0x143385b10;
/// Positioned component reader: after it runs, the terrain size is logged (the reader clamps
/// the grid position to it).
const POSITIONED_READER: usize = 0x141dd8820;

/// Readers already hooked (Ghidra addresses).
static HOOKED_READERS: Mutex<Option<HashSet<usize>>> = Mutex::new(None);

static OUT: OnceLock<PathBuf> = OnceLock::new();
static DONE: AtomicBool = AtomicBool::new(false);

pub fn init(main: &Module, out: PathBuf) -> Result<(), String> {
    OUT.set(out).ok();
    let hooker = Hooker::new(
        main.from_ghidra(LOOKUP),
        HookType::JmpBack(on_lookup),
        CallbackOption::None,
        0,
        HookFlags::empty(),
    );
    let point = unsafe { hooker.hook() }.map_err(|e| format!("hook: {e:?}"))?;
    Box::leak(Box::new(point));
    MAIN.set(*main).ok();
    let hooker = Hooker::new(
        main.from_ghidra(READ_INITIAL_STATE),
        HookType::JmpBack(on_read_initial_state),
        CallbackOption::None,
        0,
        HookFlags::empty(),
    );
    let point = unsafe { hooker.hook() }.map_err(|e| format!("hook: {e:?}"))?;
    Box::leak(Box::new(point));
    for addr in LOAD_AREA {
        let hooker = Hooker::new(
            main.from_ghidra(addr),
            HookType::Retn(on_load_area),
            CallbackOption::None,
            addr,
            HookFlags::empty(),
        );
        let point = unsafe { hooker.hook() }.map_err(|e| format!("hook load area: {e:?}"))?;
        Box::leak(Box::new(point));
    }
    Ok(())
}

type LoadAreaFn =
    unsafe extern "win64" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;

/// Runs the area load, then exports the generated layout. Stack arguments (5th..8th) are
/// passed through.
unsafe extern "win64" fn on_load_area(regs: *mut Registers, original: usize, at: usize) -> usize {
    let r = unsafe { &*regs };
    let f: LoadAreaFn = unsafe { std::mem::transmute(original) };
    let ret = unsafe {
        f(
            r.rcx as usize,
            r.rdx as usize,
            r.r8 as usize,
            r.r9 as usize,
            r.get_stack(5) as usize,
            r.get_stack(6) as usize,
            r.get_stack(7) as usize,
            r.get_stack(8) as usize,
        )
    };
    log::info!("area load {at:#x} returned");
    unsafe { export_layout(r.rcx as usize) };
    ret
}

/// Writes `terrain_layout_<tile hash>.tsv` next to the exe: a `size<TAB>w<TAB>h` line, then one
/// `x<TAB>y<TAB>tile path` line per tile. Dev bridge until the server generates terrain (M4).
unsafe fn export_layout(world: usize) {
    use crate::mem::{read, wide_string};
    // The terrain object is a member (or pointed-to member) of the world; find it by its
    // vtable instead of assuming an offset.
    let Some(m) = MAIN.get() else { return };
    let terrain_vt = m.from_ghidra(TERRAIN_VTABLE);
    // Embedded by value (the terrain's +8 points back to its owner) or behind a pointer.
    let Some((offset, terrain)) = (0..0x2000).step_by(8).find_map(|off| {
        let v = read::<usize>(world + off)?;
        if v == terrain_vt {
            return Some((off, world + off));
        }
        (read::<usize>(v)? == terrain_vt).then_some((off, v))
    }) else {
        log::warn!("layout export: no terrain object found in world {world:#x}");
        return;
    };
    log::info!("terrain at world+{offset:#x}");
    // Tile hash as logged by the client ("Tile hash: N"), also sent to the server in
    // TerrainHashes; it names the export so the server reads the matching layout. World-map
    // loads (area kind 0x20/0x21/0x23 at +0x98 -> +0x1b4) log no hash and are skipped.
    let kind = read::<usize>(world + 0x98).and_then(|a| read::<u32>(a + 0x1b4));
    if matches!(kind, Some(0x20 | 0x21 | 0x23)) {
        return;
    }
    let Some(tile_hash) = read::<u32>(world + 0x114) else { return };
    let (Some(w), Some(h), Some(tiles)) = (
        read::<u32>(terrain + 0x18),
        read::<u32>(terrain + 0x20),
        read::<usize>(terrain + 0x28),
    ) else {
        return;
    };
    let mut out = format!("size\t{w}\t{h}\n");
    for y in 0..h as usize {
        for x in 0..w as usize {
            // Tile record +8 -> tile object, whose +8 is the .tdt path (see terrain_tiles.txt).
            let path = read::<usize>(tiles + (y * w as usize + x) * 0x38 + 8)
                .and_then(|t| read::<usize>(t + 8))
                .and_then(|p| wide_string(p, 200))
                .unwrap_or_default();
            out += &format!("{x}\t{y}\t{path}\n");
        }
    }
    let path = crate::exe_dir().join(format!("terrain_layout_{tile_hash}.tsv"));
    match std::fs::write(&path, out) {
        Ok(()) => log::info!("exported {w}x{h} layout to {}", path.display()),
        Err(e) => log::error!("layout export: {e}"),
    }
}

static MAIN: OnceLock<Module> = OnceLock::new();

/// Logs each component of the entity being deserialized: object address, vtable and the
/// component's initial-state reader (vtable +0x98), as Ghidra addresses.
unsafe extern "win64" fn on_read_initial_state(regs: *mut Registers, _: usize) {
    let r = unsafe { &*regs };
    let Some(m) = MAIN.get() else { return };
    unsafe {
        let entity = r.rcx as usize;
        let blob = r.rdx as *const usize;
        let (data, len, pos) = (*blob, *blob.add(1), *blob.add(2));
        let begin = *((entity + 0x10) as *const usize);
        let end = *((entity + 0x18) as *const usize);
        let count = (end.saturating_sub(begin)) / 8;
        let id = *((entity + 0x88) as *const u32);
        // The read loop only visits the first `serialized` components (type +0x28 -> +0x78);
        // the rest are client-only and their +0x98 slot is not a reader.
        let ty = *((entity + 8) as *const usize);
        let serialized = if ty > 0x10000 {
            *((*((ty + 0x28) as *const usize) + 0x78) as *const usize)
        } else {
            0
        };
        log::info!("entity {id:#x} initial state: {count} components ({serialized} serialized), blob {len} bytes at pos {pos} (data {data:#x})");
        for i in 0..count.min(64) {
            let comp = *((begin + i * 8) as *const usize);
            if comp < 0x10000 {
                continue;
            }
            let vt = *(comp as *const usize);
            let reader = if m.contains(vt) { *((vt + 0x98) as *const usize) } else { 0 };
            // The read loop only calls components whose owner (+8) is this entity.
            let owned = *((comp + 8) as *const usize) == entity;
            log::info!(
                "  component {i}: vtable={:#x} reader={:#x} owned={owned}",
                if m.contains(vt) { m.to_ghidra(vt) } else { vt },
                if m.contains(reader) { m.to_ghidra(reader) } else { reader }
            );
            if owned && i < serialized && m.contains(reader) {
                hook_reader(m, m.to_ghidra(reader));
            }
        }
    }
}

/// Wraps a component reader to log the blob span it consumes. Installed lazily, before the
/// reader first runs (we are at the entry of the function that calls it).
fn hook_reader(m: &Module, ghidra: usize) {
    if ghidra == NOOP_READER {
        return;
    }
    let mut hooked = HOOKED_READERS.lock().unwrap_or_else(|e| e.into_inner());
    if !hooked.get_or_insert_with(HashSet::new).insert(ghidra) {
        return;
    }
    let hooker = Hooker::new(
        m.from_ghidra(ghidra),
        HookType::Retn(on_reader),
        CallbackOption::None,
        ghidra,
        HookFlags::empty(),
    );
    match unsafe { hooker.hook() } {
        Ok(point) => {
            Box::leak(Box::new(point));
        }
        Err(e) => log::error!("hook reader {ghidra:#x}: {e:?}"),
    }
}

type ReaderFn = unsafe extern "win64" fn(usize, usize, usize, usize) -> usize;

unsafe extern "win64" fn on_reader(regs: *mut Registers, original: usize, ghidra: usize) -> usize {
    let r = unsafe { &*regs };
    let blob = r.rdx as *const usize;
    let start = unsafe { *blob.add(2) };
    log::info!("    reader {ghidra:#x} starts at blob pos {start}");
    let f: ReaderFn = unsafe { std::mem::transmute(original) };
    let ret = unsafe { f(r.rcx as usize, r.rdx as usize, r.r8 as usize, r.r9 as usize) };
    let end = unsafe { *blob.add(2) };
    log::info!("    reader {ghidra:#x} read {} bytes ({start}..{end})", end - start);
    if ghidra == POSITIONED_READER {
        unsafe { log_terrain(r.rcx as usize) };
    }
    ret
}

type TerrainFn = unsafe extern "win64" fn(usize) -> usize;

/// Logs the area terrain the Positioned component was clamped to: the world's terrain object
/// (world vtable +0x1a8) has the size in tiles at +0x18/+0x20 (23 grid cells per tile).
unsafe fn log_terrain(component: usize) {
    unsafe {
        let entity = *((component + 8) as *const usize);
        let world = *((entity + 0x78) as *const usize);
        let vt = *(world as *const usize);
        let f: TerrainFn = std::mem::transmute(*((vt + 0x1a8) as *const usize));
        let terrain = f(world);
        let (w, h) = (
            *((terrain + 0x18) as *const u32),
            *((terrain + 0x20) as *const u32),
        );
        let (x, y) = (
            *((component + 0x444) as *const i32),
            *((component + 0x448) as *const i32),
        );
        let floats: Vec<String> = (0..16)
            .map(|i| format!("{:.2}", *((component + 0x480 + i * 4) as *const f32)))
            .collect();
        log::info!("    positioned +0x480.. floats [{}]", floats.join(" "));
        let head: Vec<String> = (0..16)
            .map(|i| format!("{:#x}", *((terrain + i * 8) as *const usize)))
            .collect();
        log::info!(
            "    terrain {terrain:#x}: {w}x{h} tiles ({}x{} grid); player grid ({x}, {y}); head [{}]",
            w * 23,
            h * 23,
            head.join(" ")
        );
        dump_tiles(terrain, w as usize, h as usize);
    }
}

/// Tile records (0x38 bytes, vector at terrain +0x28, row-major `w`×`h`) are written to
/// `terrain_tiles.txt` with any metadata/art paths reachable from them (pointer chains up to
/// two levels, read with guards since the record layout is not known yet). Shows where the
/// generator placed each room, e.g. the start room of an area.
fn dump_tiles(terrain: usize, w: usize, h: usize) {
    use crate::mem::{read, wide_string};
    let Some(tiles) = read::<usize>(terrain + 0x28) else { return };
    let path_at = |p: usize| -> Option<String> {
        let s = wide_string(p, 200)?;
        (s.starts_with("Metadata/") || s.starts_with("Art/")).then_some(s)
    };
    let mut out = String::new();
    for y in 0..h {
        for x in 0..w {
            let rec = tiles + (y * w + x) * 0x38;
            let mut found = Vec::new();
            for q in 0..7 {
                let Some(p1) = read::<usize>(rec + q * 8) else { continue };
                if let Some(s) = path_at(p1) {
                    found.push(format!("q{q}:{s}"));
                    continue;
                }
                for k in (0..0x60).step_by(8) {
                    let Some(p2) = read::<usize>(p1.wrapping_add(k)) else { break };
                    if let Some(s) = path_at(p2) {
                        found.push(format!("q{q}+{k:#x}:{s}"));
                        break;
                    }
                }
            }
            let raw: Vec<String> = (0..7)
                .map(|q| format!("{:x}", read::<usize>(rec + q * 8).unwrap_or(0)))
                .collect();
            out += &format!("{x} {y} [{}] {}\n", raw.join(" "), found.join(" | "));
        }
    }
    let path = crate::exe_dir().join("terrain_tiles.txt");
    match std::fs::write(&path, out) {
        Ok(()) => log::info!("    dumped {} tiles to {}", w * h, path.display()),
        Err(e) => log::error!("    tile dump: {e}"),
    }
}

unsafe fn wide(ptr: usize) -> String {
    if ptr < 0x10000 {
        return String::new();
    }
    let p = ptr as *const u16;
    let mut len = 0;
    while len < 512 && unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(p, len) })
}

unsafe extern "win64" fn on_lookup(regs: *mut Registers, _: usize) {
    let r = unsafe { &*regs };
    log::info!("object type lookup key={:#010x}", r.r8 as u32);
    if DONE.swap(true, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let registry = r.rcx as *const usize;
        let vec = *registry as *const usize;
        let (begin, end) = (*vec, *vec.add(1));
        let count = (end - begin) / ENTRY_SIZE;
        let Some(path) = OUT.get() else { return };
        let Ok(mut f) = std::fs::File::create(path) else {
            return;
        };
        for i in 0..count {
            let e = begin + i * ENTRY_SIZE;
            let key = *(e as *const u32);
            let name = wide(*((e + 8) as *const usize));
            let _ = writeln!(f, "{key:#010x}\t{name}");
        }
        log::info!("dumped {count} object types to {}", path.display());
    }
}
