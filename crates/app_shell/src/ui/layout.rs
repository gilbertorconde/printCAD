use axes::AxisSystem;
use egui::{self, Color32, Context};
use glam::Vec3;

use super::{ActiveTool, ActiveWorkbench};

pub fn draw_top_panel(
    ctx: &Context,
    active_workbench: &mut ActiveWorkbench,
    show_settings: &mut bool,
) {
    egui::TopBottomPanel::top("top_bar")
        .frame(
            egui::Frame::default()
                .inner_margin(egui::Margin::same(8))
                .fill(ctx.style().visuals.panel_fill),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("printCAD");
                ui.separator();
                ui.label("Workbench:");
                ui.selectable_value(active_workbench, ActiveWorkbench::Sketch, "Sketch");
                ui.selectable_value(active_workbench, ActiveWorkbench::PartDesign, "Part Design");
                ui.add_space(12.0);
                if ui.button("Settings").clicked() {
                    *show_settings = true;
                }
            });
        });
}

pub fn draw_left_panel(
    ctx: &Context,
    active_workbench: ActiveWorkbench,
    active_tool: &mut ActiveTool,
) {
    egui::SidePanel::left("left_panel")
        .resizable(true)
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.heading("Tools");
            ui.separator();
            match active_workbench {
                ActiveWorkbench::Sketch => {
                    ui.radio_value(active_tool, ActiveTool::Select, "Select");
                    ui.radio_value(active_tool, ActiveTool::SketchLine, "Line");
                    ui.radio_value(active_tool, ActiveTool::SketchCircle, "Circle");
                }
                ActiveWorkbench::PartDesign => {
                    ui.radio_value(active_tool, ActiveTool::Select, "Select");
                    ui.radio_value(active_tool, ActiveTool::Pad, "Pad");
                    ui.radio_value(active_tool, ActiveTool::Pocket, "Pocket");
                }
            }

            ui.separator();
            ui.heading("Feature Tree");
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.collapsing("Body 1", |ui| {
                    ui.label("▶ Pyramid");
                    ui.label("▶ Sketch.001");
                });
                ui.collapsing("Body 2", |ui| {
                    ui.label("▶ Base plane");
                });
            });
        });
}

pub fn draw_right_panel(ctx: &Context) {
    egui::SidePanel::right("right_panel")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            ui.heading("Properties");
            ui.label("Active selection: none");
            ui.separator();
            ui.heading("Inspector");
            ui.label("Nothing selected.");
            ui.separator();
            ui.heading("Log");
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.label("Events will appear here.");
            });
        });
}

pub fn draw_bottom_panel(
    ctx: &Context,
    fps: f32,
    hovered_point: Option<[f32; 3]>,
    axis_system: AxisSystem,
) {
    egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
        ui.horizontal(|ui| {
            let fps_text = if fps > 0.0 {
                format!("FPS: {:.1}", fps)
            } else {
                "FPS: …".to_string()
            };
            ui.label(fps_text);
            ui.separator();
            let axes = [
                ("H", axis_system.horizontal()),
                ("V", axis_system.vertical()),
                ("D", axis_system.depth()),
            ];
            if let Some(pos) = hovered_point {
                let canonical = axis_system.world_to_canonical(Vec3::from_array(pos));
                let values = canonical.to_array();
                let mut parts = Vec::with_capacity(3);
                for (idx, (role, axis)) in axes.iter().enumerate() {
                    parts.push(format!(
                        "{}({}): {:.3}",
                        role,
                        axis.signed_label(),
                        values[idx]
                    ));
                }
                ui.label(parts.join("  "));
            } else {
                let mut parts = Vec::with_capacity(3);
                for (role, axis) in axes {
                    parts.push(format!("{}({}): —", role, axis.signed_label()));
                }
                ui.label(parts.join("  "));
            }
        });
    });
}

pub fn draw_pivot_indicator(ctx: &Context, x: f32, y: f32) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("pivot_indicator"),
    ));

    let ppp = ctx.pixels_per_point();
    let pos = egui::pos2(x / ppp, y / ppp);

    let radius = 8.0;
    let fill_color = Color32::from_rgba_unmultiplied(255, 0, 0, 128);
    let stroke_color = Color32::from_rgba_unmultiplied(200, 0, 0, 200);

    painter.circle(
        pos,
        radius,
        fill_color,
        egui::Stroke::new(2.0, stroke_color),
    );

    let cross_size = 4.0;
    let cross_color = Color32::from_rgba_unmultiplied(255, 255, 255, 180);
    painter.line_segment(
        [
            egui::pos2(pos.x - cross_size, pos.y),
            egui::pos2(pos.x + cross_size, pos.y),
        ],
        egui::Stroke::new(1.5, cross_color),
    );
    painter.line_segment(
        [
            egui::pos2(pos.x, pos.y - cross_size),
            egui::pos2(pos.x, pos.y + cross_size),
        ],
        egui::Stroke::new(1.5, cross_color),
    );
}
