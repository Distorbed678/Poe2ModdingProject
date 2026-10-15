//! Tile definitions (`.tdt`, binary, versions 14/15): a tile's geometry (`.tgt`), its size in
//! tiles, the edge type on each side and the ground type at each corner, which is what the
//! generator matches when it places tiles. Layout after `Tools/reference/poeformats/Tdt.cs`:
//!
//! `u32 version, u32 string-table chars, UTF-16 strings (NUL-separated)`, then string
//! references (`i32` char offsets into the table, -1 = none): `inherits`; when empty: `tgt`,
//! `feature`, edge types down/right/up/left, `u8 size x, u8 size y`, ground types
//! down-left/down-right/up-right/up-left, 4 bytes, 8 edge-distance bytes. The rest (sub-tile
//! data, doodads, entities) is kept raw for now.
//!
//! A tile that inherits another (`*_Reorient_N.tdt`) has a `u8` of override flags after the
//! `inherits` reference (`0x1429153f0`) and carries only the overridden parts: 1 `tgt`, 8 the
//! header from `feature` on and the corner points, 0x10 the size and sub-tile geometry, 0x20 the
//! orientation mask. [`Tdt::resolve`] applies them to the parent.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Tdt {
    pub version: u32,
    /// Another `.tdt` this one is based on (then the fields below are not present).
    pub inherits: Option<String>,
    pub tgt: Option<String>,
    pub feature: Option<String>,
    /// Edge types (`.et`): down, right, up, left.
    pub edges: [Option<String>; 4],
    /// Size in tiles.
    pub size: (u8, u8),
    /// Ground types (`.gt`): down-left, down-right, up-right, up-left.
    pub grounds: [Option<String>; 4],
    pub unk: [u8; 4],
    pub edge_distances: [u8; 8],
    /// Everything after the header.
    pub rest: Vec<u8>,
    /// Ground type per corner point, `(w + 1) × (h + 1)` row-major in the tile's own
    /// orientation (data +0xa8; empty when the file has none).
    pub point_grounds: Vec<Option<String>>,
    /// Height per corner point (data +0xc0; empty when the file has none).
    pub point_heights: Vec<i8>,
    /// Orientations the tile may be used in (data +0x171, bit per orientation; 0 or 0xff:
    /// all), intersected with the `.tst` rotation list by the tile-set index (`0x141d09ab0`).
    pub orientation_mask: u8,
    /// Override flags of an inheriting tile (see the module docs).
    pub inherit_flags: u8,
}

#[derive(Debug, thiserror::Error)]
#[error("tdt at byte {offset}: {what}")]
pub struct TdtError {
    pub offset: usize,
    pub what: &'static str,
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    strings: Vec<u16>,
}

impl Reader<'_> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N], TdtError> {
        let b = self.data.get(self.pos..self.pos + N).ok_or(TdtError {
            offset: self.pos,
            what: "truncated",
        })?;
        self.pos += N;
        Ok(b.try_into().unwrap())
    }

    fn u8(&mut self) -> Result<u8, TdtError> {
        Ok(self.bytes::<1>()?[0])
    }

    fn i32(&mut self) -> Result<i32, TdtError> {
        Ok(i32::from_le_bytes(self.bytes()?))
    }

    /// A string reference: `i32` char offset into the table (None when out of range).
    fn string(&mut self) -> Result<Option<String>, TdtError> {
        let i = self.i32()?;
        if i < 0 || i as usize >= self.strings.len() {
            return Ok(None);
        }
        let s = &self.strings[i as usize..];
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Ok(Some(String::from_utf16_lossy(&s[..end])))
    }
}

impl Tdt {
    pub fn parse(data: &[u8]) -> Result<Self, TdtError> {
        let mut r = Reader {
            data,
            pos: 0,
            strings: Vec::new(),
        };
        let version = u32::from_le_bytes(r.bytes()?);
        let chars = u32::from_le_bytes(r.bytes()?) as usize;
        let table = data.get(8..8 + chars * 2).ok_or(TdtError {
            offset: 8,
            what: "string table truncated",
        })?;
        r.strings = table
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        r.pos = 8 + chars * 2;
        let mut t = Tdt {
            version,
            ..Default::default()
        };
        t.inherits = r.string()?.filter(|s| !s.is_empty());
        let inherit = t.inherits.is_some();
        if inherit {
            t.inherit_flags = r.u8()?;
        }
        let iflags = t.inherit_flags;
        let has = move |bit: u8| !inherit || iflags & bit != 0;
        if has(1) {
            t.tgt = r.string()?.filter(|s| !s.is_empty());
        }
        if has(8) {
            t.feature = r.string()?.filter(|s| !s.is_empty());
            for e in &mut t.edges {
                *e = r.string()?.filter(|s| !s.is_empty());
            }
            t.size = (r.u8()?, r.u8()?);
            for g in &mut t.grounds {
                *g = r.string()?.filter(|s| !s.is_empty());
            }
            t.unk = r.bytes()?;
            t.edge_distances = r.bytes()?;
        } else if t.inherit_flags & 0x10 != 0 {
            t.size = (r.u8()?, r.u8()?);
        }
        t.rest = data[r.pos..].to_vec();
        if r.pos < data.len() {
            t.read_points(&mut r)?;
        }
        Ok(t)
    }

    /// This inheriting tile's overrides applied to its (resolved) parent.
    pub fn resolve(&self, parent: &Tdt) -> Tdt {
        let f = self.inherit_flags;
        let mut t = parent.clone();
        t.inherits = self.inherits.clone();
        t.inherit_flags = f;
        if f & 1 != 0 {
            t.tgt = self.tgt.clone();
        }
        if f & 8 != 0 {
            t.feature = self.feature.clone();
            t.edges = self.edges.clone();
            t.size = self.size;
            t.grounds = self.grounds.clone();
            t.unk = self.unk;
            t.edge_distances = self.edge_distances;
            t.point_grounds = self.point_grounds.clone();
            t.point_heights = self.point_heights.clone();
        } else if f & 0x10 != 0 {
            t.size = self.size;
        }
        if f & 0x20 != 0 && self.version > 9 {
            t.orientation_mask = self.orientation_mask;
        }
        t
    }

    /// Orientation mask `m` (bit per orientation) restricted by this tile's own mask.
    pub fn restrict_orientations(&self, m: u8) -> u8 {
        if (1..0xff).contains(&self.orientation_mask) { m & self.orientation_mask } else { m }
    }

    /// The body up to the corner points (`0x1429153f0`): flags, hull planes, sub-tiles (each
    /// with an optional compressed byte map and a byte list), then per-point grounds (flag 2)
    /// and heights (flag 1).
    fn read_points(&mut self, r: &mut Reader) -> Result<(), TdtError> {
        let v = self.version;
        let mut flags = u16::from_le_bytes(r.bytes()?);
        if v < 14 {
            flags &= !0x100;
        }
        if v < 13 {
            flags &= !0x200;
        }
        let hi = (flags >> 8) as u8;
        let (inherit, iflags) = (self.inherits.is_some(), self.inherit_flags);
        let has = move |bit: u8| !inherit || iflags & bit != 0;
        if v > 9 && has(0x20) {
            self.orientation_mask = r.u8()?;
        }
        if has(0x10) && v >= 10 {
            let n = r.u8()?;
            if n > 4 {
                return Err(TdtError { offset: r.pos, what: "invalid number of hull planes" });
            }
            if n >= 2 {
                for _ in 0..n {
                    r.u8()?;
                }
            }
        }
        if has(1) && hi & 2 != 0 {
            r.u8()?;
        }
        if has(8) || has(0x10) {
            let count = u16::from_le_bytes(r.bytes()?);
            let full = has(0x10) || (has(1) && hi & 2 != 0);
            for _ in 0..count {
                let w4 = match (full, v < 10) {
                    (false, _) => 0,
                    (true, true) => r.u8()?,
                    (true, false) => u16::from_le_bytes(r.bytes()?) as u8,
                };
                if has(8) {
                    r.u8()?;
                }
                if has(1) && hi & 2 != 0 {
                    r.bytes::<3>()?;
                }
                if !has(0x10) {
                    continue;
                }
                if w4 & 4 == 0 {
                    skip_byte_map(r, v)?;
                }
                if !(w4 & 1 != 0 && w4 & 2 != 0) {
                    let n = u16::from_le_bytes(r.bytes()?) as usize;
                    r.pos = r.pos.checked_add(n).filter(|&p| p <= r.data.len()).ok_or(TdtError {
                        offset: r.pos,
                        what: "sub-tile bytes truncated",
                    })?;
                }
            }
        }
        if !has(8) {
            return Ok(());
        }
        let n = (self.size.0 as usize + 1) * (self.size.1 as usize + 1);
        if flags & 2 != 0 {
            for _ in 0..n {
                let g = r.string()?.filter(|s| !s.is_empty());
                self.point_grounds.push(g);
            }
        }
        if flags & 1 != 0 {
            for _ in 0..n {
                self.point_heights.push(r.u8()? as i8);
            }
        }
        Ok(())
    }
}

/// A sub-tile byte map (`0x1429179c0`): raw 0x211 bytes before version 10, else a header byte
/// (type in bits 0..2, extra size bytes in bits 3..4, size in bits 5..7) and, for types 1..4, a
/// compressed payload of that size.
fn skip_byte_map(r: &mut Reader, version: u32) -> Result<(), TdtError> {
    let n = if version < 10 {
        0x211
    } else {
        let b = r.u8()?;
        let kind = b & 7;
        let extra = (b >> 3) & 3;
        let mut size = ((b >> 5) & 7) as usize;
        for k in 0..extra {
            size |= (r.u8()? as usize) << (8 * k as usize + 3);
        }
        if kind == 0 { 0 } else { size }
    };
    r.pos = r.pos.checked_add(n).filter(|&p| p <= r.data.len()).ok_or(TdtError {
        offset: r.pos,
        what: "byte map truncated",
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Metadata/Terrain/Blank.tdt` (0.5.5e), the whole file.
    #[test]
    fn blank() {
        let mut data = Vec::new();
        data.extend_from_slice(&14u32.to_le_bytes());
        let table = "Art/Models/Terrain/Blank.tgt\0";
        data.extend_from_slice(&(table.len() as u32).to_le_bytes());
        for u in table.encode_utf16() {
            data.extend_from_slice(&u.to_le_bytes());
        }
        // inherits "" (the table's final NUL), tgt 0, feature "", four edges none.
        for v in [28i32, 0, 28, -1, -1, -1, -1] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        data.extend_from_slice(&[1, 1]);
        for _ in 0..4 {
            data.extend_from_slice(&(-1i32).to_le_bytes());
        }
        data.extend_from_slice(&[0, 0, 0, 0, 3, 3, 3, 3, 3, 3, 3, 3]);
        let t = Tdt::parse(&data).unwrap();
        assert_eq!(t.tgt.as_deref(), Some("Art/Models/Terrain/Blank.tgt"));
        assert_eq!(t.inherits, None);
        assert_eq!(t.size, (1, 1));
        assert_eq!(t.edge_distances, [3; 8]);
    }
}
