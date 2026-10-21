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
//! near walls until the server has an exact port of the client's pathfinder.

use crate::pathfinding::find_path;

/// Which grid cells an entity can stand on.
pub trait Walkable {
    fn walkable(&self, x: i32, y: i32) -> bool;

    /// Distance-to-wall value of a cell (0 = blocked; the client's grid has 0..5), used to
    /// find which way a wall runs.
    fn clearance(&self, x: i32, y: i32) -> u8 {
        self.walkable(x, y) as u8
    }
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
    /// Sprint (after a dodge roll while the key is held): like [`Motion::Direction`], at
    /// `scale` times the walking speed.
    Sprint {
        x: f32,
        y: f32,
        scale: f32,
    },
    /// Walk to a cell (click-to-move), then stop.
    MoveTo {
        x: f32,
        y: f32,
    },
    /// Dodge roll: `remaining` cells towards a direction, along a path, with the roll's speed
    /// profile ([`roll_speed`]); `elapsed` seconds since it started.
    Roll {
        x: f32,
        y: f32,
        remaining: f32,
        elapsed: f32,
        /// The sprint key was still down at `t0 + 15`: from there the client speeds up
        /// ([`ROLL_SPRINT_STEPS`]) instead of tapering off.
        sprint: bool,
    },
}

/// Seconds per frame of the roll table (the official 33 ms frames).
const ROLL_FRAME: f32 = 0.033;

/// Cells the client moves in each frame of a dodge roll (`t0 + 1` to `t0 + 30`), mean of the 6
/// standing rolls on open ground of the 2026-10-20 capture (`dodge_roll_travel_distance` 46:
/// the sum is 45.9, see docs/protocol/movement.md). The roll falls from 3.9 to 1.2 cells per
/// frame, holds about 0.94 and tapers to zero at `t0 + 31`.
pub const ROLL_STEPS: [f32; 30] = [
    3.89, 3.74, 3.45, 3.27, 3.02, 2.72, 2.54, 2.30, 1.98, 1.76, 1.59, 1.35, 1.23, 1.16, 1.06, 0.99,
    0.97, 0.94, 0.94, 0.94, 0.94, 0.95, 0.93, 0.85, 0.74, 0.68, 0.49, 0.31, 0.16, 0.04,
];

/// The steps of frames `t0 + 16` to `t0 + 30` of a roll that became a sprint (the key was still
/// down at `t0 + 15`), mean of the 7 sprint rolls of the 2026-10-20 capture: the client blends
/// into the sprint, so it speeds up from 0.9 to 1.7 cells per frame instead of tapering off, and
/// takes a catch-up step of about 2.4 in the last frame (median of 7: 2.0 to 3.4). From
/// `t0 + 31` on it sprints at 1.86 (the sprint motion).
pub const ROLL_SPRINT_STEPS: [f32; 15] = [
    0.91, 0.96, 1.02, 1.08, 1.09, 1.11, 1.20, 1.34, 1.36, 1.46, 1.49, 1.52, 1.54, 1.70, 2.40,
];

/// Index into [`ROLL_STEPS`] of the frame `t` seconds into the roll.
fn roll_frame(t: f32) -> usize {
    ((t / ROLL_FRAME) + 1e-3).floor().max(0.0) as usize
}

/// Cells per frame of frame `frame` (0 based) of a roll.
fn roll_step(frame: usize, sprint: bool) -> f32 {
    if sprint && frame >= 15 {
        ROLL_SPRINT_STEPS.get(frame - 15)
    } else {
        ROLL_STEPS.get(frame)
    }
    .copied()
    .unwrap_or(0.0)
}

/// Dodge roll speed (cells per second) `t` seconds into the roll: the per-frame steps of
/// [`ROLL_STEPS`] (0 once the table is over). The client drives it from the roll animation.
pub fn roll_speed(t: f32, sprint: bool) -> f32 {
    roll_step(roll_frame(t), sprint) / ROLL_FRAME
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
const PATH_BUDGET: usize = 8_000;
/// WASD plans towards this many cells along the input (the client aims about 180 cells
/// ahead; the first part of the path is what matters between re-plans).
const WASD_LOOKAHEAD: f32 = 40.0;
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

    /// The running roll turns into a sprint roll (the key was still down at `t0 + 15`): the
    /// rest of it follows [`ROLL_SPRINT_STEPS`] and is not limited to the roll's distance.
    pub fn sprint_roll(&mut self) {
        if let Motion::Roll {
            remaining, sprint, ..
        } = &mut self.motion
        {
            *sprint = true;
            *remaining = f32::MAX;
        }
    }

    /// Waypoints (cells) left of the current walk.
    pub fn path(&self) -> &[(i32, i32)] {
        &self.path
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
            Motion::Direction { x, y } => self.walk_towards(x, y, 1.0, dt, terrain),
            Motion::Sprint { x, y, scale } => self.walk_towards(x, y, scale, dt, terrain),
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
            Motion::Roll {
                x,
                y,
                remaining,
                elapsed,
                sprint,
            } => {
                let step = (roll_speed(elapsed, sprint) * dt).min(remaining);
                let moved = match unit(x, y) {
                    Some((ux, uy)) => self.slide(ux, uy, step, terrain),
                    None => 0.0,
                };
                // A blocked frame costs no distance: the roll goes on with the profile (the
                // client's roll animation runs its 30 frames even against a wall, and a later
                // steer can free it).
                let remaining = remaining - moved;
                // The last table step is the last frame of the roll.
                let last = roll_frame(elapsed) + 1 >= ROLL_STEPS.len();
                self.motion = if remaining > 1e-3 && !last {
                    Motion::Roll {
                        x,
                        y,
                        remaining,
                        elapsed: elapsed + dt,
                        sprint,
                    }
                } else {
                    Motion::Idle
                };
            }
        }
    }

    /// Moves `len` cells along a unit vector; when the next cell is blocked, moves along the
    /// wall instead (the direction minus its component into the wall, the wall's normal
    /// taken from the clearance gradient). Returns the distance moved.
    fn slide(&mut self, ux: f32, uy: f32, len: f32, terrain: &dyn Walkable) -> f32 {
        let free = |x: f32, y: f32| terrain.walkable(x.floor() as i32, y.floor() as i32);
        let (nx, ny) = (self.x + ux * len, self.y + uy * len);
        if free(nx, ny) {
            self.x = nx;
            self.y = ny;
            return len;
        }
        let (cx, cy) = self.cell();
        let c = |x: i32, y: i32| terrain.clearance(x, y) as f32;
        // Gradient over a 5-cell span (points away from the wall).
        let gx = c(cx + 2, cy) - c(cx - 2, cy);
        let gy = c(cx, cy + 2) - c(cx, cy - 2);
        let mut tangent = None;
        if let Some((gx, gy)) = unit(gx, gy) {
            let into = ux * gx + uy * gy;
            if into < 0.0 {
                tangent = unit(ux - into * gx, uy - into * gy);
            }
        }
        let candidates = tangent
            .into_iter()
            .chain([(ux.signum(), 0.0), (0.0, uy.signum())]);
        for (tx, ty) in candidates {
            if tx == 0.0 && ty == 0.0 || tx * ux + ty * uy <= 0.0 {
                continue;
            }
            // Along the wall only the part of the motion that goes that way.
            let along = len * (tx * ux + ty * uy);
            let (nx, ny) = (self.x + tx * along, self.y + ty * along);
            if free(nx, ny) {
                self.x = nx;
                self.y = ny;
                return along;
            }
        }
        0.0
    }

    /// Walks along a direction (WASD, sprint), planning towards a point ahead of the entity
    /// every few frames.
    fn walk_towards(&mut self, x: f32, y: f32, scale: f32, dt: f32, terrain: &dyn Walkable) {
        if x == 0.0 && y == 0.0 {
            self.motion = Motion::Idle;
            return;
        }
        if self.replan_in == 0 || self.path.is_empty() {
            let (cx, cy) = self.cell();
            let len = (x * x + y * y).sqrt();
            let goal = (
                cx + (x / len * WASD_LOOKAHEAD).round() as i32,
                cy + (y / len * WASD_LOOKAHEAD).round() as i32,
            );
            self.path = find_path(terrain, (cx, cy), goal, PATH_BUDGET);
            self.replan_in = REPLAN_FRAMES;
        }
        self.replan_in -= 1;
        self.follow(self.speed * scale * dt);
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
    fn roll_covers_its_length() {
        let mut m = Mover::at_cell((0, 0), 37.0);
        m.start(Motion::Roll {
            x: 0.0,
            y: 1.0,
            remaining: 46.0,
            elapsed: 0.0,
            sprint: false,
        });
        let mut frames = 0;
        while m.is_moving() {
            m.advance(0.033, &Open);
            frames += 1;
        }
        // The official profile covers 45.9 of the 46 cells in 30 frames.
        assert!(
            (m.y - (0.5 + ROLL_STEPS.iter().sum::<f32>())).abs() < 0.05,
            "{}",
            m.y
        );
        assert_eq!(frames, 30);
    }

    #[test]
    fn a_sprint_roll_speeds_up_after_frame_15() {
        let mut m = Mover::at_cell((0, 0), 37.0);
        m.start(Motion::Roll {
            x: 1.0,
            y: 0.0,
            remaining: 46.0,
            elapsed: 0.0,
            sprint: false,
        });
        let mut last = m.x;
        for i in 0..30 {
            if i == 15 {
                m.sprint_roll();
            }
            m.advance(0.033, &Open);
            let expect = if i < 15 {
                ROLL_STEPS[i]
            } else {
                ROLL_SPRINT_STEPS[i - 15]
            };
            assert!((m.x - last - expect).abs() < 1e-3, "frame {}", i + 1);
            last = m.x;
        }
        assert!(!m.is_moving());
        // About 55 cells in the 30 frames, against 46 for a tap roll.
        assert!(m.x > 0.5 + 54.0 && m.x < 0.5 + 56.0, "{}", m.x);
    }

    #[test]
    fn roll_follows_the_official_steps() {
        let mut m = Mover::at_cell((0, 0), 37.0);
        m.start(Motion::Roll {
            x: 1.0,
            y: 0.0,
            remaining: 46.0,
            elapsed: 0.0,
            sprint: false,
        });
        let mut last = m.x;
        for (i, step) in ROLL_STEPS.iter().enumerate() {
            m.advance(0.033, &Open);
            assert!((m.x - last - step).abs() < 1e-3, "frame {}", i + 1);
            last = m.x;
        }
        assert!(!m.is_moving());
    }
}
