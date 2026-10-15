//! Entity browser data: every entity the client created in the current area (from the
//! initial-state reader), with its object type (the key the object-type lookup resolved just
//! before, and that key's metadata path). Values are read live, with guarded reads only.
//!
//! Layout (0.5.5e): entity +0x10/+0x18 component vector, +0x88 id. A component's vtable slot
//! +0x98 is its initial-state reader; Positioned's is `0x141dd8820` (grid +0x444/+0x448,
//! world +0x490/+0x494, facing +0x44c).

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

use poe_hook_shared::mem::read;

const POSITIONED_READER: usize = 0x141dd8820;
const KEEP: usize = 20_000;

static LAST_KEY: AtomicU32 = AtomicU32::new(0);
static TYPES: Mutex<Option<HashMap<u32, String>>> = Mutex::new(None);
static ENTITIES: Mutex<Vec<Entity>> = Mutex::new(Vec::new());

#[derive(Clone)]
pub struct Entity {
    pub ptr: usize,
    pub id: u32,
    pub type_key: u32,
    pub path: String,
    pub created_ms: u64,
    /// Component vtables as Ghidra addresses.
    pub components: Vec<usize>,
    /// Index of the Positioned component.
    pub positioned: Option<usize>,
}

pub struct Live {
    /// The memory still holds this entity (id unchanged).
    pub alive: bool,
    pub grid: Option<(i32, i32)>,
    pub world: Option<(f32, f32)>,
    pub facing: Option<f32>,
}

pub fn set_last_key(key: u32) {
    LAST_KEY.store(key, Ordering::Relaxed);
}

pub fn types_known() -> bool {
    TYPES.lock().unwrap().is_some()
}

pub fn set_types(types: Vec<(u32, String)>) {
    *TYPES.lock().unwrap() = Some(types.into_iter().collect());
}

pub fn type_path(key: u32) -> Option<String> {
    TYPES.lock().unwrap().as_ref()?.get(&key).cloned()
}

/// A new entity is about to read its initial state (its type was the last one looked up).
pub fn register(ptr: usize, id: u32) {
    add(ptr, id, LAST_KEY.load(Ordering::Relaxed));
}

/// An entity created before this core was loaded (type unknown).
pub fn adopt(ptr: usize, id: u32) {
    add(ptr, id, 0);
}

fn add(ptr: usize, id: u32, key: u32) {
    let main = crate::dump_types::main_module();
    let begin = read::<usize>(ptr + 0x10).unwrap_or(0);
    let end = read::<usize>(ptr + 0x18).unwrap_or(0);
    let count = (end.saturating_sub(begin) / 8).min(64);
    let mut components = Vec::with_capacity(count);
    let mut positioned = None;
    for i in 0..count {
        let vt = read::<usize>(begin + i * 8)
            .and_then(read::<usize>)
            .unwrap_or(0);
        let ghidra = main
            .filter(|m| m.contains(vt))
            .map_or(vt, |m| m.to_ghidra(vt));
        let reader = main
            .filter(|m| m.contains(vt))
            .and_then(|m| read::<usize>(vt + 0x98).map(|r| m.to_ghidra(r)));
        if reader == Some(POSITIONED_READER) && positioned.is_none() {
            positioned = Some(i);
        }
        components.push(ghidra);
    }
    let entity = Entity {
        ptr,
        id,
        type_key: key,
        path: type_path(key).unwrap_or_default(),
        created_ms: poe_hook_shared::unix_ms(),
        components,
        positioned,
    };
    let mut list = ENTITIES.lock().unwrap();
    list.retain(|e| e.ptr != ptr);
    if list.len() >= KEEP {
        list.remove(0);
    }
    list.push(entity);
}

/// An area load (the hook fires several times per area): drop entities whose memory no
/// longer holds them.
pub fn clear() {
    ENTITIES
        .lock()
        .unwrap()
        .retain(|e| read::<u32>(e.ptr + 0x88) == Some(e.id));
}

pub fn list() -> Vec<Entity> {
    ENTITIES.lock().unwrap().clone()
}

pub fn live(e: &Entity) -> Live {
    let alive = read::<u32>(e.ptr + 0x88) == Some(e.id);
    let pos = e
        .positioned
        .filter(|_| alive)
        .and_then(|i| read::<usize>(e.ptr + 0x10).and_then(|c| read::<usize>(c + i * 8)));
    Live {
        alive,
        grid: pos.and_then(|p| Some((read(p + 0x444)?, read(p + 0x448)?))),
        world: pos.and_then(|p| Some((read(p + 0x490)?, read(p + 0x494)?))),
        facing: pos.and_then(|p| read(p + 0x44c)),
    }
}

/// Component names of an entity, by index: the entity type's lookup (`entity+8` → `+0x28`)
/// holds a sorted array of `{char* name, i32 index}` (16 bytes) from `+0x28` to `+0x30`, the
/// table the game's get-component-by-name uses (e.g. `0x140150cb0` for "Render").
pub fn component_names(ptr: usize) -> Vec<(usize, String)> {
    let Some(lookup) = read::<usize>(ptr + 8).and_then(|t| read::<usize>(t + 0x28)) else {
        return Vec::new();
    };
    let (Some(begin), Some(end)) = (read::<usize>(lookup + 0x28), read::<usize>(lookup + 0x30))
    else {
        return Vec::new();
    };
    if end < begin || end - begin > 256 * 16 {
        return Vec::new();
    }
    let mut out: Vec<(usize, String)> = (begin..end)
        .step_by(16)
        .filter_map(|e| {
            let name = read::<usize>(e).and_then(|p| poe_hook_shared::mem::c_string(p, 64))?;
            let index = read::<i32>(e + 8)?;
            (index >= 0).then_some((index as usize, name))
        })
        .collect();
    out.sort();
    out
}

/// Component pointers of an entity (index order).
pub fn components(ptr: usize) -> Vec<usize> {
    let begin = read::<usize>(ptr + 0x10).unwrap_or(0);
    let end = read::<usize>(ptr + 0x18).unwrap_or(0);
    let count = (end.saturating_sub(begin) / 8).min(64);
    (0..count)
        .filter_map(|i| read::<usize>(begin + i * 8))
        .collect()
}
