//! Terrain graphs (`.tgr`, version 25): the area's rooms (nodes) and the paths between them
//! (edges, e.g. rivers), which the generator lays out on a grid of tiles. UTF-16 text:
//!
//! ```text
//! version 25
//! Size: <w> <h>                     tiles
//! MasterFile: "<.tsi>"
//! Nodes: <n>
//! Edges: <m>
//! "<ground type>" x3
//! <x> <y> <k> <edge>*k "<room type>" <orientation> <d> "<tag>"*d <ints...>      (n lines)
//! <a> <b> <p> (<x> <y>)*p <i> <i> <i> "<.et>" <i> <i>                          (m lines)
//! ```
//! `.dgr` (dungeon graph) files have the same layout plus dungeon fields, which the client
//! reads with `0x14292e8e0` into the same graph: a `Default%: <branching> <extra door>
//! <fuzzy>` line after the ground types (and, from version 23, an
//! `open_extra_doors_into_impenetrable_rooms` bool line); node lines end with a void flag
//! (`Y`/`N`) and, from version 22, a branching bool; edge lines end after the `.et` with
//! branching chances, depth limit, branching type, extra door % and type, fuzzy %,
//! impenetrability, impenetrable and void flags and accessibility. Those come after the
//! numbers and are kept as raw tokens (`extra`). The `.tgr`-only numbers (node fixed axes,
//! edge widths) are absent there.
//!
//! Node positions are in grid cells (23 per tile). Orientation is `I`, `R90`..`R270`, `F...`
//! for flipped ones, or `(any)` / `(best)` (the generator chooses).

use crate::text::{decode_utf16, tokens};

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub version: u32,
    /// Size in tiles.
    pub width: u32,
    pub height: u32,
    pub master_file: String,
    pub ground_types: [String; 3],
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// `.dgr` defaults: branching %, extra door %, fuzzy % (absent for `.tgr`).
    pub dungeon_defaults: Option<[i32; 3]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Grid cells.
    pub x: i32,
    pub y: i32,
    /// Indices of the edges at this node.
    pub edges: Vec<u32>,
    /// Room type (an `.arm` room's name in the master's room set), empty for a waypoint.
    pub room_type: String,
    pub orientation: String,
    pub tags: Vec<String>,
    /// The numbers after the tags (first: existence chance in %).
    pub values: Vec<i32>,
    /// `.dgr` tokens after the numbers.
    pub extra: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    /// Intermediate points (grid cells).
    pub points: Vec<(i32, i32)>,
    /// The numbers before the edge type.
    pub values: Vec<i32>,
    /// Edge type (`.et`).
    pub edge_type: String,
    /// The numbers after the edge type (`.tgr`).
    pub tail: Vec<i32>,
    /// `.dgr` tokens after the edge type.
    pub extra: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
#[error("graph line {line}: {what}")]
pub struct GraphError {
    pub line: usize,
    pub what: String,
}

fn err(line: usize, what: impl Into<String>) -> GraphError {
    GraphError {
        line: line + 1,
        what: what.into(),
    }
}

impl Graph {
    pub fn parse_bytes(bytes: &[u8]) -> Result<Self, GraphError> {
        Self::parse(&decode_utf16(bytes))
    }

    pub fn parse(text: &str) -> Result<Self, GraphError> {
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        let field = |i: usize, key: &str| -> Result<Vec<String>, GraphError> {
            let l = lines.get(i).ok_or_else(|| err(i, "missing line"))?;
            let rest = l
                .strip_prefix(key)
                .ok_or_else(|| err(i, format!("expected `{key}`")))?;
            Ok(tokens(rest))
        };
        let int = |i: usize, s: &str| -> Result<i32, GraphError> {
            s.parse().map_err(|_| err(i, format!("bad number `{s}`")))
        };
        let version = int(0, &field(0, "version")?[0])? as u32;
        let size = field(1, "Size:")?;
        let (width, height) = (int(1, &size[0])? as u32, int(1, &size[1])? as u32);
        let master_file = field(2, "MasterFile:")?.remove(0);
        let n = int(3, &field(3, "Nodes:")?[0])? as usize;
        let m = int(4, &field(4, "Edges:")?[0])? as usize;
        let ground_types = [5, 6, 7].map(|i| {
            lines
                .get(i)
                .map(|l| l.trim_matches('"').to_string())
                .unwrap_or_default()
        });
        let mut first = 8;
        let mut dungeon_defaults = None;
        if let Some(rest) = lines.get(8).and_then(|l| l.strip_prefix("Default%:")) {
            let t = tokens(rest);
            let mut d = [0; 3];
            for (k, v) in d.iter_mut().enumerate() {
                *v = int(8, t.get(k).map_or("", String::as_str))?;
            }
            dungeon_defaults = Some(d);
            first = if version > 22 { 10 } else { 9 };
        }
        let mut nodes = Vec::with_capacity(n);
        for i in first..first + n {
            let t = tokens(lines.get(i).ok_or_else(|| err(i, "missing node"))?);
            let mut it = t.iter();
            let mut next = || it.next().ok_or_else(|| err(i, "short node line"));
            let x = int(i, next()?)?;
            let y = int(i, next()?)?;
            let k = int(i, next()?)? as usize;
            let edges = (0..k)
                .map(|_| next().and_then(|s| int(i, s)).map(|v| v as u32))
                .collect::<Result<_, _>>()?;
            let room_type = next()?.clone();
            let orientation = next()?.clone();
            let d = int(i, next()?)? as usize;
            let tags = (0..d).map(|_| next().cloned()).collect::<Result<_, _>>()?;
            let (values, extra) = numbers_then_rest(it);
            nodes.push(Node {
                x,
                y,
                edges,
                room_type,
                orientation,
                tags,
                values,
                extra,
            });
        }
        let mut edges = Vec::with_capacity(m);
        for i in first + n..first + n + m {
            let line = lines.get(i).ok_or_else(|| err(i, "missing edge"))?;
            let t = tokens(line);
            let mut it = t.iter();
            let mut next = || it.next().ok_or_else(|| err(i, "short edge line"));
            let from = int(i, next()?)? as u32;
            let to = int(i, next()?)? as u32;
            let p = int(i, next()?)? as usize;
            let mut points = Vec::with_capacity(p);
            for _ in 0..p {
                points.push((int(i, next()?)?, int(i, next()?)?));
            }
            // Numbers up to the quoted edge type (the tokenizer keeps quotes' contents; the
            // type is the first non-numeric token).
            let mut values = Vec::new();
            let edge_type = loop {
                let s = next()?;
                match s.parse::<i32>() {
                    Ok(v) => values.push(v),
                    Err(_) => break s.clone(),
                }
            };
            let (mut tail, mut extra) = numbers_then_rest(it);
            if dungeon_defaults.is_some() {
                // The numbers after a `.dgr` edge type are dungeon fields, not widths.
                let mut all: Vec<String> = tail.drain(..).map(|v| v.to_string()).collect();
                all.append(&mut extra);
                extra = all;
            }
            edges.push(Edge {
                from,
                to,
                points,
                values,
                edge_type,
                tail,
                extra,
            });
        }
        Ok(Self {
            version,
            width,
            height,
            master_file,
            ground_types,
            nodes,
            edges,
            dungeon_defaults,
        })
    }
}

/// Leading numbers of a token list, then the remaining tokens.
fn numbers_then_rest<'a>(it: impl Iterator<Item = &'a String>) -> (Vec<i32>, Vec<String>) {
    let mut values = Vec::new();
    let mut rest = Vec::new();
    for s in it {
        match (rest.is_empty(), s.parse::<i32>()) {
            (true, Ok(v)) => values.push(v),
            _ => rest.push(s.clone()),
        }
    }
    (values, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn town_graph() {
        let g = Graph::parse(
            "version 25\nSize: 33 30\nMasterFile: \"Metadata/T/master.tsi\"\nNodes: 1\nEdges: 0\n\"\"\n\"\"\n\"\"\n12 12 0 \"town\" I 6 \"default\" \"entrance1disabled\" \"entrance2south\" \"entrance3east\" \"entrance4north\" \"entrance5\" 100 0 0 0 0 0 0 0 0 1 1\n",
        )
        .unwrap();
        assert_eq!((g.width, g.height), (33, 30));
        assert_eq!(g.nodes[0].room_type, "town");
        assert_eq!(g.nodes[0].tags.len(), 6);
        assert_eq!(g.nodes[0].values[0], 100);
    }

    #[test]
    fn edge_line() {
        let g = Graph::parse(
            "version 25\nSize: 66 60\nMasterFile: \"m.tsi\"\nNodes: 2\nEdges: 1\n\"\"\n\"\"\n\"\"\n1 1 1 0 \"\" (any) 0 100\n2 2 1 0 \"\" (any) 0 100\n0 1 1 1162 127 100 0 0 \"Metadata/a.et\" 0 0\n",
        )
        .unwrap();
        let e = &g.edges[0];
        assert_eq!(e.points, vec![(1162, 127)]);
        assert_eq!(e.tail, vec![0, 0]);
        assert_eq!(e.values, vec![100, 0, 0]);
        assert_eq!(e.edge_type, "Metadata/a.et");
    }

    #[test]
    fn dungeon_graph() {
        let g = Graph::parse(
            "version 25\nSize: 5 6\nMasterFile: \"m.tsi\"\nNodes: 2\nEdges: 1\n\"\"\n\"\"\n\"\"\nDefault%: 0 0 0\n0\n104 104 1 0 \"entrance_flip\" FI 2 \"entrance1\" \"default\" 100 0 0 0 0 0 0 0 0 N 0\n150 104 1 0 \"\" (any) 0 100 0 0 0 0 0 0 0 0 N 0\n0 1 0 100 0 0 \"a.et\" 30 40 1 \"\" 100 \"a.et\" 0 0 P N 1\n",
        )
        .unwrap();
        assert_eq!(g.dungeon_defaults, Some([0, 0, 0]));
        assert_eq!(g.nodes[0].values.len(), 9);
        assert_eq!(g.nodes[0].extra, vec!["N", "0"]);
        let e = &g.edges[0];
        assert_eq!(e.values, vec![100, 0, 0]);
        assert!(e.tail.is_empty());
        assert_eq!(e.extra.len(), 11);
    }
}
