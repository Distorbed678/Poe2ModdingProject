//! Terrain seeds known to generate.
//!
//! The client generates the area from (area, seed) and some seeds fail ("Mismatching ground
//! types", `TerrainGenerationException`); the official server generates the terrain itself and
//! never hands those out. Until the server generates terrain (M4), it can pick seeds that the
//! client is known to accept, from a TSV of `area, level, seed, tile hash, doodad hash` rows
//! (`tools/re/clientlog_terrain.py` extracts them from a live `Client.txt`).

use std::{collections::HashMap, path::Path};

use tracing::{info, warn};

#[derive(Debug, Default)]
pub struct KnownSeeds {
    by_area: HashMap<String, Vec<u32>>,
}

impl KnownSeeds {
    /// Loads the TSV at `path`; a missing or empty path gives an empty set.
    pub fn load(path: &Path) -> Self {
        if path.as_os_str().is_empty() {
            return Self::default();
        }
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let seeds = Self::parse(&text);
                info!(file = %path.display(), areas = seeds.by_area.len(), "known terrain seeds");
                seeds
            }
            Err(e) => {
                warn!(file = %path.display(), error = %e, "no known terrain seeds");
                Self::default()
            }
        }
    }

    pub fn parse(text: &str) -> Self {
        let mut by_area: HashMap<String, Vec<u32>> = HashMap::new();
        for line in text.lines() {
            let mut f = line.split('\t');
            let (Some(area), Some(_level), Some(seed)) = (f.next(), f.next(), f.next()) else {
                continue;
            };
            if let Ok(seed) = seed.parse() {
                let seeds = by_area.entry(area.to_owned()).or_default();
                if !seeds.contains(&seed) {
                    seeds.push(seed);
                }
            }
        }
        Self { by_area }
    }

    /// A random known seed for the area, if any.
    pub fn pick(&self, area: &str, random: u32) -> Option<u32> {
        let seeds = self.by_area.get(area)?;
        (!seeds.is_empty()).then(|| seeds[random as usize % seeds.len()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_picks() {
        let s = KnownSeeds::parse("G1_1\t1\t10\t5\t6\nG1_1\t1\t20\t7\t8\nG1_1\t1\t10\t5\t6\nbad\n");
        assert_eq!(s.by_area["G1_1"], vec![10, 20]);
        assert_eq!(s.pick("G1_1", 3), Some(20));
        assert_eq!(s.pick("G1_2", 3), None);
    }
}
