//! Areas and terrain (M5): the client's terrain generator, ported so the server knows every
//! area's layout (collision, pathfinding, spawn points, area entities). Progress and findings:
//! `Documentation/re/terrain.md`.
//!
//! - [`rng`]: the generator's random number generator (MSVC `std::mt19937`), bit-exact.
//! - [`hash`]: the tile and doodad hashes the client reports; a layout is right when both match.
//! - [`graph`], [`tdt`]: the terrain graph (`.tgr`) and tile (`.tdt`) formats.
//! - [`export`]: the client DLL's area exports, the reference for comparisons.

pub mod export;
pub mod generate;
pub mod graph;
pub mod hash;
pub mod layout;
pub mod rng;
pub mod rooms;
pub mod smoothing;
pub mod routing;
pub mod tdt;
mod text;
pub mod tileset;
pub mod voronoi;
