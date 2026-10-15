//! Timing tab (ImPlot): frame time, messages per 100 ms in each direction and the client's
//! lockstep backlog over the last seconds. The same series go into snapshots (`timings.csv`).

use dear_implot::{ImPlotExt, LinePlot, Plot, PlotCond, PlotContext, StemPlot};
use imgui::Ui;

use crate::timing::Series;

pub struct TimingView {
    seconds: i32,
}

impl TimingView {
    pub fn new() -> Self {
        Self { seconds: 10 }
    }

    pub fn draw(&mut self, ui: &Ui, plots: &PlotContext) {
        ui.set_next_item_width(200.0);
        ui.slider("seconds", 2, 120, &mut self.seconds);
        let now = poe_hook_shared::unix_ms();
        let since = now.saturating_sub(self.seconds as u64 * 1000);
        let t = |ms: u64| (ms as f64 - now as f64) / 1000.0;
        let frame = crate::timing::samples(Series::FrameMs, since);
        let (fx, fy): (Vec<f64>, Vec<f64>) =
            frame.iter().map(|s| (t(s.unix_ms), s.value as f64)).unzip();
        let worst = fy.iter().cloned().fold(0.0, f64::max);
        let avg = if fy.is_empty() {
            0.0
        } else {
            fy.iter().sum::<f64>() / fy.len() as f64
        };
        ui.text(format!(
            "frame time: avg {avg:.2} ms, worst {worst:.2} ms over {} frames",
            fy.len()
        ));
        let plot = ui.implot(plots);
        let width = ui.content_region_avail()[0];
        let x_min = -(self.seconds as f64);
        if let Some(token) = plot.begin_plot_with_size("Frame time (ms)", [width, 220.0]) {
            plot.setup_axes_limits(x_min, 0.0, 0.0, (worst * 1.2).max(10.0), PlotCond::Always);
            LinePlot::new("frame ms", &fx, &fy).plot(&plot);
            token.end();
        }
        // Messages per 100 ms bucket.
        let buckets = (self.seconds * 10) as usize;
        let count = |series: Series| {
            let mut v = vec![0.0f64; buckets];
            for s in crate::timing::samples(series, since) {
                let i = ((s.unix_ms - since) / 100) as usize;
                if i < buckets {
                    v[i] += 1.0;
                }
            }
            v
        };
        let xs: Vec<f64> = (0..buckets).map(|i| x_min + i as f64 * 0.1).collect();
        let (inn, out) = (count(Series::PacketIn), count(Series::PacketOut));
        let top = inn.iter().chain(&out).cloned().fold(1.0, f64::max);
        if let Some(token) = plot.begin_plot_with_size("Messages per 100 ms", [width, 200.0]) {
            plot.setup_axes_limits(x_min, 0.0, 0.0, top * 1.2, PlotCond::Always);
            StemPlot::new("received", &xs, &inn).plot(&plot);
            LinePlot::new("sent", &xs, &out).plot(&plot);
            token.end();
        }
        let backlog = crate::timing::samples(Series::Backlog, since);
        let (bx, by): (Vec<f64>, Vec<f64>) = backlog
            .iter()
            .map(|s| (t(s.unix_ms), s.value as f64))
            .unzip();
        let top = by.iter().cloned().fold(2.0, f64::max);
        if let Some(token) = plot.begin_plot_with_size(
            "Lockstep backlog (frames queued in the world)",
            [width, 200.0],
        ) {
            plot.setup_axes_limits(x_min, 0.0, 0.0, top * 1.2, PlotCond::Always);
            LinePlot::new("frames", &bx, &by).plot(&plot);
            token.end();
        }
    }
}
