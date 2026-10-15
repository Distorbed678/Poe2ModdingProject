//! Read-only views of game objects for the inspector tabs (guarded reads only).
//!
//! Offsets (0.5.5e, see Documentation/re/addresses-0.5.5e.md): entity +0x10 components
//! (0 = Positioned, 9 = Actor), +0x88 id. Positioned +0x444/+0x448 grid cell, +0x44c facing,
//! +0x1e5 bit 1 facing locked, +0x490/+0x494 world position. Actor +0x238/+0x240 current
//! actions (type at +0x120), +0x400/+0x404 WASD direction, +0x2a2 bit 5 sprint held.

use poe_hook_shared::mem::read;

#[derive(Debug, Default, Clone, Copy)]
pub struct Player {
    pub entity: usize,
    pub id: u32,
    pub grid: (i32, i32),
    pub world: (f32, f32),
    pub facing: f32,
    pub facing_locked: bool,
    pub action: u16,
    pub action2: u16,
    pub wasd: (i32, i32),
    pub flags: u8,
}

pub fn player() -> Option<Player> {
    let entity = crate::dump_types::player();
    if entity == 0 {
        return None;
    }
    let comps = read::<usize>(entity + 0x10)?;
    let pos = read::<usize>(comps)?;
    let actor = read::<usize>(comps + 9 * 8)?;
    let action = |off: usize| {
        read::<usize>(actor + off)
            .filter(|&a| a != 0)
            .and_then(|a| read::<u16>(a + 0x120))
            .unwrap_or(0)
    };
    Some(Player {
        entity,
        id: read(entity + 0x88)?,
        grid: (read(pos + 0x444)?, read(pos + 0x448)?),
        world: (read(pos + 0x490)?, read(pos + 0x494)?),
        facing: read(pos + 0x44c)?,
        facing_locked: read::<u8>(pos + 0x1e5).unwrap_or(0) & 2 != 0,
        action: action(0x238),
        action2: action(0x240),
        wasd: (
            read(actor + 0x400).unwrap_or(0),
            read(actor + 0x404).unwrap_or(0),
        ),
        flags: read(actor + 0x2a2).unwrap_or(0),
    })
}

pub fn player_text() -> String {
    match player() {
        Some(p) => format!(
            "player entity {:#x} id {} grid ({}, {}) world ({:.2}, {:.2}) facing {:.3}{} action {:#06x}/{:#06x} wasd ({}, {}) flags {:#04x}\n",
            p.entity,
            p.id,
            p.grid.0,
            p.grid.1,
            p.world.0,
            p.world.1,
            p.facing,
            if p.facing_locked { " (locked)" } else { "" },
            p.action,
            p.action2,
            p.wasd.0,
            p.wasd.1,
            p.flags
        ),
        None => "player: not found yet\n".into(),
    }
}
