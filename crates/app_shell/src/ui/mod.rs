mod combo_view;
mod command_palette;
mod commands;
mod feature_tree;
mod host_ctx;
mod hud;
mod inputs;
mod log_view;
mod menu_bar;
mod overlays;
mod preferences;
mod property_panel;
mod start_page;
mod status_bar;
mod step_import_modal;
mod task_panel;
mod toolbar;
mod view_toolbar;

pub use commands::{FileCommand, StartKind, UiCommand};
pub use host_ctx::HostCtxParams;
pub use inputs::{HoverCard, UiFrameInputs};
pub use step_import_modal::StepImportDialogAction;

use core_document::WorkbenchId;
use egui::Context;
use egui_winit::{State, egui as egui_core};
use render_vk::EguiSubmission;
use settings::ProjectionMode;
use winit::{event::WindowEvent, window::Window};

use crate::orientation_cube::{self, OrientationCubeConfig, OrientationCubeResult};

/// Which top-level screen the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The launch page: new-document cards, recent files.
    Start,
    /// The modelling workspace.
    Workspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWorkbench(pub WorkbenchId);

impl Default for ActiveWorkbench {
    fn default() -> Self {
        // Part Design is the natural landing place: create a body, sketch on
        // it, pad it. The Sketch workbench is one click away.
        Self(WorkbenchId::from("wb.part"))
    }
}

#[derive(Debug, Clone, Default)]
pub struct ActiveTool {
    /// Set of active tool IDs. For Radio tools, only one per group is active.
    /// For Check tools, multiple can be active. For Action tools, this is cleared after handling.
    pub active_ids: std::collections::HashSet<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Per-frame data (submission, viewport, tool/workbench state) plus the
/// list of actions the user triggered this frame.
pub struct UiFrameOutput {
    pub submission: EguiSubmission,
    /// egui's own repaint request: `Duration::ZERO` for "next frame please"
    /// (animations), a finite delay for timed repaints (caret blink), and
    /// effectively-infinite when content is static.
    pub repaint_delay: std::time::Duration,
    pub viewport: ViewportRect,
    pub active_tool: ActiveTool,
    pub active_workbench: ActiveWorkbench,
    pub commands: Vec<UiCommand>,
    /// A workbench task is open in the right panel after this frame.
    pub task_open: bool,
}

pub struct UiLayer {
    ctx: Context,
    state: State,
    preferences: preferences::PreferencesState,
    palette: command_palette::PaletteState,
    orientation_cube_config: OrientationCubeConfig,
    /// Substring filter over the model tree; UI-local.
    tree_filter: String,
    property_tab: property_panel::PropertyTab,
    rename_buffer: Option<(TreeItemId, String)>,
    /// The start page's recent-files filter; UI-local.
    recent_search: String,
    /// Keys the workbench consumed that egui also queued; egui must not
    /// act on them (Tab would move focus, Enter would accept the task).
    swallowed_keys: Vec<egui::Key>,
}

impl UiLayer {
    pub fn new(window: &Window) -> Self {
        let ctx = Context::default();
        ui_kit::apply_theme(&ctx);
        let state = State::new(
            ctx.clone(),
            egui_core::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            None,
        );

        orientation_cube::warm_face_textures(&ctx);

        Self {
            ctx,
            state,
            preferences: preferences::PreferencesState::default(),
            palette: command_palette::PaletteState::default(),
            orientation_cube_config: OrientationCubeConfig::default(),
            tree_filter: String::new(),
            property_tab: property_panel::PropertyTab::default(),
            rename_buffer: None,
            recent_search: String::new(),
            swallowed_keys: Vec::new(),
        }
    }

    /// A text field owns the keyboard.
    pub fn wants_keyboard_input(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// The workbench consumed `key`: keep egui from acting on it too.
    pub fn swallow_key(&mut self, key: egui::Key) {
        self.swallowed_keys.push(key);
    }

    /// Drop keyboard focus (a click landed on the viewport, not a widget).
    pub fn release_focus(&self) {
        self.ctx.memory_mut(|m| {
            if let Some(id) = m.focused() {
                m.surrender_focus(id);
            }
        });
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.state.on_window_event(window, event)
    }

    pub fn run(&mut self, window: &Window, inputs: UiFrameInputs<'_>) -> UiFrameOutput {
        let UiFrameInputs {
            screen,
            recent,
            active_tool: host_active_tool,
            active_workbench: host_active_workbench,
            settings,
            document,
            registry,
            host,
            orientation_input,
            planar_view_lock,
            fps,
            scene_redraws_per_s,
            gpu_name,
            gpus,
            hovered_point,
            pivot_screen_pos,
            axis_system,
            tree_selection: active_tree_selection,
            active_document_object,
            editing_feature,
            viewport_hud,
            status_items,
            task,
            hover_card,
            dimensions,
            screen_space_overlays,
            screen_space_marks,
            screen_space_labels,
            pending_imports,
            pending_document_open,
            kernel_status,
            kernel_cancellable,
            kernel_progress,
            server_label,
            document_saving,
            mut step_import_pending,
        } = inputs;

        let mut raw_input = self.state.take_egui_input(window);
        if !self.swallowed_keys.is_empty() {
            let swallowed = std::mem::take(&mut self.swallowed_keys);
            raw_input
                .events
                .retain(|e| !matches!(e, egui::Event::Key { key, .. } if swallowed.contains(key)));
        }
        let prev_workbench = host_active_workbench.clone();
        let mut active_workbench = host_active_workbench;
        // Seed tool state from the host, not a UiLayer copy: the host owns
        // consumption of Action tools, and a stale parallel copy here would
        // resurrect them every frame (infinite "New Body" loop).
        let mut active_tool = host_active_tool;

        let mut cube_config = self.orientation_cube_config.clone();
        cube_config.planar_only = planar_view_lock;
        let mut commands: Vec<UiCommand> = Vec::new();
        let mut settings_commit: Option<preferences::Commit> = None;
        let mut palette_activate: Option<(ActiveWorkbench, String)> = None;
        let mut cube_result = OrientationCubeResult::default();
        let mut viewport_rect_logical = egui::Rect::NOTHING;
        let mut task_open = false;
        let mut tree_selection = None;
        let mut step_import_dialog = StepImportDialogAction::default();

        let projection = settings.camera.projection;
        let nav_style = match settings.camera.navigation_style {
            settings::NavigationStyle::Gesture => "Gesture",
            settings::NavigationStyle::Cad => "CAD",
        };
        let document_name = document.name().to_owned();
        let document_dirty = document.metadata().dirty();
        let breadcrumb = editing_feature
            .and_then(|id| document.get_feature_meta(id))
            .map(|node| node.name.clone());

        let full_output = self.ctx.run_ui(raw_input, |ui| {
            let menu = menu_bar::draw_menu_bar(
                ui,
                menu_bar::MenuBarInputs {
                    registry,
                    document_name: &document_name,
                    document_dirty,
                    breadcrumb: breadcrumb.as_deref(),
                    show_log_panel: settings.rendering.show_log_panel,
                    projection,
                    recent,
                    screen,
                },
                &mut active_workbench,
                &mut active_tool,
                &mut commands,
            );
            palette_activate = menu.activate_tool.clone();
            // About lands on its page; Preferences keeps the last group.
            let unit = document.display_unit();
            if menu.show_about {
                self.preferences
                    .open_at(settings, unit, preferences::PrefGroup::General, 1);
            }
            if menu.show_preferences {
                let (group, tab) = (self.preferences.group, self.preferences.tab);
                self.preferences.open_at(settings, unit, group, tab);
            }
            let mut open_palette = menu.open_palette;

            if screen == Screen::Start {
                // The start page covers the whole viewport; the scene
                // underneath carries no bodies.
                viewport_rect_logical = ui.available_rect_before_wrap();
                let start = start_page::draw_start_page(
                    ui,
                    start_page::StartPageInputs {
                        recent,
                        search: &mut self.recent_search,
                    },
                    &mut commands,
                );
                if start.show_preferences {
                    let (group, tab) = (self.preferences.group, self.preferences.tab);
                    self.preferences.open_at(settings, unit, group, tab);
                }
                settings_commit = preferences::draw_preferences(
                    ui.ctx(),
                    &mut self.preferences,
                    preferences::PreferencesInputs {
                        registry,
                        gpus,
                        gpu_name,
                    },
                );
                return;
            }

            toolbar::draw_toolbars(
                ui,
                toolbar::ToolbarInputs {
                    registry,
                    document,
                    host,
                    active_document_object,
                },
                &mut active_workbench,
                &mut active_tool,
                &mut commands,
                &mut open_palette,
            );

            if open_palette {
                self.palette.open();
            }
            {
                // Enablement for the active bench's tools, against the real
                // camera and viewport.
                let ids: Vec<String> = registry
                    .tools_for(&active_workbench.0)
                    .map(|t| t.iter().map(|d| d.id.clone()).collect())
                    .unwrap_or_default();
                let enabled: Vec<(String, bool)> = match registry.workbench_mut(&active_workbench.0)
                {
                    Ok(wb) => {
                        let ctx = host_ctx::panel_ctx(document, host, active_document_object);
                        ids.into_iter()
                            .map(|id| {
                                let on = wb.is_tool_enabled(&id, &ctx);
                                (id, on)
                            })
                            .collect()
                    }
                    Err(_) => Vec::new(),
                };
                let enabled_active = |id: &str| enabled.iter().any(|(i, on)| i == id && *on);
                let palette = command_palette::draw_command_palette(
                    ui.ctx(),
                    &mut self.palette,
                    registry,
                    &active_workbench,
                    &enabled_active,
                    &mut commands,
                );
                if palette.show_preferences {
                    let (group, tab) = (self.preferences.group, self.preferences.tab);
                    self.preferences.open_at(settings, unit, group, tab);
                }
                if palette.activate_tool.is_some() {
                    palette_activate = palette.activate_tool;
                }
            }

            // Bottom bars before the side panels so they span the width.
            let cancel = status_bar::draw_status_bar(
                ui,
                &status_bar::StatusBarInputs {
                    fps,
                    scene_redraws_per_s,
                    hovered_point,
                    axis_system,
                    display_unit: document.display_unit(),
                    pending_imports,
                    pending_document_open,
                    kernel_status: kernel_status.as_deref(),
                    kernel_cancellable,
                    kernel_progress,
                    server_label: &server_label,
                    document_saving,
                    nav_style,
                    items: status_items.as_ref(),
                    preselect: hover_card.as_ref().map(|h| h.title.as_str()),
                    dimensions: dimensions.as_deref(),
                },
            );
            if cancel {
                commands.push(UiCommand::CancelKernelJob);
            }
            log_view::draw_log_panel(ui, settings.rendering.show_log_panel);

            let combo = combo_view::draw_combo_view(
                ui,
                combo_view::ComboViewInputs {
                    active_workbench: active_workbench.clone(),
                    document,
                    registry,
                    host,
                    active_tree_selection,
                    active_document_object,
                    editing_feature,
                    filter: &mut self.tree_filter,
                    property_tab: &mut self.property_tab,
                    rename_buffer: &mut self.rename_buffer,
                },
            );
            apply_writeback(&combo.writeback, &mut commands, &mut tree_selection);
            // An explicit tree click wins over a panel-created feature.
            if combo.tree_selection.is_some() {
                tree_selection = combo.tree_selection;
            }
            if let Some(item) = combo.tree_activation {
                commands.push(UiCommand::ActivateTreeItem(item));
            }
            if let Some((node, visible)) = combo.imported_visibility_change {
                commands.push(UiCommand::SetImportedVisibility { node, visible });
            }
            if let Some((feature, command)) = combo.tree_feature_command {
                commands.push(UiCommand::TreeFeature { feature, command });
            }
            if let Some((item, name)) = combo.rename {
                commands.push(UiCommand::RenameTreeItem { item, name });
            }

            let task_result = task_panel::draw_task_panel(
                ui,
                task_panel::TaskPanelInputs {
                    active_workbench: active_workbench.clone(),
                    document,
                    registry,
                    host,
                    active_document_object,
                    task: task.as_ref(),
                },
            );
            apply_writeback(&task_result.writeback, &mut commands, &mut tree_selection);
            if let Some(outcome) = task_result.outcome {
                commands.push(UiCommand::TaskClosed(outcome));
            }
            task_open = task_result.open;

            settings_commit = preferences::draw_preferences(
                ui.ctx(),
                &mut self.preferences,
                preferences::PreferencesInputs {
                    registry,
                    gpus,
                    gpu_name,
                },
            );

            viewport_rect_logical = ui.available_rect_before_wrap();

            overlays::draw_screen_space_overlays(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_overlays,
            );
            overlays::draw_screen_space_marks(ui.ctx(), viewport_rect_logical, screen_space_marks);
            overlays::draw_screen_space_labels(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_labels,
            );
            let footer = [
                match projection {
                    ProjectionMode::Orthographic => "Orthographic".to_string(),
                    ProjectionMode::Perspective => "Perspective".to_string(),
                },
                "Shaded + edges".to_string(),
            ];
            hud::draw_viewport_hud(
                ui.ctx(),
                viewport_rect_logical,
                viewport_hud.as_ref(),
                &footer,
            );
            if let Some(card) = &hover_card {
                hud::draw_hover_card(
                    ui.ctx(),
                    viewport_rect_logical,
                    card,
                    document.display_unit(),
                );
            }
            view_toolbar::draw_view_toolbar(
                ui.ctx(),
                viewport_rect_logical,
                projection,
                &mut commands,
            );
            hud::draw_toasts(ui.ctx(), viewport_rect_logical);

            if let Some(input) = orientation_input {
                cube_result =
                    orientation_cube::draw(ui.ctx(), viewport_rect_logical, input, &cube_config);
            }

            if let Some((path, draft)) = step_import_pending.as_mut() {
                step_import_dialog =
                    step_import_modal::draw_step_import_modal(ui.ctx(), path, draft);
            }

            if let Some((px, py)) = pivot_screen_pos {
                overlays::draw_pivot_indicator(ui.ctx(), px, py);
            }
        });

        // Detect workbench change
        // A palette pick on another bench switches first, then activates.
        let plain_switch = palette_activate.is_none();
        if let Some((bench, id)) = palette_activate {
            if bench != active_workbench {
                active_workbench = bench;
                active_tool = ActiveTool::default();
            }
            if let Ok(tools) = registry.tools_for(&active_workbench.0)
                && let Some(tool) = tools.iter().find(|t| t.id == id)
            {
                toolbar::activate_tool(&mut active_tool, tools, tool, &id);
            }
        }
        let workbench_changed = active_workbench != prev_workbench;
        if workbench_changed && plain_switch {
            // Reset tool when switching workbenches
            active_tool = ActiveTool::default();
        }

        self.state
            .handle_platform_output(window, full_output.platform_output.clone());
        let primitives = self
            .ctx
            .tessellate(full_output.shapes.clone(), full_output.pixels_per_point);

        let ppp = full_output.pixels_per_point;
        let viewport = ViewportRect {
            x: (viewport_rect_logical.min.x * ppp).max(0.0) as u32,
            y: (viewport_rect_logical.min.y * ppp).max(0.0) as u32,
            width: (viewport_rect_logical.width() * ppp).max(1.0) as u32,
            height: (viewport_rect_logical.height() * ppp).max(1.0) as u32,
        };

        // Fold the remaining interactions into commands. Application order
        // is decided by `apply_ui_commands`, not by the order of this list.
        if let Some(view) = cube_result.snap_to_view {
            commands.push(UiCommand::CameraSnap(view));
        }
        if let Some(delta) = cube_result.rotate_delta {
            commands.push(UiCommand::CameraRotate(delta));
        }
        if let Some(commit) = settings_commit {
            commands.push(UiCommand::CommitSettings {
                settings: commit.settings,
                display_unit: commit.display_unit,
            });
        }
        if let Some(item) = tree_selection {
            commands.push(UiCommand::SelectTreeItem(item));
        }
        match step_import_dialog {
            StepImportDialogAction::Confirmed => commands.push(UiCommand::ConfirmStepImport),
            StepImportDialogAction::Cancelled => commands.push(UiCommand::CancelStepImport),
            StepImportDialogAction::None => {}
        }
        if workbench_changed {
            commands.push(UiCommand::SwitchWorkbench {
                from: prev_workbench,
                to: active_workbench.clone(),
            });
        }

        let repaint_delay = full_output
            .viewport_output
            .values()
            .map(|v| v.repaint_delay)
            .min()
            .unwrap_or(std::time::Duration::MAX);

        UiFrameOutput {
            submission: EguiSubmission {
                pixels_per_point: full_output.pixels_per_point,
                textures_delta: full_output.textures_delta,
                primitives,
            },
            repaint_delay,
            viewport,
            active_tool,
            active_workbench,
            commands,
            task_open,
        }
    }
}

/// Turn a panel hook's write-backs into commands. A feature the hook
/// created becomes the tree selection so the host's active object follows.
fn apply_writeback(
    writeback: &host_ctx::PanelWriteback,
    commands: &mut Vec<UiCommand>,
    tree_selection: &mut Option<TreeItemId>,
) {
    if writeback.finish_sketch_requested {
        commands.push(UiCommand::FinishSketch);
    }
    if let Some(req) = writeback.camera_orient_request.clone() {
        commands.push(UiCommand::OrientCameraToPlane(req));
    }
    match writeback.active_object_changed {
        Some(Some(id)) => *tree_selection = Some(TreeItemId::Feature(id)),
        Some(None) => commands.push(UiCommand::ReleaseActiveObject),
        None => {}
    }
    if let Some(wb) = writeback.workbench_switch_request.clone() {
        commands.push(UiCommand::RequestWorkbench(ActiveWorkbench(wb)));
    }
}

pub use feature_tree::{TreeFeatureCommand, TreeItemId};
