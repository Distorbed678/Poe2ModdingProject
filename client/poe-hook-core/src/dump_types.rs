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

use ilhook::x64::{HookType, Registers};

use poe_hook_shared::module::Module;

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
/// Last entity created with id 1 (our player) and how often the per-frame entity update
/// (`ENTITY_UPDATE`) ran for it.
static PLAYER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static PLAYER_UPDATES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Per-frame update of an awake entity (`(entity, dt)`).
const ENTITY_UPDATE: usize = 0x141e71090;
/// World frame drain (`world, f32 dt`): applies queued simulation packets, updates entities.
const FRAME_DRAIN: usize = 0x1421306c0;
/// Per-entity tail update in frame mode (`entity + 0x58, dt`), from `0x1421370f0`.
const ENTITY_TICK: usize = 0x141e7d7f0;
/// Positioned::SetOrientation (`this`, angle): writes +0x44c unless the orientation is locked.
const SET_ORIENTATION: usize = 0x141ddc6a0;
/// Game update vfunc (`this, ?, f32 xmm2, f32 dt xmm3`): adds dt to this +0x17cc, which the
/// world update (`0x14169c0b0`) takes (and zeroes) as the frame-mode dt.
const GAME_UPDATE: usize = 0x14167ea40;
static DRAINS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn init(main: &Module, out: PathBuf) -> Result<(), String> {
    OUT.set(out).ok();
    crate::hooks::install(main.from_ghidra(LOOKUP), HookType::JmpBack(on_lookup), 0)
        .map_err(|e| format!("hook: {e}"))?;
    MAIN.set(*main).ok();
    crate::hooks::install(
        main.from_ghidra(READ_INITIAL_STATE),
        HookType::JmpBack(on_read_initial_state),
        0,
    )
    .map_err(|e| format!("hook: {e}"))?;
    crate::hooks::install(
        main.from_ghidra(ENTITY_UPDATE),
        HookType::JmpBack(on_entity_update),
        0,
    )
    .map_err(|e| format!("hook entity update: {e}"))?;
    for (addr, cb) in [
        (
            FRAME_DRAIN,
            on_frame_drain as unsafe extern "win64" fn(*mut Registers, usize),
        ),
        (ENTITY_TICK, on_entity_tick),
        (GAME_UPDATE, on_game_update),
        (SET_ORIENTATION, on_set_orientation),
    ] {
        crate::hooks::install(main.from_ghidra(addr), HookType::JmpBack(cb), 0)
            .map_err(|e| format!("hook {addr:#x}: {e}"))?;
    }
    for addr in LOAD_AREA {
        crate::hooks::install(main.from_ghidra(addr), HookType::Retn(on_load_area), addr)
            .map_err(|e| format!("hook load area: {e}"))?;
    }
    Ok(())
}

fn begin_components(entity: usize) -> usize {
    poe_hook_shared::mem::read::<usize>(entity + 0x10).unwrap_or(0)
}

/// Movement research: who sets the player's facing (logged once per distinct call stack, with
/// the movement log on).
unsafe extern "win64" fn on_set_orientation(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("dump_types::on_set_orientation");
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<Vec<usize>>> = Mutex::new(Vec::new());
    if !MOVEMENT_LOG.load(Ordering::Relaxed) {
        return;
    }
    let Some(m) = MAIN.get() else { return };
    let r = unsafe { &*regs };
    let player = PLAYER.load(Ordering::Relaxed);
    let pos = poe_hook_shared::mem::read::<usize>(player + 0x10)
        .and_then(poe_hook_shared::mem::read::<usize>);
    if player == 0 || pos != Some(r.rcx as usize) {
        return;
    }
    let refs = poe_hook_shared::module::stack_refs(m, r.rsp as usize, 8);
    let mut seen = SEEN.lock().unwrap();
    if seen.contains(&refs) || seen.len() > 40 {
        return;
    }
    seen.push(refs.clone());
    let refs: Vec<String> = refs.iter().map(|a| format!("{a:#x}")).collect();
    log::info!(
        "player SetOrientation angle={:.3}: {}",
        f32::from_bits(r.xmm1 as u32),
        refs.join(" ")
    );
}

unsafe extern "win64" fn on_game_update(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("dump_types::on_game_update");
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    static LAST: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed) + 1;
    let this = unsafe { (*regs).rcx } as usize;
    let changed = LAST.swap(this, Ordering::Relaxed) != this;
    if changed || matches!(n, 1 | 2 | 10 | 100 | 1000 | 5000 | 10000) {
        let r = unsafe { &*regs };
        log::info!(
            "game update #{n}: this {:#x} rdx {:#x} xmm2={} xmm3={} acc={:?}",
            r.rcx,
            r.rdx,
            f32::from_bits(r.xmm2 as u32),
            f32::from_bits(r.xmm3 as u32),
            poe_hook_shared::mem::read::<f32>(r.rcx as usize + 0x17cc)
        );
    }
}

unsafe extern "win64" fn on_frame_drain(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("dump_types::on_frame_drain");
    use poe_hook_shared::mem::read;
    let world = unsafe { (*regs).rcx } as usize;
    // Frame-mode worlds (the game world): log a few calls with dt and pause flags.
    if read::<u8>(world + 0x661).is_some_and(|m| m != 0) {
        if let Some(frames) = read::<u64>(world + 0x6a8) {
            crate::timing::record(crate::timing::Series::Backlog, frames as f32);
        }
        static PAUSE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0xffff);
        let flags = read::<u16>(world + 0x868).unwrap_or(0);
        if PAUSE.swap(flags, Ordering::Relaxed) != flags {
            log::info!("game world pause flags +0x868/+0x869 = {:#06x}", flags);
        }
        static GAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = GAME.fetch_add(1, Ordering::Relaxed) + 1;
        if matches!(n, 1 | 2 | 3 | 30 | 300 | 3000) {
            log::info!(
                "game frame drain #{n}: world {world:#x} dt={} xmm2={} ret={:#x} +0x868={:?} +0x869={:?} frames={:?}",
                f32::from_bits(unsafe { (*regs).xmm1 } as u32),
                f32::from_bits(unsafe { (*regs).xmm2 } as u32),
                read::<usize>(unsafe { (*regs).rsp } as usize)
                    .map(|a| MAIN.get().map_or(a, |m| if m.contains(a) {
                        m.to_ghidra(a)
                    } else {
                        a
                    }))
                    .unwrap_or(0),
                read::<u8>(world + 0x868),
                read::<u8>(world + 0x869),
                read::<u64>(world + 0x6a8)
            );
        }
    }
    let n = DRAINS.fetch_add(1, Ordering::Relaxed) + 1;
    if matches!(n, 1 | 10 | 100 | 1000 | 3000) {
        use poe_hook_shared::mem::read;
        let world = unsafe { (*regs).rcx } as usize;
        let dt = f32::from_bits(unsafe { (*regs).xmm1 } as u32);
        log::info!(
            "frame drain #{n}: world {world:#x} dt={dt} +0x661={:?} frames(+0x6a8)={:?} player updates={}",
            read::<u8>(world + 0x661),
            read::<u64>(world + 0x6a8),
            PLAYER_UPDATES.load(Ordering::Relaxed)
        );
    }
}

unsafe extern "win64" fn on_entity_tick(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("dump_types::on_entity_tick");
    let entity = (unsafe { (*regs).rcx } as usize).wrapping_sub(0x58);
    // After a core reload the player was created before this core: adopt it from its tick
    // (our server gives the player id 1).
    if PLAYER.load(Ordering::Relaxed) == 0
        && entity > 0x10000
        && poe_hook_shared::mem::read::<u32>(entity + 0x88) == Some(player_id())
    {
        PLAYER.store(entity, Ordering::SeqCst);
        log::info!("player entity {entity:#x} found from its frame tick");
        crate::entities::adopt(entity, player_id());
        track_player(entity);
    }
    if entity != 0 && entity == PLAYER.load(Ordering::Relaxed) {
        let n = PLAYER_UPDATES.fetch_add(1, Ordering::Relaxed) + 1;
        let dt = f32::from_bits(unsafe { (*regs).xmm1 } as u32);
        if matches!(n, 1 | 10 | 100 | 1000) {
            log::info!(
                "player entity tick #{n} dt={dt} (edx {:#x})",
                unsafe { (*regs).rdx } as u32
            );
        }
        log_player_frame(entity, n, dt);
        crate::position_sync::report(entity, n as u32);
    }
}

unsafe extern "win64" fn on_entity_update(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("dump_types::on_entity_update");
    let entity = unsafe { (*regs).rcx } as usize;
    if entity != 0 && entity == PLAYER.load(Ordering::Relaxed) {
        let n = PLAYER_UPDATES.fetch_add(1, Ordering::Relaxed) + 1;
        if matches!(n, 1 | 10 | 100 | 1000) {
            log::info!("player entity update #{n} dt={:#x}", unsafe { (*regs).rdx });
        }
    }
}

type LoadAreaFn =
    unsafe extern "win64" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;

/// Runs the area load, then exports the generated layout. Stack arguments (5th..8th) are
/// passed through.
unsafe extern "win64" fn on_load_area(regs: *mut Registers, original: usize, at: usize) -> usize {
    let mut guard = crate::hook_guard!("dump_types::on_load_area");
    crate::entities::clear();
    let r = unsafe { &*regs };
    let f: LoadAreaFn = unsafe { std::mem::transmute(original) };
    let ret = guard.original(|| unsafe {
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
    });
    log::info!("area load {at:#x} returned");
    // The layout files feed our server's spawn placement; not needed on official servers.
    if !crate::host::read_only() {
        unsafe { export_layout(r.rcx as usize) };
    }
    ret
}

/// Writes `terrain_layout_<tile hash>.tsv` next to the exe: a `size<TAB>w<TAB>h` line, then one
/// `x<TAB>y<TAB>tile path` line per tile. Dev bridge until the server generates terrain (M5).
unsafe fn export_layout(world: usize) {
    use poe_hook_shared::mem::{read, wide_string};
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
    // Exploration: vectors (begin, end, capacity) inside the terrain object and their sizes,
    // to locate per-cell grids (walkability, heights).
    for off in (0..0x400).step_by(8) {
        let (Some(b), Some(e), Some(c)) = (
            read::<usize>(terrain + off),
            read::<usize>(terrain + off + 8),
            read::<usize>(terrain + off + 16),
        ) else {
            continue;
        };
        if b > 0x10000 && e >= b && c >= e && e - b > 0x1000 && e - b < 0x4000000 {
            log::info!("  terrain+{off:#x}: vector of {:#x} bytes", e - b);
        }
    }
    // Tile hash as logged by the client ("Tile hash: N"), also sent to the server in
    // TerrainHashes; it names the export so the server reads the matching layout. World-map
    // loads (area kind 0x20/0x21/0x23 at +0x98 -> +0x1b4) log no hash and are skipped.
    let kind = read::<usize>(world + 0x98).and_then(|a| read::<u32>(a + 0x1b4));
    if matches!(kind, Some(0x20 | 0x21 | 0x23)) {
        return;
    }
    let Some(tile_hash) = read::<u32>(world + 0x114) else {
        return;
    };
    let (Some(w), Some(h), Some(tiles)) = (
        read::<u32>(terrain + 0x18),
        read::<u32>(terrain + 0x20),
        read::<usize>(terrain + 0x28),
    ) else {
        return;
    };
    // Tile record (0x38 bytes): +0 -> the sub-tile entry in the tile object's array (0x38
    // bytes each, in the tile's own orientation, row-major), +8 -> tile object (+8: .tdt path).
    // A tile object's array start is the lowest sub-tile pointer among its records, so
    // (q0 - start) / 0x38 is the sub-tile's canonical index; with the map position it gives the
    // tile's rotation/mirroring.
    let record = |x: usize, y: usize| tiles + (y * w as usize + x) * 0x38;
    let mut starts: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
    for y in 0..h as usize {
        for x in 0..w as usize {
            if let (Some(q0), Some(t)) =
                (read::<usize>(record(x, y)), read::<usize>(record(x, y) + 8))
            {
                let e = starts.entry(t).or_insert(q0);
                *e = (*e).min(q0);
            }
        }
    }
    let mut out = format!("size\t{w}\t{h}\n");
    for y in 0..h as usize {
        for x in 0..w as usize {
            let t = read::<usize>(record(x, y) + 8);
            let path = t
                .and_then(|t| read::<usize>(t + 8))
                .and_then(|p| wide_string(p, 200))
                .unwrap_or_default();
            let sub = match (read::<usize>(record(x, y)), t.and_then(|t| starts.get(&t))) {
                (Some(q0), Some(&start)) => (q0 - start) / 0x38,
                _ => 0,
            };
            out += &format!("{x}\t{y}\t{path}\t{sub}\n");
        }
    }
    unsafe { export_grids(terrain, w as usize, h as usize, tile_hash) };
    let path = poe_hook_shared::exe_dir().join(format!("terrain_layout_{tile_hash}.tsv"));
    match std::fs::write(&path, out) {
        Ok(()) => log::info!("exported {w}x{h} layout to {}", path.display()),
        Err(e) => log::error!("layout export: {e}"),
    }
}

/// The four per-cell layers of the terrain (vectors at +0xd0, +0xe8, +0x100, +0x118; 4 bits
/// per cell, `ceil(cells_x / 2)` bytes per row, rows = cells_y). Written as
/// `terrain_grid_<tile hash>.bin`: `b"PWG1"`, u32 cells_x, u32 cells_y, u32 bytes_per_row,
/// u32 layer count, then each layer's bytes.
unsafe fn export_grids(terrain: usize, w: usize, h: usize, tile_hash: u32) {
    use poe_hook_shared::mem::read;
    let (cells_x, cells_y) = (w * 23, h * 23);
    let bytes_per_row = cells_x.div_ceil(2);
    let mut out = Vec::new();
    out.extend_from_slice(b"PWG1");
    for v in [cells_x, cells_y, bytes_per_row, 4] {
        out.extend_from_slice(&(v as u32).to_le_bytes());
    }
    for off in [0xd0, 0xe8, 0x100, 0x118] {
        let (Some(b), Some(e)) = (
            read::<usize>(terrain + off),
            read::<usize>(terrain + off + 8),
        ) else {
            return;
        };
        if e - b != bytes_per_row * cells_y || !poe_hook_shared::mem::readable(b, e - b) {
            log::warn!("grid at terrain+{off:#x}: unexpected size {:#x}", e - b);
            return;
        }
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(b as *const u8, e - b) });
    }
    let path = poe_hook_shared::exe_dir().join(format!("terrain_grid_{tile_hash}.bin"));
    match std::fs::write(&path, out) {
        Ok(()) => log::info!(
            "exported {cells_x}x{cells_y} cell grids to {}",
            path.display()
        ),
        Err(e) => log::error!("grid export: {e}"),
    }
}

static MAIN: OnceLock<Module> = OnceLock::new();

/// Per-frame player position logging (config `movement_log`).
pub static MOVEMENT_LOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// `[features] entity_trace`: log each entity's components and the blob span each component
/// reader consumes (research; slow, off by default).
pub static ENTITY_TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Movement research: the player's world position at the start of each of its updates
/// (one per simulation frame), logged when it changed.
fn log_player_frame(entity: usize, n: usize, dt: f32) {
    use poe_hook_shared::mem::read;
    if !MOVEMENT_LOG.load(Ordering::Relaxed) {
        return;
    }
    static LAST: std::sync::Mutex<(u32, u32)> = std::sync::Mutex::new((0, 0));
    let Some(comps) = read::<usize>(entity + 0x10) else {
        return;
    };
    let Some(pos) = read::<usize>(comps) else {
        return;
    };
    let (x, y) = (
        read::<f32>(pos + 0x490).unwrap_or(0.0),
        read::<f32>(pos + 0x494).unwrap_or(0.0),
    );
    let mut last = LAST.lock().unwrap();
    if *last != (x.to_bits(), y.to_bits()) {
        let (lx, ly) = (f32::from_bits(last.0), f32::from_bits(last.1));
        *last = (x.to_bits(), y.to_bits());
        log::info!(
            "frame #{n} dt={dt} world ({x:.3}, {y:.3}) step ({:.3}, {:.3}) grid ({}, {})",
            x - lx,
            y - ly,
            read::<i32>(pos + 0x444).unwrap_or(0),
            read::<i32>(pos + 0x448).unwrap_or(0)
        );
    }
}

/// Movement research: logs the player's position whenever it changes (polled every 20 ms):
/// Positioned (component 0) grid cell +0x444/+0x448 and world position +0x490/+0x494, and the
/// Actor's (component 9) WASD direction +0x400/+0x404 as set by server message 0x64.
fn track_player(entity: usize) {
    if !MOVEMENT_LOG.load(Ordering::Relaxed) {
        return;
    }
    crate::hooks::spawn(move || {
        use poe_hook_shared::mem::read;
        let start = std::time::Instant::now();
        let mut last = None;
        let mut last_actions = String::new();
        let mut last_facing = (i32::MIN, 0u32);
        while PLAYER.load(Ordering::SeqCst) == entity && crate::hooks::running() {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let Some(comps) = read::<usize>(entity + 0x10) else {
                return;
            };
            let (Some(pos), Some(actor)) = (read::<usize>(comps), read::<usize>(comps + 9 * 8))
            else {
                return;
            };
            let now = (
                read::<i32>(pos + 0x444).unwrap_or(0),
                read::<i32>(pos + 0x448).unwrap_or(0),
                read::<f32>(pos + 0x490).unwrap_or(0.0).to_bits(),
                read::<f32>(pos + 0x494).unwrap_or(0.0).to_bits(),
                read::<i32>(actor + 0x400).unwrap_or(0),
                read::<i32>(actor + 0x404).unwrap_or(0),
            );
            // Action slots (movement research): type (+0x120) and flags (+0x144) of each
            // action pointer at Actor +0x220..+0x250, and the Actor's state bytes.
            let slots: Vec<String> = (0x220..=0x250usize)
                .step_by(8)
                .filter_map(|o| {
                    let a = read::<usize>(actor + o).filter(|&a| a > 0x10000)?;
                    let ty = read::<u16>(a + 0x120)?;
                    let fl = read::<u16>(a + 0x144).unwrap_or(0);
                    let vt = read::<usize>(a).unwrap_or(0);
                    let vt = MAIN
                        .get()
                        .map_or(vt, |m| if m.contains(vt) { m.to_ghidra(vt) } else { vt });
                    Some(format!("+{o:#x}:{ty:#06x}/{fl:#x}/vt{vt:#x}"))
                })
                .collect();
            let state = format!(
                "2a0={:#x} 2a1={:#x} 2a2={:#x} 915={} 916={}",
                read::<u8>(actor + 0x2a0).unwrap_or(0),
                read::<u8>(actor + 0x2a1).unwrap_or(0),
                read::<u8>(actor + 0x2a2).unwrap_or(0),
                read::<u8>(actor + 0x915).unwrap_or(0),
                read::<u8>(actor + 0x916).unwrap_or(0)
            );
            // Facing: Positioned +0x44c (radians, written by SetOrientation 0x141ddc6a0 unless
            // the orientation is locked, +0x1e5 bit 1); logged when it changes by > 0.05.
            let angle = read::<f32>(pos + 0x44c).unwrap_or(0.0);
            if (angle - f32::from_bits(last_facing.1)).abs() > 0.05 || last_facing.0 == i32::MIN {
                last_facing = (0, angle.to_bits());
                log::info!(
                    "player facing t={}ms angle={angle:.3} locked={} pos={pos:#x}",
                    start.elapsed().as_millis(),
                    read::<u8>(pos + 0x1e5).unwrap_or(0) >> 1 & 1
                );
            }
            let summary = format!("{} {state}", slots.join(" "));
            if summary != last_actions {
                log::info!(
                    "player actions t={}ms {summary}",
                    start.elapsed().as_millis()
                );
                last_actions = summary;
            }
            // Positions are logged per frame (log_player_frame); here only direction changes.
            if last.map(|l: (i32, i32, u32, u32, i32, i32)| (l.4, l.5)) != Some((now.4, now.5)) {
                last = Some(now);
                log::info!(
                    "player t={}ms grid ({}, {}) world ({:.2}, {:.2}) wasd ({}, {})",
                    start.elapsed().as_millis(),
                    now.0,
                    now.1,
                    f32::from_bits(now.2),
                    f32::from_bits(now.3),
                    now.4,
                    now.5
                );
            }
        }
    });
}

/// Logs each component of the entity being deserialized: object address, vtable and the
/// component's initial-state reader (vtable +0x98), as Ghidra addresses.
unsafe extern "win64" fn on_read_initial_state(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("dump_types::on_read_initial_state");
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
        crate::entities::register(entity, id);
        if id == player_id() {
            PLAYER.store(entity, Ordering::SeqCst);
            PLAYER_UPDATES.store(0, Ordering::SeqCst);
            track_player(entity);
            // State flags a few seconds later (+0x8c bit 3: in the wake/update set).
            crate::hooks::spawn(move || {
                if !crate::hooks::sleep(std::time::Duration::from_secs(5)) {
                    return;
                }
                if PLAYER.load(Ordering::SeqCst) != entity {
                    return;
                }
                use poe_hook_shared::mem::read;
                let b = |o: usize| read::<u8>(entity + o).unwrap_or(0xff);
                log::info!(
                    "player entity after 5 s: +0x8c={:#04x} +0x8d={:#04x} updates={}",
                    b(0x8c),
                    b(0x8d),
                    PLAYER_UPDATES.load(Ordering::SeqCst)
                );
                // Positioned (component 0) scale/transform floats and Actor (component 9) +0x230.
                let comps0 = read::<usize>(entity + 0x10).unwrap_or(0);
                if let (Some(pos), Some(actor)) =
                    (read::<usize>(comps0), read::<usize>(comps0 + 9 * 8))
                {
                    let f = |a: usize| read::<f32>(a).unwrap_or(f32::NAN);
                    log::info!(
                        "  positioned +0x46c={} +0x470={} +0x474={} +0x490=({}, {}) +0x49c={}; actor +0x230={}",
                        f(pos + 0x46c),
                        f(pos + 0x470),
                        f(pos + 0x474),
                        f(pos + 0x490),
                        f(pos + 0x494),
                        f(pos + 0x49c),
                        f(actor + 0x230)
                    );
                }
                // The world object (entity +0x78) and the vtable slots InitialState's header
                // drives (0x178 u32, 0x200 two flags, 0x60 flag, 0x1f0).
                if let (Some(m), Some(world)) = (MAIN.get(), read::<usize>(entity + 0x78)) {
                    let vt = read::<usize>(world).unwrap_or(0);
                    let slot = |o: usize| {
                        let f = read::<usize>(vt + o).unwrap_or(0);
                        if m.contains(f) { m.to_ghidra(f) } else { f }
                    };
                    log::info!(
                        "  world {world:#x} vtable {:#x}: 0x178={:#x} 0x200={:#x} 0x60={:#x} 0x1f0={:#x}",
                        if m.contains(vt) { m.to_ghidra(vt) } else { vt },
                        slot(0x178),
                        slot(0x200),
                        slot(0x60),
                        slot(0x1f0)
                    );
                    // Frame drain 0x1421306c0: +0x661 frame mode, +0x868 paused (entities
                    // update with dt 0), +0x86a tick markers accepted; +0x11c set by 0x2f5.
                    let w = |o: usize| read::<u8>(world + o).unwrap_or(0xff);
                    log::info!(
                        "  world +0x661={} +0x868={} +0x869={} +0x86a={} +0x11c={} +0x84a={}",
                        w(0x661),
                        w(0x868),
                        w(0x869),
                        w(0x86a),
                        w(0x11c),
                        w(0x84a)
                    );
                }
                // Render component (getter 0x140150cb0, by name): +0x39a non-zero = rendering
                // disabled (GEAL EnableRendering clears it, then calls 0x1416a9810).
                if let Some(m) = MAIN.get() {
                    let get_render: extern "win64" fn(usize) -> usize =
                        unsafe { std::mem::transmute(m.from_ghidra(0x140150cb0)) };
                    let render = get_render(entity);
                    let bytes: Vec<String> = (0x390..0x3a0)
                        .map(|o| format!("{:02x}", read::<u8>(render + o).unwrap_or(0xee)))
                        .collect();
                    log::info!("  render {render:#x} +0x390..: {}", bytes.join(" "));
                    // Experiment: re-apply visibility (shows the AO scene node).
                    if render != 0 && std::env::var_os("POE_HOOK_FORCE_SHOW").is_some() {
                        let refresh: extern "win64" fn(usize) =
                            unsafe { std::mem::transmute(m.from_ghidra(0x1416a9810)) };
                        refresh(render);
                        log::info!("  forced render visibility refresh");
                    }
                }
                // Positioned height as the client samples it (0x141de5390 returns f32 in xmm0).
                if let (Some(m), Some(pos)) = (MAIN.get(), read::<usize>(begin_components(entity)))
                {
                    let height: extern "win64" fn(usize) -> f32 =
                        unsafe { std::mem::transmute(m.from_ghidra(0x141de5390)) };
                    log::info!("  positioned height {}", height(pos));
                    // Terrain height samples across the map (world vfunc +0x1a8 -> terrain,
                    // 0x141d0d420(terrain, &[x, y]) -> f32).
                    if let Some(world) = read::<usize>(entity + 0x78) {
                        let vt = read::<usize>(world).unwrap_or(0);
                        if let Some(f) = read::<usize>(vt + 0x1a8) {
                            let get_terrain: extern "win64" fn(usize) -> usize =
                                unsafe { std::mem::transmute(f) };
                            let sample: extern "win64" fn(usize, *const [f32; 2]) -> f32 =
                                unsafe { std::mem::transmute(m.from_ghidra(0x141d0d420)) };
                            let terrain = get_terrain(world);
                            let mut row = Vec::new();
                            for gy in 0..6 {
                                for gx in 0..6 {
                                    let p =
                                        [gx as f32 * 1500.0 + 250.0, gy as f32 * 1400.0 + 250.0];
                                    row.push(format!("{:.0}", sample(terrain, &p)));
                                }
                            }
                            let (px, py) = (
                                read::<f32>(pos + 0x490).unwrap_or(0.0),
                                read::<f32>(pos + 0x494).unwrap_or(0.0),
                            );
                            let near: Vec<String> = (-2..=2)
                                .flat_map(|dy| (-2..=2).map(move |dx| (dx, dy)))
                                .map(|(dx, dy)| {
                                    let p = [px + dx as f32 * 60.0, py + dy as f32 * 60.0];
                                    format!("{:.0}", sample(terrain, &p))
                                })
                                .collect();
                            log::info!(
                                "  heights around player (60-unit steps): {}",
                                near.join(" ")
                            );
                            let here = sample(terrain, &[px, py]);
                            log::info!(
                                "  terrain {terrain:#x} height here {here}, grid 6x6: {}",
                                row.join(" ")
                            );
                        }
                    }
                }
                // NaN/inf scan of every component (first 0x800 bytes): a degenerate transform
                // would explain an invisible model and a grey camera.
                let begin = read::<usize>(entity + 0x10).unwrap_or(0);
                let end = read::<usize>(entity + 0x18).unwrap_or(0);
                for (i, c) in (begin..end).step_by(8).enumerate() {
                    let Some(comp) = read::<usize>(c) else {
                        continue;
                    };
                    let vt = read::<usize>(comp).unwrap_or(0);
                    let bad: Vec<String> = (0..0x800usize)
                        .step_by(4)
                        .filter_map(|o| {
                            let v = read::<u32>(comp + o)?;
                            // NaN or inf only (exponent all ones), skip 0xffffffff fillers.
                            (v & 0x7f80_0000 == 0x7f80_0000 && v != 0xffff_ffff)
                                .then(|| format!("+{o:#x}={v:#x}"))
                        })
                        .take(12)
                        .collect();
                    log::info!(
                        "  component {i} {comp:#x} vtable {:#x}: {}",
                        MAIN.get().map_or(vt, |m| if m.contains(vt) {
                            m.to_ghidra(vt)
                        } else {
                            vt
                        }),
                        bad.join(" ")
                    );
                }
                // Animated component (index 5): its animated object (+0x350) and that
                // object's path (+8 -> wide string).
                use poe_hook_shared::mem::wide_string;
                let comps = read::<usize>(entity + 0x10).unwrap_or(0);
                if let Some(animated) = read::<usize>(comps + 5 * 8) {
                    let ao = read::<usize>(animated + 0x350).unwrap_or(0);
                    let path = read::<usize>(ao + 8).and_then(|p| wide_string(p, 200));
                    let words: Vec<String> = (0..8)
                        .map(|i| format!("{:x}", read::<usize>(ao + i * 8).unwrap_or(0)))
                        .collect();
                    log::info!(
                        "  animated {animated:#x} ao {ao:#x} path {path:?} head [{}]",
                        words.join(" ")
                    );
                }
            });
        }
        // Per-entity tracing (component spans, reader hooks, terrain dumps) is heavy: it froze
        // the game for seconds per spawn on official servers. Opt-in only.
        if !ENTITY_TRACE.load(Ordering::Relaxed) {
            return;
        }
        // The read loop only visits the first `serialized` components (type +0x28 -> +0x78);
        // the rest are client-only and their +0x98 slot is not a reader.
        let ty = *((entity + 8) as *const usize);
        let serialized = if ty > 0x10000 {
            *((*((ty + 0x28) as *const usize) + 0x78) as *const usize)
        } else {
            0
        };
        log::info!(
            "entity {id:#x} at {entity:#x} initial state: {count} components ({serialized} serialized), blob {len} bytes at pos {pos} (data {data:#x})"
        );
        let refs: Vec<String> = poe_hook_shared::module::stack_refs(m, r.rsp as usize, 24)
            .into_iter()
            .map(|a| format!("{a:#x}"))
            .collect();
        log::info!("  initial state stack refs: {}", refs.join(" "));
        for i in 0..count.min(64) {
            let comp = *((begin + i * 8) as *const usize);
            if comp < 0x10000 {
                continue;
            }
            let vt = *(comp as *const usize);
            let reader = if m.contains(vt) {
                *((vt + 0x98) as *const usize)
            } else {
                0
            };
            // The read loop only calls components whose owner (+8) is this entity.
            let owned = *((comp + 8) as *const usize) == entity;
            log::info!(
                "  component {i}: vtable={:#x} reader={:#x} owned={owned}",
                if m.contains(vt) { m.to_ghidra(vt) } else { vt },
                if m.contains(reader) {
                    m.to_ghidra(reader)
                } else {
                    reader
                }
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
    if let Err(e) = crate::hooks::install(m.from_ghidra(ghidra), HookType::Retn(on_reader), ghidra)
    {
        log::error!("hook reader {ghidra:#x}: {e}");
    }
}

type ReaderFn = unsafe extern "win64" fn(usize, usize, usize, usize) -> usize;

unsafe extern "win64" fn on_reader(regs: *mut Registers, original: usize, ghidra: usize) -> usize {
    let mut guard = crate::hook_guard!("dump_types::on_reader");
    let r = unsafe { &*regs };
    let blob = r.rdx as *const usize;
    let start = unsafe { *blob.add(2) };
    log::info!("    reader {ghidra:#x} starts at blob pos {start}");
    let f: ReaderFn = unsafe { std::mem::transmute(original) };
    let ret = guard
        .original(|| unsafe { f(r.rcx as usize, r.rdx as usize, r.r8 as usize, r.r9 as usize) });
    let end = unsafe { *blob.add(2) };
    log::info!(
        "    reader {ghidra:#x} read {} bytes ({start}..{end})",
        end - start
    );
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
    use poe_hook_shared::mem::{read, wide_string};
    let Some(tiles) = read::<usize>(terrain + 0x28) else {
        return;
    };
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
                let Some(p1) = read::<usize>(rec + q * 8) else {
                    continue;
                };
                if let Some(s) = path_at(p1) {
                    found.push(format!("q{q}:{s}"));
                    continue;
                }
                for k in (0..0x60).step_by(8) {
                    let Some(p2) = read::<usize>(p1.wrapping_add(k)) else {
                        break;
                    };
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
    let path = poe_hook_shared::exe_dir().join("terrain_tiles.txt");
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
    let _guard = crate::hook_guard!("dump_types::on_lookup");
    let r = unsafe { &*regs };
    log::debug!("object type lookup key={:#010x}", r.r8 as u32);
    crate::entities::set_last_key(r.r8 as u32);
    if crate::entities::types_known() {
        return;
    }
    unsafe {
        let registry = r.rcx as *const usize;
        let vec = *registry as *const usize;
        let (begin, end) = (*vec, *vec.add(1));
        let count = (end - begin) / ENTRY_SIZE;
        let mut types = Vec::with_capacity(count);
        for i in 0..count {
            let e = begin + i * ENTRY_SIZE;
            types.push((*(e as *const u32), wide(*((e + 8) as *const usize))));
        }
        if !DONE.swap(true, Ordering::SeqCst)
            && let Some(path) = OUT.get().cloned()
        {
            // The file is written off the game thread.
            let list = types.clone();
            crate::hooks::spawn(move || {
                let Ok(mut f) = std::fs::File::create(&path) else {
                    return;
                };
                for (key, name) in &list {
                    let _ = writeln!(f, "{key:#010x}\t{name}");
                }
                log::info!("dumped {} object types to {}", list.len(), path.display());
            });
        }
        crate::entities::set_types(types);
    }
}

/// The player entity (0 until known).
pub fn player() -> usize {
    PLAYER.load(Ordering::Relaxed)
}

/// The game module (set when the research hooks start).
pub fn main_module() -> Option<Module> {
    MAIN.get().copied()
}

/// The controlled character's entity id: from the server's PlayerEntity message (packet
/// capture); 1 until known (our server's player id).
static PLAYER_ID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

pub fn player_id() -> u32 {
    PLAYER_ID.load(Ordering::Relaxed)
}

pub fn set_player_id(id: u32) {
    if PLAYER_ID.swap(id, Ordering::Relaxed) != id {
        log::info!("player entity id {id:#x} (PlayerEntity)");
    }
}
