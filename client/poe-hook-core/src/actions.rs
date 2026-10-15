//! Action / skill trace: the player's Actor actions (current +0x238, second +0x240; type at
//! action +0x120 = `ActionTypes` key), sampled every simulation frame from the player's frame
//! tick. Starts and ends are kept for the Actions tab and logged.

use std::{collections::VecDeque, sync::Mutex};

use poe_hook_shared::mem::read;

const KEEP: usize = 2000;

#[derive(Clone)]
pub struct Event {
    pub unix_ms: u64,
    pub frame: u32,
    pub slot: u8,
    pub started: bool,
    pub kind: u16,
    pub action: usize,
}

impl Event {
    pub fn name(&self) -> String {
        crate::gamedata::action_name(self.kind as u32)
            .map_or_else(|| format!("{:#06x}", self.kind), str::to_owned)
    }
}

static EVENTS: Mutex<VecDeque<Event>> = Mutex::new(VecDeque::new());
/// Last seen (action pointer, type) per slot.
static LAST: Mutex<[(usize, u16); 2]> = Mutex::new([(0, 0); 2]);

fn push(e: Event) {
    log::info!(
        "action {} slot {}: {} (action {:#x}, frame {})",
        if e.started { "start" } else { "end" },
        e.slot,
        e.name(),
        e.action,
        e.frame
    );
    let mut ev = EVENTS.lock().unwrap();
    if ev.len() >= KEEP {
        ev.pop_front();
    }
    ev.push_back(e);
}

/// Called every simulation frame for the player entity.
pub fn sample(entity: usize, frame: u32) {
    let Some(actor) = read::<usize>(entity + 0x10).and_then(|c| read::<usize>(c + 9 * 8)) else {
        return;
    };
    let now = poe_hook_shared::unix_ms();
    let mut last = LAST.lock().unwrap();
    for (slot, off) in [0x238usize, 0x240].into_iter().enumerate() {
        let ptr = read::<usize>(actor + off).unwrap_or(0);
        let kind = if ptr != 0 {
            read::<u16>(ptr + 0x120).unwrap_or(0)
        } else {
            0
        };
        let (old_ptr, old_kind) = last[slot];
        if (ptr, kind) == (old_ptr, old_kind) {
            continue;
        }
        last[slot] = (ptr, kind);
        if old_ptr != 0 {
            push(Event {
                unix_ms: now,
                frame,
                slot: slot as u8,
                started: false,
                kind: old_kind,
                action: old_ptr,
            });
        }
        if ptr != 0 {
            push(Event {
                unix_ms: now,
                frame,
                slot: slot as u8,
                started: true,
                kind,
                action: ptr,
            });
        }
    }
}

pub fn events() -> Vec<Event> {
    EVENTS.lock().unwrap().iter().cloned().collect()
}

pub fn clear() {
    EVENTS.lock().unwrap().clear();
}
