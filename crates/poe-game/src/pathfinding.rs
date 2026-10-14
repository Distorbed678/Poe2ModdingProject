//! Grid pathfinding: A* over walkable cells (8-connected, no corner cutting), then the path is
//! shortened to the cells that are still in line of sight of each other.
//!
//! An approximation of the client's pathfinder (the client also walks around obstacles to
//! reach a click or the WASD target); M4 replaces it with an exact port.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
};

use crate::movement::Walkable;

#[derive(Copy, Clone, PartialEq)]
struct Node {
    f: f32,
    cell: (i32, i32),
}

impl Eq for Node {}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        other.f.total_cmp(&self.f)
    }
}

impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn octile(a: (i32, i32), b: (i32, i32)) -> f32 {
    let (dx, dy) = ((a.0 - b.0).abs() as f32, (a.1 - b.1).abs() as f32);
    dx.max(dy) + (std::f32::consts::SQRT_2 - 1.0) * dx.min(dy)
}

/// Path from `start` towards `goal` as waypoints (cells, excluding the start). When the goal
/// cannot be reached within `budget` expanded cells, the path leads to the reachable cell
/// closest to it. Empty when the start is the best cell.
pub fn find_path(
    terrain: &dyn Walkable,
    start: (i32, i32),
    goal: (i32, i32),
    budget: usize,
) -> Vec<(i32, i32)> {
    let mut open = BinaryHeap::new();
    let mut came: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut cost: HashMap<(i32, i32), f32> = HashMap::new();
    cost.insert(start, 0.0);
    open.push(Node {
        f: octile(start, goal),
        cell: start,
    });
    let mut best = (octile(start, goal), start);
    let mut expanded = 0;
    while let Some(Node { cell, .. }) = open.pop() {
        if cell == goal {
            best = (0.0, cell);
            break;
        }
        expanded += 1;
        if expanded > budget {
            break;
        }
        let g = cost[&cell];
        for (dx, dy) in [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ] {
            let next = (cell.0 + dx, cell.1 + dy);
            if !terrain.walkable(next.0, next.1) {
                continue;
            }
            // No cutting corners past blocked cells.
            if dx != 0
                && dy != 0
                && (!terrain.walkable(cell.0 + dx, cell.1)
                    || !terrain.walkable(cell.0, cell.1 + dy))
            {
                continue;
            }
            let step = if dx != 0 && dy != 0 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
            let ng = g + step;
            if cost.get(&next).is_none_or(|&c| ng < c) {
                cost.insert(next, ng);
                came.insert(next, cell);
                let h = octile(next, goal);
                if h < best.0 {
                    best = (h, next);
                }
                open.push(Node {
                    f: ng + h,
                    cell: next,
                });
            }
        }
    }
    let mut cells = vec![best.1];
    while let Some(&prev) = came.get(cells.last().unwrap()) {
        if prev == start {
            break;
        }
        cells.push(prev);
    }
    if cells == [start] {
        return Vec::new();
    }
    cells.reverse();
    smooth(terrain, start, &cells)
}

/// Whether the straight segment between two cell centres crosses only walkable cells.
pub fn line_of_sight(terrain: &dyn Walkable, a: (i32, i32), b: (i32, i32)) -> bool {
    let (ax, ay) = (a.0 as f32 + 0.5, a.1 as f32 + 0.5);
    let (bx, by) = (b.0 as f32 + 0.5, b.1 as f32 + 0.5);
    let steps = ((bx - ax).abs().max((by - ay).abs()) * 2.0).ceil() as i32;
    (0..=steps).all(|i| {
        let t = if steps == 0 {
            0.0
        } else {
            i as f32 / steps as f32
        };
        let (x, y) = (ax + (bx - ax) * t, ay + (by - ay) * t);
        terrain.walkable(x.floor() as i32, y.floor() as i32)
    })
}

/// Keeps only the waypoints needed to stay in line of sight.
fn smooth(terrain: &dyn Walkable, start: (i32, i32), cells: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    let mut from = start;
    let mut i = 0;
    while i < cells.len() {
        // Farthest cell still visible from `from`.
        let mut j = i;
        while j + 1 < cells.len() && line_of_sight(terrain, from, cells[j + 1]) {
            j += 1;
        }
        out.push(cells[j]);
        from = cells[j];
        i = j + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wall at x = 5 for y < 8.
    struct Wall;
    impl Walkable for Wall {
        fn walkable(&self, x: i32, y: i32) -> bool {
            (0..20).contains(&x) && (0..20).contains(&y) && !(x == 5 && y < 8)
        }
    }

    #[test]
    fn straight_when_open() {
        let path = find_path(&Wall, (0, 10), (10, 10), 10_000);
        assert_eq!(path, vec![(10, 10)]);
    }

    #[test]
    fn around_a_wall() {
        let path = find_path(&Wall, (2, 2), (8, 2), 10_000);
        assert_eq!(path.last(), Some(&(8, 2)));
        assert!(path.len() >= 2);
        // Every leg is clear.
        let mut from = (2, 2);
        for &p in &path {
            assert!(line_of_sight(&Wall, from, p));
            from = p;
        }
    }

    #[test]
    fn closest_reachable_when_blocked() {
        let path = find_path(&Wall, (2, 2), (5, 2), 10_000);
        let end = *path.last().unwrap();
        assert!(Wall.walkable(end.0, end.1));
        assert!(octile(end, (5, 2)) <= 1.5);
    }
}
