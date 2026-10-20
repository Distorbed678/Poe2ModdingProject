//! Replay tests: input sequences from the official captures (fixtures derived by
//! `scripts/re/movement_fixtures.py`) are fed to a [`World`] and the events and their frame
//! offsets are compared with what the official server sent.
//!
//! A fixture holds the inputs with the frame the server handles them in (`f`; the replay
//! submits them before tick `f + 1`, so the first reply lands at `f + 1`) and the expected
//! events with the batch they were sent in. Cells are compared with a tolerance: the official
//! server's cell during rolls is not reproducible from the captures (gap #10), and walking
//! differs by a fraction of a cell per frame.

use std::{collections::BTreeMap, sync::Arc};

use poe_game::{
    locomotion::{ActionKind, MoveEvent, MoveInput, MovementConfig, SkillIds, UseSkill},
    world::{World, WorldConfig},
};
use serde_json::{Value, json};

const MOVE_SKILL: u32 = 0x2000_2909;
const ROLL_SKILL: u32 = 0x8000_0040;
/// Buff rows of the sprint, as in the 0.5.5e buff definitions (the tests' stand-in for game
/// data).
const BUFF_ROWS: [u16; 3] = [2758, 2737, 2853];

fn config() -> MovementConfig {
    MovementConfig::new(
        SkillIds {
            move_skill: MOVE_SKILL,
            roll_skill: ROLL_SKILL,
        },
        BUFF_ROWS,
    )
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/tests/fixtures/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("fixture")).expect("json")
}

fn i(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap_or_else(|| panic!("{k} in {v}")) as i32
}

fn input(v: &Value) -> MoveInput {
    match v["op"].as_str().unwrap() {
        "wasd" => MoveInput::Direction {
            x: i(v, "x"),
            y: i(v, "y"),
        },
        "use_skill" => MoveInput::UseSkill(UseSkill {
            skill: i(v, "skill") as u32,
            flags: i(v, "flags") as u16,
            unk: i(v, "unk") as u8,
            target_flags: i(v, "target_flags") as u8,
            entity: v.get("entity").map(|e| e.as_u64().unwrap() as u32),
            x: i(v, "x"),
            y: i(v, "y"),
        }),
        "update_target" => MoveInput::UpdateSkillTarget {
            x: i(v, "x"),
            y: i(v, "y"),
            skill: i(v, "skill") as u32,
        },
        "input_end" => MoveInput::SkillInputEnd,
        "sprint" => MoveInput::SprintInput {
            held: v["held"].as_bool().unwrap(),
        },
        "aim" => MoveInput::Aim {
            x: i(v, "x"),
            y: i(v, "y"),
        },
        op => panic!("fixture input {op}"),
    }
}

/// The fixture's JSON form of an event (without the frame).
fn to_json(e: &MoveEvent) -> Value {
    match e {
        MoveEvent::MoveAck { dir, cell } => {
            json!({"ev": "move_ack", "dir": [dir.0, dir.1], "cell": [cell.0, cell.1]})
        }
        MoveEvent::ActionStart {
            skill,
            cell,
            target,
            kind,
            ..
        } => {
            let flags = if *kind == ActionKind::Move { 0 } else { 0x410 };
            json!({"ev": "action_start", "skill": skill, "flags": flags,
                   "cell": [cell.0, cell.1], "target": [target.0, target.1]})
        }
        MoveEvent::ActionEvent => json!({"ev": "action_event"}),
        MoveEvent::Stats(s) => {
            let s: Vec<_> = s.iter().map(|(k, v)| json!([k.id(), v])).collect();
            json!({"ev": "stats", "stats": s})
        }
        MoveEvent::SkillAck {
            skill,
            flags,
            unk,
            target_flags,
            x,
            y,
            ..
        } => json!({"ev": "skill_ack", "skill": skill, "flags": flags, "unk": unk,
                    "target_flags": target_flags, "x": x, "y": y}),
        MoveEvent::TargetAck { skill, x, y } => {
            json!({"ev": "target_ack", "skill": skill, "x": x, "y": y})
        }
        MoveEvent::InputEndAck => json!({"ev": "input_end_ack"}),
        MoveEvent::SprintState { held } => json!({"ev": "sprint", "held": held}),
        MoveEvent::BuffAdded { index, row, fade } => {
            let mut v = json!({"ev": "buff_added", "index": index, "row": row});
            if let Some(f) = fade {
                v["fade"] = json!(f);
            }
            v
        }
        MoveEvent::BuffRemoved { index } => json!({"ev": "buff_removed", "index": index}),
        MoveEvent::AimEcho { cell } => {
            json!({"ev": "aim_echo", "kind": 0, "x": cell.0, "y": cell.1})
        }
        other => panic!("event not in fixtures: {other:?}"),
    }
}

/// Position fields and how far they may differ (cells).
///
/// The official server's cell while a roll runs does not follow a straight roll (gap #10: the
/// captures hold no client positions to compare with), so `walk` and `steer` are wide in
/// roll fixtures; `start` (the cell a roll starts from) stays tight where the walk before it
/// is part of the fixture.
#[derive(Clone, Copy)]
struct Tol {
    /// `move_ack` cells.
    walk: i64,
    /// `action_start` cells and targets, `skill_ack` targets: the cell at the start.
    start: i64,
    /// `target_ack`: the cell at the time plus a cursor offset.
    steer: i64,
}

const ANY: i64 = 1000;

fn close(exp: &Value, act: &Value, tol: Tol, dev: &mut BTreeMap<String, i64>) -> bool {
    let (Some(e), Some(a)) = (exp.as_object(), act.as_object()) else {
        return false;
    };
    let kind = exp["ev"].as_str().unwrap();
    if e.len() != a.len() || e.keys().any(|k| !a.contains_key(k)) {
        return false;
    }
    for (k, ev) in e {
        let av = &a[k];
        let pos_tol = match (kind, k.as_str()) {
            ("move_ack", "cell") => Some(tol.walk),
            ("action_start", "cell" | "target") => Some(tol.start),
            ("skill_ack", "x" | "y") => Some(tol.start),
            ("target_ack", "x" | "y") => Some(tol.steer),
            _ => None,
        };
        let mut note = |d: i64| {
            let e = dev.entry(format!("{kind}.{k}")).or_default();
            *e = (*e).max(d);
        };
        let ok = match (pos_tol, ev, av) {
            (Some(t), Value::Array(x), Value::Array(y)) => x.iter().zip(y).all(|(p, q)| {
                let d = (p.as_i64().unwrap() - q.as_i64().unwrap()).abs();
                note(d);
                d <= t
            }),
            (Some(t), x, y) => {
                let d = (x.as_i64().unwrap() - y.as_i64().unwrap()).abs();
                note(d);
                d <= t
            }
            (None, Value::Number(x), Value::Number(y)) if x.is_f64() || y.is_f64() => {
                (x.as_f64().unwrap() - y.as_f64().unwrap()).abs() < 1e-3
            }
            (None, x, y) => x == y,
        };
        if !ok {
            return false;
        }
    }
    true
}

/// Replays a fixture, asserts every frame's events and returns the largest cell deviation.
fn replay(name: &str, tol: Tol) {
    let fx = fixture(name);
    let start = (
        fx["start_cell"][0].as_i64().unwrap() as i32,
        fx["start_cell"][1].as_i64().unwrap() as i32,
    );
    let mut world = World::new(
        WorldConfig::new(config()),
        Arc::new(poe_game::movement::Open),
    );
    let player = world.spawn_player(start);
    let inputs = fx["inputs"].as_array().unwrap();
    let expect = fx["expect"].as_array().unwrap();
    let last = expect
        .iter()
        .map(|e| e["f"].as_u64().unwrap())
        .max()
        .unwrap();
    let mut dev = BTreeMap::new();
    let mut problems = Vec::new();
    for frame in 1..=last {
        for inp in inputs
            .iter()
            .filter(|v| v["f"].as_u64().unwrap() + 1 == frame)
        {
            assert!(world.input(player, input(inp)));
        }
        let events = world.tick();
        assert_eq!(world.frame(), frame);
        let act: Vec<Value> = events
            .iter()
            .map(|e| {
                assert_eq!(e.frame, frame);
                assert_eq!(e.entity, player);
                to_json(e.movement().unwrap())
            })
            .collect();
        let exp: Vec<&Value> = expect
            .iter()
            .filter(|e| e["f"].as_u64().unwrap() == frame)
            .collect();
        let same = exp.len() == act.len()
            && exp.iter().zip(&act).all(|(e, a)| {
                let mut e = (*e).clone();
                e.as_object_mut().unwrap().remove("f");
                close(&e, a, tol, &mut dev)
            });
        if !same {
            problems.push(format!(
                "frame {frame}:\n  official {}\n  ours     {}",
                exp.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("\n           "),
                act.iter()
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
                    .join("\n           ")
            ));
        }
    }
    println!("{name}: largest cell deviations {dev:?}");
    assert!(problems.is_empty(), "{name}:\n{}", problems.join("\n"));
}

/// WASD start, turns and stop: the cell stays within 4 cells of the official one over 20
/// frames, every aim input is echoed one frame later.
#[test]
fn wasd_start_turn_stop_with_aim_echoes() {
    replay(
        "wasd_start_turn_stop",
        Tol {
            walk: 4,
            start: ANY,
            steer: ANY,
        },
    );
}

/// Walk, tap roll (Space up 4 frames in), steering acks, WASD during the roll, roll end at
/// t0 + 16, no aim echo while the roll action runs.
#[test]
fn tap_roll_while_walking() {
    replay(
        "tap_roll_while_walking",
        Tol {
            walk: ANY,
            start: 6,
            steer: ANY,
        },
    );
}

/// Two tap rolls; the second is asked for 27 frames after the first started and is acked
/// at once but starts at t0 + 30 (the 2-batch delay of movement.md).
#[test]
fn tap_rolls_chained_second_one_waits() {
    replay(
        "tap_rolls_chained",
        Tol {
            walk: ANY,
            start: ANY,
            steer: ANY,
        },
    );
}

/// Roll with Space held: sprint start (t0 + 15), roll end (t0 + 30), must_sprint off (t0 + 35),
/// the scaled `MoveAck` stream, steering acks, key release with the sprint-off sequence and
/// the unscaled `MoveAck` one frame later.
#[test]
fn sprint_hold_with_release() {
    replay(
        "sprint_hold_release",
        Tol {
            walk: ANY,
            start: 6,
            steer: ANY,
        },
    );
}

/// A sprint ended by an attack in the frame the key is released.
#[test]
fn sprint_ended_by_an_attack() {
    replay(
        "sprint_ended_by_attack",
        Tol {
            walk: ANY,
            start: 10,
            steer: ANY,
        },
    );
}
