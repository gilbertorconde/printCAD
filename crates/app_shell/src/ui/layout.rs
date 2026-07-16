use axes::AxisSystem;
use core_document::{format_length_mm, DocumentService, Unit, WorkbenchId};
use egui::{
    self, Color32, Context, Id, Key, KeyboardShortcut, Modifiers, TextureHandle, TextureOptions,
};
use std::collections::HashMap;

use crate::{log_panel, orientation_cube::rasterize_svg};
use glam::Vec3;
use workbenches::REGISTERED_WORKBENCHES;

use super::{feature_tree, ActiveTool, ActiveWorkbench};

/// Outcome of a frame's interaction with the top bar (menu items + keyboard
/// shortcuts). Each flag is `true` only on the frame the action was triggered.
pub struct TopBarResult {
    pub new_requested: bool,
    pub open_requested: bool,
    pub save_requested: bool,
    pub save_as_requested: bool,
    pub import_step_requested: bool,
    pub reset_view_requested: bool,
    pub show_settings_requested: bool,
    pub show_about_requested: bool,
    pub quit_requested: bool,
}

pub fn draw_top_panel(
    ui: &mut egui::Ui,
    active_workbench: &mut ActiveWorkbench,
    active_tool: &mut ActiveTool,
    registry: &mut DocumentService,
    document: &mut core_document::Document,
    active_document_object: Option<core_document::FeatureId>,
    selected_body_id: Option<core_document::BodyId>,
) -> TopBarResult {
    let mut result = TopBarResult {
        new_requested: false,
        open_requested: false,
        save_requested: false,
        save_as_requested: false,
        import_step_requested: false,
        reset_view_requested: false,
        show_settings_requested: false,
        show_about_requested: false,
        quit_requested: false,
    };

    // Define the standard menu accelerators. `Modifiers::COMMAND` maps to
    // Ctrl on Linux/Windows and Cmd on macOS, matching what users expect.
    let sc_open = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
    let sc_save = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
    let sc_save_as = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, Key::S);
    let sc_import = KeyboardShortcut::new(Modifiers::COMMAND, Key::I);
    let sc_new = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);
    let sc_quit = KeyboardShortcut::new(Modifiers::COMMAND, Key::Q);
    let sc_fit = KeyboardShortcut::new(Modifiers::NONE, Key::F);

    // Consume shortcuts up-front so the matching menu rows don't double-fire
    // when an item is also clicked in the same frame.
    ui.ctx().input_mut(|i| {
        if i.consume_shortcut(&sc_new) {
            result.new_requested = true;
        }
        if i.consume_shortcut(&sc_open) {
            result.open_requested = true;
        }
        if i.consume_shortcut(&sc_save_as) {
            // Save-As must be checked before Save: COMMAND+SHIFT+S would also
            // satisfy COMMAND+S without this ordering.
            result.save_as_requested = true;
        }
        if i.consume_shortcut(&sc_save) {
            result.save_requested = true;
        }
        if i.consume_shortcut(&sc_import) {
            result.import_step_requested = true;
        }
        if i.consume_shortcut(&sc_quit) {
            result.quit_requested = true;
        }
        if i.consume_shortcut(&sc_fit) {
            result.reset_view_requested = true;
        }
    });

    let panel_fill = ui.ctx().global_style().visuals.panel_fill;
    egui::Panel::top("top_bar")
        .frame(
            egui::Frame::default()
                .inner_margin(egui::Margin::symmetric(6, 2))
                .fill(panel_fill),
        )
        .show_inside(ui, |ui| {
            ui.vertical(|ui| {
                // ----------------- Menu bar (thin row) -----------------
                egui::MenuBar::new().ui(ui, |ui| {
                    // --- File menu ---
                    let file_resp = ui.menu_button("File", |ui| {
                        let new_btn = egui::Button::new("New")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_new));
                        if ui
                            .add(new_btn)
                            .on_hover_text("Create a new untitled document")
                            .clicked()
                        {
                            result.new_requested = true;
                            ui.close();
                        }

                        ui.separator();

                        let open_btn = egui::Button::new("Open...")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_open));
                        if ui
                            .add(open_btn)
                            .on_hover_text("Open an existing .prtcad document")
                            .clicked()
                        {
                            result.open_requested = true;
                            ui.close();
                        }

                        let save_btn = egui::Button::new("Save")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_save));
                        if ui
                            .add(save_btn)
                            .on_hover_text("Save the active document")
                            .clicked()
                        {
                            result.save_requested = true;
                            ui.close();
                        }

                        let save_as_btn = egui::Button::new("Save As...")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_save_as));
                        if ui
                            .add(save_as_btn)
                            .on_hover_text("Save the active document under a new name")
                            .clicked()
                        {
                            result.save_as_requested = true;
                            ui.close();
                        }

                        ui.separator();

                        let import_btn = egui::Button::new("Import STEP...")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_import));
                        if ui
                            .add(import_btn)
                            .on_hover_text("Import a STEP/STP file into this document")
                            .clicked()
                        {
                            result.import_step_requested = true;
                            ui.close();
                        }

                        ui.separator();

                        if ui
                            .button("Preferences...")
                            .on_hover_text("Open application preferences")
                            .clicked()
                        {
                            result.show_settings_requested = true;
                            ui.close();
                        }

                        ui.separator();

                        let quit_btn = egui::Button::new("Quit")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_quit));
                        if ui.add(quit_btn).on_hover_text("Exit printCAD").clicked() {
                            result.quit_requested = true;
                            ui.close();
                        }
                    });
                    file_resp
                        .response
                        .on_hover_text("File operations: open, save, import, preferences, quit");

                    // --- View menu ---
                    let view_resp = ui.menu_button("View", |ui| {
                        let fit_btn = egui::Button::new("Fit View")
                            .shortcut_text(ui.ctx().format_shortcut(&sc_fit));
                        if ui
                            .add(fit_btn)
                            .on_hover_text("Frame the document or origin in the viewport")
                            .clicked()
                        {
                            result.reset_view_requested = true;
                            ui.close();
                        }

                        ui.separator();

                        let wb_submenu = ui.menu_button("Workbench", |ui| {
                            let workbenches = REGISTERED_WORKBENCHES.lock().unwrap();
                            for wb in workbenches.iter() {
                                let wb_id = WorkbenchId::from(wb.id.as_str());
                                let target = ActiveWorkbench(wb_id);
                                let is_active = *active_workbench == target;
                                if ui
                                    .selectable_label(is_active, &wb.label)
                                    .on_hover_text(&wb.description)
                                    .clicked()
                                {
                                    *active_workbench = target;
                                    ui.close();
                                }
                            }
                        });
                        wb_submenu
                            .response
                            .on_hover_text("Switch the active workbench");
                    });
                    view_resp
                        .response
                        .on_hover_text("View controls: fit, workbench switcher");

                    // --- Help menu ---
                    let help_resp = ui.menu_button("Help", |ui| {
                        if ui
                            .button("About printCAD")
                            .on_hover_text("Show version and system information")
                            .clicked()
                        {
                            result.show_about_requested = true;
                            ui.close();
                        }
                    });
                    help_resp
                        .response
                        .on_hover_text("Help and about information");

                    // --- Right-aligned workbench combo ---
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let workbenches = REGISTERED_WORKBENCHES.lock().unwrap();
                        let current_label = workbenches
                            .iter()
                            .find(|wb| wb.id == active_workbench.0)
                            .map(|wb| wb.label.clone())
                            .unwrap_or_else(|| "(none)".to_string());
                        let current_desc = workbenches
                            .iter()
                            .find(|wb| wb.id == active_workbench.0)
                            .map(|wb| wb.description.clone())
                            .unwrap_or_default();
                        let combo = egui::ComboBox::from_id_salt("workbench_combo")
                            .selected_text(&current_label)
                            .show_ui(ui, |ui| {
                                for wb in workbenches.iter() {
                                    let wb_id = WorkbenchId::from(wb.id.as_str());
                                    let target = ActiveWorkbench(wb_id);
                                    ui.selectable_value(active_workbench, target, &wb.label)
                                        .on_hover_text(&wb.description);
                                }
                            });
                        let tooltip = if current_desc.is_empty() {
                            "Active workbench".to_string()
                        } else {
                            format!("Active workbench: {}", current_desc)
                        };
                        combo.response.on_hover_text(tooltip);
                    });
                });

                // ----------------- Workbench tool ribbon -----------------
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    // Snapshot the tool list so we can reborrow the registry
                    // mutably below for `is_tool_enabled` lookups.
                    let tools: Vec<_> = match registry.tools_for(&active_workbench.0) {
                        Ok(t) => t.to_vec(),
                        Err(_) => return,
                    };

                    // Minimal runtime context just for tool-enabling checks.
                    let cam_pos = [0.0, 0.0, 5.0];
                    let cam_target = [0.0, 0.0, 0.0];
                    let viewport = (0, 0, 1920, 1080);
                    let mut wb_ctx = core_document::WorkbenchRuntimeContext::new(
                        document, cam_pos, cam_target, viewport,
                    );
                    wb_ctx.active_document_object = active_document_object;
                    wb_ctx.selected_body_id = selected_body_id.map(|id| id.0);

                    let workbench = match registry.workbench_mut(&active_workbench.0) {
                        Ok(wb) => wb,
                        Err(_) => return,
                    };

                    for tool in &tools {
                        let is_active = active_tool.active_ids.contains(&tool.id);
                        let enabled = workbench.is_tool_enabled(&tool.id, &wb_ctx);

                        // Icon convention: crates/workbenches/<crate>/src/icons/<tool_id>.svg.
                        let icon = get_tool_icon_for(ui.ctx(), &active_workbench.0, &tool.id);

                        let response = if let Some(icon) = icon {
                            let mut button = egui::Button::image(egui::Image::from(&icon));
                            if tool.behavior != core_document::ToolBehavior::Action && is_active {
                                button = button.selected(true);
                            }
                            ui.add_enabled(enabled, button).on_hover_text(&tool.label)
                        } else if tool.behavior == core_document::ToolBehavior::Action {
                            ui.add_enabled(enabled, egui::Button::new(&tool.label))
                                .on_hover_text(&tool.label)
                        } else {
                            ui.add_enabled(
                                enabled,
                                egui::Button::new(&tool.label).selected(is_active),
                            )
                            .on_hover_text(&tool.label)
                        };

                        if response.clicked() && enabled {
                            match tool.behavior {
                                core_document::ToolBehavior::Action => {
                                    // Fire-and-forget: always (re)select the action tool for this
                                    // frame. The host clears it after handling the input.
                                    active_tool.active_ids.insert(tool.id.clone());
                                }
                                core_document::ToolBehavior::Check => {
                                    if is_active {
                                        active_tool.active_ids.remove(&tool.id);
                                    } else {
                                        active_tool.active_ids.insert(tool.id.clone());
                                    }
                                }
                                core_document::ToolBehavior::Radio => {
                                    if is_active {
                                        active_tool.active_ids.remove(&tool.id);
                                    } else {
                                        if let Some(group) = &tool.group {
                                            active_tool.active_ids.retain(|active_id| {
                                                tools
                                                    .iter()
                                                    .find(|t| &t.id == active_id)
                                                    .map(|t| t.group.as_deref() != Some(group))
                                                    .unwrap_or(true)
                                            });
                                        } else {
                                            active_tool.active_ids.clear();
                                        }
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

fn load_svg_icon(
    ctx: &Context,
    cache_id: Id,
    cache_key: &str,
    texture_name_prefix: &str,
    svg: &str,
) -> Option<TextureHandle> {
    // Try cache first
    if let Some(handle) = ctx.data(|data| {
        data.get_temp::<IconCache>(cache_id)
            .and_then(|cache| cache.handles.get(cache_key).cloned())
    }) {
        return Some(handle);
    }

    // Icon SVGs declare a 48px natural size over a 24px viewBox, so they
    // rasterize at 2x the display size for crisp HiDPI rendering.
    let image = rasterize_svg(svg)?;

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

    // Icons are embedded at compile time so loading never depends on the
    // process working directory.
    let svg = super::icons::embedded_tool_svg(workbench_id.as_str(), tool_id)?;

    load_svg_icon(ctx, cache_id, &key, "tool_icon_", svg)
}

#[derive(Default)]
pub struct LeftPanelResult {
    pub finish_sketch_requested: bool,
    /// Camera orient request written by a panel hook (e.g. after creating a
    /// sketch from the plane picker).
    pub camera_orient_request: Option<core_document::CameraOrientRequest>,
    /// The panel hook changed the active document object (feature created).
    pub activated_feature: Option<core_document::FeatureId>,
    pub tree_selection: Option<feature_tree::TreeItemId>,
    pub tree_activation: Option<feature_tree::TreeItemId>,
    pub imported_visibility_change: Option<(uuid::Uuid, bool)>,
    /// History context-menu action on a tree feature row.
    pub tree_feature_command: Option<(core_document::FeatureId, feature_tree::TreeFeatureCommand)>,
}

pub fn draw_left_panel(
    ui: &mut egui::Ui,
    active_workbench: ActiveWorkbench,
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    active_tree_selection: Option<feature_tree::TreeItemId>,
    active_document_object: Option<core_document::FeatureId>,
) -> LeftPanelResult {
    let mut panel_result = LeftPanelResult::default();

    egui::Panel::left("left_panel")
        .resizable(true)
        .default_size(260.0)
        .show_inside(ui, |ui| {
            ui.heading("Model");
            egui::ScrollArea::vertical().show(ui, |ui| {
                let tree_model = feature_tree::DocumentTree::build(document);
                let selected_id = active_tree_selection
                    .or_else(|| active_document_object.map(feature_tree::TreeItemId::from))
                    .unwrap_or(feature_tree::TreeItemId::DocumentRoot);
                let tree_ui_result = feature_tree::draw_tree(ui, &tree_model, Some(selected_id));
                panel_result.tree_selection = tree_ui_result.selection;
                panel_result.tree_activation = tree_ui_result.activation;
                panel_result.imported_visibility_change = tree_ui_result.imported_visibility_change;
                panel_result.tree_feature_command = tree_ui_result.feature_command;
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
                // Panel hooks can create features (e.g. the sketch plane
                // picker); surface their write-backs instead of dropping
                // them.
                panel_result.camera_orient_request = ctx.camera_orient_request.take();
                if ctx.active_document_object != active_document_object {
                    panel_result.activated_feature = ctx.active_document_object;
                }
                flush_ctx_logs(&mut ctx);
            }
        });

    panel_result
}

/// Returns true when the workbench requested to finish sketch editing.
pub fn draw_right_panel(
    ui: &mut egui::Ui,
    active_workbench: ActiveWorkbench,
    document: &mut core_document::Document,
    registry: &mut core_document::DocumentService,
    active_document_object: Option<core_document::FeatureId>,
) -> bool {
    let wants_panel = registry
        .workbench_mut(&active_workbench.0)
        .map(|wb| wb.wants_right_panel())
        .unwrap_or(false);

    if !wants_panel {
        return false;
    }

    let mut finish_requested = false;
    egui::Panel::right("right_panel")
        .resizable(true)
        .default_size(280.0)
        .show_inside(ui, |ui| {
            if let Ok(wb) = registry.workbench_mut(&active_workbench.0) {
                let cam_pos = [0.0, 0.0, 5.0];
                let cam_target = [0.0, 0.0, 0.0];
                let viewport = (0, 0, 1920, 1080);
                let mut ctx = core_document::WorkbenchRuntimeContext::new(
                    document, cam_pos, cam_target, viewport,
                );
                ctx.active_document_object = active_document_object;
                wb.ui_right_panel(ui, &mut ctx);
                finish_requested = ctx.finish_sketch_requested;
                flush_ctx_logs(&mut ctx);
            }
        });
    finish_requested
}

/// Panel UI hooks log through the runtime context; route those entries to
/// the app log panel instead of dropping them.
fn flush_ctx_logs(ctx: &mut core_document::WorkbenchRuntimeContext) {
    for entry in ctx.drain_logs() {
        match entry.level {
            core_document::LogLevel::Info => log_panel::info(entry.message),
            core_document::LogLevel::Warn => log_panel::warn(entry.message),
            core_document::LogLevel::Error => log_panel::error(entry.message),
        }
    }
}

pub fn draw_log_panel(ui: &mut egui::Ui, show: bool) {
    if !show {
        return;
    }

    let entries = log_panel::entries();
    if entries.is_empty() {
        return;
    }

    egui::Panel::bottom("log_panel")
        .resizable(true)
        .default_size(160.0)
        .min_size(80.0)
        .show_inside(ui, |ui| {
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
    ui: &mut egui::Ui,
    fps: f32,
    hovered_point: Option<[f32; 3]>,
    axis_system: AxisSystem,
    display_unit: Unit,
    pending_imports: u32,
    pending_document_open: u32,
) {
    egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
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
                    // Stored coordinates are in millimetres; format through the
                    // active document's display unit so the same world point
                    // reads naturally in mm, in, etc.
                    parts.push(format!(
                        "{}({}): {}",
                        role,
                        axis.signed_label(),
                        format_length_mm(values[idx], display_unit, 3),
                    ));
                }
                ui.label(parts.join("  "));
            } else {
                let suffix = display_unit.short_label();
                let mut parts = Vec::with_capacity(3);
                for (role, axis) in axes {
                    parts.push(format!("{}({}): — {}", role, axis.signed_label(), suffix));
                }
                ui.label(parts.join("  "));
            }

            // Right-aligned activity indicator: spinner while the kernel worker
            // imports STEP or a document loads from disk off the UI thread.
            if pending_imports > 0 || pending_document_open > 0 {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut parts = Vec::new();
                    if pending_imports > 0 {
                        if pending_imports == 1 {
                            parts.push("Importing STEP…".to_string());
                        } else {
                            parts.push(format!("Importing {pending_imports} STEPs…"));
                        }
                    }
                    if pending_document_open > 0 {
                        parts.push("Opening document…".to_string());
                    }
                    ui.label(parts.join(" · "));
                    ui.add(egui::Spinner::new());
                });
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
        egui::Stroke::new(2.0_f32, stroke_color),
    );

    let cross_size = 4.0;
    let cross_color = Color32::from_rgba_unmultiplied(255, 255, 255, 180);
    painter.line_segment(
        [
            egui::pos2(pos.x - cross_size, pos.y),
            egui::pos2(pos.x + cross_size, pos.y),
        ],
        egui::Stroke::new(1.5_f32, cross_color),
    );
    painter.line_segment(
        [
            egui::pos2(pos.x, pos.y - cross_size),
            egui::pos2(pos.x, pos.y + cross_size),
        ],
        egui::Stroke::new(1.5_f32, cross_color),
    );
}

/// Draw screen-space overlays in the viewport area.
/// These are rendered as 2D lines in screen coordinates, maintaining constant thickness.
pub fn draw_screen_space_overlays(
    ctx: &egui::Context,
    viewport_rect: egui::Rect,
    overlays: &[core_document::ScreenSpaceOverlay],
) {
    if overlays.is_empty() {
        return;
    }

    let ppp = ctx.pixels_per_point();

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
