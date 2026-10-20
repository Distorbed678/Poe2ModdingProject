//! The client's terrain hashes (`docs/re/terrain.md`): the tile hash
//! (`0x141d141d0`) and the doodad hash, which the client sends in TerrainHashes. A generated
//! layout is right when both match.

const M: u32 = 0x5bd1_e995;
/// Seed of both hashes.
pub const SEED: u32 = 0x00f3_6b12;

fn mix(k: u32) -> u32 {
    let k = k.wrapping_mul(M);
    (k ^ (k >> 24)).wrapping_mul(M)
}

fn finish(h: u32) -> u32 {
    let h = (h ^ (h >> 13)).wrapping_mul(M);
    h ^ (h >> 15)
}

/// MurmurHash2 over UTF-16 code units, two per block (`0x140419970`).
pub fn murmur2_utf16(s: &str, seed: u32) -> u32 {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut h = (units.len() as u32 * 2) ^ seed;
    let mut chunks = units.chunks_exact(2);
    for c in &mut chunks {
        h = h.wrapping_mul(M) ^ mix(c[0] as u32 | (c[1] as u32) << 16);
    }
    if let [u] = chunks.remainder() {
        h = (*u as u32 ^ h).wrapping_mul(M);
    }
    finish(h)
}

/// MurmurHash2 (`0x142194f90`).
pub fn murmur2(data: &[u8], seed: u32) -> u32 {
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

/// One placed tile as the tile hash sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashedTile<'a> {
    pub x: u32,
    pub y: u32,
    /// The tile's `.tdt` path.
    pub path: &'a str,
    pub height: i16,
    /// 0..7: rotation and mirroring.
    pub orientation: u8,
}

fn tile_entry(t: &HashedTile, path_hash: u32) -> u32 {
    let mut h = (mix(t.height as i32 as u32) ^ 0x9bfb_c07e).wrapping_mul(M);
    h = (mix(t.orientation as u32) ^ h).wrapping_mul(M);
    h = (mix(t.x) ^ h).wrapping_mul(M);
    h = mix(t.y) ^ h;
    finish(h.wrapping_mul(M) ^ mix(path_hash))
}

/// The tile hash: `tiles` in the client's order (column by column: x outer, y inner), only
/// the hashed ones (a sub-tile set, not skipped). 0 becomes 1, as in the client.
pub fn tile_hash<'a>(tiles: impl IntoIterator<Item = HashedTile<'a>>) -> u32 {
    let mut bytes = Vec::new();
    for t in tiles {
        let e = tile_entry(&t, murmur2_utf16(t.path, SEED));
        bytes.extend_from_slice(&e.to_le_bytes());
    }
    match murmur2(&bytes, SEED) {
        0 => 1,
        h => h,
    }
}

/// The doodad hash: the wrapping sum of `y + murmur2_utf16(path) + (x << 16)` over the placed
/// doodads (cells).
pub fn doodad_hash<'a>(doodads: impl IntoIterator<Item = (i32, i32, &'a str)>) -> u32 {
    doodads.into_iter().fold(0u32, |h, (x, y, path)| {
        h.wrapping_add(y as u32)
            .wrapping_add(murmur2_utf16(path, SEED))
            .wrapping_add((x as u32) << 16)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn murmur2_reference_values() {
        assert_eq!(murmur2(b"", 0), 0);
        // Textbook MurmurHash2 test vector.
        assert_eq!(murmur2(b"hello", 0), 0xe56129cb);
    }
}
