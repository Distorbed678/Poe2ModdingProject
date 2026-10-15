//! Scanner tab: watch a memory region change live. Bytes that just changed are red, recently
//! changed ones fade through orange, bytes that differ from the baseline are yellow.
//! "Rows that changed" hides the rest. Other tabs send a region here (component inspector).

use std::{sync::Mutex, time::Instant};

use imgui::{ListClipper, Ui};

use crate::scanner::Region;

/// A region requested by another tab (address expression, size).
static REQUEST: Mutex<Option<(String, usize)>> = Mutex::new(None);

pub fn request(expr: String, len: usize) {
    *REQUEST.lock().unwrap() = Some((expr, len));
}

pub struct ScannerView {
    expr: String,
    len: i32,
    region: Option<Region>,
    changed_only: bool,
    /// Columns after the hex: 0 none, 1 u32, 2 f32, 3 pointers.
    view: usize,
}

impl ScannerView {
    pub fn new() -> Self {
        Self {
            expr: "pos".into(),
            len: 0x600,
            region: None,
            changed_only: false,
            view: 2,
        }
    }

    /// Called every drawn frame (also while another tab is shown), so changes are not missed.
    pub fn update(&mut self) {
        if let Some((expr, len)) = REQUEST.lock().unwrap().take() {
            self.expr = expr.clone();
            self.len = len as i32;
            self.region = Some(Region::new(expr, len));
        }
        if let Some(r) = self.region.as_mut() {
            r.update();
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(320.0);
        ui.input_text("address (watch syntax)", &mut self.expr)
            .build();
        ui.same_line();
        ui.set_next_item_width(120.0);
        ui.input_int("size", &mut self.len);
        ui.same_line();
        if ui.button("Watch") {
            self.region = Some(Region::new(self.expr.clone(), self.len.max(16) as usize));
        }
        let Some(r) = self.region.as_mut() else {
            ui.text_disabled(
                "Enter an address (e.g. pos, actor, [player+0x10]+0x8) and press Watch.",
            );
            return;
        };
        if ui.button("Set baseline") {
            r.set_baseline();
        }
        ui.same_line();
        ui.checkbox("rows that changed", &mut self.changed_only);
        ui.same_line();
        ui.set_next_item_width(120.0);
        ui.combo_simple_string("view", &mut self.view, &["hex", "u32", "f32", "pointers"]);
        if let Some(e) = &r.error {
            ui.text_colored([1.0, 0.4, 0.4, 1.0], e);
            return;
        }
        ui.text_disabled(format!(
            "{:#x}, {} bytes; red = just changed, orange = changed recently, yellow = differs from baseline",
            r.addr.unwrap_or(0),
            r.len
        ));
        let rows: Vec<usize> = (0..r.bytes.len())
            .step_by(16)
            .filter(|&row| !self.changed_only || r.row_differs(row))
            .collect();
        let now = Instant::now();
        ui.child_window("scan_rows")
            .flags(imgui::WindowFlags::HORIZONTAL_SCROLLBAR)
            .build(ui, || {
                let mut clipper = ListClipper::new(rows.len()).begin(ui);
                while clipper.step() {
                    for i in clipper.display_start()..clipper.display_end() {
                        let row = rows[i as usize];
                        ui.text_disabled(format!("+{row:04x}"));
                        for b in row..(row + 16).min(r.bytes.len()) {
                            ui.same_line();
                            let age = r.changed_at[b].map(|t| now.duration_since(t).as_secs_f32());
                            let color = match age {
                                Some(a) if a < 0.5 => [1.0, 0.25, 0.25, 1.0],
                                Some(a) if a < 3.0 => [1.0, 0.6 + a * 0.1, 0.3, 1.0],
                                _ if r.baseline.get(b) != r.bytes.get(b) => [1.0, 0.9, 0.3, 1.0],
                                _ => [0.7, 0.7, 0.7, 1.0],
                            };
                            ui.text_colored(color, format!("{:02x}", r.bytes[b]));
                            if ui.is_item_hovered() {
                                ui.tooltip_text(describe(&r.bytes, b, r.changes[b]));
                            }
                        }
                        let words: Vec<String> = match self.view {
                            1 => chunks::<4>(&r.bytes, row)
                                .map(|c| u32::from_le_bytes(c).to_string())
                                .collect(),
                            2 => chunks::<4>(&r.bytes, row)
                                .map(|c| format!("{:.3}", f32::from_le_bytes(c)))
                                .collect(),
                            3 => chunks::<8>(&r.bytes, row)
                                .map(|c| {
                                    let v = usize::from_le_bytes(c);
                                    match crate::dump_types::main_module().filter(|m| m.contains(v))
                                    {
                                        Some(m) => crate::symbols::fmt(m.to_ghidra(v)),
                                        None => format!("{v:#x}"),
                                    }
                                })
                                .collect(),
                            _ => Vec::new(),
                        };
                        if !words.is_empty() {
                            ui.same_line();
                            ui.text_disabled(format!("  {}", words.join("  ")));
                        }
                    }
                }
            });
    }
}

fn chunks<const N: usize>(bytes: &[u8], row: usize) -> impl Iterator<Item = [u8; N]> + '_ {
    (row..row + 16)
        .step_by(N)
        .filter_map(move |o| bytes.get(o..o + N)?.try_into().ok())
}

fn describe(bytes: &[u8], at: usize, changes: u32) -> String {
    let take = |n: usize| bytes.get(at..at + n);
    let mut s = format!("+{at:#x}, changed {changes} times\n");
    if let Some(b) = take(2) {
        s += &format!("u16 {}\n", u16::from_le_bytes([b[0], b[1]]));
    }
    if let Some(b) = take(4) {
        let a = [b[0], b[1], b[2], b[3]];
        s += &format!(
            "u32 {} / i32 {} / f32 {}\n",
            u32::from_le_bytes(a),
            i32::from_le_bytes(a),
            f32::from_le_bytes(a)
        );
    }
    if let Some(b) = take(8) {
        let a: [u8; 8] = b.try_into().unwrap();
        s += &format!(
            "u64 {:#x} / f64 {}",
            u64::from_le_bytes(a),
            f64::from_le_bytes(a)
        );
    }
    s
}
