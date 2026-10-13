//! Hashes the client uses to identify game content compactly.

/// MurmurHash2 (32-bit). With seed 0 over the ASCII metadata path, this is the object type
/// key the client looks up (`0x1416936c0`) for server-spawned entities, e.g.
/// `"Metadata/Characters/Dex/DexFour"` -> `0xc393f3d5`.
pub fn murmur2(data: &[u8], seed: u32) -> u32 {
    const M: u32 = 0x5bd1_e995;
    let mut h = seed ^ data.len() as u32;
    let mut chunks = data.chunks_exact(4);
    for chunk in &mut chunks {
        let mut k = u32::from_le_bytes(chunk.try_into().expect("4-byte chunk"));
        k = k.wrapping_mul(M);
        k ^= k >> 24;
        k = k.wrapping_mul(M);
        h = h.wrapping_mul(M) ^ k;
    }
    let tail = chunks.remainder();
    if !tail.is_empty() {
        for (i, &b) in tail.iter().enumerate() {
            h ^= (b as u32) << (8 * i);
        }
        h = h.wrapping_mul(M);
    }
    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^ (h >> 15)
}

/// Object type key of a metadata path (`Metadata/...`).
pub fn object_type_key(metadata_path: &str) -> u32 {
    murmur2(metadata_path.as_bytes(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranger_type_key() {
        assert_eq!(
            object_type_key("Metadata/Characters/Dex/DexFour"),
            0xc393_f3d5
        );
    }
}
