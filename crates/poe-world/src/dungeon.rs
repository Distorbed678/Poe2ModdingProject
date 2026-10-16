//! Dungeon layouts (`.dgr` topologies, `0x1420ac3e0`): rooms on a grid of room units, joined by
//! corridors. Findings: `Documentation/re/dungeon.md`. Ported so far: typed-node room placement.

use std::collections::HashMap;

use crate::{
    generate::{Files, GenError},
    graph::Graph,
    rng::TerrainRng,
    rooms::{RoomSet, TerrainTypes, node_orientation},
};

fn unsupported(s: impl Into<String>) -> GenError {
    GenError::Unsupported(s.into())
}

/// A `.dgr` edge's dungeon fields (the tokens after the edge type).
#[derive(Debug, Clone, Default)]
pub struct EdgeFields {
    pub chance2: i32,
    pub chance1: i32,
    pub depth_limit: i32,
    pub branching_type: String,
    /// Edge +0x54.
    pub extra_door: u32,
    /// Edge +0x70 (empty: `Metadata/Terrain/Dungeon/rooms.et`).
    pub extra_door_type: String,
    pub fuzzy: i32,
    pub impenetrability: i32,
    pub impenetrable: String,
    pub void: bool,
    pub accessible: bool,
}

impl EdgeFields {
    pub fn parse(extra: &[String]) -> Result<Self, GenError> {
        let int = |i: usize| -> Result<i32, GenError> {
            extra.get(i).and_then(|s| s.parse().ok()).ok_or_else(|| unsupported(format!(".dgr edge field {i} in {extra:?}")))
        };
        let s = |i: usize| extra.get(i).cloned().unwrap_or_default();
        Ok(Self {
            chance2: int(0)?,
            chance1: int(1)?,
            depth_limit: int(2)?,
            branching_type: s(3),
            extra_door: int(4)? as u32,
            extra_door_type: s(5),
            fuzzy: int(6)?,
            impenetrability: int(7)?,
            impenetrable: s(8),
            void: s(9).eq_ignore_ascii_case("Y"),
            accessible: int(10)? != 0,
        })
    }
}

/// One door around a room: the door's edge type (lower case), height and chirality flag
/// (`ExitDatum`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Exit {
    pub door: Option<String>,
    pub height: i32,
    pub chiral: bool,
}

/// `0x141d00fd0`: where ring position `i` of a `w`×`h` room goes when the room is turned by `o`.
pub fn turn_index(w: u32, h: u32, i: u32, o: u8) -> u32 {
    const A: [u32; 9] = [0, 1, 1, 2, 0, 1, 1, 2, 0];
    const B: [u32; 9] = [0, 0, 1, 1, 1, 1, 2, 2, 0];
    let n = 2 * (w + h);
    let o = o.min(8) as usize;
    let i = if (4..8).contains(&o) { n - 1 - i } else { i };
    (i + A[o] * h + B[o] * w) % n
}

/// The unit cell outside ring position `u` of a `w`×`h` room at `origin` (`0x1420af280`):
/// top left→right, right top→bottom, bottom right→left, left bottom→top.
fn ring_cell(origin: (i32, i32), (w, h): (u32, u32), u: u32) -> (i32, i32) {
    let (x, y) = origin;
    let (w, h, u) = (w as i32, h as i32, u as i32);
    if u < w {
        (x + u, y - 1)
    } else if u < w + h {
        (x + w, y + (u - w))
    } else if u < 2 * w + h {
        (x - 1 + 2 * w + h - u, y + h)
    } else {
        (x - 1, y - 1 + 2 * (w + h) - u)
    }
}

/// The room cell just inside ring position `u`.
fn ring_inner(origin: (i32, i32), size: (u32, u32), u: u32) -> (i32, i32) {
    let (cx, cy) = ring_cell(origin, size, u);
    let (w, h) = (size.0 as i32, size.1 as i32);
    (cx.clamp(origin.0, origin.0 + w - 1), cy.clamp(origin.1, origin.1 + h - 1))
}

/// `0x142f10760`: angle (0x8000 = π) for `cos·127 + 127`, 0..=255.
const ACOS: [u16; 256] = [
    0x8000, 0x7ae2, 0x78c2, 0x7720, 0x75bf, 0x7487, 0x736d, 0x7269, 0x7176, 0x7092, 0x6fb9, 0x6eeb,
    0x6e25, 0x6d67, 0x6cb0, 0x6bff, 0x6b54, 0x6aad, 0x6a0b, 0x696d, 0x68d3, 0x683c, 0x67a8, 0x6718,
    0x668a, 0x65ff, 0x6576, 0x64f0, 0x646c, 0x63ea, 0x636a, 0x62eb, 0x626f, 0x61f4, 0x617a, 0x6102,
    0x608c, 0x6017, 0x5fa3, 0x5f31, 0x5ebf, 0x5e4f, 0x5de0, 0x5d72, 0x5d05, 0x5c99, 0x5c2e, 0x5bc4,
    0x5b5a, 0x5af2, 0x5a8a, 0x5a23, 0x59bd, 0x5958, 0x58f3, 0x588f, 0x582c, 0x57c9, 0x5767, 0x5705,
    0x56a4, 0x5644, 0x55e4, 0x5585, 0x5526, 0x54c8, 0x546a, 0x540c, 0x53af, 0x5353, 0x52f7, 0x529b,
    0x5240, 0x51e5, 0x518a, 0x5130, 0x50d6, 0x507d, 0x5024, 0x4fcb, 0x4f72, 0x4f1a, 0x4ec2, 0x4e6a,
    0x4e13, 0x4dbc, 0x4d65, 0x4d0e, 0x4cb8, 0x4c61, 0x4c0b, 0x4bb6, 0x4b60, 0x4b0b, 0x4ab6, 0x4a61,
    0x4a0c, 0x49b7, 0x4963, 0x490f, 0x48bb, 0x4867, 0x4813, 0x47bf, 0x476b, 0x4718, 0x46c5, 0x4671,
    0x461e, 0x45cb, 0x4578, 0x4526, 0x44d3, 0x4480, 0x442e, 0x43db, 0x4389, 0x4336, 0x42e4, 0x4291,
    0x423f, 0x41ed, 0x419b, 0x4149, 0x40f6, 0x40a4, 0x4052, 0x4000, 0x3fae, 0x3f5c, 0x3f0a, 0x3eb7,
    0x3e65, 0x3e13, 0x3dc1, 0x3d6f, 0x3d1c, 0x3cca, 0x3c77, 0x3c25, 0x3bd2, 0x3b80, 0x3b2d, 0x3ada,
    0x3a88, 0x3a35, 0x39e2, 0x398f, 0x393b, 0x38e8, 0x3895, 0x3841, 0x37ed, 0x3799, 0x3745, 0x36f1,
    0x369d, 0x3649, 0x35f4, 0x359f, 0x354a, 0x34f5, 0x34a0, 0x344a, 0x33f5, 0x339f, 0x3348, 0x32f2,
    0x329b, 0x3244, 0x31ed, 0x3196, 0x313e, 0x30e6, 0x308e, 0x3035, 0x2fdc, 0x2f83, 0x2f2a, 0x2ed0,
    0x2e76, 0x2e1b, 0x2dc0, 0x2d65, 0x2d09, 0x2cad, 0x2c51, 0x2bf4, 0x2b96, 0x2b38, 0x2ada, 0x2a7b,
    0x2a1c, 0x29bc, 0x295c, 0x28fb, 0x2899, 0x2837, 0x27d4, 0x2771, 0x270d, 0x26a8, 0x2643, 0x25dd,
    0x2576, 0x250e, 0x24a6, 0x243c, 0x23d2, 0x2367, 0x22fb, 0x228e, 0x2220, 0x21b1, 0x2141, 0x20cf,
    0x205d, 0x1fe9, 0x1f74, 0x1efe, 0x1e86, 0x1e0c, 0x1d91, 0x1d15, 0x1c96, 0x1c16, 0x1b94, 0x1b10,
    0x1a8a, 0x1a01, 0x1976, 0x18e8, 0x1858, 0x17c4, 0x172d, 0x1693, 0x15f5, 0x1553, 0x14ac, 0x1401,
    0x1350, 0x1299, 0x11db, 0x1115, 0x1047, 0x0f6e, 0x0e8a, 0x0d97, 0x0c93, 0x0b79, 0x0a41, 0x08e0,
    0x073e, 0x051e, 0x0000, 0x0000,
];

/// `0x14196e010`: integer square root.
fn isqrt(v: u32) -> u32 {
    let mut r = (v as f64).sqrt() as u32;
    while r.saturating_mul(r) > v {
        r -= 1;
    }
    while (r + 1).saturating_mul(r + 1) <= v {
        r += 1;
    }
    r
}

/// `0x1421c4e50`: the angle between two vectors (`ACOS` units; 0 for a zero vector).
fn angle(a: (i32, i32), b: (i32, i32)) -> u16 {
    if a == (0, 0) || b == (0, 0) {
        return 0;
    }
    let la = a.0.wrapping_mul(a.0).wrapping_add(a.1.wrapping_mul(a.1));
    let lb = b.0.wrapping_mul(b.0).wrapping_add(b.1.wrapping_mul(b.1));
    let s = isqrt(lb.wrapping_mul(la) as u32).max(1) as i32;
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let twice_dot = lb - dy * dy - dx * dx + la;
    let idx = (twice_dot.wrapping_mul(0x7f) / s) / 2 + 0x7f;
    ACOS[idx.clamp(0, 0xfe) as usize]
}

/// A room of the set as the dungeon sees it.
struct DungeonRoom {
    path: String,
    /// The room type (the `.arm` tag; empty for `special…` rooms, whose tag the parser clears).
    tag: String,
    /// Kept out of the type buckets and size map (`0x141f99b20`): `special…` rooms and the
    /// shrine, mission, stone circle, preunique and injector types.
    generic: bool,
    /// The `.arm` uniqueness flag (room data +6).
    unique: bool,
    /// Room data +8: a `special` / `specialN` / `specialchance N` tag (`0x142931cf0`).
    special: bool,
    /// Room data +4: the special list (`specialN`; 0xff: the chance list).
    list: u8,
    /// Room data +5: `specialchance N` (default 100).
    chance: u8,
    weight: u32,
    mask: u8,
    units: (u32, u32),
    exits: Vec<Exit>,
}

/// A room candidate (`RoomSet` entry): a room turned by `o`.
#[derive(Debug, Clone)]
struct Candidate {
    room: usize,
    o: u8,
    size: (u32, u32),
    exits: Vec<Exit>,
    weight: u32,
}

/// A room placed on the grid.
#[derive(Debug, Clone)]
pub struct PlacedRoom {
    pub path: String,
    pub node: Option<usize>,
    pub origin: (i32, i32),
    pub size: (u32, u32),
    pub orientation: u8,
    pub exits: Vec<Exit>,
    /// The extra door type the node's edge pick chose (`0x1420966f0`).
    pub extra_door: Option<String>,
    /// Room height (+0x12): the node's, or the edge's on a corridor.
    pub height: i32,
    /// Flags +0x15 bit 0: reachable (an accessible edge or tag).
    pub accessible: bool,
    /// Flags +0x15 bit 1: closed to extra doors (typed or tagged nodes, impenetrable edges).
    pub blocked: bool,
    /// +0x10: extra door chance (%).
    pub extra_pct: u32,
    /// +0x40: the door type a placeholder's room must take (its node's or corridor's edge type).
    pub door_type: Option<String>,
    /// Kind 8 (+0x14): a special room placed over a generic one (`0x1420a7da0`).
    pub special: bool,
}

impl PlacedRoom {
    /// A placeholder (no `.arm` yet; room +0 null).
    pub fn is_placeholder(&self) -> bool {
        self.path.is_empty()
    }
}

/// The unit grid (`0x141ff05f0`'s 0x70-byte object).
pub struct Grid {
    pub w: u32,
    pub h: u32,
    /// Per cell: 0/1 free, n ≥ 2 room n − 2.
    pub cells: Vec<u32>,
    pub rooms: Vec<PlacedRoom>,
    /// Grid +0x34: the tiles rooms share at their borders (master +0x14b).
    pub overlap: u32,
    /// Door types whose two sides differ (`0x141d047b0`): their doors carry a chirality.
    pub chiral: std::collections::HashSet<String>,
}

impl Grid {
    fn is_chiral(&self, door: &str) -> bool {
        self.chiral.contains(door)
    }

    /// A room's door ring turned by `o` (`0x141f964f0`): mirrored orientations flip the
    /// chirality of chiral doors.
    fn turned(&self, w: u32, h: u32, exits: &[Exit], o: u8) -> Vec<Exit> {
        let n = exits.len() as u32;
        let mut out = vec![Exit { door: None, height: 0, chiral: false }; n as usize];
        for i in 0..n {
            let mut e = exits[i as usize].clone();
            if o >= 4 && e.door.as_deref().is_some_and(|d| self.is_chiral(d)) {
                e.chiral = !e.chiral;
            }
            out[turn_index(w, h, i, o) as usize] = e;
        }
        out
    }

    fn state(&self, (x, y): (i32, i32)) -> Option<u32> {
        (x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h).then(|| self.cells[(y as u32 * self.w + x as u32) as usize])
    }

    /// `0x1420a6d30`.
    fn free(&self, origin: (i32, i32), (w, h): (u32, u32)) -> bool {
        (0..h as i32).all(|dy| (0..w as i32).all(|dx| self.state((origin.0 + dx, origin.1 + dy)).is_some_and(|s| s < 2)))
    }

    /// `0x1420a73d0` (`0x1420a6fb0` gives a placeholder an empty door ring).
    fn stamp(&mut self, mut room: PlacedRoom) -> Result<(), GenError> {
        if room.exits.is_empty() {
            room.exits = vec![Exit { door: None, height: 0, chiral: false }; (2 * (room.size.0 + room.size.1)) as usize];
        }
        let id = self.rooms.len() as u32 + 2;
        for dy in 0..room.size.1 as i32 {
            for dx in 0..room.size.0 as i32 {
                let (x, y) = (room.origin.0 + dx, room.origin.1 + dy);
                match self.state((x, y)) {
                    Some(s) if s < 2 => self.cells[(y as u32 * self.w + x as u32) as usize] = id,
                    _ => return Err(unsupported(format!("dungeon room overlap at ({x},{y})"))),
                }
            }
        }
        self.rooms.push(room);
        Ok(())
    }
}

/// The dungeon layout of a pruned `.dgr` graph (`0x1420ac3e0`).
pub fn layout(files: &dyn Files, graph: &Graph, removed: &[bool], set: &RoomSet, rng: &mut TerrainRng) -> Result<Grid, GenError> {
    let mut types = TerrainTypes::default();
    let mut rooms: Vec<DungeonRoom> = set
        .rooms
        .iter()
        .map(|r| {
            let arm = &r.room.arm;
            let units = (arm.numbers1.first().copied().unwrap_or(1), arm.numbers1.get(1).copied().unwrap_or(1));
            let exits = arm
                .thingies
                .iter()
                // Version > 29 puts a height override flag before the chirality.
                .map(|t| Exit { door: t.et_file.as_ref().map(|s| s.to_ascii_lowercase()), height: t.int, chiral: if arm.version > 29 { t.bool2 } else { t.bool1 }.unwrap_or(false) })
                .collect();
            let tag = r.tag.to_ascii_lowercase();
            let special = tag.starts_with("special");
            let (list, chance) = match tag.strip_prefix("specialchance") {
                Some(n) => (0xff, n.trim().parse().unwrap_or(100)),
                None => (tag.strip_prefix("special").and_then(|n| n.trim().parse().ok()).unwrap_or(0xff), 100),
            };
            let generic = !special && !matches!(tag.as_str(), "shrine" | "mission" | "stonecircle" | "preunique") && !tag.starts_with("telepad_injector") && !tag.starts_with("room_injector") && !tag.starts_with("tile_injector");
            let unique = arm.bools.first().copied().unwrap_or(false);
            DungeonRoom { path: r.path.clone(), tag: if special { String::new() } else { r.tag.clone() }, generic, unique, special, list, chance, weight: r.weight, mask: r.orientations, units, exits }
        })
        .collect();
    for r in &rooms {
        if r.exits.len() as u32 != 2 * (r.units.0 + r.units.1) {
            return Err(unsupported(format!("room {}: {} exits for {}x{} units", r.path, r.exits.len(), r.units.0, r.units.1)));
        }
    }
    let fields: Vec<EdgeFields> = graph.edges.iter().map(|e| EdgeFields::parse(&e.extra)).collect::<Result<_, _>>()?;
    for e in &graph.edges {
        let t = types.edge(files, &e.edge_type)?;
        if t.height_change != 0 || t.virtual_edge {
            return Err(unsupported(format!("dungeon edge type {} (chirality not ported)", e.edge_type)));
        }
    }
    // A node's side slots: its edges by edge index (`0x141d1c580`).
    let slots: Vec<Vec<usize>> = (0..graph.nodes.len())
        .map(|n| (0..graph.edges.len()).filter(|&e| !removed[e] && (graph.edges[e].from as usize == n || graph.edges[e].to as usize == n)).collect())
        .collect();
    // Master +0x14b; the rooms' `.arm` dimensions carry the same overlap.
    let overlap = set.rooms.first().and_then(|r| r.room.arm.dimensions.uint1).unwrap_or(1);
    let mut grid = Grid { w: graph.width, h: graph.height, cells: vec![0; (graph.width * graph.height) as usize], rooms: Vec::new(), overlap, chiral: Default::default() };
    // Door types with a chirality: those of the rooms' doors and the graph's edges.
    let mut door_types: Vec<String> = rooms.iter().flat_map(|r| r.exits.iter().filter_map(|e| e.door.clone())).collect();
    door_types.extend(graph.edges.iter().map(|e| e.edge_type.to_ascii_lowercase()));
    door_types.sort();
    door_types.dedup();
    for t in door_types {
        if !t.is_empty() && chiral_type(&mut types, files, &t)? {
            grid.chiral.insert(t);
        }
    }
    for r in &mut rooms {
        for e in &mut r.exits {
            e.chiral &= e.door.as_deref().is_some_and(|d| grid.chiral.contains(d));
        }
    }
    let unit = |n: usize| (graph.nodes[n].x / 23, graph.nodes[n].y / 23);
    let height = |n: usize| graph.nodes[n].values.get(1).copied().unwrap_or(0);
    let priority = |n: usize| graph.nodes[n].values.get(2).copied().unwrap_or(0) as i8;

    // `0x1420aa500`: typed nodes, then typeless ones, each stable-sorted by priority.
    let mut typed: Vec<usize> = (0..graph.nodes.len()).filter(|&n| !graph.nodes[n].room_type.is_empty()).collect();
    let mut typeless: Vec<usize> = (0..graph.nodes.len()).filter(|&n| graph.nodes[n].room_type.is_empty()).collect();
    typed.sort_by_key(|&n| std::cmp::Reverse(priority(n)));
    typeless.sort_by_key(|&n| std::cmp::Reverse(priority(n)));
    let _ = &typeless;

    let void = |n: usize| graph.nodes[n].extra.first().is_some_and(|v| v.eq_ignore_ascii_case("Y"));
    // `0x142095d90`: an accessible edge, or an `accessible` / `default` / `entrance…` /
    // `sideentrance…` tag.
    let node_accessible = |n: usize| {
        slots[n].iter().any(|&e| fields[e].accessible)
            || graph.nodes[n].tags.iter().any(|t| {
                let t = t.to_ascii_lowercase();
                t == "accessible" || t == "default" || t.starts_with("entrance") || t.starts_with("sideentrance")
            })
    };
    // `0x1420af7d0`: typed or tagged nodes, or nodes with an impenetrable edge.
    let node_blocked = |n: usize| {
        !graph.nodes[n].room_type.is_empty() || !graph.nodes[n].tags.is_empty() || slots[n].iter().any(|&e| fields[e].impenetrable.eq_ignore_ascii_case("I"))
    };
    let node_extra = |n: usize| slots[n].iter().map(|&e| fields[e].extra_door).max().unwrap_or(0);
    for &n in &typed {
        if void(n) {
            continue;
        }
        let node = &graph.nodes[n];
        let no = node_orientation(&node.orientation);
        let empty = Exit { door: None, height: 0, chiral: false };
        let exit_for = |e: usize| {
            let edge = &graph.edges[e];
            let h = height(edge.from as usize).max(height(edge.to as usize)) - height(n);
            let door = edge.edge_type.to_ascii_lowercase();
            let chiral = grid.is_chiral(&door) && ((edge.from as usize == n) ^ (edge.values.get(1).copied().unwrap_or(0) != 0));
            Exit { door: Some(door), height: h, chiral }
        };
        // Required exits (`0x1420ad6b0`, `0x141f96730`).
        let required: Vec<Exit> = slots[n].iter().map(|&e| exit_for(e)).collect();
        // The type's rooms in every allowed orientation (`RoomSet` entries).
        let mut all: Vec<Candidate> = Vec::new();
        for (ri, r) in rooms.iter().enumerate() {
            if !r.generic || !r.tag.eq_ignore_ascii_case(&node.room_type) {
                continue;
            }
            for o in 0..8u8 {
                if r.mask >> o & 1 == 0 {
                    continue;
                }
                let exits = grid.turned(r.units.0, r.units.1, &r.exits, o);
                let size = if o & 1 == 1 { (r.units.1, r.units.0) } else { r.units };
                all.push(Candidate { room: ri, o, size, exits, weight: r.weight });
            }
        }
        let cands: Vec<Candidate> = if no == 9 {
            // `(best)`: each edge takes the free side closest to its direction, then
            // `GetExactRooms` (`0x141fc0350`), else the looser `0x141fc0460`.
            match best_sides(graph, &slots[n], n) {
                Some(sides) => {
                    let sides: [Exit; 4] = sides.map(|s| s.map_or(empty.clone(), exit_for));
                    let exact: Vec<Candidate> = all.iter().filter(|c| exact_sides(c, &sides, None)).cloned().collect();
                    if exact.is_empty() {
                        let mut loose = Vec::new();
                        for c in &all {
                            if exact_sides(c, &sides, Some(&mut |door: &str| types.edge(files, door).map(|t| t.can_end).unwrap_or(false))) {
                                loose.push(c.clone());
                            }
                        }
                        loose
                    } else {
                        exact
                    }
                }
                None => Vec::new(),
            }
        } else {
            // `GetPossibleRooms` (`0x141f9b8f0`): doors containing the required ones.
            all.into_iter().filter(|c| contains(&c.exits, &required)).collect()
        };
        if cands.is_empty() {
            return Err(unsupported(format!("dungeon node {n} ({}): no room (GetFeatureRooms not ported)", node.room_type)));
        }
        // `0x1420d1110`: orientation and fit; candidates whose doors disagree with a neighbour
        // are dropped when others remain.
        let pos = unit(n);
        if std::env::var("POE_DBG").is_ok() {
            eprintln!("DBG node {n} {} o{no} pos {pos:?} req {:?} best {:?}", node.room_type, required, if no == 9 { best_sides(graph, &slots[n], n) } else { None });
            for c in &cands {
                let origin = origin_for(&grid, graph.region, pos, c.size);
                eprintln!("DBG   cand {} o{} size {:?} w{} origin {origin:?} fits {:?} exits {:?}", rooms[c.room].path.rsplit('/').next().unwrap(), c.o, c.size, c.weight, fits(&grid, c, origin, required.len() as u32), c.exits.iter().map(|e| e.door.is_some() as u8).collect::<Vec<_>>());
            }
        }
        let mut kept: Vec<(Candidate, (i32, i32), bool)> = Vec::new();
        for c in cands {
            if !(no >= 8 || c.o == no) {
                continue;
            }
            let origin = origin_for(&grid, graph.region, pos, c.size);
            if let Some(mismatch) = fits(&grid, &c, origin, required.len() as u32) {
                kept.push((c, origin, mismatch));
            }
        }
        if kept.iter().any(|k| !k.2) {
            kept.retain(|k| !k.2);
        }
        if kept.is_empty() {
            return Err(unsupported(format!("dungeon node {n} ({}): no suitable room", node.room_type)));
        }
        let total: u32 = kept.iter().map(|k| k.0.weight).sum();
        let mut k = pick(&kept, rng.below(total));
        if repeats(&grid, &rooms[kept[k].0.room].path, kept[k].1, kept[k].0.size) {
            k = pick(&kept, rng.below(total));
        }
        let (c, origin, _) = kept[k].clone();
        let extra_door = extra_door_pick(&slots[n], &fields, rng);
        grid.stamp(PlacedRoom { path: rooms[c.room].path.clone(), node: Some(n), origin, size: c.size, orientation: c.o, exits: c.exits, extra_door, height: height(n), accessible: node_accessible(n), blocked: node_blocked(n), extra_pct: node_extra(n), door_type: None, special: false })?;
    }
    // Typeless nodes (`0x1420afdd0`): a room size drawn from the size map of the node's first
    // edge type until one fits; the room itself comes later.
    let sizes = size_map_final(&rooms);
    for &n in &typeless {
        if void(n) {
            continue;
        }
        let pos = unit(n);
        let i = match grid.state(pos) {
            Some(_) => (pos.1 as u32 * grid.w + pos.0 as u32) as usize,
            None => return Err(unsupported(format!("dungeon node {n} out of bounds at {pos:?}"))),
        };
        if grid.cells[i] == 1 {
            grid.cells[i] = 0;
        }
        if grid.cells[i] != 0 {
            return Err(unsupported(format!("dungeon node {n} overlaps another room at {pos:?}")));
        }
        let door = slots[n].first().map(|&e| graph.edges[e].edge_type.to_ascii_lowercase()).unwrap_or_default();
        let mut list: Vec<((u32, u32), u32)> = sizes.iter().find(|s| s.0 == door).map(|s| s.1.clone()).unwrap_or_default();
        if let Some(forced) = forced_size(&graph.nodes[n].tags) {
            list.retain(|s| s.0 == forced);
        }
        if list.is_empty() {
            return Err(unsupported(format!("dungeon node {n}: no room sizes for edge type {door}")));
        }
        let mut total: u32 = list.iter().map(|s| s.1).sum();
        loop {
            let mut r = rng.below(total);
            let mut k = list.len() - 1;
            for (j, s) in list.iter().enumerate() {
                if r < s.1 {
                    k = j;
                    break;
                }
                r -= s.1;
            }
            let (w, h) = list[k].0;
            let origin = size_origin(graph.region, pos, (w, h));
            let inside = origin.0 >= 0 && origin.1 >= 0 && origin.0 as u32 + w <= grid.w && origin.1 as u32 + h <= grid.h;
            let clear = inside && (0..h as i32).all(|dy| (0..w as i32).all(|dx| grid.state((origin.0 + dx, origin.1 + dy)) == Some(0)));
            // `0x1420af6a0`: the rectangle must not hold another node.
            let swallows = |m: usize| {
                let p = unit(m);
                m != n && p.0 >= origin.0 && p.0 < origin.0 + w as i32 && p.1 >= origin.1 && p.1 < origin.1 + h as i32
            };
            if clear && !(0..graph.nodes.len()).any(swallows) {
                let extra_door = extra_door_pick(&slots[n], &fields, rng);
                grid.stamp(PlacedRoom { path: String::new(), node: Some(n), origin, size: (w, h), orientation: 0, exits: Vec::new(), extra_door, height: height(n), accessible: node_accessible(n), blocked: node_blocked(n), extra_pct: node_extra(n), door_type: Some(door.clone()), special: false })?;
                break;
            }
            let size = list[k].0;
            if let Some(j) = list.iter().position(|s| s.0 == size) {
                total -= list[j].1;
                list.remove(j);
            }
            if list.is_empty() {
                return Err(unsupported(format!("dungeon node {n}: no room size fits at {pos:?}")));
            }
        }
    }
    corridors(files, graph, removed, &fields, &rooms, &sizes, &mut grid, &mut types, rng)?;
    // `0x1420a9590` (open door counts) and `0x1420b19f0` (ground spreading) draw nothing; then
    // the special rooms (`0x1420aa710`). The request-driven steps after it (`0x1420aab60` /
    // `0x1420aacf0` extra rooms, `0x1420abf60` shrines) have nothing to place for a plain area.
    special_rooms(files, &rooms, &mut grid, &mut types, rng)?;
    Ok(grid)
}

/// `0x1420ad6b0` for `(best)` nodes: per edge, the sides its direction leans to (x first, then
/// y) with their angle to that side; sorted by angle, the smallest first takes its side if free
/// (dropping the edge's other proposals). `None` unless every edge gets a side. Sides: 0 up,
/// 1 right, 2 down, 3 left.
fn best_sides(graph: &Graph, slots: &[usize], n: usize) -> Option<[Option<usize>; 4]> {
    let node = (graph.nodes[n].x, graph.nodes[n].y);
    let mut props: Vec<(usize, u32, u32)> = Vec::new();
    for &e in slots {
        let edge = &graph.edges[e];
        let target = if edge.from as usize == n {
            edge.points.first().copied().unwrap_or((graph.nodes[edge.to as usize].x, graph.nodes[edge.to as usize].y))
        } else {
            edge.points.last().copied().unwrap_or((graph.nodes[edge.from as usize].x, graph.nodes[edge.from as usize].y))
        };
        let d = (target.0 - node.0, target.1 - node.1);
        if d.0 < 0 {
            props.push((e, 3, angle(d, (-100, 0)) as u32));
        } else if d.0 > 0 {
            props.push((e, 1, angle(d, (100, 0)) as u32));
        }
        if d.1 < 0 {
            props.push((e, 0, angle(d, (0, -100)) as u32));
        } else if d.1 > 0 {
            props.push((e, 2, angle(d, (0, 100)) as u32));
        }
    }
    // `0x1420d5840`: stable, largest angle first.
    props.sort_by_key(|p| std::cmp::Reverse(p.2));
    let mut sides: [Option<usize>; 4] = [None; 4];
    while let Some(&(e, side, _)) = props.last() {
        if sides[side as usize].is_none() {
            sides[side as usize] = Some(e);
            props.retain(|p| p.0 != e);
        } else {
            props.pop();
        }
    }
    (sides.iter().flatten().count() == slots.len()).then_some(sides)
}

/// `0x141fc0350` (`loose` = `None`) / `0x141fc0460`: every side of the candidate's door ring
/// holds that side's required exit; the loose form also accepts a side with no required door
/// whose doors may all be left closed (`.et` flag 0x10).
fn exact_sides(c: &Candidate, sides: &[Exit; 4], mut loose: Option<&mut dyn FnMut(&str) -> bool>) -> bool {
    let (w, h) = c.size;
    let bounds = [0, w, w + h, 2 * w + h, c.exits.len() as u32];
    (0..4).all(|k| {
        let range = bounds[k] as usize..bounds[k + 1] as usize;
        if c.exits[range.clone()].iter().any(|e| *e == sides[k]) {
            return true;
        }
        match &mut loose {
            None => false,
            Some(optional) => sides[k].door.is_none() && c.exits[range].iter().all(|e| e.door.as_deref().is_none_or(|d| optional(d))),
        }
    })
}

/// `0x1420966f0`: the extra door type, drawn from the node's edges weighted by their extra
/// door %.
fn extra_door_pick(slots: &[usize], fields: &[EdgeFields], rng: &mut TerrainRng) -> Option<String> {
    let weighted: Vec<(usize, u32)> = slots.iter().map(|&e| (e, fields[e].extra_door)).filter(|w| w.1 != 0).collect();
    if weighted.is_empty() {
        return None;
    }
    let mut r = rng.below(weighted.iter().map(|w| w.1).sum());
    let mut chosen = weighted[weighted.len() - 1].0;
    for &(e, w) in &weighted {
        if r < w {
            chosen = e;
            break;
        }
        r -= w;
    }
    let t = &fields[chosen].extra_door_type;
    Some(if t.is_empty() { "metadata/terrain/dungeon/rooms.et".to_string() } else { t.to_ascii_lowercase() })
}

/// `0x1420d0000`: a `size:W` or `size:WxH` node tag.
fn forced_size(tags: &[String]) -> Option<(u32, u32)> {
    let t = tags.iter().find_map(|t| t.strip_prefix("size:"))?;
    match t.split_once('x') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)).filter(|s: &(u32, u32)| s.0 != 0 && s.1 != 0),
        None => t.parse().ok().map(|v| (v, v)),
    }
}

/// `0x1420afdd0`: the origin of a `w`×`h` size at a node's unit cell (the extent is taken off on
/// axes the region mirrors; not clamped).
fn size_origin(region: u8, pos: (i32, i32), (w, h): (u32, u32)) -> (i32, i32) {
    const TABLE: [[u8; 3]; 9] = crate::region::TABLE;
    let [a, b, c] = TABLE[region.min(8) as usize];
    let (rw, rh) = if region & 1 == 1 && region < 8 { (h as i32, w as i32) } else { (w as i32, h as i32) };
    let v = [rw - 1, rh - 1, 0];
    let x = pos.0 - [v[0], 0, v[1], 0][(b + a * 2) as usize];
    let y = pos.1 - [v[0], 0, v[1], 0][((if a == 0 { 2 } else { 0 }) + c) as usize];
    (x, y)
}

/// The size lists per door type after `0x141f995a0`: sizes of generic rooms with their summed
/// weights, keeping only sizes some non-unique generic room has.
fn size_map_final(rooms: &[DungeonRoom]) -> Vec<(String, Vec<((u32, u32), u32)>)> {
    let mut all: Vec<(String, Vec<((u32, u32), u32)>)> = Vec::new();
    let mut plain: Vec<(String, Vec<(u32, u32)>)> = Vec::new();
    for r in rooms.iter().filter(|r| r.generic) {
        let (w, h) = r.units;
        let mut seen: Vec<&String> = Vec::new();
        for door in r.exits.iter().filter_map(|e| e.door.as_ref()) {
            if seen.contains(&door) {
                continue;
            }
            seen.push(door);
            let mut sizes = Vec::new();
            if r.mask == 0 || r.mask & 0x55 != 0 {
                sizes.push((w, h));
            }
            if w != h && (r.mask == 0 || r.mask & 0xaa != 0) {
                sizes.push((h, w));
            }
            let k = all.iter().position(|e| &e.0 == door).unwrap_or_else(|| {
                all.push((door.clone(), Vec::new()));
                plain.push((door.clone(), Vec::new()));
                all.len() - 1
            });
            for size in sizes {
                match all[k].1.iter_mut().find(|e| e.0 == size) {
                    Some(e) => e.1 += r.weight,
                    None => all[k].1.push((size, r.weight)),
                }
                if !r.unique && !plain[k].1.contains(&size) {
                    plain[k].1.push(size);
                }
            }
        }
    }
    for (k, (_, sizes)) in all.iter_mut().enumerate() {
        sizes.retain(|s| plain[k].1.contains(&s.0) && s.1 != 0);
        sizes.sort_by_key(|s| s.0);
    }
    all
}

/// Weighted pick over kept candidates.
fn pick(kept: &[(Candidate, (i32, i32), bool)], mut r: u32) -> usize {
    for (i, k) in kept.iter().enumerate() {
        if r < k.0.weight {
            return i;
        }
        r -= k.0.weight;
    }
    kept.len() - 1
}

/// `0x141f9b7a0`: every required door is among the candidate's (as a multiset).
fn contains(exits: &[Exit], required: &[Exit]) -> bool {
    let mut count: HashMap<&Exit, i32> = HashMap::new();
    for e in exits {
        *count.entry(e).or_default() += 1;
    }
    required.iter().all(|r| {
        let c = count.entry(r).or_default();
        *c -= 1;
        *c >= 0
    })
}

/// `0x1420aefc0`: the room's origin for a node at unit `pos` (the room's extent is taken off on
/// axes the region mirrors), clamped into the grid.
fn origin_for(grid: &Grid, region: u8, pos: (i32, i32), (w, h): (u32, u32)) -> (i32, i32) {
    const TABLE: [[u8; 3]; 9] = crate::region::TABLE;
    let [a, b, c] = TABLE[region.min(8) as usize];
    // The graph's orientation swaps the sides for odd orientations.
    let (rw, rh) = if region & 1 == 1 && region < 8 { (h as i32, w as i32) } else { (w as i32, h as i32) };
    let v = [rw - 1, 0, rh - 1, 0];
    let x = pos.0 - v[(b + a * 2) as usize];
    let y = pos.1 - v[((if a == 0 { 2 } else { 0 }) + c) as usize];
    let x = x.max(0).min(grid.w as i32 - w as i32);
    let y = y.max(0).min(grid.h as i32 - h as i32);
    (x, y)
}

/// `0x1420af280`: the room fits at `origin` (free cells; at least `need` doors lead into the
/// grid). Returns whether a door faces a placed room whose door there differs.
fn fits(grid: &Grid, c: &Candidate, origin: (i32, i32), need: u32) -> Option<bool> {
    if !grid.free(origin, c.size) {
        return None;
    }
    if c.size == (grid.w, grid.h) {
        return Some(false);
    }
    let (mut any, mut inside, mut mismatch) = (false, 0, false);
    for (u, e) in c.exits.iter().enumerate() {
        let Some(door) = &e.door else { continue };
        any = true;
        let cell = ring_cell(origin, c.size, u as u32);
        let Some(state) = grid.state(cell) else { continue };
        inside += 1;
        if state < 2 || mismatch {
            continue;
        }
        let p = &grid.rooms[state as usize - 2];
        let inner = ring_inner(origin, c.size, u as u32);
        let n = p.exits.len() as u32;
        if let Some(v) = (0..n).find(|&v| ring_cell(p.origin, p.size, v) == inner && ring_inner(p.origin, p.size, v) == cell) {
            if p.exits[v as usize].door.as_ref() != Some(door) {
                mismatch = true;
            }
        }
    }
    if any && inside < need {
        return None;
    }
    Some(mismatch)
}

/// `0x1420b1550`: a room right next to the rectangle is the same `.arm`.
fn repeats(grid: &Grid, path: &str, origin: (i32, i32), (w, h): (u32, u32)) -> bool {
    let mut cells = Vec::new();
    for dx in 0..w as i32 {
        cells.push((origin.0 + dx, origin.1 - 1));
        cells.push((origin.0 + dx, origin.1 + h as i32));
    }
    for dy in 0..h as i32 {
        cells.push((origin.0 - 1, origin.1 + dy));
        cells.push((origin.0 + w as i32, origin.1 + dy));
    }
    cells.iter().any(|&c| grid.state(c).is_some_and(|s| s >= 2 && grid.rooms[s as usize - 2].path.eq_ignore_ascii_case(path)))
}

/// The room-size map per door type (`0x141f98bb0`, room set +0x28): every generic room adds its
/// `.rs` weight to (w, h) under each door type it has (once per type), and to (h, w) when it is
/// not square and its orientations include odd ones. Rooms tagged `shrine` or `mission` are kept
/// apart and add nothing.
pub fn size_map(set: &RoomSet) -> Vec<(String, Vec<((u32, u32), u32)>)> {
    let mut out: Vec<(String, Vec<((u32, u32), u32)>)> = Vec::new();
    for r in &set.rooms {
        let tag = r.tag.to_ascii_lowercase();
        if tag == "shrine" || tag == "mission" || tag == "stonecircle" {
            continue;
        }
        let a = &r.room.arm;
        let (w, h) = (a.numbers1.first().copied().unwrap_or(1), a.numbers1.get(1).copied().unwrap_or(1));
        let mut seen: Vec<String> = Vec::new();
        for t in &a.thingies {
            let Some(door) = t.et_file.as_ref().map(|s| s.to_ascii_lowercase()) else { continue };
            if seen.contains(&door) {
                continue;
            }
            seen.push(door.clone());
            let k = match out.iter().position(|e| e.0 == door) {
                Some(k) => k,
                None => {
                    out.push((door.clone(), Vec::new()));
                    out.len() - 1
                }
            };
            let mut add = |size: (u32, u32)| match out[k].1.iter_mut().find(|e| e.0 == size) {
                Some(e) => e.1 += r.weight,
                None => out[k].1.push((size, r.weight)),
            };
            if r.orientations == 0 || r.orientations & 0x55 != 0 {
                add((w, h));
            }
            if w != h && (r.orientations == 0 || r.orientations & 0xaa != 0) {
                add((h, w));
            }
        }
    }
    out
}

/// [`size_map`] as text, with totals.
pub fn size_report(set: &RoomSet) -> String {
    size_map(set)
        .iter()
        .map(|(door, sizes)| format!("{door}: total {} {:?}", sizes.iter().map(|s| s.1).sum::<u32>(), sizes))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Per placed room (`0x1420a5e50` records, 0x30 bytes): a random cost and search state.
#[derive(Debug, Clone, Default)]
struct Record {
    /// +0: on an impenetrable corridor.
    impen: bool,
    /// +1: on a corridor (or a node's room).
    used: bool,
    /// +4: the search predecessor.
    pred: Option<usize>,
    /// +8: the corridor's impenetrability.
    impval: i32,
    /// +0xc: `below(128)`.
    cost: u32,
    /// +0x10 / +0x14: A* g and h.
    g: i32,
    h: i32,
    /// +0x18: corridor heights through the room.
    heights: Vec<i32>,
}

/// The corridor stage of `0x1420ac3e0` (step 4): edge batches, the shuffle (`0x141fa8b60`), then
/// per batch (`0x1420a2c70` → `0x1420acdd0`): placeholders on every free cell (`0x1420a6140`),
/// records (`0x1420a6030`) and the edge connections (`0x1420a3a20`, not ported).
#[allow(clippy::too_many_arguments)]
fn corridors(
    files: &dyn Files,
    graph: &Graph,
    removed: &[bool],
    fields: &[EdgeFields],
    rooms: &[DungeonRoom],
    sizes: &[(String, Vec<((u32, u32), u32)>)],
    grid: &mut Grid,
    types: &mut TerrainTypes,
    rng: &mut TerrainRng,
) -> Result<(), GenError> {
    let _ = rooms;
    // `0x1420a5e50`: one record per placed room with a `below(128)` cost.
    let mut records: Vec<Record> = Vec::new();
    let grow = |records: &mut Vec<Record>, grid: &Grid, rng: &mut TerrainRng| {
        while records.len() < grid.rooms.len() {
            records.push(Record { cost: rng.below(128), ..Default::default() });
        }
    };
    grow(&mut records, grid, rng);
    // `0x141fa8df0`: non-void edges by priority (ascending), then three containers (edge types
    // with exactly one room size, other types, the default type), each ordered by
    // `0x141fa7920` with equal keys kept in edge order.
    let priority = |e: usize| graph.edges[e].values.get(2).copied().unwrap_or(0) as i8;
    let mut keys: Vec<(i8, u8, (bool, i32, u8, String), usize)> = Vec::new();
    for e in 0..graph.edges.len() {
        if removed[e] || fields[e].void {
            continue;
        }
        let t = graph.edges[e].edge_type.to_ascii_lowercase();
        let n_sizes = sizes.iter().find(|s| s.0 == t).map_or(0, |s| s.1.len());
        let container = if t.is_empty() { 2 } else if n_sizes == 1 { 0 } else { 1 };
        let hc = types.edge(files, &t)?.height_change;
        let impen = fields[e].impenetrable.eq_ignore_ascii_case("I");
        keys.push((priority(e), container, (impen, fields[e].impenetrability, hc, t), e));
    }
    // Impenetrable first, then higher impenetrability, higher height change, type name.
    keys.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(b.2.0.cmp(&a.2.0))
            .then(b.2.1.cmp(&a.2.1))
            .then(b.2.2.cmp(&a.2.2))
            .then(a.2.3.cmp(&b.2.3))
            .then(a.3.cmp(&b.3))
    });
    let mut batches: Vec<Vec<usize>> = Vec::new();
    for (i, k) in keys.iter().enumerate() {
        if i > 0 && keys[i - 1].0 == k.0 && keys[i - 1].1 == k.1 && keys[i - 1].2 == k.2 {
            batches.last_mut().unwrap().push(k.3);
        } else {
            batches.push(vec![k.3]);
        }
    }
    // `0x141fc00f0`: a discarded `below(n)`, a forward Fisher-Yates shuffle, then a stable sort
    // by priority.
    for b in &mut batches {
        rng.below(b.len() as u32);
        let n = b.len();
        for i in 0..n {
            let k = rng.uniform((n - i) as u32) as usize;
            b.swap(i, i + k);
        }
        b.sort_by_key(|&e| priority(e));
    }
    for room in 0..grid.rooms.len().min(records.len()) {
        records[room].used = true;
    }
    let mut paths: Vec<Corridor> = Vec::new();
    let mark_used = |grid: &Grid, records: &mut [Record]| {
        // `0x1420a6030`: rooms with an `.arm` count as used (placeholders with a tag list would
        // too, but only typed nodes' rooms carry their tags, and those have an `.arm`).
        for (i, r) in grid.rooms.iter().enumerate() {
            if !r.is_placeholder() {
                records[i].used = true;
            }
        }
    };
    for b in &batches {
        let t = graph.edges[b[0]].edge_type.to_ascii_lowercase();
        let list: Vec<((u32, u32), u32)> = sizes.iter().find(|s| s.0 == t).map(|s| s.1.clone()).unwrap_or_default();
        fill_placeholders(grid, &list, Some(t.clone()), rng)?;
        grow(&mut records, grid, rng);
        mark_used(grid, &mut records);
        for &e in b {
            connect(graph, e, &fields[e], grid, &mut records, &mut paths, rng)?;
        }
        compact(grid, &mut records, &mut paths);
    }
    // `0x1420a50e0`: the corridor types sorted by path (`0x1420db6f0`), a discarded `below(n)`
    // and a shuffle, then stable-partitioned into types with exactly one room size
    // (`0x1420a55b0`), other types, and the default type last; types with a branching edge get
    // placeholders, records and branches (`0x1420a5680`).
    let mut keys: Vec<String> = Vec::new();
    for c in &paths {
        if !keys.contains(&c.key) {
            keys.push(c.key.clone());
        }
    }
    keys.sort();
    rng.below(keys.len() as u32);
    for i in 0..keys.len() {
        let k = rng.uniform((keys.len() - i) as u32) as usize;
        keys.swap(i, i + k);
    }
    let is_default = |k: &str| k.is_empty() || k == "metadata/terrain/dungeon/rooms.et";
    let single = |k: &str| sizes.iter().find(|s| s.0 == k).is_some_and(|s| s.1.len() == 1);
    let rank = |k: &str| if is_default(k) { 2 } else if single(k) { 0 } else { 1 };
    keys.sort_by_key(|k| rank(k));
    let generic: Vec<(u32, u32, Vec<Exit>)> = generic_shapes(grid, rooms);
    for key in &keys {
        if !paths.iter().any(|c| &c.key == key && fields[c.edge].chance1 != 0) {
            continue;
        }
        let list: Vec<((u32, u32), u32)> = sizes.iter().find(|s| &s.0 == key).map(|s| s.1.clone()).unwrap_or_default();
        fill_placeholders(grid, &list, Some(key.clone()), rng)?;
        grow(&mut records, grid, rng);
        mark_used(grid, &mut records);
        branch(graph, fields, key, &generic, grid, &mut records, &paths, rng)?;
        compact(grid, &mut records, &mut paths);
    }
    extra_doors(grid, &generic, open_into_impenetrable(graph), rng)?;
    assign_rooms(files, graph, rooms, grid, &records, types, rng)?;
    Ok(())
}

/// `0x1420b0fe0` per used room (`0x1420da7c0`): a placeholder takes a non-unique generic room
/// whose doors match its own (`RoomSet` size buckets, `0x141f9b280` / `0x141f9b430`), drawn by
/// weight, with an orientation drawn from the matching ones (`below(840)`); a second draw when a
/// neighbour is the same room.
fn assign_rooms(files: &dyn Files, graph: &Graph, rooms: &[DungeonRoom], grid: &mut Grid, records: &[Record], types: &mut TerrainTypes, rng: &mut TerrainRng) -> Result<(), GenError> {
    // Size buckets: (room, o, size, turned doors, weight) in room order, then orientation.
    let mut entries: Vec<(usize, u8, (u32, u32), Vec<Exit>, u32)> = Vec::new();
    for (ri, r) in rooms.iter().enumerate() {
        if !r.generic || r.unique {
            continue;
        }
        for o in 0..8u8 {
            if r.mask >> o & 1 == 0 {
                continue;
            }
            let exits = grid.turned(r.units.0, r.units.1, &r.exits, o);
            let size = if o & 1 == 1 { (r.units.1, r.units.0) } else { r.units };
            entries.push((ri, o, size, exits, r.weight));
        }
    }
    for i in 0..grid.rooms.len() {
        let p = &grid.rooms[i];
        if !records.get(i).is_some_and(|r| r.used) || !p.is_placeholder() {
            continue;
        }
        let Some(door_type) = p.door_type.clone() else { continue };
        let optional = |types: &mut TerrainTypes, d: &str| types.edge(files, d).map(|t| t.can_end).unwrap_or(false);
        let strict = !(p.extra_pct == 0 && optional(types, &door_type));
        let mut list = match_rooms(files, &entries, p, if strict { 2 } else { 1 }, &grid.chiral, types);
        if list.is_empty() && !strict {
            list = match_rooms(files, &entries, p, 0, &grid.chiral, types);
        }
        if list.is_empty() && strict {
            list = match_rooms(files, &entries, p, 1, &grid.chiral, types);
            if list.is_empty() {
                list = match_rooms(files, &entries, p, 0, &grid.chiral, types);
            }
        }
        if list.is_empty() {
            return Err(unsupported(format!("dungeon: no room fits placeholder {i} at {:?}", p.origin)));
        }
        let total: u32 = list.iter().map(|l| l.2).sum();
        if std::env::var("POE_DBG_ASSIGN").is_ok() {
            eprintln!("DBG assign {i} at {:?} size {:?} strict {strict} doors {:?} total {total}", p.origin, p.size, p.exits.iter().map(|e| e.door.as_deref().map_or("-", |d| d.rsplit('/').next().unwrap())).collect::<Vec<_>>());
            for l in &list {
                eprintln!("DBG   {} mask {:#x} w {}", rooms[l.0].path.rsplit('/').next().unwrap(), l.1, l.2);
            }
        }
        let draw = |rng: &mut TerrainRng| {
            let mut r = rng.below(total);
            let mut k = list.len() - 1;
            for (j, l) in list.iter().enumerate() {
                if r < l.2 {
                    k = j;
                    break;
                }
                r -= l.2;
            }
            let o = crate::region::pick(list[k].1, rng.below(840));
            (k, o)
        };
        let (mut k, mut o) = draw(rng);
        let origin = origin_for(grid, graph.region, p.origin, p.size);
        if list.len() > 1 && repeats(grid, &rooms[list[k].0].path, origin, p.size) {
            (k, o) = draw(rng);
        }
        let r = &rooms[list[k].0];
        let exits = grid.turned(r.units.0, r.units.1, &r.exits, o);
        let p = &mut grid.rooms[i];
        p.path = r.path.clone();
        p.orientation = o;
        p.exits = exits;
    }
    Ok(())
}

/// `0x141f9b430`: entries of the placeholder's size whose doors include its doors (type,
/// height); `mode` 2 allows no other doors, 1 only optional ones (`.et` flag 0x10), 0 any.
/// Orientations of one room merge (mask); its weight is (empty matches² × 10 + 1) × the `.rs`
/// weight of its first matching orientation.
fn match_rooms(files: &dyn Files, entries: &[(usize, u8, (u32, u32), Vec<Exit>, u32)], p: &PlacedRoom, mode: u8, chiral: &std::collections::HashSet<String>, types: &mut TerrainTypes) -> Vec<(usize, u8, u32)> {
    let mut out: Vec<(usize, u8, u32)> = Vec::new();
    for (room, o, size, exits, weight) in entries {
        if *size != p.size || exits.len() != p.exits.len() {
            continue;
        }
        let mut empty = 0u32;
        let mut ok = true;
        for (want, have) in p.exits.iter().zip(exits) {
            match (&want.door, &have.door) {
                (Some(w), Some(h)) => {
                    if w != h || want.height != have.height || (chiral.contains(w.as_str()) && want.chiral != have.chiral) {
                        ok = false;
                        break;
                    }
                }
                (Some(_), None) => {
                    ok = false;
                    break;
                }
                (None, None) => empty += 1,
                (None, Some(h)) => {
                    let optional = types.edge(files, h).map(|t| t.can_end).unwrap_or(false);
                    if mode == 2 || (mode == 1 && !optional) {
                        ok = false;
                        break;
                    }
                }
            }
        }
        if !ok {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.0 == *room => last.1 |= 1 << o,
            _ => {
                let w = (empty * empty * 10 + 1) * weight;
                if w != 0 {
                    out.push((*room, 1 << o, w));
                }
            }
        }
    }
    out
}

/// The `.dgr` header's `open_extra_doors_into_impenetrable_rooms` (version > 22).
fn open_into_impenetrable(graph: &Graph) -> bool {
    graph.extra_header.as_deref().is_some_and(|v| v != "0")
}

/// `0x1420a99e0`: extra doors. Per room with an extra door chance: its door positions (empty or
/// of its extra door type) facing a room it is not yet joined to (open, and one of the two
/// accessible) are shuffled (a discarded `below(n)` first); each one both sides could take
/// opens with `below(100)` under the chance (a chance over 100 opens more than one).
fn extra_doors(grid: &mut Grid, generic: &[(u32, u32, Vec<Exit>)], into_impenetrable: bool, rng: &mut TerrainRng) -> Result<(), GenError> {
    let mut reached: Vec<usize> = Vec::new();
    for r in 0..grid.rooms.len() {
        if grid.rooms[r].extra_pct == 0 {
            continue;
        }
        let door = grid.rooms[r].extra_door.clone().unwrap_or_else(|| "metadata/terrain/dungeon/rooms.et".into());
        let mut list: Vec<(u32, usize)> = Vec::new();
        for u in 0..grid.rooms[r].exits.len() as u32 {
            let room = &grid.rooms[r];
            let d = &room.exits[u as usize].door;
            if !(d.is_none() || d.as_deref() == Some(door.as_str())) {
                continue;
            }
            let Some(s) = grid.state(ring_cell(room.origin, room.size, u)) else { continue };
            if s < 2 {
                continue;
            }
            let n = s as usize - 2;
            let nr = &grid.rooms[n];
            if (into_impenetrable || !nr.blocked) && (room.accessible || nr.accessible) && !joined(room, nr) {
                list.push((u, n));
            }
        }
        rng.below(list.len() as u32);
        let k = list.len();
        for i in 0..k {
            let j = rng.uniform((k - i) as u32) as usize;
            list.swap(i, i + j);
        }
        let mut pct = grid.rooms[r].extra_pct;
        for &(_, n) in &list {
            let height = grid.rooms[r].height.max(grid.rooms[n].height);
            if can_branch(grid, generic, r, n, height, &door) && can_branch(grid, generic, n, r, height, &door) && rng.below(100) < pct {
                open_door(grid, r, n, &door, false, true, rng)?;
                if (grid.rooms[r].accessible || grid.rooms[n].accessible) && !reached.contains(&n) {
                    reached.push(n);
                }
                if pct < 101 {
                    break;
                }
                pct -= 100;
            }
        }
    }
    for n in reached {
        grid.rooms[n].accessible = true;
    }
    Ok(())
}

/// `0x141fa8320`: the rooms already share an open door.
fn joined(a: &PlacedRoom, b: &PlacedRoom) -> bool {
    (0..a.exits.len() as u32).any(|u| a.exits[u as usize].door.is_some() && facing(a, b, u).is_some_and(|v| b.exits[v as usize].door == a.exits[u as usize].door))
}

/// A corridor (`0x1420a3a20`'s 0x28-byte record under its type key): rooms from the edge's
/// to-node room back to its from-node room.
#[derive(Debug, Clone)]
struct Corridor {
    key: String,
    rooms: Vec<usize>,
    edge: usize,
    height: i32,
}

/// Untyped generic rooms in every allowed orientation: size and turned doors.
fn generic_shapes(grid: &Grid, rooms: &[DungeonRoom]) -> Vec<(u32, u32, Vec<Exit>)> {
    let mut out = Vec::new();
    for r in rooms.iter().filter(|r| r.generic && r.tag.is_empty()) {
        for o in 0..8u8 {
            if r.mask >> o & 1 == 0 {
                continue;
            }
            let exits = grid.turned(r.units.0, r.units.1, &r.exits, o);
            let (w, h) = if o & 1 == 1 { (r.units.1, r.units.0) } else { r.units };
            out.push((w, h, exits));
        }
    }
    out
}

/// `0x1420a2e90`: a corridor height on the room at `cell` (unused placeholders only).
fn mark_height(grid: &Grid, records: &mut [Record], cell: (i32, i32), height: i32) {
    let Some(s) = grid.state(cell) else { return };
    if s < 2 {
        return;
    }
    let i = s as usize - 2;
    if records[i].used || !grid.rooms[i].is_placeholder() {
        return;
    }
    if !records[i].heights.contains(&height) {
        records[i].heights.push(height);
        records[i].heights.sort();
    }
}

/// `0x1420a2fb0` (doors without chirality): with an overlap, the corridor's height goes to the
/// rooms outside `room`'s door-less positions.
fn mark_around(grid: &Grid, records: &mut [Record], room: usize) {
    if grid.overlap == 0 {
        return;
    }
    let r = grid.rooms[room].clone();
    let (x, y, w, h) = (r.origin.0, r.origin.1, r.size.0 as i32, r.size.1 as i32);
    let n = r.exits.len() as i32;
    let mut mark = |idx: i32, cell: (i32, i32)| {
        let e = &r.exits[idx as usize];
        if e.door.is_none() {
            mark_height(grid, records, cell, e.height + r.height);
        }
    };
    if y != 0 {
        for u in 0..w {
            mark(u, (x + u, y - 1));
        }
    }
    if ((y + h) as u32) < grid.h {
        for u in 0..w {
            mark(h - 1 + 2 * w - u, (x + u, y + h));
        }
    }
    if x != 0 {
        for u in 0..h {
            mark(n - u - 1, (x - 1, y + u));
        }
    }
    if ((x + w) as u32) < grid.w {
        for u in 0..h {
            mark(u + w, (x + w, y + u));
        }
    }
}

/// A branch start (`0x1420a4a10`, 0x18 bytes).
#[derive(Debug, Clone)]
struct BranchCand {
    room: usize,
    parent: usize,
    chance: i32,
    depth: i32,
    height: i32,
    impen: bool,
    accessible: bool,
    extra_pct: u32,
    extra_door: String,
}

/// `0x1420a4910` / `0x1420a4a10`: neighbours of `parent` that can start a branch.
#[allow(clippy::too_many_arguments)]
fn branch_starts(grid: &Grid, records: &[Record], generic: &[(u32, u32, Vec<Exit>)], parent: usize, proto: &BranchCand, door: &str, out: &mut Vec<BranchCand>) {
    let height = proto.height;
    for n in neighbours(grid, parent) {
        let rec = &records[n];
        let fits_height = if rec.heights.is_empty() { height == 0 } else { rec.heights.len() == 1 && rec.heights[0] == height };
        if !fits_height || rec.used || rec.impen {
            continue;
        }
        if can_branch(grid, generic, parent, n, height, door) {
            out.push(BranchCand { room: n, parent, ..proto.clone() });
        }
    }
}

/// `0x1420a4090`: a door from `a` towards `b`: an existing one at the height, or a free position
/// of a placeholder that some untyped room could still take with that door added.
fn can_branch(grid: &Grid, generic: &[(u32, u32, Vec<Exit>)], a: usize, b: usize, height: i32, door: &str) -> bool {
    let (ra, rb) = (&grid.rooms[a], &grid.rooms[b]);
    for u in shared_doors(grid, ra, rb, door, false) {
        let e = &ra.exits[u as usize];
        if e.door.is_none() && ra.is_placeholder() {
            let mut exits = ra.exits.clone();
            exits[u as usize] = Exit { door: Some(door.to_string()), height: height - ra.height, chiral: false };
            let ok = generic.iter().any(|(w, h, g)| {
                (*w, *h) == ra.size && exits.iter().zip(g).all(|(x, y)| x.door.is_none() || x == y)
            });
            if ok {
                return true;
            }
        }
        if e.door.as_deref() == Some(door) && e.height + ra.height == height {
            return true;
        }
    }
    false
}

/// `⌊chance·n/100⌋`, plus one with `below(100)` under the remainder; at most `n`.
fn chance_count(chance: i32, n: usize, rng: &mut TerrainRng) -> usize {
    let c = (chance.clamp(0, 100) as u64) * n as u64;
    let k = c / 100 + u64::from((rng.below(100) as u64) < c % 100);
    (k as usize).min(n)
}

/// `0x1420a5680`: branches off the corridors of one type.
#[allow(clippy::too_many_arguments)]
fn branch(graph: &Graph, fields: &[EdgeFields], key: &str, generic: &[(u32, u32, Vec<Exit>)], grid: &mut Grid, records: &mut [Record], paths: &[Corridor], rng: &mut TerrainRng) -> Result<(), GenError> {
    let branching = |n: u32| graph.nodes[n as usize].extra.get(1).is_some_and(|v| v != "0");
    for c in paths.iter().filter(|c| c.key == key) {
        let edge = &graph.edges[c.edge];
        // The corridor's end rooms at branching nodes mark their neighbours.
        if branching(edge.to) {
            for n in neighbours(grid, c.rooms[0]) {
                let o = grid.rooms[n].origin;
                mark_height(grid, records, o, c.height);
            }
        }
        if branching(edge.from) {
            for n in neighbours(grid, *c.rooms.last().unwrap()) {
                let o = grid.rooms[n].origin;
                mark_height(grid, records, o, c.height);
            }
        }
    }
    // `0x1420a4c90`: branch starts along each corridor with a branching chance.
    let mut starts: Vec<BranchCand> = Vec::new();
    for c in paths.iter().filter(|c| c.key == key) {
        let f = &fields[c.edge];
        if f.chance1 == 0 {
            continue;
        }
        let edge = &graph.edges[c.edge];
        let first = usize::from(!branching(edge.to));
        let end = if branching(edge.from) { c.rooms.len() } else { c.rooms.len() - 1 };
        let impen = f.impenetrable.eq_ignore_ascii_case("I");
        let mut list = Vec::new();
        let t = if f.extra_door_type.is_empty() { "metadata/terrain/dungeon/rooms.et".to_string() } else { f.extra_door_type.to_ascii_lowercase() };
        let proto = BranchCand { room: 0, parent: 0, chance: f.chance2, depth: f.depth_limit - 1, height: c.height, impen, accessible: f.accessible, extra_pct: f.extra_door, extra_door: t };
        for i in first..end.max(first) {
            branch_starts(grid, records, generic, c.rooms[i], &proto, key, &mut list);
        }
        if f.chance1 < 100 {
            let k = chance_count(f.chance1, list.len(), rng);
            rng.below(list.len() as u32);
            let n = list.len();
            for i in 0..n {
                let j = rng.uniform((n - i) as u32) as usize;
                list.swap(i, i + j);
            }
            list.truncate(k);
        }
        starts.extend(list);
    }
    let mut next: Vec<BranchCand> = Vec::new();
    for c in &starts {
        grow_branch(grid, records, generic, c, key, &mut next, rng)?;
    }
    while !next.is_empty() {
        rng.below(next.len() as u32);
        let n = next.len();
        for i in 0..n {
            let j = rng.uniform((n - i) as u32) as usize;
            next.swap(i, i + j);
        }
        let mut groups: Vec<(i32, Vec<BranchCand>)> = Vec::new();
        for c in next.drain(..) {
            match groups.iter_mut().find(|g| g.0 == c.chance) {
                Some(g) => g.1.push(c),
                None => groups.push((c.chance, vec![c])),
            }
        }
        groups.sort_by_key(|g| g.0);
        for (chance, group) in groups {
            let k = chance_count(chance, group.len(), rng);
            for c in group.iter().take(k) {
                grow_branch(grid, records, generic, c, key, &mut next, rng)?;
            }
        }
    }
    Ok(())
}

/// `0x1420a5ba0`: a branch room off its parent (a door; heights on its surroundings; deeper
/// starts while depth remains).
fn grow_branch(grid: &mut Grid, records: &mut [Record], generic: &[(u32, u32, Vec<Exit>)], c: &BranchCand, door: &str, next: &mut Vec<BranchCand>, rng: &mut TerrainRng) -> Result<(), GenError> {
    let rec = &records[c.room];
    if rec.used {
        return Ok(());
    }
    let fits_height = if rec.heights.is_empty() { c.height == 0 } else { rec.heights.len() == 1 && rec.heights[0] == c.height };
    if !fits_height {
        return Ok(());
    }
    grid.rooms[c.room].height = c.height;
    grid.rooms[c.room].door_type = Some(door.to_string());
    grid.rooms[c.room].accessible = c.accessible;
    grid.rooms[c.room].blocked = c.impen;
    grid.rooms[c.room].extra_pct = grid.rooms[c.room].extra_pct.max(c.extra_pct);
    if grid.rooms[c.room].extra_door.as_deref().is_none_or(|d| d == "metadata/terrain/dungeon/rooms.et") {
        grid.rooms[c.room].extra_door = Some(c.extra_door.clone());
    }
    records[c.room].used = true;
    records[c.room].impen = c.impen;
    open_door(grid, c.room, c.parent, door, false, false, rng)?;
    if grid.overlap != 0 {
        let r = grid.rooms[c.room].clone();
        for n in neighbours(grid, c.room) {
            let o = grid.rooms[n].origin;
            mark_height(grid, records, o, c.height);
        }
        let (x, y, w, h) = (r.origin.0, r.origin.1, r.size.0 as i32, r.size.1 as i32);
        let corners = [((x - 1, y - 1), (x, y - 1), (x - 1, y)), ((x + w, y - 1), (x + w - 1, y - 1), (x + w, y)), ((x + w, y + h), (x + w - 1, y + h), (x + w, y + h - 1)), ((x - 1, y + h), (x, y + h), (x - 1, y + h - 1))];
        for (cell, a, b) in corners {
            if let Some(s) = grid.state(cell) {
                if grid.state(a) != Some(s) && grid.state(b) != Some(s) {
                    mark_height(grid, records, cell, c.height);
                }
            }
        }
    }
    if c.depth != 0 {
        let proto = BranchCand { depth: c.depth - 1, ..c.clone() };
        branch_starts(grid, records, generic, c.room, &proto, door, next);
    }
    Ok(())
}

/// A room's centre in half units.
fn centre(r: &PlacedRoom) -> (i32, i32) {
    (2 * r.origin.0 + r.size.0 as i32, 2 * r.origin.1 + r.size.1 as i32)
}

/// `0x1420a3a20`: an edge between the rooms under its nodes: A* over rooms (`0x1420a4330`),
/// then doors along the path (`0x141fa8140`).
fn connect(graph: &Graph, e: usize, f: &EdgeFields, grid: &mut Grid, records: &mut [Record], paths: &mut Vec<Corridor>, rng: &mut TerrainRng) -> Result<(), GenError> {
    let edge = &graph.edges[e];
    let cell = |n: usize| (graph.nodes[n].x / 23, graph.nodes[n].y / 23);
    let (a, b) = (grid.state(cell(edge.from as usize)), grid.state(cell(edge.to as usize)));
    let (Some(a), Some(b)) = (a, b) else {
        return Err(unsupported(format!("dungeon edge {e}: an end outside the grid (0x1420a3770 not ported)")));
    };
    if a < 2 || b < 2 {
        return Err(unsupported(format!("dungeon edge {e}: connects from a void room")));
    }
    let (start, goal) = (a as usize - 2, b as usize - 2);
    let door = edge.edge_type.to_ascii_lowercase();
    let height = grid.rooms[start].height.max(grid.rooms[goal].height);
    let impenetrable = f.impenetrable.eq_ignore_ascii_case("I");
    let chir = edge.values.get(1).copied().unwrap_or(0) != 0;
    if !search(grid, records, start, goal, height, f.fuzzy, &door, chir, impenetrable) {
        return Err(unsupported(format!("dungeon edge {e}: failed to connect")));
    }
    let (mut cur, mut prev, mut prev2) = (Some(goal), None::<usize>, None::<usize>);
    let key = if f.branching_type.is_empty() { door.clone() } else { f.branching_type.to_ascii_lowercase() };
    let mut path = Vec::new();
    while let Some(c) = cur {
        path.push(c);
        if !records[c].heights.contains(&height) {
            records[c].heights.push(height);
            records[c].heights.sort();
        }
        records[c].used = true;
        if grid.rooms[c].is_placeholder() {
            records[c].impval = f.impenetrability;
            records[c].impen = impenetrable;
            grid.rooms[c].blocked = impenetrable;
            if c != start && c != goal {
                grid.rooms[c].height = height;
                grid.rooms[c].extra_pct = f.extra_door;
                let t = if f.extra_door_type.is_empty() { "metadata/terrain/dungeon/rooms.et".to_string() } else { f.extra_door_type.to_ascii_lowercase() };
                grid.rooms[c].extra_door = Some(t);
                if grid.rooms[c].door_type.as_deref().is_none_or(|d| d == "metadata/terrain/dungeon/rooms.et") {
                    grid.rooms[c].door_type = Some(door.clone());
                }
            }
        }
        if let Some(p) = prev {
            let either = grid.rooms[c].is_placeholder() || grid.rooms[p].is_placeholder();
            open_door(grid, c, p, &door, chir, either, rng)?;
            if c != start && c != goal {
                grid.rooms[c].accessible |= f.accessible;
            }
            if prev2.is_some() {
                mark_around(grid, records, p);
            }
        }
        prev2 = prev;
        prev = Some(c);
        cur = records[c].pred;
    }
    paths.push(Corridor { key, rooms: path, edge: e, height });
    for r in records.iter_mut() {
        r.pred = None;
    }
    Ok(())
}

/// `0x1420a4330`: A* from `start` to `goal`; open set by g + h, then room index.
#[allow(clippy::too_many_arguments)]
fn search(grid: &Grid, records: &mut [Record], start: usize, goal: usize, height: i32, fuzzy: i32, door: &str, chir: bool, impenetrable: bool) -> bool {
    records[start].g = 0;
    records[start].pred = None;
    records[goal].h = 0;
    let mut open: Vec<usize> = vec![start];
    let mut closed: Vec<bool> = vec![false; grid.rooms.len()];
    let key = |records: &[Record], i: usize| (records[i].g.wrapping_add(records[i].h), i);
    loop {
        let Some(k) = (0..open.len()).min_by_key(|&k| key(records, open[k])) else { return false };
        let cur = open.remove(k);
        if cur == goal {
            return true;
        }
        closed[cur] = true;
        for n in neighbours(grid, cur) {
            if !connectable(grid, cur, n, height, door, chir) {
                continue;
            }
            let ok = n == goal || ((!records[n].used || (!records[n].impen && !impenetrable)) && !closed[n] && records[cur].pred != Some(n));
            if !ok || !connectable(grid, n, cur, height, door, !chir) {
                continue;
            }
            let (c0, c1) = (centre(&grid.rooms[cur]), centre(&grid.rooms[n]));
            let d = isqrt(((c0.0 - c1.0).pow(2) + (c0.1 - c1.1).pow(2)) as u32);
            let g = (d >> 1) as i32 + records[cur].g;
            if let Some(j) = open.iter().position(|&o| o == n) {
                if records[n].g <= g {
                    continue;
                }
                open.remove(j);
            } else {
                let cg = centre(&grid.rooms[goal]);
                let dg = isqrt(((c1.0 - cg.0).pow(2) + (c1.1 - cg.1).pow(2)) as u32);
                records[n].h = ((dg >> 1) as i32).wrapping_mul(100 - fuzzy) + (((records[n].impval << 7) as u32) / 100) as i32 + fuzzy.wrapping_mul(records[n].cost as i32);
            }
            records[n].g = g;
            open.push(n);
            records[n].pred = Some(cur);
        }
    }
}

/// `0x1420a7ab0`: rooms along the four strips around `room` (above, below, left, right), each
/// once per strip.
fn neighbours(grid: &Grid, room: usize) -> Vec<usize> {
    let r = &grid.rooms[room];
    let (x, y, w, h) = (r.origin.0, r.origin.1, r.size.0 as i32, r.size.1 as i32);
    let mut out = Vec::new();
    let mut strip = |horizontal: bool, fixed: i32, from: i32, len: i32| {
        let mut u = 0;
        while u < len {
            let cell = if horizontal { (from + u, fixed) } else { (fixed, from + u) };
            match grid.state(cell) {
                Some(s) if s >= 2 => {
                    out.push(s as usize - 2);
                    let o = &grid.rooms[s as usize - 2];
                    u = if horizontal { o.origin.0 + o.size.0 as i32 - from } else { o.origin.1 + o.size.1 as i32 - from };
                }
                _ => u += 1,
            }
        }
    };
    if y != 0 {
        strip(true, y - 1, x, w);
    }
    if ((y + h) as u32) < grid.h {
        strip(true, y + h, x, w);
    }
    if x != 0 {
        strip(false, x - 1, y, h);
    }
    if ((x + w) as u32) < grid.w {
        strip(false, x + w, y, h);
    }
    out
}

/// Ring position of `b` facing ring position `u` of `a` (`0x141fa79e0`).
fn facing(a: &PlacedRoom, b: &PlacedRoom, u: u32) -> Option<u32> {
    let outside = ring_cell(a.origin, a.size, u);
    let inner = ring_inner(a.origin, a.size, u);
    (0..b.exits.len() as u32).find(|&v| ring_cell(b.origin, b.size, v) == inner && ring_inner(b.origin, b.size, v) == outside)
}

/// `0x141fa7d40`: `a`'s ring positions on the border shared with `b` where both sides take a
/// door of `door` (`0x141fa7bf0`: a placeholder's empty position, or that door already; for a
/// chiral door type with `a` holding `!chir` and `b` holding `chir`). A chiral door of another
/// type on either side of a checked position empties the list.
fn shared_doors(grid: &Grid, a: &PlacedRoom, b: &PlacedRoom, door: &str, chir: bool) -> Vec<u32> {
    let (ax, ay, aw, ah) = (a.origin.0, a.origin.1, a.size.0 as i32, a.size.1 as i32);
    let (bx, by, bw, bh) = (b.origin.0, b.origin.1, b.size.0 as i32, b.size.1 as i32);
    let positions: Vec<u32> = if by + bh == ay {
        (ax.max(bx)..(ax + aw).min(bx + bw)).map(|x| (x - ax) as u32).collect()
    } else if ay + ah == by {
        (ax.max(bx)..(ax + aw).min(bx + bw)).map(|x| (2 * aw + ah - 1 - (x - ax)) as u32).collect()
    } else if bx + bw == ax {
        (ay.max(by)..(ay + ah).min(by + bh)).map(|y| (2 * (aw + ah) - 1 - (y - ay)) as u32).collect()
    } else {
        (ay.max(by)..(ay + ah).min(by + bh)).map(|y| (aw + (y - ay)) as u32).collect()
    };
    let conflict = std::cell::Cell::new(false);
    let ok = |r: &PlacedRoom, i: u32, want: bool| {
        let e = &r.exits[i as usize];
        match e.door.as_deref() {
            Some(d) if d != door => {
                if grid.is_chiral(d) {
                    conflict.set(true);
                }
                false
            }
            None if r.is_placeholder() => true,
            None => false,
            Some(_) => !grid.is_chiral(door) || e.chiral == want,
        }
    };
    let mut out = Vec::new();
    for u in positions {
        if ok(a, u, !chir) && facing(a, b, u).is_some_and(|v| ok(b, v, chir)) {
            out.push(u);
        }
        if conflict.get() {
            return Vec::new();
        }
    }
    out
}

/// `0x1420a4260`: `a` can open towards `b` at the corridor height.
fn connectable(grid: &Grid, a: usize, b: usize, height: i32, door: &str, chir: bool) -> bool {
    let (ra, rb) = (&grid.rooms[a], &grid.rooms[b]);
    shared_doors(grid, ra, rb, door, chir).into_iter().any(|u| {
        let e = &ra.exits[u as usize];
        (e.door.is_none() && ra.is_placeholder()) || (e.door.as_deref() == Some(door) && e.height + ra.height == height)
    })
}

/// `0x141fa8140` / `0x141fa7fd0`: a door between neighbouring rooms `a` and `b` at a shuffled
/// shared position (a discarded `below(n)`, then the shuffle); for a chiral type `a` takes
/// chirality `!chir` and `b` takes `chir` (`0x141f96730`).
fn open_door(grid: &mut Grid, a: usize, b: usize, door: &str, chir: bool, either: bool, rng: &mut TerrainRng) -> Result<(), GenError> {
    let mut list = shared_doors(grid, &grid.rooms[a], &grid.rooms[b], door, chir);
    if list.is_empty() {
        return Err(unsupported("dungeon: could not open doors"));
    }
    rng.below(list.len() as u32);
    let n = list.len();
    for i in 0..n {
        let k = rng.uniform((n - i) as u32) as usize;
        list.swap(i, i + k);
    }
    let u = list[0];
    let v = facing(&grid.rooms[a], &grid.rooms[b], u).expect("shared position");
    let c = grid.is_chiral(door);
    grid.rooms[a].exits[u as usize].door = Some(door.to_string());
    grid.rooms[a].exits[u as usize].chiral = c && !chir;
    grid.rooms[b].exits[v as usize].door = Some(door.to_string());
    grid.rooms[b].exits[v as usize].chiral = c && chir;
    if either {
        let (ha, hb) = (grid.rooms[a].height, grid.rooms[b].height);
        if grid.rooms[a].is_placeholder() {
            grid.rooms[a].exits[u as usize].height = (hb - ha).max(0);
        }
        if grid.rooms[b].is_placeholder() {
            grid.rooms[b].exits[v as usize].height = (ha - hb).max(0);
        }
    }
    Ok(())
}

/// `0x1420a6610`: unused placeholders leave (each swapped with the last room), then the grid is
/// stamped again (`0x1420a6bb0`).
fn compact(grid: &mut Grid, records: &mut Vec<Record>, paths: &mut [Corridor]) {
    if grid.rooms.is_empty() {
        return;
    }
    let mut perm: Vec<usize> = (0..grid.rooms.len()).collect();
    let (mut i, mut last) = (0usize, grid.rooms.len() - 1);
    while i < last {
        if !records[i].used {
            grid.rooms.swap(i, last);
            records.swap(i, last);
            perm.swap(i, last);
            last -= 1;
        } else {
            i += 1;
        }
    }
    if records[i].used {
        i += 1;
    }
    grid.rooms.truncate(i);
    records.truncate(i);
    let mut new_index = vec![usize::MAX; perm.len()];
    for (new, &old) in perm.iter().enumerate() {
        new_index[old] = new;
    }
    for c in paths.iter_mut() {
        for r in c.rooms.iter_mut() {
            *r = new_index[*r];
        }
    }
    for c in grid.cells.iter_mut() {
        if *c != 1 {
            *c = 0;
        }
    }
    for (k, r) in grid.rooms.iter().enumerate() {
        for dy in 0..r.size.1 as i32 {
            for dx in 0..r.size.0 as i32 {
                let (x, y) = (r.origin.0 + dx, r.origin.1 + dy);
                grid.cells[(y as u32 * grid.w + x as u32) as usize] = k as u32 + 2;
            }
        }
    }
}

/// `0x1420a6140`: every free unit cell (row by row) gets a placeholder of a size drawn from the
/// edge type's list (up to five draws until one fits; else the list's first size, else 1×1).
fn fill_placeholders(grid: &mut Grid, list: &[((u32, u32), u32)], door_type: Option<String>, rng: &mut TerrainRng) -> Result<(), GenError> {
    if list.is_empty() {
        return Ok(());
    }
    let total: u32 = list.iter().map(|s| s.1).sum();
    if std::env::var("POE_DBG_GRID").is_ok() {
        eprintln!("DBG fill {:?} list {:?}", door_type, list);
        for y in 0..grid.h {
            eprintln!("DBG grid {}", (0..grid.w).map(|x| match grid.cells[(y * grid.w + x) as usize] { 0 => ".".to_string(), 1 => "#".to_string(), v => format!("{}", (v - 2) % 36).chars().last().unwrap().to_string() }).collect::<String>());
        }
    }
    let fits = |grid: &Grid, (x, y): (i32, i32), (w, h): (u32, u32)| {
        x >= 0 && y >= 0 && x as u32 + w <= grid.w && y as u32 + h <= grid.h && (0..h as i32).all(|dy| (0..w as i32).all(|dx| grid.state((x + dx, y + dy)) == Some(0)))
    };
    for y in 0..grid.h as i32 {
        let mut x = 0;
        while x < grid.w as i32 {
            if grid.state((x, y)) != Some(0) {
                x += 1;
                continue;
            }
            let mut chosen = None;
            for _ in 0..5 {
                let mut r = rng.below(total);
                let mut k = list.len() - 1;
                for (j, s) in list.iter().enumerate() {
                    if r < s.1 {
                        k = j;
                        break;
                    }
                    r -= s.1;
                }
                if fits(grid, (x, y), list[k].0) {
                    chosen = Some(list[k].0);
                    break;
                }
            }
            let size = chosen.unwrap_or(list[0].0);
            let stamp = if fits(grid, (x, y), size) { size } else { (1, 1) };
            grid.stamp(PlacedRoom { path: String::new(), node: None, origin: (x, y), size: stamp, orientation: 0, exits: Vec::new(), extra_door: None, height: 0, accessible: false, blocked: chosen.is_none() && stamp == (1, 1) && size != (1, 1), extra_pct: 0, door_type: door_type.clone(), special: false })?;
            x += size.0 as i32;
        }
    }
    Ok(())
}

/// `0x1420aa710`: special rooms. The `.rs` order fills one list per `specialN` index, each
/// shuffled (a discarded `below(n)`, then a forward Fisher-Yates) and tried until one room
/// places; the last list (`special` / `specialchance N`) places every room whose chance roll
/// passes (`below(100)` only for chances 1..99).
fn special_rooms(files: &dyn Files, rooms: &[DungeonRoom], grid: &mut Grid, types: &mut TerrainTypes, rng: &mut TerrainRng) -> Result<(), GenError> {
    let mut lists: Vec<Vec<usize>> = Vec::new();
    let mut last: Vec<usize> = Vec::new();
    for (i, r) in rooms.iter().enumerate() {
        if !r.special {
            continue;
        }
        if r.list == 0xff {
            last.push(i);
        } else {
            if lists.len() <= r.list as usize {
                lists.resize(r.list as usize + 1, Vec::new());
            }
            lists[r.list as usize].push(i);
        }
    }
    for (li, list) in lists.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let mut l = list.clone();
        rng.below(l.len() as u32);
        for i in 0..l.len() {
            let k = rng.uniform((l.len() - i) as u32) as usize;
            l.swap(i, i + k);
        }
        let mut placed = false;
        for &r in &l {
            if place_special(files, rooms, &[r], grid, types, rng)?.is_some() {
                placed = true;
                break;
            }
        }
        if !placed {
            return Err(unsupported(format!("dungeon: failed to place a special{li} room")));
        }
    }
    for &r in &last {
        let c = rooms[r].chance as u32;
        if c == 0 || (c < 100 && rng.below(100) >= c) {
            continue;
        }
        if place_special(files, rooms, &[r], grid, types, rng)?.is_none() {
            return Err(unsupported(format!("dungeon: failed to place special room {}", rooms[r].path)));
        }
    }
    Ok(())
}

/// `0x1420a7da0`: a special room replaces a placed room. Candidates are reachable rooms with an
/// `.arm` that is neither special nor tagged, whose size and doors match the special room in
/// some allowed orientation (`0x1420a8810`, `0x1420a7c40`: the first such orientation, weight 1);
/// `below(count)` picks the room, `below(weight)` the orientation. The room keeps its doors and
/// becomes kind 8 (+0x14).
fn place_special(files: &dyn Files, rooms: &[DungeonRoom], entries: &[usize], grid: &mut Grid, types: &mut TerrainTypes, rng: &mut TerrainRng) -> Result<Option<usize>, GenError> {
    let mut exact: Vec<(usize, Vec<(usize, u8)>)> = Vec::new();
    // `0x1420a8980`: spots where the special room would cover whole rooms (by position).
    let mut split: Vec<((i32, i32), Vec<(usize, u8, (u32, u32))>)> = Vec::new();
    for &si in entries {
        let s = &rooms[si];
        if s.tag.to_ascii_lowercase().contains("injector") {
            return Err(unsupported(format!("dungeon: injector special room {}", s.path)));
        }
        let blank = s.exits.iter().all(|e| e.door.is_none());
        if blank {
            return Err(unsupported(format!("dungeon: blank special room {} (ground match not ported)", s.path)));
        }
        let variants: Vec<Option<((u32, u32), Vec<Exit>)>> = (0..8u8)
            .map(|o| {
                (s.mask == 0 || s.mask >> o & 1 != 0).then(|| {
                    let exits = grid.turned(s.units.0, s.units.1, &s.exits, o);
                    (if o & 1 == 1 { (s.units.1, s.units.0) } else { s.units }, exits)
                })
            })
            .collect();
        for i in 0..grid.rooms.len() {
            let p = &grid.rooms[i];
            if !p.accessible || p.is_placeholder() {
                continue;
            }
            let Some(pr) = rooms.iter().find(|r| r.path == p.path) else { continue };
            if pr.special || !pr.tag.is_empty() {
                continue;
            }
            for (o, v) in variants.iter().enumerate() {
                let Some((size, exits)) = v else { continue };
                if *size == p.size && special_fits(files, &p.exits, exits, types)? {
                    match exact.iter_mut().find(|e| e.0 == i) {
                        Some(e) => e.1.push((si, o as u8)),
                        None => exact.push((i, vec![(si, o as u8)])),
                    }
                    break;
                }
            }
            if !exact.is_empty() {
                continue;
            }
            // No exact spot yet: the special room at the room's corners.
            let p = &grid.rooms[i];
            for (o, v) in variants.iter().enumerate() {
                let Some((size, exits)) = v else { continue };
                let (sw, sh) = (size.0 as i32, size.1 as i32);
                let (px, py, pw, ph) = (p.origin.0, p.origin.1, p.size.0 as i32, p.size.1 as i32);
                let mut corners = vec![(px, py)];
                if sw >= 2 {
                    corners.push((px + pw - sw, py));
                    if sh >= 2 {
                        corners.push((px, py + ph - sh));
                        corners.push((px + pw - sw, py + ph - sh));
                    }
                } else if sh >= 2 {
                    corners.push((px, py + ph - sh));
                }
                for pos in corners {
                    if split_fits(grid, rooms, *size, exits, pos) {
                        match split.iter_mut().find(|e| e.0 == pos) {
                            Some(e) => e.1.push((si, o as u8, *size)),
                            None => split.push((pos, vec![(si, o as u8, *size)])),
                        }
                    }
                }
            }
        }
    }
    if exact.is_empty() {
        if split.is_empty() {
            return Ok(None);
        }
        // `0x1420a8448` / `0x1420a8461`: the spot, then the variant; the rooms under it leave
        // (highest index first) and the special room takes their place at the height of the
        // last one removed.
        let k = rng.below(split.len() as u32) as usize;
        let v = rng.below(split[k].1.len() as u32) as usize;
        let (pos, (si, o, size)) = (split[k].0, split[k].1[v]);
        let mut ids: Vec<u32> = Vec::new();
        for dy in 0..size.1 as i32 {
            for dx in 0..size.0 as i32 {
                let st = grid.state((pos.0 + dx, pos.1 + dy)).unwrap_or(0);
                if st >= 2 && !ids.contains(&st) {
                    ids.push(st);
                }
            }
        }
        ids.sort();
        let mut height = 0;
        for &id in ids.iter().rev() {
            height = grid.rooms[id as usize - 2].height;
            grid.rooms.remove(id as usize - 2);
        }
        for c in grid.cells.iter_mut() {
            if *c != 1 {
                *c = 0;
            }
        }
        let placed = std::mem::take(&mut grid.rooms);
        for r in placed {
            grid.stamp(r)?;
        }
        let r = &rooms[si];
        let exits = grid.turned(r.units.0, r.units.1, &r.exits, o);
        grid.stamp(PlacedRoom { path: r.path.clone(), node: None, origin: pos, size, orientation: o, exits, extra_door: None, height, accessible: true, blocked: false, extra_pct: 0, door_type: None, special: true })?;
        return Ok(Some(grid.rooms.len() - 1));
    }
    let k = rng.below(exact.len() as u32) as usize;
    let v = rng.below(exact[k].1.len() as u32) as usize;
    let (i, (si, o)) = (exact[k].0, exact[k].1[v]);
    let p = &mut grid.rooms[i];
    p.path = rooms[si].path.clone();
    p.orientation = o;
    p.special = true;
    Ok(Some(i))
}

/// `0x1420a7c40`: the placed room's doors against a special room's (same size checked by the
/// caller). Where the special door is optional (`.et` flag 0x10) the placed one may be closed;
/// elsewhere they must be equal. With optional special doors, the placed room needs at least
/// one optional door of its own.
fn special_fits(files: &dyn Files, placed: &[Exit], special: &[Exit], types: &mut TerrainTypes) -> Result<bool, GenError> {
    if placed.len() != special.len() {
        return Ok(false);
    }
    let optional = |d: &Option<String>, types: &mut TerrainTypes| -> Result<bool, GenError> {
        Ok(match d {
            Some(d) => types.edge(files, d)?.can_end,
            None => false,
        })
    };
    let (mut any_optional, mut own) = (false, 0);
    for (p, s) in placed.iter().zip(special) {
        if optional(&s.door, types)? {
            if p != s && p.door.is_some() {
                return Ok(false);
            }
            any_optional = true;
        } else if p != s {
            return Ok(false);
        }
        if optional(&p.door, types)? {
            own += 1;
        }
    }
    Ok(!(any_optional && own == 0))
}

/// `0x141d047b0`: an edge type whose sides differ (a height change or two grounds; a virtual
/// edge unless both halves are the same chiral type with equal flags).
fn chiral_type(types: &mut TerrainTypes, files: &dyn Files, t: &str) -> Result<bool, GenError> {
    let e = types.edge(files, t)?.clone();
    if e.height_change != 0 || e.grounds[0] != e.grounds[1] {
        return Ok(true);
    }
    if !e.virtual_edge {
        return Ok(false);
    }
    if e.splits[0].0 == e.splits[1].0 {
        if !chiral_type(types, files, &e.splits[0].0)? || e.splits[0].1 != e.splits[1].1 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `0x1420a8bc0`: the special room at `pos` lies on the grid over free cells and whole
/// non-special, untagged rooms of one height, and its doors agree with theirs along its sides
/// (`0x1420a8fc0`; a default-type special door may face a closed one, but not all of them).
fn split_fits(grid: &Grid, rooms: &[DungeonRoom], size: (u32, u32), exits: &[Exit], pos: (i32, i32)) -> bool {
    let (x0, y0) = pos;
    let (x1, y1) = (x0 + size.0 as i32, y0 + size.1 as i32);
    if x0 < 0 || y0 < 0 || x1 > grid.w as i32 || y1 > grid.h as i32 {
        return false;
    }
    const DEFAULT: &str = "metadata/terrain/dungeon/rooms.et";
    let defaults = exits.iter().filter(|e| e.door.as_deref() == Some(DEFAULT)).count();
    let (sw, sh, n) = (size.0 as i32, size.1 as i32, exits.len() as i32);
    let mut skipped = 0usize;
    let mut height: Option<i32> = None;
    let empty = [Exit { door: None, height: 0, chiral: false }, Exit { door: None, height: 0, chiral: false }, Exit { door: None, height: 0, chiral: false }, Exit { door: None, height: 0, chiral: false }];
    // `0x1420a8fc0` for a record at `(px, py)`.
    let mut check = |(px, py): (i32, i32), (pw, ph): (i32, i32), pe: &[Exit]| -> bool {
        let np = pe.len() as i32;
        let mut cmp = |s: &Exit, p: &Exit| -> bool {
            if s == p {
                return true;
            }
            skipped += 1;
            s.door.as_deref() == Some(DEFAULT) && p.door.is_none()
        };
        if px == x0 {
            for k in 0..ph {
                if !cmp(&exits[(n - (k + (py - y0)) - 1) as usize], &pe[(np - k - 1) as usize]) {
                    return false;
                }
            }
        }
        if px + pw == x1 {
            for k in 0..ph {
                if !cmp(&exits[(sw + (py - y0) + k) as usize], &pe[(pw + k) as usize]) {
                    return false;
                }
            }
        }
        if py == y0 {
            for k in 0..pw {
                if !cmp(&exits[((px - x0) + k) as usize], &pe[k as usize]) {
                    return false;
                }
            }
        }
        if py + ph == y1 {
            for k in 0..pw {
                if !cmp(&exits[(n - sh - (px - x0 + k) - 1) as usize], &pe[(np - ph - k - 1) as usize]) {
                    return false;
                }
            }
        }
        true
    };
    for y in y0..y1 {
        for x in x0..x1 {
            match grid.state((x, y)) {
                Some(0) => {
                    if !check((x, y), (1, 1), &empty) {
                        return false;
                    }
                }
                Some(st) if st >= 2 => {
                    let p = &grid.rooms[st as usize - 2];
                    let (pw, ph) = (p.size.0 as i32, p.size.1 as i32);
                    if p.origin.0 < x0 || p.origin.1 < y0 || p.origin.0 + pw > x1 || p.origin.1 + ph > y1 {
                        return false;
                    }
                    if p.origin != (x, y) {
                        continue;
                    }
                    if !p.is_placeholder() {
                        if let Some(r) = rooms.iter().find(|r| r.path == p.path) {
                            if r.special || !r.tag.is_empty() {
                                return false;
                            }
                        }
                    }
                    if height.is_some_and(|h| h != p.height) {
                        return false;
                    }
                    height = Some(p.height);
                    if !check((x, y), (pw, ph), &p.exits) {
                        return false;
                    }
                }
                _ => return false,
            }
        }
    }
    defaults == 0 || skipped != defaults
}
