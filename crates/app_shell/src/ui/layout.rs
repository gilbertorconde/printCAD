use axes::AxisSystem;
use egui::{self, Color32, Context};

use crate::log_panel;
use glam::Vec3;

use core_document::ToolDescriptor;

use super::{feature_tree, ActiveTool, ActiveWorkbench};

pub fn draw_top_panel(
    ctx: &Context,
    active_workbench: &mut ActiveWorkbench,
    show_settings: &mut bool,
    active_tool: &mut ActiveTool,
    tools: &[ToolDescriptor],
    has_active_sketch: bool,
    has_body: bool,
) -> bool {
    let mut new_body_requested = false;
    egui::TopBottomPanel::top("top_bar")
        .frame(
            egui::Frame::default()
                .inner_margin(egui::Margin::same(8))
                .fill(ctx.style().visuals.panel_fill),
        )
        .show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.heading("printCAD");
                    ui.separator();
                    if ui.button("Settings").clicked() {
                        *show_settings = true;
                    }
                    ui.separator();
                    ui.label("Workbench:");
                    ui.selectable_value(active_workbench, ActiveWorkbench::Sketch, "Sketch");
                    ui.selectable_value(
                        active_workbench,
                        ActiveWorkbench::PartDesign,
                        "Part Design",
                    );
                });

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Button::new("New Body").min_size(egui::vec2(80.0, 0.0)))
                        .clicked()
                    {
                        new_body_requested = true;
                    }
                    // Future general actions will be add here (like zoom, measure, etc.)
                });

                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    for tool in tools {
                        let is_active = active_tool
                            .id
                            .as_deref()
                            .map(|id| id == tool.id)
                            .unwrap_or(false);

                        let enabled = match tool.kind {
                            core_document::ToolKind::Action => {
                                if tool.id == "sketch.create" {
                                    has_body
                                } else {
                                    true
                                }
                            }
                            _ => has_active_sketch,
                        };

                        let button = ui.add_enabled(
                            enabled,
                            egui::Button::new(&tool.label).selected(is_active),
                        );

                        if button.clicked() && enabled {
                            if tool.kind == core_document::ToolKind::Action {
                                if is_active {
                                    active_tool.id = None;
                                } else {
                                    active_tool.id = Some(tool.id.clone());
                                }
                            } else if is_active {
                                active_tool.id = None;
                            } else {
                                active_tool.id = Some(tool.id.clone());
                            }
                        }
                    }
                });
            });
        });
    new_body_requested
}

pub struct LeftPanelResult {
    pub finish_sketch_requested: bool,
    pub tree_selection: Option<feature_tree::TreeItemId>,
    pub tree_activation: Option<feature_tree::TreeItemId>,
}

impl Default for LeftPanelResult {
    fn default() -> Self {
        Self {
            finish_sketch_requested: false,
            tree_selection: None,
            tree_activation: None,
        }
    }
}

pub fn draw_left_panel(
    ctx: &Context,
    active_workbench: ActiveWorkbench,
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    active_tree_selection: Option<feature_tree::TreeItemId>,
    active_document_object: Option<core_document::FeatureId>,
) -> LeftPanelResult {
    let mut panel_result = LeftPanelResult::default();

    egui::SidePanel::left("left_panel")
        .resizable(true)
        .default_width(260.0)
        .show(ctx, |ui| {
            ui.heading("Feature Tree");
            egui::ScrollArea::vertical().show(ui, |ui| {
                let tree_model = feature_tree::DocumentTree::build(document);
                let selected_id = active_tree_selection
                    .or_else(|| active_document_object.map(feature_tree::TreeItemId::from))
                    .unwrap_or(feature_tree::TreeItemId::DocumentRoot);
                let tree_ui_result = feature_tree::draw_tree(ui, &tree_model, Some(selected_id));
                panel_result.tree_selection = tree_ui_result.selection;
                panel_result.tree_activation = tree_ui_result.activation;
            });

            ui.separator();

            // Call workbench's ui_left_panel hook
            let wb_id = match active_workbench {
                ActiveWorkbench::Sketch => core_document::WorkbenchId::from("wb.sketch"),
                ActiveWorkbench::PartDesign => core_document::WorkbenchId::from("wb.part-design"),
            };

            if let Ok(wb) = registry.workbench_mut(&wb_id) {
                // Build a minimal runtime context for UI hooks
                let cam_pos = [0.0, 0.0, 5.0]; // Placeholder
                let cam_target = [0.0, 0.0, 0.0]; // Placeholder
                let viewport = (0, 0, 1920, 1080); // Placeholder
                let mut ctx = core_document::WorkbenchRuntimeContext::new(
                    document, cam_pos, cam_target, viewport,
                );
                ctx.active_document_object = active_document_object;

                wb.ui_left_panel(ui, &mut ctx);

                // Check for finish sketch request
                if ctx.finish_sketch_requested {
                    panel_result.finish_sketch_requested = true;
                }
            }
        });

    panel_result
}

pub fn draw_right_panel(
    ctx: &Context,
    active_workbench: ActiveWorkbench,
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    active_document_object: Option<core_document::FeatureId>,
) {
    let wb_id = match active_workbench {
        ActiveWorkbench::Sketch => core_document::WorkbenchId::from("wb.sketch"),
        ActiveWorkbench::PartDesign => core_document::WorkbenchId::from("wb.part-design"),
    };

    let wants_panel = registry
        .workbench_mut(&wb_id)
        .map(|wb| wb.wants_right_panel())
        .unwrap_or(false);

    if !wants_panel {
        return;
    }

    egui::SidePanel::right("right_panel")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            if let Ok(wb) = registry.workbench_mut(&wb_id) {
                let cam_pos = [0.0, 0.0, 5.0];
                let cam_target = [0.0, 0.0, 0.0];
                let viewport = (0, 0, 1920, 1080);
                let mut ctx = core_document::WorkbenchRuntimeContext::new(
                    document, cam_pos, cam_target, viewport,
                );
                ctx.active_document_object = active_document_object;
                wb.ui_right_panel(ui, &mut ctx);
            }
        });
}

pub fn draw_log_panel(ctx: &Context, show: bool) {
    if !show {
        return;
    }

    let entries = log_panel::entries();
    if entries.is_empty() {
        return;
    }

    egui::TopBottomPanel::bottom("log_panel")
        .resizable(true)
        .default_height(160.0)
        .min_height(80.0)
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Log");
                ui.add_space(8.0);
                if ui.button("Clear").clicked() {
                    log_panel::clear();
                }
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for entry in entries {
                        let secs = entry.timestamp_secs % 86_400;
                        let h = secs / 3600;
                        let m = (secs % 3600) / 60;
                        let s = secs % 60;
                        let time_str = format!("{h:02}:{m:02}:{s:02}");
                        let (label, color) = match entry.level {
                            log_panel::LogLevel::Info => ("INFO", Color32::from_rgb(180, 220, 255)),
                            log_panel::LogLevel::Warn => ("WARN", Color32::from_rgb(255, 210, 120)),
                            log_panel::LogLevel::Error => {
                                ("ERROR", Color32::from_rgb(255, 140, 140))
                            }
                        };
                        ui.colored_label(color, format!("[{time_str}] {label}: {}", entry.message));
                    }
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
