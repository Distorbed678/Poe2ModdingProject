//! Terrain minimap data: the client's cell grids around the player, read live from the area's
//! terrain object (tiles `+0x18`/`+0x20`, 23 cells per tile; 4 grids of 4 bits per cell, low
//! nibble first, at `+0xd0`, `+0xe8`, `+0x100`, `+0x118`; layer 0 non-zero = walkable, as our
//! server's spawn code uses it).

use poe_hook_shared::mem::read;

pub const LAYERS: [usize; 4] = [0xd0, 0xe8, 0x100, 0x118];

pub struct Grid {
    pub cells_x: usize,
    pub cells_y: usize,
    bytes_per_row: usize,
    data: usize,
}

impl Grid {
    pub fn current(layer: usize) -> Option<Self> {
        let world = crate::dump_types::game_world();
        if world == 0 {
            return None;
        }
        let terrain = crate::dump_types::terrain(world)?;
        let w = read::<u32>(terrain + 0x18)? as usize;
        let h = read::<u32>(terrain + 0x20)? as usize;
        let (cells_x, cells_y) = (w * 23, h * 23);
        let bytes_per_row = cells_x.div_ceil(2);
        let off = *LAYERS.get(layer)?;
        let (b, e) = (
            read::<usize>(terrain + off)?,
            read::<usize>(terrain + off + 8)?,
        );
        if e.checked_sub(b)? != bytes_per_row * cells_y || !poe_hook_shared::mem::readable(b, e - b)
        {
            return None;
        }
        Some(Self {
            cells_x,
            cells_y,
            bytes_per_row,
            data: b,
        })
    }

    /// The 4-bit value of a cell (None outside the grid).
    pub fn get(&self, x: i64, y: i64) -> Option<u8> {
        if x < 0 || y < 0 || x as usize >= self.cells_x || y as usize >= self.cells_y {
            return None;
        }
        let byte = unsafe {
            *((self.data + y as usize * self.bytes_per_row + x as usize / 2) as *const u8)
        };
        Some(if x & 1 == 1 { byte >> 4 } else { byte & 0xf })
    }
}
