//! Memory change scanner: a region of game memory copied every overlay frame; each byte
//! remembers when it last changed and how often, so fields that react to an action (sprint,
//! attack, equip) light up. Read-only (guarded reads).

use std::time::Instant;

pub const MAX_LEN: usize = 0x2000;

pub struct Region {
    pub expr: String,
    pub len: usize,
    pub addr: Option<usize>,
    pub error: Option<String>,
    pub bytes: Vec<u8>,
    pub baseline: Vec<u8>,
    pub changed_at: Vec<Option<Instant>>,
    pub changes: Vec<u32>,
}

impl Region {
    pub fn new(expr: String, len: usize) -> Self {
        let len = len.clamp(16, MAX_LEN);
        Self {
            expr,
            len,
            addr: None,
            error: None,
            bytes: Vec::new(),
            baseline: Vec::new(),
            changed_at: vec![None; len],
            changes: vec![0; len],
        }
    }

    /// Reads the region again and records changes.
    pub fn update(&mut self) {
        let addr = match crate::watches::address(&self.expr) {
            Ok(a) => a,
            Err(e) => {
                self.error = Some(e);
                return;
            }
        };
        if self.addr != Some(addr) {
            // Another object: start over.
            self.addr = Some(addr);
            self.bytes.clear();
            self.baseline.clear();
            self.changed_at = vec![None; self.len];
            self.changes = vec![0; self.len];
        }
        if !poe_hook_shared::mem::readable(addr, self.len) {
            self.error = Some(format!("{addr:#x}: not readable"));
            return;
        }
        self.error = None;
        let now: Vec<u8> =
            unsafe { std::slice::from_raw_parts(addr as *const u8, self.len) }.to_vec();
        if self.bytes.len() == now.len() {
            let t = Instant::now();
            for (i, (a, b)) in self.bytes.iter().zip(&now).enumerate() {
                if a != b {
                    self.changed_at[i] = Some(t);
                    self.changes[i] = self.changes[i].saturating_add(1);
                }
            }
        }
        if self.baseline.is_empty() {
            self.baseline = now.clone();
        }
        self.bytes = now;
    }

    pub fn set_baseline(&mut self) {
        self.baseline = self.bytes.clone();
        self.changes = vec![0; self.len];
        self.changed_at = vec![None; self.len];
    }

    /// Whether the 16-byte row starting at `row` differs from the baseline.
    pub fn row_differs(&self, row: usize) -> bool {
        (row..(row + 16).min(self.bytes.len())).any(|i| self.baseline.get(i) != self.bytes.get(i))
    }
}
