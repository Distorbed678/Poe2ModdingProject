//! Room nodes: the room set (`.rs`), edge types (`.et`), ground types (`.gt`) and the
//! placement of a graph node's room (`0x1420c87a0`). See `Documentation/re/terrain.md`,
//! "Room placement".
//!
//! - A node's key (`0x1420d2e20`): each graph edge goes to the side it leaves the node by
//!   (`0x1420d1220`: towards its first internal point or other node; `|dx| > |dy|` → right/left,
//!   else down/up; sides in key order 0 up, 1 right, 2 down, 3 left); the key holds the edges'
//!   `.et` per side, nothing else.
//! - Candidates (`0x1420c76d0`): the room set's entries for the node's room type (one per room
//!   and orientation, `.rs` order) whose turned edges equal the key's. A fixed-orientation node
//!   takes every match in its own orientation; an `(any)` node keeps each match's orientation
//!   when the edges' ground types and heights agree with the room's corners (`0x1420b87b0`).
//! - One is drawn by weight; it is centred on the node's tile, clamped to the map, and must fit
//!   and not overlap an earlier room, otherwise every entry of that room and orientation is
//!   dropped and another drawn.

use std::collections::HashMap;

use poe_data_tools::file_parsers::{FileParser, arm::types::Slot, rs::RSParser};

use crate::{
    generate::{Files, GenError, Room, load_room, slot_key},
    graph::Graph,
    rng::TerrainRng,
    text::{decode_utf16, tokens},
    tileset::{TileKey, orientation_mask},
};

fn norm(p: &str) -> String {
    p.trim_matches('"').to_ascii_lowercase().replace('\\', "/")
}

/// `.et` data the room matching reads (`0x1429195a0` → `0x141d03d80`/`0x141d03f20`).
#[derive(Debug, Clone, Default)]
pub struct EdgeType {
    pub name: String,
    /// Left and right ground type (`None` = wildcard), data +0x20/+0x28.
    pub grounds: [Option<String>; 2],
    /// Height change (first number, +0x4c).
    pub height_change: u8,
    /// Edge size (second number, 1 or 3, +0x4b).
    pub size: u8,
    /// The name starts with "Wildcard" (flag 0x80): such edges take no side.
    pub wildcard: bool,
    pub virtual_edge: bool,
}

impl EdgeType {
    pub fn parse(text: &str) -> Self {
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
        let name = lines
            .next()
            .and_then(|l| tokens(l).into_iter().next())
            .unwrap_or_default();
        let ground = |l: Option<&str>| {
            l.map(|l| l.trim_matches('"'))
                .filter(|l| !l.eq_ignore_ascii_case("wildcard") && !l.is_empty())
                .map(norm)
        };
        let g0 = ground(lines.next());
        let g1 = ground(lines.next());
        let nums: Vec<u32> = lines
            .next()
            .map(|l| l.split_whitespace().filter_map(|v| v.parse().ok()).collect())
            .unwrap_or_default();
        let virtual_edge = lines.next().is_some_and(|l| l == "virtual");
        Self {
            wildcard: name.starts_with("Wildcard"),
            name,
            grounds: [g0, g1],
            height_change: nums.first().copied().unwrap_or(0) as u8,
            size: nums.get(1).copied().unwrap_or(1) as u8,
            virtual_edge,
        }
    }
}

/// `.gt` flags (`0x142918970`): the numbers line's five bools as bits 0..4 (default
/// 1 1 1 1 0); bit 7 when bools 3..5 are all 0 and the name contains Abyss/Water/Pool/Void.
pub fn ground_flags(text: &str) -> u8 {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let name = lines.next().unwrap_or("").to_string();
    let mut b = [1u8, 1, 1, 1, 0];
    if let Some(l) = lines.next() {
        for (i, v) in l.split_whitespace().take(5).enumerate() {
            b[i] = (v.parse::<u32>().unwrap_or(0) != 0) as u8;
        }
    }
    let mut flags = b.iter().enumerate().fold(0u8, |f, (i, &v)| f | (v << i));
    if b[2] == 0
        && b[3] == 0
        && b[4] == 0
        && ["Abyss", "Water", "Pool", "Void"].iter().any(|w| name.contains(w))
    {
        flags |= 0x80;
    }
    flags
}

/// Edge and ground types, loaded on demand.
#[derive(Default)]
pub struct TerrainTypes {
    edges: HashMap<String, EdgeType>,
    grounds: HashMap<String, u8>,
}

impl TerrainTypes {
    pub fn edge(&mut self, files: &dyn Files, path: &str) -> Result<&EdgeType, GenError> {
        let key = norm(path);
        if !self.edges.contains_key(&key) {
            let et = EdgeType::parse(&decode_utf16(&files.read(&key)?));
            self.edges.insert(key.clone(), et);
        }
        Ok(&self.edges[&key])
    }

    pub fn ground(&mut self, files: &dyn Files, path: &str) -> Result<u8, GenError> {
        let key = norm(path);
        if let Some(&f) = self.grounds.get(&key) {
            return Ok(f);
        }
        let f = ground_flags(&decode_utf16(&files.read(&key)?));
        self.grounds.insert(key, f);
        Ok(f)
    }
}

/// One room of the room set.
pub struct RoomDef {
    pub path: String,
    pub room: Room,
    /// The room's type name (the `.arm` tag), matched against graph nodes' room types.
    pub tag: String,
    pub weight: u32,
    pub orientations: u8,
    /// The root slot's key.
    pub key: TileKey,
}

pub struct RoomSet {
    pub rooms: Vec<RoomDef>,
}

impl RoomSet {
    pub fn load(files: &dyn Files, path: &str) -> Result<Self, GenError> {
        let bytes = files.read(path)?;
        let rs = RSParser
            .parse(&bytes)
            .map_err(|e| GenError::Data(path.into(), format!("{e:?}")))?;
        let mut rooms = Vec::new();
        for r in &rs.rooms {
            let room = load_room(files, &r.arm_file)?;
            let Slot::K(k) = &room.arm.root_slot else {
                return Err(GenError::Unsupported(format!("room {} has no root key", r.arm_file)));
            };
            rooms.push(RoomDef {
                path: r.arm_file.clone(),
                tag: room.arm.tag.clone(),
                key: slot_key(k),
                weight: r.weight.unwrap_or(1),
                orientations: orientation_mask(&r.rotations),
                room,
            });
        }
        Ok(Self { rooms })
    }
}

/// `.tgr` orientation names: `I`..`FR270` → 0..7, `(any)` 8, `(best)` 9.
pub fn node_orientation(s: &str) -> u8 {
    crate::tileset::ORIENTATION_NAMES
        .iter()
        .position(|n| n.eq_ignore_ascii_case(s))
        .map_or(if s.eq_ignore_ascii_case("(best)") { 9 } else { 8 }, |o| o as u8)
}

/// A graph laid out so far: node and internal-point positions after the jitter.
pub struct PlacedGraph<'a> {
    pub graph: &'a Graph,
    pub nodes: Vec<(i32, i32)>,
    /// Internal points per edge.
    pub points: Vec<Vec<(i32, i32)>>,
    /// Edges removed by their existence roll.
    pub removed: Vec<bool>,
}

impl PlacedGraph<'_> {
    /// The node's edge slots (file order, removed edges empty).
    fn slots(&self, node: usize) -> [Option<usize>; 4] {
        let mut s = [None; 4];
        for (i, &e) in self.graph.nodes[node].edges.iter().take(4).enumerate() {
            let e = e as usize;
            if e < self.removed.len() && !self.removed[e] {
                s[i] = Some(e);
            }
        }
        s
    }

    /// The point an edge leaves `node` towards.
    fn towards(&self, node: usize, e: usize) -> (i32, i32) {
        let edge = &self.graph.edges[e];
        let pts = &self.points[e];
        if edge.from as usize == node {
            pts.first().copied().unwrap_or(self.nodes[edge.to as usize])
        } else {
            pts.last().copied().unwrap_or(self.nodes[edge.from as usize])
        }
    }
}

/// `0x142f10760`: angle (0x8000 = 180°) by `127·cos + 127`.
const ANGLE: [u16; 255] = [
    32768, 31458, 30914, 30496, 30143, 29831, 29549, 29289, 29046, 28818, 28601, 28395,
    28197, 28007, 27824, 27647, 27476, 27309, 27147, 26989, 26835, 26684, 26536, 26392,
    26250, 26111, 25974, 25840, 25708, 25578, 25450, 25323, 25199, 25076, 24954, 24834,
    24716, 24599, 24483, 24369, 24255, 24143, 24032, 23922, 23813, 23705, 23598, 23492,
    23386, 23282, 23178, 23075, 22973, 22872, 22771, 22671, 22572, 22473, 22375, 22277,
    22180, 22084, 21988, 21893, 21798, 21704, 21610, 21516, 21423, 21331, 21239, 21147,
    21056, 20965, 20874, 20784, 20694, 20605, 20516, 20427, 20338, 20250, 20162, 20074,
    19987, 19900, 19813, 19726, 19640, 19553, 19467, 19382, 19296, 19211, 19126, 19041,
    18956, 18871, 18787, 18703, 18619, 18535, 18451, 18367, 18283, 18200, 18117, 18033,
    17950, 17867, 17784, 17702, 17619, 17536, 17454, 17371, 17289, 17206, 17124, 17041,
    16959, 16877, 16795, 16713, 16630, 16548, 16466, 16384, 16302, 16220, 16138, 16055,
    15973, 15891, 15809, 15727, 15644, 15562, 15479, 15397, 15314, 15232, 15149, 15066,
    14984, 14901, 14818, 14735, 14651, 14568, 14485, 14401, 14317, 14233, 14149, 14065,
    13981, 13897, 13812, 13727, 13642, 13557, 13472, 13386, 13301, 13215, 13128, 13042,
    12955, 12868, 12781, 12694, 12606, 12518, 12430, 12341, 12252, 12163, 12074, 11984,
    11894, 11803, 11712, 11621, 11529, 11437, 11345, 11252, 11158, 11064, 10970, 10875,
    10780, 10684, 10588, 10491, 10393, 10295, 10196, 10097, 9997, 9896, 9795, 9693,
    9590, 9486, 9382, 9276, 9170, 9063, 8955, 8846, 8736, 8625, 8513, 8399,
    8285, 8169, 8052, 7934, 7814, 7692, 7569, 7445, 7318, 7190, 7060, 6928,
    6794, 6657, 6518, 6376, 6232, 6084, 5933, 5779, 5621, 5459, 5292, 5121,
    4944, 4761, 4571, 4373, 4167, 3950, 3722, 3479, 3219, 2937, 2625, 2272,
    1854, 1310, 0,
];

/// `0x1421c4e50`: the angle between two vectors.
fn angle(a: (i32, i32), b: (i32, i32)) -> u16 {
    if a == (0, 0) || b == (0, 0) {
        return 0;
    }
    let la = a.0.wrapping_mul(a.0).wrapping_add(a.1.wrapping_mul(a.1));
    let lb = b.0.wrapping_mul(b.0).wrapping_add(b.1.wrapping_mul(b.1));
    let mut d = crate::voronoi::isqrt(lb.wrapping_mul(la) as u32) as i32;
    if d == 0 {
        d = 1;
    }
    let dy = b.1 - a.1;
    let dx = b.0 - a.0;
    let dot2 = lb
        .wrapping_sub(dy.wrapping_mul(dy))
        .wrapping_sub(dx.wrapping_mul(dx))
        .wrapping_add(la);
    let i = (((dot2.wrapping_mul(0x7f) / d) / 2) as i8 as i32) + 0x7f;
    ANGLE[i.clamp(0, 0xfe) as usize]
}

/// Side direction vectors (`0x142f0c608`): up, right, down, left.
const SIDE_DIR: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// Side per slot (4 = none) and the key edges (`0x1420d1220`/`0x1420d2e20`).
pub fn node_key(
    g: &PlacedGraph,
    node: usize,
    types: &mut TerrainTypes,
    files: &dyn Files,
) -> Result<([usize; 4], TileKey), GenError> {
    let slots = g.slots(node);
    let mut per_side: [Vec<usize>; 4] = Default::default();
    let (nx, ny) = g.nodes[node];
    for (i, s) in slots.iter().enumerate() {
        let Some(e) = *s else { continue };
        if types.edge(files, &g.graph.edges[e].edge_type)?.wildcard {
            continue;
        }
        let (px, py) = g.towards(node, e);
        let (dx, dy) = (px - nx, py - ny);
        let side = if dy.abs() < dx.abs() {
            if dx > 0 { 1 } else { 3 }
        } else if dy > 0 {
            2
        } else {
            0
        };
        per_side[side].push(i);
    }
    // `0x1420d15b0`/`0x1420d16c0`: while a side holds several edges, move the edge whose
    // direction is closest to a neighbouring side that is empty (else holds one edge).
    loop {
        let mut moves: Vec<(u16, usize, usize, usize)> = Vec::new();
        for want in [0usize, 1] {
            for s in 0..4 {
                if per_side[s].len() < 2 {
                    continue;
                }
                for &slot in &per_side[s] {
                    for n in [(s + 3) & 3, (s + 1) & 3] {
                        if per_side[n].len() == want {
                            let (px, py) = g.towards(node, slots[slot].unwrap());
                            let d = SIDE_DIR[n];
                            let cost = angle((d.0 * 10, d.1 * 10), (px - nx, py - ny));
                            moves.push((cost, s, n, slot));
                        }
                    }
                }
            }
            if !moves.is_empty() {
                break;
            }
        }
        let Some(&best) = moves.iter().reduce(|a, b| if b.0 < a.0 { b } else { a }) else {
            break;
        };
        let (_, from, to, slot) = best;
        per_side[to].push(slot);
        per_side[from].retain(|&x| x != slot);
    }
    if per_side.iter().any(|l| l.len() > 1) {
        return Err(GenError::Unsupported(format!(
            "node {node}: several edges on one side {per_side:?}"
        )));
    }
    let mut sides = [4usize; 4];
    let mut key = TileKey::default();
    for (side, l) in per_side.iter().enumerate() {
        if let Some(&slot) = l.first() {
            sides[slot] = side;
            key.edges[side] = Some(norm(&g.graph.edges[slots[slot].unwrap()].edge_type));
        }
    }
    Ok((sides, key))
}

/// `0x1420b87b0`: whether a turned room key agrees with the node's edges (ground types on each
/// side of the edge by its direction, heights).
#[allow(clippy::too_many_arguments)]
fn fits_edges(
    key: &TileKey,
    sides: &[usize; 4],
    g: &PlacedGraph,
    node: usize,
    strict_height: bool,
    types: &mut TerrainTypes,
    files: &dyn Files,
) -> Result<bool, GenError> {
    let slots = g.slots(node);
    for i in 0..4 {
        if sides[i] < 4 {
            let et = slots[i].map(|e| norm(&g.graph.edges[e].edge_type));
            if key.edges[sides[i]] != et {
                return Ok(false);
            }
        }
    }
    let height = |n: usize| g.graph.nodes[n].values.get(1).copied().unwrap_or(0);
    let (mut dir, mut climb) = ([0usize; 4], [0i32; 4]);
    for i in 0..4 {
        let Some(e) = slots[i] else { continue };
        let edge = &g.graph.edges[e];
        if types.edge(files, &edge.edge_type)?.wildcard || sides[i] >= 4 {
            continue;
        }
        let s = sides[i];
        let chirality = edge.values.get(1).copied().unwrap_or(0) != 0;
        dir[s] = ((edge.from as usize == node) ^ chirality) as usize;
        let other = if edge.from as usize == node { edge.to } else { edge.from } as usize;
        climb[s] = (height(other) - height(node)).max(0);
    }
    let h = key.heights.map(|v| v as i8 as i32);
    for s in 0..4 {
        let Some(path) = &key.edges[s] else { continue };
        let et = types.edge(files, path)?.clone();
        if et.wildcard {
            continue;
        }
        let a = (s + 4 + 1 - dir[s]) & 3;
        let b = (s + dir[s]) & 3;
        let lo = h[a].min(h[b]);
        let mut wild = |gt: &Option<String>| -> Result<bool, GenError> {
            Ok(match gt {
                None => true,
                Some(p) => types.ground(files, p)? & 8 != 0,
            })
        };
        let ok_b = key.grounds[b] == et.grounds[0] || (et.grounds[0].is_none() && wild(&key.grounds[b])?);
        let ok_a = key.grounds[a] == et.grounds[1] || (et.grounds[1].is_none() && wild(&key.grounds[a])?);
        let mismatch = !(ok_b && ok_a);
        let mut flag1 = |gt: &Option<String>| -> Result<bool, GenError> {
            Ok(match gt {
                None => true,
                Some(p) => types.ground(files, p)? & 1 != 0,
            })
        };
        let both = flag1(&key.grounds[b])? && flag1(&key.grounds[a])?;
        let step = !(h[b] == lo && h[a] - lo == et.height_change as i32);
        let climb_bad = et.height_change == 0 && h[a].max(h[b]) != climb[s];
        if mismatch || (both && step) || (strict_height && climb_bad) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoomCandidate {
    pub room: usize,
    pub orientation: u8,
    pub weight: u32,
    /// Size with the orientation.
    pub size: (u8, u8),
}

/// `0x1420c51c0`: when no room has the node's edges, move one edge to a free side. With two
/// edges the first moves next to it (to the following side if free, else the preceding one);
/// with three, the edge after the free side moves into it, and if no room matches that, the
/// edge before it instead. Returns whether a room matches afterwards.
fn adjust_sides(
    g: &PlacedGraph,
    node: usize,
    sides: &mut [usize; 4],
    key: &mut TileKey,
    any_match: &dyn Fn(&TileKey) -> bool,
    types: &mut TerrainTypes,
    files: &dyn Files,
) -> Result<bool, GenError> {
    let slots = g.slots(node);
    let mut real = Vec::new();
    for (i, s) in slots.iter().enumerate() {
        if let Some(e) = s {
            if !types.edge(files, &g.graph.edges[*e].edge_type)?.wildcard {
                real.push(i);
            }
        }
    }
    let mv = |key: &mut TileKey, from: usize, to: usize| {
        key.edges[to] = key.edges[from].take();
    };
    match real.len() {
        2 => {
            let slot = real[0];
            let s = sides[slot];
            let to = if key.edges[(s + 1) & 3].is_none() { (s + 1) & 3 } else { (s + 3) & 3 };
            mv(key, s, to);
            sides[slot] = to;
            Ok(any_match(key))
        }
        3 => {
            let mut at = [4usize; 4];
            for (slot, &side) in sides.iter().enumerate() {
                if side < 4 {
                    at[side] = slot;
                }
            }
            let empty = (0..4).find(|&i| at[i] == 4).unwrap_or(0);
            // The edge after the empty side moves back into it.
            let a = at[(empty + 1) & 3];
            let sa = sides[a];
            let to = (sa + 3) & 3;
            mv(key, sa, to);
            at[to] = a;
            at[sa] = 4;
            sides[a] = to;
            if any_match(key) {
                return Ok(true);
            }
            // Undo, then move the edge before the empty side forward into it.
            let b = at[empty];
            let sb = sides[b];
            let to = (sb + 1) & 3;
            mv(key, sb, to);
            at[to] = b;
            sides[b] = to;
            at[sb] = 4;
            let c = at[(empty + 3) & 3];
            let sc = sides[c];
            let to = (sc + 1) & 3;
            mv(key, sc, to);
            sides[c] = to;
            Ok(any_match(key))
        }
        _ => Ok(false),
    }
}

/// `0x1420c76d0` for a node with a room type.
pub fn room_candidates(
    set: &RoomSet,
    g: &PlacedGraph,
    node: usize,
    types: &mut TerrainTypes,
    files: &dyn Files,
) -> Result<Vec<RoomCandidate>, GenError> {
    let (mut sides, mut key) = node_key(g, node, types, files)?;
    let n = &g.graph.nodes[node];
    let node_o = node_orientation(&n.orientation);
    let any_match = |key: &TileKey| {
        set.rooms.iter().any(|r| {
            r.tag.eq_ignore_ascii_case(&n.room_type)
                && (0..8u8).any(|o| r.orientations & (1 << o) != 0 && r.key.oriented(o).edges == key.edges)
        })
    };
    if !any_match(&key) && !adjust_sides(g, node, &mut sides, &mut key, &any_match, types, files)? {
        return Ok(Vec::new());
    }
    let mut matches = Vec::new();
    for (ri, r) in set.rooms.iter().enumerate() {
        if !r.tag.eq_ignore_ascii_case(&n.room_type) {
            continue;
        }
        for o in 0..8u8 {
            if r.orientations & (1 << o) == 0 {
                continue;
            }
            if r.key.oriented(o).edges == key.edges {
                matches.push((ri, o));
            }
        }
    }
    let mut out = Vec::new();
    if node_o < 8 {
        for (ri, _) in matches {
            let k = set.rooms[ri].key.oriented(node_o);
            out.push(RoomCandidate {
                room: ri,
                orientation: node_o,
                weight: set.rooms[ri].weight,
                size: k.size,
            });
        }
        return Ok(out);
    }
    for strict in [true, false] {
        for &(ri, o) in &matches {
            let k = set.rooms[ri].key.oriented(o);
            if fits_edges(&k, &sides, g, node, strict, types, files)? {
                out.push(RoomCandidate {
                    room: ri,
                    orientation: o,
                    weight: set.rooms[ri].weight,
                    size: k.size,
                });
            }
        }
        if !out.is_empty() {
            break;
        }
    }
    Ok(out)
}

/// A placed room node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacedRoom {
    pub node: usize,
    pub room: usize,
    pub orientation: u8,
    pub pos: (i32, i32),
    pub size: (u8, u8),
}

/// `0x141ef3290`: a node's alignment: an `align<N>` description, else the largest size of its
/// edges' types.
fn node_alignment(
    g: &PlacedGraph,
    node: usize,
    types: &mut TerrainTypes,
    files: &dyn Files,
) -> Result<i32, GenError> {
    for t in &g.graph.nodes[node].tags {
        if let Some(n) = t.strip_prefix("align") {
            if let Ok(n) = n.parse::<i32>() {
                if n != 0 {
                    return Ok(n);
                }
            }
        }
    }
    let mut a = 0;
    for e in g.slots(node).into_iter().flatten() {
        a = a.max(types.edge(files, &g.graph.edges[e].edge_type)?.size as i32);
    }
    Ok(a)
}

/// `0x1420c87a0`: draws and places the room of `node` (graph `tiles` in tiles).
pub fn place_node(
    set: &RoomSet,
    g: &PlacedGraph,
    node: usize,
    tiles: (i32, i32),
    placed: &[PlacedRoom],
    types: &mut TerrainTypes,
    files: &dyn Files,
    rng: &mut TerrainRng,
) -> Result<PlacedRoom, GenError> {
    let mut list = room_candidates(set, g, node, types, files)?;
    let align = node_alignment(g, node, types, files)?;
    let mut max_size = 0;
    for e in g.slots(node).into_iter().flatten() {
        max_size = max_size.max(types.edge(files, &g.graph.edges[e].edge_type)?.size as i32);
    }
    let (x, y) = g.nodes[node];
    let cx = (x / 23).min(tiles.0 - 1);
    let cy = (y / 23).min(tiles.1 - 1);
    loop {
        let total: u32 = list.iter().map(|c| c.weight).sum();
        let mut r = rng.below(total);
        if list.is_empty() {
            return Err(GenError::Unsupported(format!(
                "node {node} ({:?}): no room fits",
                g.graph.nodes[node].room_type
            )));
        }
        let mut i = 0;
        while i + 1 < list.len() && r >= list[i].weight {
            r -= list[i].weight;
            i += 1;
        }
        let c = list[i].clone();
        let (w, h) = (c.size.0 as i32, c.size.1 as i32);
        // Centred, then snapped to the node's alignment (`0x141fe7400`; region orientation 0:
        // a remainder above half rounds up), then clamped to the map.
        let snap = |v: i32| {
            if align <= 1 {
                return v;
            }
            let r = v.rem_euclid(align);
            v - r + if r > align / 2 { align } else { 0 }
        };
        let px = snap((cx - w / 2).max(0)).max(0).min(tiles.0 - w);
        let py = snap((cy - h / 2).max(0)).max(0).min(tiles.1 - h);
        let inside = px >= 0 && py >= 0 && px + w <= tiles.0 && py + h <= tiles.1;
        let overlaps = placed.iter().any(|p| {
            px < p.pos.0 + p.size.0 as i32
                && p.pos.0 < px + w
                && py < p.pos.1 + p.size.1 as i32
                && p.pos.1 < py + h
        });
        if inside && !overlaps {
            // `0x1420c59e0`: keep the room off the map border by the largest edge size
            // (twice for virtual edges) on each side its key has an edge.
            let key = set.rooms[c.room].key.oriented(c.orientation);
            let mut margin_of = |side: usize| -> Result<Option<i32>, GenError> {
                Ok(match &key.edges[side] {
                    None => None,
                    Some(p) => {
                        let v = types.edge(files, p)?.virtual_edge;
                        Some((v as i32 + 1) * max_size)
                    }
                })
            };
            let (mut px, mut py) = (px, py);
            if let Some(m) = margin_of(0)? {
                py = py.max(m).min(tiles.1 - h);
            }
            if let Some(m) = margin_of(3)? {
                px = px.max(m).min(tiles.0 - w);
            }
            if let Some(m) = margin_of(2)? {
                py = py.min(tiles.1 - m - h).max(0);
            }
            if let Some(m) = margin_of(1)? {
                px = px.min(tiles.0 - m - w).max(0);
            }
            return Ok(PlacedRoom {
                node,
                room: c.room,
                orientation: c.orientation,
                pos: (px, py),
                size: c.size,
            });
        }
        list.retain(|o| !(o.room == c.room && o.orientation == c.orientation));
    }
}

/// A master file's (`.tsi`) entries: key → value (quotes removed; `//` lines skipped).
pub fn tsi_entries(bytes: &[u8]) -> HashMap<String, String> {
    decode_utf16(bytes)
        .lines()
        .map(str::trim)
        .filter(|l| !l.starts_with("//"))
        .filter_map(|l| {
            let t = tokens(l);
            (t.len() >= 2).then(|| (t[0].clone(), t[t.len() - 1].clone()))
        })
        .collect()
}
