//! Tile definitions (`.tdt`, binary, versions 14/15): a tile's geometry (`.tgt`), its size in
//! tiles, the edge type on each side and the ground type at each corner, which is what the
//! generator matches when it places tiles. Layout after `Tools/reference/poeformats/Tdt.cs`:
//!
//! `u32 version, u32 string-table chars, UTF-16 strings (NUL-separated)`, then string
//! references (`i32` char offsets into the table, -1 = none): `inherits`; when empty: `tgt`,
//! `feature`, edge types down/right/up/left, `u8 size x, u8 size y`, ground types
//! down-left/down-right/up-right/up-left, 4 bytes, 8 edge-distance bytes. The rest (sub-tile
//! data, doodads, entities) is kept raw for now.

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
        if t.inherits.is_none() {
            t.tgt = r.string()?.filter(|s| !s.is_empty());
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
        }
        t.rest = data[r.pos..].to_vec();
        Ok(t)
    }
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
