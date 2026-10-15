//! Checks our message layouts against a capture from the client DLL (`packets.jsonl`): every
//! message we have a decoder for is decoded; failures and bytes left over mean our layout is
//! wrong for that message. Official captures are the reference.
//!
//! cargo run -p poe-proto --example check_capture -- <protocol dir> <packets.jsonl> [--show N]

use std::collections::BTreeMap;

use poe_proto::{Direction, OpcodeTable, describe::describe};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len() / 2)
        .filter_map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = std::path::Path::new(&args[1]);
    // --dump <channel> <in|out> <0xop>: print the decoded messages of one opcode instead.
    let dump = args.iter().position(|a| a == "--dump").map(|i| {
        (
            args[i + 1].clone(),
            args[i + 2].clone(),
            args[i + 3].clone(),
        )
    });
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1)?.parse().ok())
        .unwrap_or(1);
    let tables = [
        (
            "login",
            OpcodeTable::parse(
                "login",
                &std::fs::read_to_string(dir.join("login.toml")).unwrap(),
            )
            .unwrap(),
        ),
        (
            "instance",
            OpcodeTable::parse(
                "instance",
                &std::fs::read_to_string(dir.join("instance.toml")).unwrap(),
            )
            .unwrap(),
        ),
    ];
    // (channel, dir, op, name) -> (ok, errors, trailing, examples)
    let mut stats: BTreeMap<(String, String, String, String), (u32, u32, u32, Vec<String>)> =
        BTreeMap::new();
    for line in std::fs::read_to_string(&args[2]).unwrap().lines() {
        let Ok(m) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if m["redacted"] == true || m["framed"] == false {
            continue;
        }
        let chan = m["chan"].as_str().unwrap_or("");
        let d = m["dir"].as_str().unwrap_or("");
        let op = m["op"].as_str().unwrap_or("");
        let Some((_, table)) = tables.iter().find(|(c, _)| *c == chan) else {
            continue;
        };
        let direction = if d == "in" {
            Direction::Server
        } else {
            Direction::Client
        };
        let id = u16::from_str_radix(op.trim_start_matches("0x"), 16).unwrap_or(0);
        let Some(name) = table.name(direction, id) else {
            continue;
        };
        let data = hex(m["hex"].as_str().unwrap_or(""));
        let Some(desc) = describe(name, direction, data.get(2..).unwrap_or(&[])) else {
            continue;
        };
        if let Some((dc, dd, dop)) = &dump {
            if dc == chan && dd == d && dop == op {
                println!(
                    "#{} {}\n{}\n",
                    m["seq"],
                    desc.text.as_deref().unwrap_or_else(|e| e),
                    if desc.trailing > 0 {
                        format!("({} bytes left)", desc.trailing)
                    } else {
                        String::new()
                    }
                );
            }
            continue;
        }
        let e = stats
            .entry((chan.into(), d.into(), op.into(), name.into()))
            .or_default();
        match (&desc.text, desc.trailing) {
            (Ok(_), 0) => e.0 += 1,
            (Ok(_), n) => {
                e.2 += 1;
                if e.3.len() < show {
                    e.3.push(format!(
                        "{n} bytes left: {}",
                        m["hex"].as_str().unwrap_or("")
                    ));
                }
            }
            (Err(err), _) => {
                e.1 += 1;
                if e.3.len() < show {
                    e.3.push(format!("error {err}: {}", m["hex"].as_str().unwrap_or("")));
                }
            }
        }
    }
    for ((c, d, op, name), (ok, err, trail, ex)) in stats {
        let mark = if err + trail > 0 { "MISMATCH" } else { "ok" };
        println!("{mark:8} {c:8} {d:3} {op} {name:24} ok {ok:5} errors {err:4} leftover {trail:4}");
        for e in ex {
            println!("           {}", &e[..e.len().min(220)]);
        }
    }
}
