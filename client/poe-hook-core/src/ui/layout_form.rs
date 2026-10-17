//! Area picker with seed, topology and orientation: the arguments of the server's
//! `transfer` / `instance_new` commands (Instances tab, Commands tab).

use imgui::{ListClipper, Ui};

const KINDS: [&str; 5] = ["all", "town", "area", "map", "hideout"];

pub struct LayoutForm {
    search: String,
    kind: usize,
    /// 0 = every act.
    act: i32,
    pub area: String,
    pub seed: String,
    /// 0 = natural (the client's choice), else 1-based topology.
    topology: usize,
    /// 0 = natural, else orientation + 1.
    orientation: usize,
}

/// A random seed (no RNG crate in the core: time and a counter, mixed).
fn random_seed() -> u32 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let mut x = poe_hook_shared::unix_ms() ^ N.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x as u32).max(1)
}

impl LayoutForm {
    pub fn new() -> Self {
        Self { search: String::new(), kind: 0, act: 0, area: String::new(), seed: String::new(), topology: 0, orientation: 0 }
    }

    /// Area, seed, topology and orientation as command arguments (`random` / `natural` for
    /// the server's or client's own choice).
    pub fn args(&self) -> String {
        let seed = match self.seed.trim().parse::<u32>() {
            Ok(s) if s != 0 => s.to_string(),
            _ => "random".into(),
        };
        let topology = if self.topology == 0 { "natural".to_string() } else { self.topology.to_string() };
        let orientation = if self.orientation == 0 { "natural".to_string() } else { (self.orientation - 1).to_string() };
        format!("{} {seed} {topology} {orientation}", self.area)
    }

    pub fn has_area(&self) -> bool {
        !self.area.trim().is_empty()
    }

    pub fn draw(&mut self, ui: &Ui, id: &str) {
        let _id = ui.push_id(id);
        let areas = crate::areas::all();
        if areas.is_empty() {
            ui.text_disabled("No area list: run `cargo xtask dll-data` (or deploy-client), then reload the core.");
        }
        ui.set_next_item_width(200.0);
        ui.input_text("search", &mut self.search).hint("id or name").build();
        ui.same_line();
        ui.set_next_item_width(90.0);
        ui.combo_simple_string("kind", &mut self.kind, &KINDS);
        ui.same_line();
        ui.set_next_item_width(90.0);
        ui.input_int("act (0 = all)", &mut self.act);
        let recent = crate::areas::recent();
        if !recent.is_empty() {
            ui.text_disabled("recent:");
            for r in &recent {
                ui.same_line();
                if ui.small_button(r) {
                    self.select(r);
                }
            }
        }
        let f = self.search.to_lowercase();
        let shown: Vec<&crate::areas::Area> = areas
            .iter()
            .filter(|a| self.kind == 0 || a.kind == KINDS[self.kind])
            .filter(|a| self.act <= 0 || a.act == self.act as i64)
            .filter(|a| f.is_empty() || a.id.to_lowercase().contains(&f) || a.name.to_lowercase().contains(&f))
            .collect();
        ui.child_window("areas").size([0.0, 170.0]).border(true).build(ui, || {
            let mut clipper = ListClipper::new(shown.len()).begin(ui);
            while clipper.step() {
                for i in clipper.display_start()..clipper.display_end() {
                    let a = shown[i as usize];
                    let label = format!(
                        "{:22} {:36} act {:2} lvl {:3} {:7} {} topologies##{}",
                        a.id,
                        a.name,
                        a.act,
                        a.level,
                        a.kind,
                        a.topologies.len(),
                        a.id
                    );
                    if ui.selectable_config(label).selected(a.id.eq_ignore_ascii_case(&self.area)).build() {
                        let id = a.id.clone();
                        self.select(&id);
                    }
                }
            }
        });
        ui.text_disabled(format!("{} of {} areas", shown.len(), areas.len()));
        ui.set_next_item_width(200.0);
        ui.input_text("area", &mut self.area).build();
        ui.same_line();
        ui.set_next_item_width(130.0);
        ui.input_text("seed", &mut self.seed).hint("random").build();
        ui.same_line();
        if ui.button("Random seed") {
            self.seed = random_seed().to_string();
        }
        ui.same_line();
        if ui.button("Server picks") {
            self.seed.clear();
        }
        let area = crate::areas::find(&self.area);
        let topologies: Vec<(String, u8)> = area.map(|a| a.topologies.clone()).unwrap_or_default();
        let mut names = vec!["natural (client's choice)".to_string()];
        names.extend(topologies.iter().enumerate().map(|(i, (g, _))| format!("{} {g}", i + 1)));
        if self.topology >= names.len() {
            self.topology = 0;
        }
        ui.set_next_item_width(260.0);
        ui.combo_simple_string("topology", &mut self.topology, &names);
        ui.same_line();
        let mask = if self.topology == 0 { 0xff } else { topologies[self.topology - 1].1 };
        let mut orients = vec!["natural".to_string()];
        orients.extend((0..8u8).map(|o| {
            let allowed = mask == 0 || mask == 0xff || mask >> o & 1 != 0;
            format!("{o}{}", if allowed { "" } else { " (not allowed)" })
        }));
        ui.set_next_item_width(160.0);
        ui.combo_simple_string("orientation", &mut self.orientation, &orients);
    }

    fn select(&mut self, id: &str) {
        if !self.area.eq_ignore_ascii_case(id) {
            self.topology = 0;
            self.orientation = 0;
        }
        self.area = id.to_owned();
    }

    /// Records the area as recently used (after a command was sent with it).
    pub fn used(&self) {
        if self.has_area() {
            crate::areas::remember(self.area.trim());
        }
    }
}
