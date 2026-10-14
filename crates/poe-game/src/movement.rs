//! Server-side movement of an entity, in grid cells (23 per tile; a cell is 250/23 world
//! units).
//!
//! The client runs in lockstep: it moves an entity only when the server tells it to (WASD
//! direction `0x64`, actions such as Move or DodgeRoll `0x1c8`), then simulates the movement
//! itself, frame by frame. When the server later sends a position that differs from the
//! client's, the client snaps to it. So the server has to simulate the same movement: same
//! speed, same frames, same obstacles.
//!
//! The client walks along paths from its pathfinder, for clicks and for WASD (whose target is
//! the position plus the input vector, in cells). This follows paths from
//! [`crate::pathfinding`], an approximation: it matches the client in the open and can drift
//! near walls until the server has an exact port of the client's pathfinder (M4).

use crate::pathfinding::find_path;

/// Which grid cells an entity can stand on.
pub trait Walkable {
    fn walkable(&self, x: i32, y: i32) -> bool;
}

/// Everything is walkable (no terrain data).
pub struct Open;

impl Walkable for Open {
    fn walkable(&self, _: i32, _: i32) -> bool {
        true
    }
}

/// What the entity is doing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Motion {
    Idle,
    /// WASD: keep moving along this direction (client vector, any length).
    Direction {
        x: f32,
        y: f32,
    },
    /// Walk to a cell (click-to-move), then stop.
    MoveTo {
        x: f32,
        y: f32,
    },
    /// Fixed-length dash along a direction (dodge roll): `remaining` cells at `speed`.
    Dash {
        x: f32,
        y: f32,
        remaining: f32,
        speed: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mover {
    /// Position in grid cells (cell centre = integer + 0.5).
    pub x: f32,
    pub y: f32,
    /// Walking speed in cells per second.
    pub speed: f32,
    pub motion: Motion,
    /// Waypoints (cells) of the current walk.
    path: Vec<(i32, i32)>,
    /// Frames until the WASD path is planned again.
    replan_in: u32,
}

/// Cells the pathfinder may expand per plan.
const PATH_BUDGET: usize = 40_000;
/// WASD paths are planned again every this many frames.
const REPLAN_FRAMES: u32 = 6;

impl Mover {
    /// An idle entity at the centre of a grid cell.
    pub fn at_cell((x, y): (i32, i32), speed: f32) -> Self {
        Self {
            x: x as f32 + 0.5,
            y: y as f32 + 0.5,
            speed,
            motion: Motion::Idle,
            path: Vec::new(),
            replan_in: 0,
        }
    }

    /// Starts a motion (plans walks on the next frame).
    pub fn start(&mut self, motion: Motion) {
        self.motion = motion;
        self.path.clear();
        self.replan_in = 0;
    }

    /// Moves the entity to a cell centre and stops it.
    pub fn place(&mut self, (x, y): (i32, i32)) {
        self.x = x as f32 + 0.5;
        self.y = y as f32 + 0.5;
        self.start(Motion::Idle);
    }

    /// The grid cell the entity stands in (what the client compares positions by).
    pub fn cell(&self) -> (i32, i32) {
        (self.x.floor() as i32, self.y.floor() as i32)
    }

    pub fn is_moving(&self) -> bool {
        self.motion != Motion::Idle
    }

    /// Advances one frame of `dt` seconds.
    pub fn advance(&mut self, dt: f32, terrain: &dyn Walkable) {
        match self.motion {
            Motion::Idle => {}
            Motion::Direction { x, y } => {
                if x == 0.0 && y == 0.0 {
                    self.motion = Motion::Idle;
                    return;
                }
                if self.replan_in == 0 || self.path.is_empty() {
                    let (cx, cy) = self.cell();
                    let goal = (cx + x.round() as i32, cy + y.round() as i32);
                    self.path = find_path(terrain, (cx, cy), goal, PATH_BUDGET);
                    self.replan_in = REPLAN_FRAMES;
                }
                self.replan_in -= 1;
                self.follow(self.speed * dt);
            }
            Motion::MoveTo { x, y } => {
                if self.path.is_empty() && self.replan_in == 0 {
                    let goal = (x.floor() as i32, y.floor() as i32);
                    self.path = find_path(terrain, self.cell(), goal, PATH_BUDGET);
                    // Plan once per click.
                    self.replan_in = u32::MAX;
                }
                if !self.follow(self.speed * dt) {
                    self.start(Motion::Idle);
                }
            }
            Motion::Dash {
                x,
                y,
                remaining,
                speed,
            } => {
                let step = (speed * dt).min(remaining);
                let moved = match unit(x, y) {
                    Some((ux, uy)) => self.step(ux, uy, step, terrain),
                    None => false,
                };
                let remaining = remaining - step;
                self.motion = if moved && remaining > 1e-3 {
                    Motion::Dash {
                        x,
                        y,
                        remaining,
                        speed,
                    }
                } else {
                    Motion::Idle
                };
            }
        }
    }

    /// Walks `len` cells along the path (waypoints are cell centres). Returns whether there is
    /// path left.
    fn follow(&mut self, mut len: f32) -> bool {
        while len > 0.0 {
            let Some(&(wx, wy)) = self.path.first() else {
                return false;
            };
            let (tx, ty) = (wx as f32 + 0.5, wy as f32 + 0.5);
            let (dx, dy) = (tx - self.x, ty - self.y);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist <= len {
                self.x = tx;
                self.y = ty;
                len -= dist;
                self.path.remove(0);
            } else {
                self.x += dx / dist * len;
                self.y += dy / dist * len;
                len = 0.0;
            }
        }
        !self.path.is_empty()
    }

    /// Moves `len` cells along the unit vector, sliding along an axis when the target cell is
    /// blocked. Returns whether the entity moved.
    fn step(&mut self, ux: f32, uy: f32, len: f32, terrain: &dyn Walkable) -> bool {
        let candidates = [(ux, uy), (ux.signum() * ux.abs(), 0.0), (0.0, uy)];
        for (cx, cy) in candidates {
            if cx == 0.0 && cy == 0.0 {
                continue;
            }
            let (nx, ny) = (self.x + cx * len, self.y + cy * len);
            if terrain.walkable(nx.floor() as i32, ny.floor() as i32) {
                self.x = nx;
                self.y = ny;
                return true;
            }
        }
        false
    }
}

fn unit(x: f32, y: f32) -> Option<(f32, f32)> {
    let len = (x * x + y * y).sqrt();
    (len > 0.0).then(|| (x / len, y / len))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Wall;
    impl Walkable for Wall {
        /// Cells with x >= 10 are blocked.
        fn walkable(&self, x: i32, _: i32) -> bool {
            x < 10
        }
    }

    #[test]
    fn walks_in_a_direction() {
        let mut m = Mover::at_cell((0, 0), 37.0);
        m.start(Motion::Direction { x: 180.0, y: 0.0 });
        for _ in 0..30 {
            m.advance(0.033, &Open);
        }
        assert!((m.x - (0.5 + 37.0 * 0.99)).abs() < 1e-3);
        assert_eq!(m.cell().1, 0);
    }

    #[test]
    fn move_to_stops_at_the_target() {
        let mut m = Mover::at_cell((0, 0), 37.0);
        m.start(Motion::MoveTo { x: 5.5, y: 5.5 });
        for _ in 0..30 {
            m.advance(0.033, &Open);
        }
        assert_eq!(m.cell(), (5, 5));
        assert_eq!(m.motion, Motion::Idle);
    }

    #[test]
    fn walks_along_a_wall() {
        let mut m = Mover::at_cell((8, 0), 37.0);
        m.start(Motion::Direction { x: 127.0, y: 127.0 });
        for _ in 0..30 {
            m.advance(0.033, &Wall);
        }
        assert!(m.x < 10.0);
        assert!(m.y > 10.0);
    }

    #[test]
    fn dash_covers_its_length() {
        let mut m = Mover::at_cell((0, 0), 37.0);
        m.start(Motion::Dash {
            x: 0.0,
            y: 1.0,
            remaining: 20.0,
            speed: 60.0,
        });
        for _ in 0..30 {
            m.advance(0.033, &Open);
        }
        assert!((m.y - 20.5).abs() < 1e-3);
        assert_eq!(m.motion, Motion::Idle);
    }
}
