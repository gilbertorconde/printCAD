use axes::AxisSystem;
use core_document::{DocumentService, WorkbenchId};
use egui::{self, Color32, Context, Id, TextureHandle, TextureOptions};
use std::collections::HashMap;

use crate::{log_panel, orientation_cube::rasterize_svg};
use glam::Vec3;
use workbenches::REGISTERED_WORKBENCHES;

use super::{feature_tree, ActiveTool, ActiveWorkbench};

pub struct TopBarResult {
    pub open_requested: bool,
    pub save_requested: bool,
    pub save_as_requested: bool,
    pub new_body_requested: bool,
    pub reset_view_requested: bool,
}

pub fn draw_top_panel(
    ctx: &Context,
    active_workbench: &mut ActiveWorkbench,
    show_settings: &mut bool,
    active_tool: &mut ActiveTool,
    registry: &mut DocumentService,
    document: &mut core_document::Document,
    active_document_object: Option<core_document::FeatureId>,
    selected_body_id: Option<core_document::BodyId>,
) -> TopBarResult {
    let mut result = TopBarResult {
        open_requested: false,
        save_requested: false,
        save_as_requested: false,
        new_body_requested: false,
        reset_view_requested: false,
    };
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
                    let workbenches = REGISTERED_WORKBENCHES.lock().unwrap();
                    for wb in workbenches.iter() {
                        let wb_id = WorkbenchId::from(wb.id.as_str());
                        let wb_active = ActiveWorkbench(wb_id.clone());
                        ui.selectable_value(active_workbench, wb_active, &wb.label);
                    }
                });

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    // Open
                    if let Some(icon) = get_global_icon(ctx, "open") {
                        let resp = ui
                            .add(egui::Button::image(egui::Image::from(&icon)))
                            .on_hover_text("Open");
                        if resp.clicked() {
                            result.open_requested = true;
                        }
                    } else if ui.button("Open").clicked() {
                        result.open_requested = true;
                    }

                    // Save
                    if let Some(icon) = get_global_icon(ctx, "save") {
                        let resp = ui
                            .add(egui::Button::image(egui::Image::from(&icon)))
                            .on_hover_text("Save");
                        if resp.clicked() {
                            result.save_requested = true;
                        }
                    } else if ui.button("Save").clicked() {
                        result.save_requested = true;
                    }

                    // Save As
                    if let Some(icon) = get_global_icon(ctx, "save_as") {
                        let resp = ui
                            .add(egui::Button::image(egui::Image::from(&icon)))
                            .on_hover_text("Save As");
                        if resp.clicked() {
                            result.save_as_requested = true;
                        }
                    } else if ui.button("Save As").clicked() {
                        result.save_as_requested = true;
                    }

                    ui.separator();

                    // New Body
                    if let Some(icon) = get_global_icon(ctx, "new_body") {
                        let resp = ui
                            .add(egui::Button::image(egui::Image::from(&icon)))
                            .on_hover_text("New Body");
                        if resp.clicked() {
                            result.new_body_requested = true;
                        }
                    } else if ui
                        .add(egui::Button::new("New Body").min_size(egui::vec2(80.0, 0.0)))
                        .clicked()
                    {
                        result.new_body_requested = true;
                    }

                    // Fit View
                    if let Some(icon) = get_global_icon(ctx, "fit_view") {
                        let resp = ui
                            .add(egui::Button::image(egui::Image::from(&icon)))
                            .on_hover_text("Fit View");
                        if resp.clicked() {
                            result.reset_view_requested = true;
                        }
                    } else if ui.button("Fit View").clicked() {
                        result.reset_view_requested = true;
                    }
                });

                ui.add_space(6.0);

                ui.horizontal_wrapped(|ui| {
                    // Collect tools into Vec first to release the immutable borrow
                    let tools: Vec<_> = match registry.tools_for(&active_workbench.0) {
                        Ok(t) => t.to_vec(),
                        Err(_) => return,
                    };

                    // Build a minimal runtime context for tool enabling checks
                    let cam_pos = [0.0, 0.0, 5.0]; // Placeholder
                    let cam_target = [0.0, 0.0, 0.0]; // Placeholder
                    let viewport = (0, 0, 1920, 1080); // Placeholder
                    let mut wb_ctx = core_document::WorkbenchRuntimeContext::new(
                        document, cam_pos, cam_target, viewport,
                    );
                    wb_ctx.active_document_object = active_document_object;
                    wb_ctx.selected_body_id = selected_body_id.map(|id| id.0);

                    // Get workbench once for tool enabling checks (now we can get mutable borrow)
                    let workbench = match registry.workbench_mut(&active_workbench.0) {
                        Ok(wb) => wb,
                        Err(_) => return,
                    };

                    for tool in &tools {
                        let is_active = active_tool.active_ids.contains(&tool.id);

                        // Check with workbench if tool is enabled
                        let enabled = workbench.is_tool_enabled(&tool.id, &wb_ctx);

                        // Try to load an icon for this tool based on convention:
                        // - Workbench id, e.g. "wb.sketch"
                        // - Tool id, e.g. "sketch.line"
                        // - Icon path: crates/workbenches/<wb_crate>/src/icons/<tool_id>.svg
                        //   where <wb_crate> is derived from the workbench id (wb.sketch -> wb_sketch)
                        let icon = get_tool_icon_for(ctx, &active_workbench.0, &tool.id);

                        // Action tools behave like simple buttons (fire-and-forget),
                        // Radio and Check tools show selected state.
                        let button = if let Some(icon) = icon {
                            // Icon-based button (tooltip shows label).
                            // Build an Image widget from the texture handle and wrap it in a button.
                            let mut button = egui::Button::image(egui::Image::from(&icon));
                            if tool.behavior != core_document::ToolBehavior::Action && is_active {
                                button = button.selected(true);
                            }
                            ui.add_enabled(enabled, button).on_hover_text(&tool.label)
                        } else if tool.behavior == core_document::ToolBehavior::Action {
                            ui.add_enabled(enabled, egui::Button::new(&tool.label))
                        } else {
                            ui.add_enabled(
                                enabled,
                                egui::Button::new(&tool.label).selected(is_active),
                            )
                        };

                        if button.clicked() && enabled {
                            match tool.behavior {
                                core_document::ToolBehavior::Action => {
                                    // Fire-and-forget: always select the action tool for this frame.
                                    // The host will clear it after handling the input.
                                    active_tool.active_ids.insert(tool.id.clone());
                                }
                                core_document::ToolBehavior::Check => {
                                    // Check behavior: toggle independently
                                    if is_active {
                                        active_tool.active_ids.remove(&tool.id);
                                    } else {
                                        active_tool.active_ids.insert(tool.id.clone());
                                    }
                                }
                                core_document::ToolBehavior::Radio => {
                                    // Radio behavior: only one tool per group can be active
                                    if is_active {
                                        // Clicking an active tool deactivates it
                                        active_tool.active_ids.remove(&tool.id);
                                    } else {
                                        // Deactivate other tools in the same group
                                        if let Some(group) = &tool.group {
                                            // Remove all tools in this group
                                            active_tool.active_ids.retain(|active_id| {
                                                // Find the tool descriptor to check its group
                                                tools
                                                    .iter()
                                                    .find(|t| &t.id == active_id)
                                                    .map(|t| t.group.as_deref() != Some(group))
                                                    .unwrap_or(true)
                                            });
                                        } else {
                                            // No group: this tool is its own group, so just clear all
                                            active_tool.active_ids.clear();
                                        }
                                        // Activate this tool
                                        active_tool.active_ids.insert(tool.id.clone());
                                    }
                                }
                            }
                        }
                    }
                });
            });
        });
    result
}

#[derive(Default, Clone)]
struct IconCache {
    handles: HashMap<String, TextureHandle>,
}

fn workbench_id_to_crate_folder(id: &str) -> String {
    // Map workbench IDs to crate folder names.
    // Convention:
    //   - "wb.sketch" -> "wb_sketch"
    //   - "wb.part"   -> "wb_part"
    //
    // Implementation: replace '.' with '_' and keep the full id.
    id.replace('.', "_")
}

fn load_svg_icon(
    ctx: &Context,
    cache_id: Id,
    cache_key: &str,
    texture_name_prefix: &str,
    path: &std::path::Path,
) -> Option<TextureHandle> {
    // Try cache first
    if let Some(handle) = ctx.data(|data| {
        data.get_temp::<IconCache>(cache_id)
            .and_then(|cache| cache.handles.get(cache_key).cloned())
    }) {
        return Some(handle);
    }

    let svg = std::fs::read_to_string(path).ok()?;
    let image = rasterize_svg(&svg)?;

    let tex_name = format!("{texture_name_prefix}{cache_key}");
    let texture = ctx.load_texture(tex_name, image, TextureOptions::LINEAR);

    // Store in cache
    ctx.data_mut(|data| {
        let cache = data.get_temp_mut_or_insert_with(cache_id, IconCache::default);
        cache.handles.insert(cache_key.to_string(), texture.clone());
    });

    Some(texture)
}

fn get_tool_icon_for(
    ctx: &Context,
    workbench_id: &WorkbenchId,
    tool_id: &str,
) -> Option<TextureHandle> {
    // Unique cache key per workbench/tool pair
    let key = format!("tool::{}::{}", workbench_id.as_str(), tool_id);
    let cache_id = Id::new("icon_cache");

    // Derive crate folder from workbench id
    let crate_folder = workbench_id_to_crate_folder(workbench_id.as_str());
    let file_name = format!("{tool_id}.svg");

    // Path: crates/workbenches/<crate_folder>/src/icons/<tool_id>.svg
    let candidate = std::path::PathBuf::from("crates")
        .join("workbenches")
        .join(crate_folder)
        .join("src")
        .join("icons")
        .join(&file_name);

    load_svg_icon(ctx, cache_id, &key, "tool_icon_", &candidate)
}

fn get_global_icon(ctx: &Context, name: &str) -> Option<TextureHandle> {
    let key = format!("global::{name}");
    let cache_id = Id::new("icon_cache");

    // Path: crates/app_shell/src/icons/<name>.svg
    let candidate = std::path::PathBuf::from("crates")
        .join("app_shell")
        .join("src")
        .join("icons")
        .join(format!("{name}.svg"));

    load_svg_icon(ctx, cache_id, &key, "global_icon_", &candidate)
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
            ui.heading("Model");
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
            if let Ok(wb) = registry.workbench_mut(&active_workbench.0) {
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
    let wants_panel = registry
        .workbench_mut(&active_workbench.0)
        .map(|wb| wb.wants_right_panel())
        .unwrap_or(false);

    if !wants_panel {
        return;
    }

    egui::SidePanel::right("right_panel")
        .resizable(true)
        .default_width(280.0)
        .show(ctx, |ui| {
            if let Ok(wb) = registry.workbench_mut(&active_workbench.0) {
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

/// Draw screen-space overlays in the viewport area.
/// These are rendered as 2D lines in screen coordinates, maintaining constant thickness.
pub fn draw_screen_space_overlays(
    ctx: &egui::Context,
    overlays: &[core_document::ScreenSpaceOverlay],
) {
    if overlays.is_empty() {
        return;
    }

    let ppp = ctx.pixels_per_point();
    let viewport_rect = ctx.available_rect();

    // Use Background order to draw beneath UI panels, and clip to viewport area
    let layer_id = egui::LayerId::new(
        egui::Order::Background, // Draw beneath UI but on top of 3D scene (3D is rendered separately)
        egui::Id::new("screen_space_overlays"),
    );
    let painter = ctx.layer_painter(layer_id).with_clip_rect(viewport_rect);

    for overlay in overlays {
        // Screen coordinates are already in pixels relative to the viewport origin (0,0)
        // We need to convert them to egui logical coordinates and add the viewport offset
        // The viewport_rect gives us the logical position of the viewport in the UI
        let start_x = viewport_rect.min.x + (overlay.start[0] / ppp);
        let start_y = viewport_rect.min.y + (overlay.start[1] / ppp);
        let end_x = viewport_rect.min.x + (overlay.end[0] / ppp);
        let end_y = viewport_rect.min.y + (overlay.end[1] / ppp);

        let start = egui::pos2(start_x, start_y);
        let end = egui::pos2(end_x, end_y);

        // Convert RGB [0.0-1.0] to egui Color32
        let r = (overlay.color[0] * 255.0) as u8;
        let g = (overlay.color[1] * 255.0) as u8;
        let b = (overlay.color[2] * 255.0) as u8;
        let color = Color32::from_rgb(r, g, b);

        // Draw line with constant screen-space thickness (convert pixels to logical points)
        let stroke_width = overlay.thickness / ppp;
        painter.line_segment([start, end], egui::Stroke::new(stroke_width, color));
    }
}
