//! Memory watches: user-entered address expressions read every frame (read-only, guarded).
//!
//! Syntax: numbers (hex `0x..` or decimal), names, `+`/`-`, and `[expr]` to read a pointer.
//! Numbers from `0x140000000` up are Ghidra addresses and are moved to the loaded image.
//! Names: `player` (the player entity), `exe` (image base), `pos` / `actor` (the player's
//! Positioned / Actor component). Example: `[player+0x10]` or `pos+0x44c`.
//! The list is saved in `logs/poe-hook/watches.txt` (`type<TAB>expression` per line).

use poe_hook_shared::mem::read;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    U8,
    U16,
    U32,
    U64,
    I32,
    F32,
    F64,
    Ptr,
}

pub const KINDS: [Kind; 8] = [
    Kind::U8,
    Kind::U16,
    Kind::U32,
    Kind::U64,
    Kind::I32,
    Kind::F32,
    Kind::F64,
    Kind::Ptr,
];

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::U8 => "u8",
            Kind::U16 => "u16",
            Kind::U32 => "u32",
            Kind::U64 => "u64",
            Kind::I32 => "i32",
            Kind::F32 => "f32",
            Kind::F64 => "f64",
            Kind::Ptr => "ptr",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        KINDS.into_iter().find(|k| k.name() == s)
    }
}

#[derive(Clone)]
pub struct Watch {
    pub expr: String,
    pub kind: Kind,
}

fn path() -> std::path::PathBuf {
    poe_hook_shared::exe_dir()
        .join("logs")
        .join("poe-hook")
        .join("watches.txt")
}

pub fn load() -> Vec<Watch> {
    std::fs::read_to_string(path())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (k, e) = l.split_once('\t')?;
            Some(Watch {
                expr: e.to_owned(),
                kind: Kind::parse(k)?,
            })
        })
        .collect()
}

pub fn save(list: &[Watch]) {
    let text: String = list
        .iter()
        .map(|w| format!("{}\t{}\n", w.kind.name(), w.expr))
        .collect();
    let _ = std::fs::write(path(), text);
}

/// The address `expr` points to now.
pub fn address(expr: &str) -> Result<usize, String> {
    let tokens = tokenize(expr)?;
    let mut p = Parser { tokens, at: 0 };
    let v = p.expr()?;
    if p.at != p.tokens.len() {
        return Err("unexpected text".into());
    }
    Ok(v)
}

/// The value as text (or why it cannot be read).
pub fn value(w: &Watch) -> String {
    let addr = match address(&w.expr) {
        Ok(a) => a,
        Err(e) => return format!("error: {e}"),
    };
    let v = match w.kind {
        Kind::U8 => read::<u8>(addr).map(|v| format!("{v} ({v:#04x})")),
        Kind::U16 => read::<u16>(addr).map(|v| format!("{v} ({v:#06x})")),
        Kind::U32 => read::<u32>(addr).map(|v| format!("{v} ({v:#010x})")),
        Kind::U64 => read::<u64>(addr).map(|v| format!("{v} ({v:#x})")),
        Kind::I32 => read::<i32>(addr).map(|v| v.to_string()),
        Kind::F32 => read::<f32>(addr).map(|v| format!("{v}")),
        Kind::F64 => read::<f64>(addr).map(|v| format!("{v}")),
        Kind::Ptr => read::<usize>(addr).map(|v| {
            match crate::dump_types::main_module().filter(|m| m.contains(v)) {
                Some(m) => format!("{v:#x} (ghidra {:#x})", m.to_ghidra(v)),
                None => format!("{v:#x}"),
            }
        }),
    };
    match v {
        Some(v) => format!("{v}   @ {addr:#x}"),
        None => format!("unreadable @ {addr:#x}"),
    }
}

#[derive(Debug, PartialEq)]
enum Tok {
    Num(usize),
    Name(String),
    Plus,
    Minus,
    Open,
    Close,
}

fn tokenize(s: &str) -> Result<Vec<Tok>, String> {
    let mut out = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' => i += 1,
            '+' => {
                out.push(Tok::Plus);
                i += 1
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1
            }
            '[' => {
                out.push(Tok::Open);
                i += 1
            }
            ']' => {
                out.push(Tok::Close);
                i += 1
            }
            _ if c.is_ascii_alphanumeric() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                let num = match word.strip_prefix("0x") {
                    Some(hex) => usize::from_str_radix(hex, 16).ok(),
                    None if word.chars().all(|c| c.is_ascii_digit()) => word.parse().ok(),
                    None => None,
                };
                out.push(match num {
                    Some(n) => Tok::Num(n),
                    None => Tok::Name(word),
                });
            }
            _ => return Err(format!("unexpected '{c}'")),
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn expr(&mut self) -> Result<usize, String> {
        let mut v = self.term()?;
        loop {
            match self.tokens.get(self.at) {
                Some(Tok::Plus) => {
                    self.at += 1;
                    v = v.wrapping_add(self.term()?);
                }
                Some(Tok::Minus) => {
                    self.at += 1;
                    v = v.wrapping_sub(self.term()?);
                }
                _ => return Ok(v),
            }
        }
    }

    fn term(&mut self) -> Result<usize, String> {
        let t = self.tokens.get(self.at).ok_or("expression ends early")?;
        self.at += 1;
        match t {
            Tok::Num(n) => Ok(match crate::dump_types::main_module() {
                Some(m) if (0x1_4000_0000..0x1_4000_0000 + m.size).contains(n) => m.from_ghidra(*n),
                _ => *n,
            }),
            Tok::Name(name) => named(name),
            Tok::Open => {
                let inner = self.expr()?;
                if self.tokens.get(self.at) != Some(&Tok::Close) {
                    return Err("missing ']'".into());
                }
                self.at += 1;
                read::<usize>(inner).ok_or_else(|| format!("cannot read {inner:#x}"))
            }
            _ => Err("expected a value".into()),
        }
    }
}

fn named(name: &str) -> Result<usize, String> {
    let player = crate::dump_types::player();
    let component = |i: usize| {
        read::<usize>(player + 0x10)
            .and_then(|c| read::<usize>(c + i * 8))
            .ok_or_else(|| "no player".to_string())
    };
    match name {
        "player" if player != 0 => Ok(player),
        "player" => Err("no player".into()),
        "exe" => crate::dump_types::main_module()
            .map(|m| m.base)
            .ok_or_else(|| "no module".into()),
        "pos" => component(0),
        "actor" => component(9),
        _ => Err(format!("unknown name {name}")),
    }
}
