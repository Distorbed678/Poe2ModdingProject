//! Areas and terrain: the client's terrain generator, ported so the server knows every
//! area's layout (collision, pathfinding, spawn points, area entities). Progress and findings:
//! `docs/re/terrain/README.md`.
//!
//! - [`rng`]: the generator's random number generator (MSVC `std::mt19937`), bit-exact.
//! - [`hash`]: the tile and doodad hashes the client reports; a layout is right when both match.
//! - [`graph`], [`tdt`]: the terrain graph (`.tgr`) and tile (`.tdt`) formats.
//! - [`export`]: the client DLL's area exports, the reference for comparisons.

pub mod area;
pub mod dungeon;
pub mod dungeon_tiles;
pub mod edge_tiles;
pub mod export;
pub mod fill;
pub mod generate;
pub mod graph;
pub mod hash;
pub mod lattice;
pub mod outer_buffer;
pub mod overlay;
pub mod region;
pub mod layout;
pub mod node_tiles;
pub mod rng;
pub mod rooms;
pub mod smoothing;
pub mod split;
pub mod routing;
pub mod tdt;
mod text;
pub mod tileset;
pub mod voronoi;

/// Decodes a UTF-16 (or UTF-8) text file from the game data.
pub fn text_utf16(bytes: &[u8]) -> String {
    text::decode_utf16(bytes)
}
