//! Readable code addresses: `name+0x1c` for functions we named (assets/symbols.tsv, from
//! Ghidra and Documentation/re/addresses-0.5.5e.md via tools/re/export_symbols.py) and
//! `FUN_1420e6bc0+0x1c` for the rest. Function bounds come from the executable's `.pdata`
//! (the x64 unwind table lists every non-leaf function), so this works for any address.

use std::sync::OnceLock;

use poe_hook_shared::module::Module;

struct Table {
    /// (start, end) as Ghidra addresses, sorted.
    functions: Vec<(usize, usize)>,
    /// (address, name), sorted.
    names: Vec<(usize, String)>,
}

static TABLE: OnceLock<Table> = OnceLock::new();

fn table() -> &'static Table {
    TABLE.get_or_init(|| {
        let main = Module::main();
        let mut functions = pdata(&main);
        functions.sort_unstable();
        let mut names: Vec<(usize, String)> = include_str!("../assets/symbols.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (a, n) = l.split_once('\t')?;
                Some((usize::from_str_radix(a, 16).ok()?, n.to_owned()))
            })
            .collect();
        names.sort_unstable_by_key(|(a, _)| *a);
        log::info!(
            "symbols: {} functions from .pdata, {} names",
            functions.len(),
            names.len()
        );
        Table { functions, names }
    })
}

/// RUNTIME_FUNCTION entries of the exception directory (data directory 3).
fn pdata(m: &Module) -> Vec<(usize, usize)> {
    use poe_hook_shared::mem::read;
    let base = m.base;
    let Some(lfanew) = read::<u32>(base + 0x3c) else {
        return Vec::new();
    };
    // IMAGE_NT_HEADERS64: optional header at +0x18, data directories at +0x70 of it.
    let dir = base + lfanew as usize + 0x18 + 0x70 + 3 * 8;
    let (Some(rva), Some(size)) = (read::<u32>(dir), read::<u32>(dir + 4)) else {
        return Vec::new();
    };
    let count = size as usize / 12;
    let to_ghidra = |rva: u32| rva as usize + 0x1_4000_0000;
    (0..count)
        .filter_map(|i| {
            let e = base + rva as usize + i * 12;
            Some((to_ghidra(read::<u32>(e)?), to_ghidra(read::<u32>(e + 4)?)))
        })
        .collect()
}

/// The function containing a Ghidra address: its start, if `.pdata` covers it.
pub fn function_start(addr: usize) -> Option<usize> {
    let t = table();
    let i = t.functions.partition_point(|(s, _)| *s <= addr);
    let (s, e) = *t.functions.get(i.checked_sub(1)?)?;
    (addr < e).then_some(s)
}

pub fn name_of(start: usize) -> Option<&'static str> {
    let t = table();
    t.names
        .binary_search_by_key(&start, |(a, _)| *a)
        .ok()
        .map(|i| t.names[i].1.as_str())
}

/// `name+0x..`, `FUN_..+0x..`, or the plain address.
pub fn fmt(addr: usize) -> String {
    match function_start(addr) {
        Some(s) => {
            let off = addr - s;
            let name = name_of(s).map_or_else(|| format!("FUN_{s:x}"), |n| n.to_owned());
            if off == 0 {
                name
            } else {
                format!("{name}+{off:#x}")
            }
        }
        None => format!("{addr:#x}"),
    }
}

/// Game-code addresses on the stack from `rsp` up (mostly return addresses), symbolized.
pub fn stack(rsp: usize, max: usize) -> String {
    let m = Module::main();
    poe_hook_shared::module::stack_refs(&m, rsp, max)
        .into_iter()
        .map(|a| format!("{a:#x} {}", fmt(a)))
        .collect::<Vec<_>>()
        .join(" | ")
}
