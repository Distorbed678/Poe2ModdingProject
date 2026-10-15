//! Minimap tab: the client's cell grids around the player (shade = the 4-bit value), the
//! player and its facing, and the server's idea of the player (our server). Which grid is
//! walkability is not settled: the player was seen standing on grid 0 = 0 with grids 1-3 = 5. "Export" writes the area's
//! layout and grids (`terrain_layout_*.tsv`, `terrain_grid_*.bin`) for validating our terrain
//! generator (M5), also on official servers.

use imgui::Ui;

use crate::minimap::Grid;

pub struct MinimapView {
    layer: usize,
    radius: i32,
    cell_px: f32,
}

impl MinimapView {
    pub fn new() -> Self {
        Self {
            layer: 1,
            radius: 60,
            cell_px: 4.0,
        }
    }

    pub fn draw(&mut self, ui: &Ui) {
        ui.set_next_item_width(140.0);
        ui.combo_simple_string("grid", &mut self.layer, &["0", "1", "2", "3"]);
        ui.same_line();
        ui.set_next_item_width(160.0);
        ui.slider("radius (cells)", 20, 200, &mut self.radius);
        ui.same_line();
        ui.set_next_item_width(140.0);
        ui.slider("cell px", 1.0, 10.0, &mut self.cell_px);
        ui.same_line();
        if ui.button("Export area") {
            let ok = crate::dump_types::export_layout_now();
            crate::ui::notice(match crate::terrain_tools::export_now() {
                Some(dir) => format!(
                    "Area exported to {} (with server entities and walkability probe)",
                    dir.display()
                ),
                None if ok => "Area layout and grids exported next to the exe (terrain_*)".into(),
                None => "No area loaded".to_string(),
            });
        }
        let Some(grid) = Grid::current(self.layer) else {
            ui.text_disabled("No terrain (not in an area yet).");
            return;
        };
        let Some(p) = crate::inspect::player() else {
            ui.text_disabled("No player yet.");
            return;
        };
        let (px, py) = (p.grid.0 as i64, p.grid.1 as i64);
        let here: Vec<String> = (0..4)
            .map(|l| {
                Grid::current(l)
                    .and_then(|g| g.get(px, py))
                    .map_or("-".into(), |v| v.to_string())
            })
            .collect();
        ui.text(format!(
            "area {}x{} cells; player cell ({px}, {py}); grids 0-3 here: {}",
            grid.cells_x,
            grid.cells_y,
            here.join(" ")
        ));
        let r = self.radius as i64;
        let cell = self.cell_px;
        let origin = ui.cursor_screen_pos();
        let size = (2 * r + 1) as f32 * cell;
        let draw = ui.get_window_draw_list();
        draw.add_rect(
            origin,
            [origin[0] + size, origin[1] + size],
            [0.4, 0.4, 0.4, 1.0],
        )
        .build();
        // Rows top (high y) to bottom: the game's y grows "up" on screen in this view.
        for dy in -r..=r {
            let y = py + dy;
            let sy = origin[1] + (r - dy) as f32 * cell;
            let mut run_start = -r;
            let mut run_val = grid.get(px - r, y);
            for dx in -r..=r + 1 {
                let v = if dx <= r {
                    grid.get(px + dx, y)
                } else {
                    Some(255)
                };
                if v != run_val {
                    if let Some(val) = run_val.filter(|&v| v != 0) {
                        let s = 0.2 + val as f32 / 15.0 * 0.7;
                        let shade = [0.15, s, 0.25, 1.0];
                        let x0 = origin[0] + (run_start + r) as f32 * cell;
                        let x1 = origin[0] + (dx + r) as f32 * cell;
                        draw.add_rect([x0, sy], [x1, sy + cell], shade)
                            .filled(true)
                            .build();
                    }
                    run_start = dx;
                    run_val = v;
                }
            }
        }
        // The player and its facing (angle 0 = +y; atan2(dy, dx) + pi/2).
        let c = [
            origin[0] + (r as f32 + 0.5) * cell,
            origin[1] + (r as f32 + 0.5) * cell,
        ];
        draw.add_circle(c, (cell * 1.5).max(3.0), [1.0, 0.9, 0.2, 1.0])
            .filled(true)
            .build();
        let a = p.facing - std::f32::consts::FRAC_PI_2;
        let len = (cell * 6.0).max(14.0);
        draw.add_line(
            c,
            [c[0] + a.cos() * len, c[1] - a.sin() * len],
            [1.0, 0.9, 0.2, 1.0],
        )
        .thickness(2.0)
        .build();
        if let Some(s) = crate::devlink::server_cell() {
            let sx = origin[0] + ((s.0 as i64 - px + r) as f32 + 0.5) * cell;
            let sy = origin[1] + ((r - (s.1 as i64 - py)) as f32 + 0.5) * cell;
            draw.add_circle([sx, sy], (cell * 1.5).max(3.0), [0.3, 0.7, 1.0, 1.0])
                .thickness(2.0)
                .build();
        }
        ui.dummy([size, size]);
        ui.text_disabled(
            "yellow = client position and facing, blue ring = the server's position (our server)",
        );
    }
}
