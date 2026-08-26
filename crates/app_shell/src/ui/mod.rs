mod commands;
mod feature_tree;
mod icons;
mod inputs;
mod layout;
mod settings_panel;
mod step_import_modal;

pub use commands::{FileCommand, UiCommand};
pub use inputs::UiFrameInputs;
pub use step_import_modal::StepImportDialogAction;

use core_document::WorkbenchId;
use egui::Context;
use egui_winit::{egui as egui_core, State};
use render_vk::EguiSubmission;
use winit::{event::WindowEvent, window::Window};

use crate::orientation_cube::{self, OrientationCubeConfig, OrientationCubeResult};

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
    pub viewport: ViewportRect,
    pub active_tool: ActiveTool,
    pub active_workbench: ActiveWorkbench,
    pub commands: Vec<UiCommand>,
}

pub struct UiLayer {
    ctx: Context,
    state: State,
    settings_tab: settings_panel::SettingsTab,
    show_settings: bool,
    orientation_cube_config: OrientationCubeConfig,
}

impl UiLayer {
    pub fn new(window: &Window) -> Self {
        let ctx = Context::default();
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
            settings_tab: settings_panel::SettingsTab::Camera,
            show_settings: false,
            orientation_cube_config: OrientationCubeConfig::default(),
        }
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
            active_tool: host_active_tool,
            active_workbench: host_active_workbench,
            settings,
            document,
            registry,
            orientation_input,
            fps,
            gpu_name,
            gpus,
            hovered_point,
            pivot_screen_pos,
            axis_system,
            tree_selection: active_tree_selection,
            active_document_object,
            selected_body_id,
            screen_space_overlays,
            screen_space_labels,
            pending_imports,
            pending_document_open,
            kernel_status,
            kernel_cancellable,
            kernel_progress,
            document_saving,
            mut step_import_pending,
        } = inputs;

        let raw_input = self.state.take_egui_input(window);
        let prev_workbench = host_active_workbench.clone();
        let mut active_workbench = host_active_workbench;
        // Seed tool state from the host, not a UiLayer copy: the host owns
        // consumption of Action tools, and a stale parallel copy here would
        // resurrect them every frame (infinite "New Body" loop).
        let mut active_tool = host_active_tool;
        let mut show_settings = self.show_settings;
        let mut settings_tab = self.settings_tab;

        let cube_config = self.orientation_cube_config.clone();
        let mut settings_changed = false;
        let mut camera_settings_changed = false;
        let mut cube_result = OrientationCubeResult::default();
        let mut viewport_rect_logical = egui::Rect::NOTHING;
        let mut finish_requested = false;
        let mut orient_request: Option<core_document::CameraOrientRequest> = None;

        let mut tree_selection = None;
        let mut tree_activation = None;
        let mut imported_visibility_change = None;
        let mut tree_feature_command = None;
        let mut cancel_kernel_requested = false;
        let mut open_requested = false;
        let mut new_requested = false;
        let mut save_requested = false;
        let mut save_as_requested = false;
        let mut import_step_requested = false;
        let mut reset_view_requested = false;
        let mut quit_requested = false;
        let mut step_import_dialog = StepImportDialogAction::default();

        let full_output = self.ctx.run_ui(raw_input, |ui| {
            let top = layout::draw_top_panel(
                ui,
                &mut active_workbench,
                &mut active_tool,
                registry,
                document,
                active_document_object,
                selected_body_id,
            );
            new_requested = top.new_requested;
            open_requested = top.open_requested;
            save_requested = top.save_requested;
            save_as_requested = top.save_as_requested;
            import_step_requested = top.import_step_requested;
            reset_view_requested = top.reset_view_requested;
            quit_requested = top.quit_requested;

            // Translate menu-driven Settings/About requests into the persistent
            // window state owned by `UiLayer`. About forces the About tab so
            // the user lands on the right page; Preferences keeps whatever
            // tab they used last.
            if top.show_about_requested {
                show_settings = true;
                settings_tab = settings_panel::SettingsTab::About;
            }
            if top.show_settings_requested {
                show_settings = true;
            }
            let left_panel = layout::draw_left_panel(
                ui,
                active_workbench.clone(),
                document,
                registry,
                active_tree_selection,
                active_document_object,
            );
            finish_requested = left_panel.finish_sketch_requested;
            orient_request = left_panel.camera_orient_request;
            // A feature created from a panel becomes the tree selection so
            // the host's active-object state follows.
            if let Some(id) = left_panel.activated_feature {
                tree_selection = Some(TreeItemId::Feature(id));
            }
            // An explicit tree click wins over a panel-created feature.
            if left_panel.tree_selection.is_some() {
                tree_selection = left_panel.tree_selection;
            }
            tree_activation = left_panel.tree_activation;
            imported_visibility_change = left_panel.imported_visibility_change;
            tree_feature_command = left_panel.tree_feature_command;
            finish_requested |= layout::draw_right_panel(
                ui,
                active_workbench.clone(),
                document,
                registry,
                active_document_object,
            );
            let settings_outcome = settings_panel::draw_settings_window(
                ui.ctx(),
                settings,
                document,
                &mut show_settings,
                &mut settings_tab,
                gpus,
                gpu_name,
            );
            settings_changed |= settings_outcome.any;
            camera_settings_changed |= settings_outcome.camera_prefs;
            layout::draw_log_panel(ui, settings.rendering.show_log_panel);
            cancel_kernel_requested = layout::draw_bottom_panel(
                ui,
                fps,
                hovered_point,
                axis_system,
                document.display_unit(),
                pending_imports,
                pending_document_open,
                kernel_status.as_deref(),
                kernel_cancellable,
                kernel_progress,
                document_saving,
            );

            viewport_rect_logical = ui.available_rect_before_wrap();

            // Draw screen-space overlays in the viewport area (before other overlays)
            layout::draw_screen_space_overlays(
                ui.ctx(),
                viewport_rect_logical,
                screen_space_overlays,
            );
            layout::draw_screen_space_labels(ui.ctx(), viewport_rect_logical, screen_space_labels);

            if let Some(input) = orientation_input {
                cube_result =
                    orientation_cube::draw(ui.ctx(), viewport_rect_logical, input, &cube_config);
            }

            if let Some((path, draft)) = step_import_pending.as_mut() {
                step_import_dialog =
                    step_import_modal::draw_step_import_modal(ui.ctx(), path, draft);
            }

            if let Some((px, py)) = pivot_screen_pos {
                layout::draw_pivot_indicator(ui.ctx(), px, py);
            }
        });

        // Detect workbench change
        let workbench_changed = active_workbench != prev_workbench;
        if workbench_changed {
            // Reset tool when switching workbenches
            active_tool = ActiveTool::default();
        }

        self.show_settings = show_settings;
        self.settings_tab = settings_tab;
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

        // Fold this frame's interactions into commands. Application order is
        // decided by `apply_ui_commands`, not by the order of this list.
        let mut commands = Vec::new();
        if new_requested {
            commands.push(UiCommand::File(FileCommand::New));
        }
        if open_requested {
            commands.push(UiCommand::File(FileCommand::Open));
        }
        if save_requested {
            commands.push(UiCommand::File(FileCommand::Save));
        }
        if save_as_requested {
            commands.push(UiCommand::File(FileCommand::SaveAs));
        }
        if import_step_requested {
            commands.push(UiCommand::File(FileCommand::ImportStep));
        }
        if reset_view_requested {
            commands.push(UiCommand::FitView);
        }
        if cancel_kernel_requested {
            commands.push(UiCommand::CancelKernelJob);
        }
        if quit_requested {
            commands.push(UiCommand::Quit);
        }
        if let Some(view) = cube_result.snap_to_view {
            commands.push(UiCommand::CameraSnap(view));
        }
        if let Some(delta) = cube_result.rotate_delta {
            commands.push(UiCommand::CameraRotate(delta));
        }
        if settings_changed {
            commands.push(UiCommand::PersistSettings);
        }
        if camera_settings_changed {
            commands.push(UiCommand::ApplyCameraSettings);
        }
        if let Some(item) = tree_selection {
            commands.push(UiCommand::SelectTreeItem(item));
        }
        if let Some(item) = tree_activation {
            commands.push(UiCommand::ActivateTreeItem(item));
        }
        if let Some((node, visible)) = imported_visibility_change {
            commands.push(UiCommand::SetImportedVisibility { node, visible });
        }
        if let Some((feature, command)) = tree_feature_command {
            commands.push(UiCommand::TreeFeature { feature, command });
        }
        match step_import_dialog {
            StepImportDialogAction::Confirmed => commands.push(UiCommand::ConfirmStepImport),
            StepImportDialogAction::Cancelled => commands.push(UiCommand::CancelStepImport),
            StepImportDialogAction::None => {}
        }
        if finish_requested {
            commands.push(UiCommand::FinishSketch);
        }
        if let Some(req) = orient_request {
            commands.push(UiCommand::OrientCameraToPlane(req));
        }
        if workbench_changed {
            commands.push(UiCommand::SwitchWorkbench {
                from: prev_workbench,
                to: active_workbench.clone(),
            });
        }

        UiFrameOutput {
            submission: EguiSubmission {
                pixels_per_point: full_output.pixels_per_point,
                textures_delta: full_output.textures_delta,
                primitives,
            },
            viewport,
            active_tool,
            active_workbench,
            commands,
        }
    }
}

pub use feature_tree::{TreeFeatureCommand, TreeItemId};
