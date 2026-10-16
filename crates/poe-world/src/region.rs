//! Region orientation: a topology may be laid out turned or mirrored (`0x141ff89e0` picks it,
//! `0x141d14990` turns the graph). The table at `0x142f020c8` (three bytes per orientation)
//! drives the point transform, the centring of even-sized placements (`0x1420c87a0`,
//! `0x1420cbe60`) and the alignment snap (`0x141fe7400`).

use crate::graph::Graph;
use crate::tileset::ORIENTATION_NAMES;

/// `0x142f020c8`: per orientation (0..8; 8 = none) the bytes `(a, b, c)`.
const TABLE: [[u8; 3]; 9] = [
    [0, 1, 1],
    [1, 0, 1],
    [0, 0, 0],
    [1, 1, 0],
    [0, 0, 1],
    [1, 1, 1],
    [0, 1, 0],
    [1, 0, 0],
    [0, 1, 1],
];

/// `0x141d00f00`: orientation `o` followed by `r`.
pub fn compose(o: u8, r: u8) -> u8 {
    const T: [[u8; 8]; 8] = [
        [0, 1, 2, 3, 4, 5, 6, 7],
        [1, 2, 3, 0, 7, 4, 5, 6],
        [2, 3, 0, 1, 6, 7, 4, 5],
        [3, 0, 1, 2, 5, 6, 7, 4],
        [4, 5, 6, 7, 0, 1, 2, 3],
        [5, 6, 7, 4, 3, 0, 1, 2],
        [6, 7, 4, 5, 2, 3, 0, 1],
        [7, 4, 5, 6, 1, 2, 3, 0],
    ];
    if o >= 8 || r >= 8 {
        return 8;
    }
    T[o as usize][r as usize]
}

/// A point `(x, y)` of a `w`×`h` area turned by `o` (`0x141d14b30`).
pub fn turn_point(o: u8, w: i32, h: i32, x: i32, y: i32) -> (i32, i32) {
    let [a, b, c] = TABLE[o.min(8) as usize];
    let v = [w - x - 1, x, h - y - 1, y];
    let nx = v[(b + a * 2) as usize];
    let ny = v[(if a == 0 { 2 } else { 0 }) + c as usize];
    (nx, ny)
}

/// Which axes round an even size's centre up (`[1, 0, 1, 0]` indexed by the table).
fn flags(o: u8) -> (bool, bool) {
    let [a, b, c] = TABLE[o.min(8) as usize];
    let f = |i: u8| i % 2 == 0;
    (f(b + a * 2), f((if a == 0 { 2 } else { 0 }) + c))
}

/// Corner of a `size` placement centred on tile `c` (x axis when `x`).
pub fn centred(o: u8, c: i32, size: i32, x: bool) -> i32 {
    let (fx, fy) = flags(o);
    let f = if x { fx } else { fy } as i32;
    c - ((size >> 1) - ((size - 1) & 1) * f)
}

/// `0x141fe7400`: `(x, y)` snapped to multiples of `a`; a remainder above half rounds up, one of
/// exactly half by the orientation.
pub fn snap(o: u8, p: (i32, i32), a: i32) -> (i32, i32) {
    if a <= 1 {
        return p;
    }
    let (fx, fy) = flags(o);
    let half = a >> 1;
    let (rx, ry) = ((p.0 as u32 % a as u32) as i32, (p.1 as u32 % a as u32) as i32);
    let (up_y, odd_flag) = if half < ry {
        (true, a & 1)
    } else if !fy || a & 1 != 0 || ry != half {
        (false, a & 1)
    } else {
        (true, 0)
    };
    let up_x = half < rx || (fx && odd_flag == 0 && rx == half);
    (p.0 - p.0 % a + if up_x { a } else { 0 }, p.1 - p.1 % a + if up_y { a } else { 0 })
}

/// `0x1421c3d00`: the orientation for a draw `v` among those in `mask` (bit per orientation).
pub fn pick(mask: u8, v: u32) -> u8 {
    if mask == 0 || mask == 0xff {
        return (v & 7) as u8;
    }
    let allowed: Vec<u8> = (0..8).filter(|o| mask & (1 << o) != 0).collect();
    allowed[(v as usize) % allowed.len()]
}

/// `0x141d14990`: turns a graph by `o` (positions, node orientations, odd orientations swap
/// width and height). `symmetric` says whether an edge type is the same seen from either
/// side; mirrored orientations flip the chirality of the other edges (`0x141d14d10`).
pub fn turn_graph(g: &mut Graph, o: u8, symmetric: &dyn Fn(&str) -> bool) {
    if o == 0 || o >= 8 {
        return;
    }
    let (w, h) = (g.width as i32 * 23, g.height as i32 * 23);
    for n in &mut g.nodes {
        (n.x, n.y) = turn_point(o, w, h, n.x, n.y);
        let no = crate::rooms::node_orientation(&n.orientation);
        if no < 8 {
            n.orientation = ORIENTATION_NAMES[compose(no, o) as usize].to_string();
        }
    }
    for e in &mut g.edges {
        for p in &mut e.points {
            *p = turn_point(o, w, h, p.0, p.1);
        }
        if o >= 4 && !symmetric(&e.edge_type) {
            if let Some(v) = e.values.get_mut(1) {
                *v = (*v == 0) as i32;
            }
        }
    }
    if o & 1 == 1 {
        std::mem::swap(&mut g.width, &mut g.height);
    }
    g.region = o;
}
