//! Server and Desync tabs (our server only): the instance server's simulation state for this
//! session, dev commands, and the client's position against the server's prediction.

use dear_implot::{ImPlotExt, LinePlot, Plot, PlotCond, PlotContext};
use imgui::Ui;

pub struct ServerView {
    tx: i32,
    ty: i32,
    turn_cell: usize,
    seconds: i32,
}

impl ServerView {
    pub fn new() -> Self {
        Self {
            tx: 0,
            ty: 0,
            turn_cell: 2,
            seconds: 10,
        }
    }

    pub fn draw_server(&mut self, ui: &Ui) {
        if crate::host::read_only() {
            ui.text_disabled("Read-only: the debug link talks to our server only.");
            return;
        }
        let Some((age, values)) = crate::devlink::state() else {
            ui.text_disabled(
                "No state from the server yet (needs position sync and [instance] debug_link).",
            );
            return;
        };
        ui.text(format!("last state {} ms ago", age.as_millis()));
        ui.separator();
        ui.text("Dev commands");
        ui.set_next_item_width(120.0);
        ui.input_int("x##tp", &mut self.tx);
        ui.same_line();
        ui.set_next_item_width(120.0);
        ui.input_int("y##tp", &mut self.ty);
        ui.same_line();
        if ui.button("Teleport") {
            crate::devlink::command(&format!("teleport {} {}", self.tx, self.ty));
        }
        ui.same_line();
        if ui.button("Here") {
            if let Some(p) = crate::inspect::player() {
                self.tx = p.grid.0;
                self.ty = p.grid.1;
            }
        }
        ui.set_next_item_width(160.0);
        if ui.combo_simple_string(
            "turn cell",
            &mut self.turn_cell,
            &["own", "heading", "natural"],
        ) {
            let mode = ["own", "heading", "natural"][self.turn_cell];
            crate::devlink::command(&format!("turn_cell {mode}"));
        }
        let get = |k: &str| values.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let mut timing = get("turn_timing") == Some("true");
        if ui.checkbox("turn timing", &mut timing) {
            crate::devlink::command(&format!(
                "turn_timing {}",
                if timing { "on" } else { "off" }
            ));
        }
        ui.same_line();
        let mut face = get("face_cursor") == Some("true");
        if ui.checkbox("face cursor", &mut face) {
            crate::devlink::command(&format!("face_cursor {}", if face { "on" } else { "off" }));
        }
        ui.separator();
        if let Some(_t) = ui.begin_table_header_with_flags(
            "server_state",
            [
                imgui::TableColumnSetup::new("server value"),
                imgui::TableColumnSetup::new(""),
            ],
            crate::ui::table_opts(),
        ) {
            for (k, v) in &values {
                ui.table_next_column();
                ui.text(k);
                ui.table_next_column();
                ui.text(v);
            }
        }
    }

    pub fn draw_desync(&mut self, ui: &Ui, plots: &PlotContext) {
        ui.set_next_item_width(200.0);
        ui.slider("seconds##desync", 2, 120, &mut self.seconds);
        let now = poe_hook_shared::unix_ms();
        let since = now.saturating_sub(self.seconds as u64 * 1000);
        let samples = crate::devlink::samples(since);
        if samples.is_empty() {
            ui.text_disabled("No server states yet (our server, debug link).");
            return;
        }
        let t: Vec<f64> = samples
            .iter()
            .map(|s| (s.unix_ms as f64 - now as f64) / 1000.0)
            .collect();
        let dist = |f: fn(&crate::devlink::Sample) -> Option<(f32, f32)>| -> Vec<f64> {
            samples
                .iter()
                .map(|s| f(s).map_or(0.0, |p| ((p.0 - s.client.0).hypot(p.1 - s.client.1)) as f64))
                .collect()
        };
        let pred = dist(|s| s.predicted);
        let report = dist(|s| s.report);
        let over = pred
            .iter()
            .filter(|d| **d > crate::devlink::DESYNC_CELLS as f64)
            .count();
        let worst = pred.iter().cloned().fold(0.0, f64::max);
        ui.text_disabled(
            "The prediction is for the frame the server's next message applies in (one frame \
             ahead), so about one frame of walking (~1.2 cells) is expected while moving.",
        );
        ui.text(format!(
            "{} samples; prediction off by more than {} cells in {over}; worst {worst:.2} cells",
            samples.len(),
            crate::devlink::DESYNC_CELLS
        ));
        let plot = ui.implot(plots);
        let width = ui.content_region_avail()[0];
        if let Some(token) = plot.begin_plot_with_size(
            "Distance from the client's position (cells)",
            [width, 260.0],
        ) {
            plot.setup_axes_limits(
                -(self.seconds as f64),
                0.0,
                0.0,
                (worst * 1.2).max(3.0),
                PlotCond::Always,
            );
            LinePlot::new("server prediction", &t, &pred).plot(&plot);
            LinePlot::new("last report", &t, &report).plot(&plot);
            token.end();
        }
        let xs: Vec<f64> = samples.iter().map(|s| s.client.0 as f64).collect();
        let pxs: Vec<f64> = samples
            .iter()
            .map(|s| s.predicted.map_or(f64::NAN, |p| p.0 as f64))
            .collect();
        let (lo, hi) = xs
            .iter()
            .chain(pxs.iter().filter(|v| v.is_finite()))
            .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        if let Some(token) = plot.begin_plot_with_size("x (cells)", [width, 200.0]) {
            plot.setup_axes_limits(
                -(self.seconds as f64),
                0.0,
                lo - 2.0,
                hi + 2.0,
                PlotCond::Always,
            );
            LinePlot::new("client", &t, &xs).plot(&plot);
            LinePlot::new("server prediction", &t, &pxs).plot(&plot);
            token.end();
        }
    }
}
