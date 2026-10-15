//! Protocol knowledge from the client itself.
//!
//! **Field tracer.** While a message is decoded (inside the packet capture's message-reader
//! hook) or encoded (inside the session send), every call of the stream primitives is recorded:
//! `STREAM_READ (stream, dst, len)` at the message offset `stream+0x278` (bytes read so far),
//! `STREAM_WRITE (stream, src, len)` at the send-buffer offset `+0x258` minus the message start.
//! That gives each field's offset and size, and the caller (return address) is inside the
//! message's deserializer or serializer.
//!
//! **Per opcode** ([`observe`]): the functions that read or write it, the field layouts seen
//! (variable messages have several), sizes, and the last bodies for the analyzer
//! ([`analyze`]), which classifies each field over the samples (constant, ranges, plausible
//! floats, strings).
//!
//! Exports (session folder, with the packet files): `protocol.tsv` (per opcode: functions,
//! most common layout, sizes) and `ghidra_labels.tsv` (function → suggested name, only for
//! functions that serve exactly one opcode; applied to the Ghidra project by hand or with the
//! Ghidra MCP).

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    sync::Mutex,
};

use ilhook::x64::{HookType, Registers};
use poe_hook_shared::{mem::read, module::Module};

use crate::packets::{Channel, Dir, Msg};

const STREAM_READ: usize = 0x142197d30;
const STREAM_WRITE: usize = 0x142197e60;
/// Bodies kept per opcode for the analyzer.
const SAMPLES: usize = 200;

/// One traced primitive: message offset, size, and the calling function (Ghidra address of
/// the call's return address).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Field {
    pub offset: u32,
    pub len: u32,
    pub caller: usize,
}

struct Trace {
    stream: usize,
    out: bool,
    base: usize,
    fields: Vec<Field>,
}

thread_local! {
    static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) };
}

static MAIN: std::sync::OnceLock<Module> = std::sync::OnceLock::new();

pub fn init(main: &Module) -> Result<(), String> {
    MAIN.set(*main).ok();
    crate::hooks::install(main.from_ghidra(STREAM_READ), HookType::JmpBack(on_read), 0)?;
    crate::hooks::install(
        main.from_ghidra(STREAM_WRITE),
        HookType::JmpBack(on_write),
        0,
    )?;
    Ok(())
}

/// Starts tracing `stream` on this thread (`base`: the send-buffer offset of the message).
pub fn begin(stream: usize, out: bool, base: usize) {
    TRACE.with(|t| {
        *t.borrow_mut() = Some(Trace {
            stream,
            out,
            base,
            fields: Vec::new(),
        })
    });
}

pub fn end() -> Vec<Field> {
    TRACE.with(|t| t.borrow_mut().take().map(|t| t.fields).unwrap_or_default())
}

fn caller(rsp: u64) -> usize {
    let ret = read::<usize>(rsp as usize).unwrap_or(0);
    match MAIN.get() {
        Some(m) if m.contains(ret) => m.to_ghidra(ret),
        _ => 0,
    }
}

unsafe extern "win64" fn on_read(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("protocol::on_read");
    let r = unsafe { &*regs };
    let stream = r.rcx as usize;
    TRACE.with(|t| {
        let mut t = t.borrow_mut();
        let Some(t) = t.as_mut().filter(|t| !t.out && t.stream == stream) else {
            return;
        };
        if read::<u8>(stream + 0x252) != Some(0) {
            return;
        }
        let offset = read::<usize>(stream + 0x278).unwrap_or(0);
        t.fields.push(Field {
            offset: offset as u32,
            len: r.r8 as u32,
            caller: caller(r.rsp),
        });
    });
}

unsafe extern "win64" fn on_write(regs: *mut Registers, _: usize) {
    let _guard = crate::hook_guard!("protocol::on_write");
    let r = unsafe { &*regs };
    let stream = r.rcx as usize;
    TRACE.with(|t| {
        let mut t = t.borrow_mut();
        let Some(t) = t.as_mut().filter(|t| t.out && t.stream == stream) else {
            return;
        };
        let at = read::<usize>(stream + 0x258).unwrap_or(0);
        t.fields.push(Field {
            offset: at.wrapping_sub(t.base) as u32,
            len: r.r8 as u32,
            caller: caller(r.rsp),
        });
    });
}

// ---- per-opcode knowledge ----------------------------------------------------------------

pub type Key = (Channel, Dir, u16);

#[derive(Default, Clone)]
pub struct OpInfo {
    pub count: u64,
    /// Functions (starts) that read or wrote fields of this message, with call counts.
    pub functions: HashMap<usize, u64>,
    /// Field layouts (offset, size) seen, with counts.
    pub layouts: HashMap<Vec<(u32, u32)>, u64>,
    pub min_len: usize,
    pub max_len: usize,
    /// Last message bodies (opcode included).
    pub samples: VecDeque<Vec<u8>>,
}

impl OpInfo {
    pub fn main_layout(&self) -> Option<&Vec<(u32, u32)>> {
        self.layouts.iter().max_by_key(|(_, c)| **c).map(|(l, _)| l)
    }

    /// The function that handled most fields (the deserializer or serializer), ignoring shared
    /// helpers (string readers and the like: used by more than two opcodes).
    pub fn main_function(&self) -> Option<usize> {
        let shared = SHARED.lock().unwrap();
        self.functions
            .iter()
            .filter(|(f, _)| !shared.contains(f))
            .max_by_key(|(_, c)| **c)
            .or_else(|| self.functions.iter().max_by_key(|(_, c)| **c))
            .map(|(f, _)| *f)
    }
}

/// Functions seen in more than two opcodes (generic readers/writers), refreshed by
/// [`snapshot`].
static SHARED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

static OPS: Mutex<Option<HashMap<Key, OpInfo>>> = Mutex::new(None);

/// Adds a captured message to the per-opcode knowledge.
pub fn observe(m: &Msg) {
    let mut ops = OPS.lock().unwrap();
    let info = ops
        .get_or_insert_with(HashMap::new)
        .entry((m.channel, m.dir, m.opcode))
        .or_default();
    info.count += 1;
    info.min_len = if info.count == 1 {
        m.len
    } else {
        info.min_len.min(m.len)
    };
    info.max_len = info.max_len.max(m.len);
    if !m.fields.is_empty() {
        let layout: Vec<(u32, u32)> = m.fields.iter().map(|f| (f.offset, f.len)).collect();
        *info.layouts.entry(layout).or_default() += 1;
        for f in &m.fields {
            if let Some(start) = crate::symbols::function_start(f.caller) {
                *info.functions.entry(start).or_default() += 1;
            }
        }
    }
    if !m.redacted && m.framed {
        if info.samples.len() >= SAMPLES {
            info.samples.pop_front();
        }
        info.samples.push_back(m.data.clone());
    }
}

pub fn snapshot() -> Vec<(Key, OpInfo)> {
    let ops = OPS.lock().unwrap();
    let mut v: Vec<_> = ops
        .as_ref()
        .map(|m| m.iter().map(|(k, v)| (*k, v.clone())).collect())
        .unwrap_or_default();
    v.sort_by_key(|(k, _)| *k);
    let mut uses: HashMap<usize, usize> = HashMap::new();
    for (_, info) in &v {
        for f in info.functions.keys() {
            *uses.entry(*f).or_default() += 1;
        }
    }
    *SHARED.lock().unwrap() = uses
        .into_iter()
        .filter(|(_, n)| *n > 2)
        .map(|(f, _)| f)
        .collect();
    v
}

/// How one field behaves across the samples.
pub struct FieldStats {
    pub offset: u32,
    pub len: u32,
    pub samples: usize,
    pub distinct: usize,
    pub kind: String,
    pub example: String,
}

/// Classifies each field of `layout` over `samples` (big-endian, as the protocol is).
pub fn analyze(layout: &[(u32, u32)], samples: &VecDeque<Vec<u8>>) -> Vec<FieldStats> {
    layout
        .iter()
        .map(|&(offset, len)| {
            let (o, l) = (offset as usize, len as usize);
            let values: Vec<&[u8]> = samples.iter().filter_map(|s| s.get(o..o + l)).collect();
            let mut distinct: Vec<&[u8]> = values.clone();
            distinct.sort();
            distinct.dedup();
            let int = |b: &[u8]| -> u64 { b.iter().fold(0u64, |a, &x| (a << 8) | x as u64) };
            let kind = if values.is_empty() {
                "-".to_string()
            } else if distinct.len() == 1 {
                format!("constant {}", crate::util::hex(distinct[0], 16))
            } else if l <= 8 && l != 3 {
                let ints: Vec<u64> = values.iter().map(|b| int(b)).collect();
                let (lo, hi) = (ints.iter().min().unwrap(), ints.iter().max().unwrap());
                let mut k = format!("u{} {lo}..{hi}", l * 8);
                if l == 4 {
                    let floats: Vec<f32> = ints.iter().map(|&v| f32::from_bits(v as u32)).collect();
                    let plausible = floats
                        .iter()
                        .all(|f| f.is_finite() && (*f == 0.0 || (f.abs() > 1e-6 && f.abs() < 1e7)));
                    if plausible {
                        let (flo, fhi) = floats
                            .iter()
                            .fold((f32::MAX, f32::MIN), |(a, b), &f| (a.min(f), b.max(f)));
                        k += &format!(" | f32 {flo:.3}..{fhi:.3}");
                    }
                }
                k
            } else if values
                .iter()
                .all(|b| b.iter().all(|&c| (0x20..0x7f).contains(&c)))
            {
                "ascii text".into()
            } else if l % 2 == 0
                && values.iter().all(|b| {
                    b.chunks(2)
                        .all(|c| c[1] == 0 && (0x20..0x7f).contains(&c[0]))
                })
            {
                "utf-16 text".into()
            } else {
                format!("{l} bytes, varying")
            };
            let example = values
                .last()
                .map(|b| crate::util::hex(b, 12))
                .unwrap_or_default();
            FieldStats {
                offset,
                len,
                samples: values.len(),
                distinct: distinct.len(),
                kind,
                example,
            }
        })
        .collect()
}

pub fn layout_text(layout: &[(u32, u32)]) -> String {
    layout
        .iter()
        .map(|(o, l)| format!("{o}:{l}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Writes `protocol.tsv` and `ghidra_labels.tsv` into the session folder.
pub fn export() {
    let ops = snapshot();
    if ops.is_empty() {
        return;
    }
    let dir = crate::host::session_dir();
    let mut tsv = String::from(
        "channel\tdir\topcode\tname\tcount\tmin_len\tmax_len\tfunction\tlayout\tlayouts\n",
    );
    let mut labels = String::from("# function\tsuggested name (from the DLL's field tracer)\n");
    let mut uses: HashMap<usize, usize> = HashMap::new();
    for (_, info) in &ops {
        for f in info.functions.keys() {
            *uses.entry(*f).or_default() += 1;
        }
    }
    for ((c, d, op), info) in &ops {
        let name = crate::packets::name(*c, *d, *op).unwrap_or("");
        let func = info.main_function();
        tsv.push_str(&format!(
            "{}\t{}\t{op:#06x}\t{name}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            c.name(),
            d.name(),
            info.count,
            info.min_len,
            info.max_len,
            func.map(crate::symbols::fmt).unwrap_or_default(),
            info.main_layout()
                .map(|l| layout_text(l))
                .unwrap_or_default(),
            info.layouts.len()
        ));
        if let Some(f) = func
            && crate::symbols::name_of(f).is_none()
            && uses.get(&f) == Some(&1)
        {
            let side = match d {
                Dir::In => "Read_S2C",
                Dir::Out => "Write_C2S",
            };
            let label = if name.is_empty() {
                format!("{side}_{}_{op:04x}", c.name())
            } else {
                format!("{side}_{}_{op:04x}_{name}", c.name())
            };
            labels.push_str(&format!("{f:x}\t{label}\n"));
        }
    }
    let _ = std::fs::write(dir.join("protocol.tsv"), tsv);
    let _ = std::fs::write(dir.join("ghidra_labels.tsv"), labels);
}
