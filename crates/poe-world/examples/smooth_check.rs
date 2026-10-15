//! Replays the client's logged smoothing steps (`arc_start`, `arc_end`, `round` trace lines)
//! with the server's port: `cargo run -p poe-world --example smooth_check -- <trace.tsv>`.

use poe_world::smoothing::{SmoothEdge, arc_end, arc_start};

fn pt(s: &str) -> (i32, i32) {
    let (a, b) = s.split_once(',').unwrap_or(("0", "0"));
    (a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(0))
}

fn pts(s: &str) -> Vec<(i32, i32)> {
    s.split_whitespace().map(pt).collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: smooth_check <trace.tsv>")?;
    let text = std::fs::read_to_string(path)?;
    let (mut ok, mut bad) = (0, 0);
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() < 5 || !matches!(f[1], "arc_start" | "arc_end") {
            continue;
        }
        // "fx,fy->tx,ty off [a, b, c, d] before [pts] after ..."
        let args = f[3];
        let (ends, rest) = args.split_once(" off [").unwrap();
        let (from, to) = ends.split_once("->").unwrap();
        let (off, rest) = rest.split_once("] before [").unwrap();
        let o: Vec<i32> = off.split(',').map(|v| v.trim().parse().unwrap_or(0)).collect();
        let (before, _) = rest.split_once("] after").unwrap();
        let mut e = SmoothEdge {
            from: pt(from),
            to: pt(to),
            points: pts(before),
            offsets: [(o[0], o[1]), (o[2], o[3])],
            width: 0,
        };
        if f[1] == "arc_start" {
            arc_start(&mut e);
        } else {
            arc_end(&mut e);
        }
        let want = pts(f[4]);
        if e.points == want {
            ok += 1;
        } else {
            bad += 1;
            if bad <= 6 {
                println!("{} {}: ours {:?}\n  client {:?}", f[1], ends, e.points, want);
            }
        }
    }
    println!("arc steps: {ok} identical, {bad} different");
    Ok(())
}
