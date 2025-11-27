mod layout;
mod settings_panel;

use egui::Context;
use egui_winit::{egui as egui_core, State};
use render_vk::EguiSubmission;
use settings::UserSettings;
use winit::{event::WindowEvent, window::Window};

use crate::orientation_cube::{
    self, CameraSnapView, OrientationCubeConfig, OrientationCubeInput, OrientationCubeResult,
    RotateDelta,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveWorkbench {
    Sketch,
    PartDesign,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTool {
    Select,
    SketchLine,
    SketchCircle,
    Pad,
    Pocket,
}

impl Default for ActiveTool {
    fn default() -> Self {
        ActiveTool::Select
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ViewportRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct UiFrameResult {
    pub submission: EguiSubmission,
    pub settings_changed: bool,
    pub active_tool: ActiveTool,
    pub snap_to_view: Option<CameraSnapView>,
    pub rotate_delta: Option<RotateDelta>,
    pub viewport: ViewportRect,
}

pub struct UiLayer {
    ctx: Context,
    state: State,
    active_workbench: ActiveWorkbench,
    active_tool: ActiveTool,
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

        Self {
            ctx,
            state,
            active_workbench: ActiveWorkbench::Sketch,
            active_tool: ActiveTool::default(),
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

    #[allow(clippy::too_many_arguments)]
    pub fn run(
        &mut self,
        window: &Window,
        settings: &mut UserSettings,
        orientation_input: Option<&OrientationCubeInput>,
        fps: f32,
        gpu_name: Option<&str>,
        gpus: &[String],
        hovered_point: Option<[f32; 3]>,
        pivot_screen_pos: Option<(f32, f32)>,
    ) -> UiFrameResult {
        let raw_input = self.state.take_egui_input(window);
        let mut active_workbench = self.active_workbench;
        let mut active_tool = self.active_tool;
        let mut show_settings = self.show_settings;
        let mut settings_tab = self.settings_tab;

        let cube_config = self.orientation_cube_config.clone();
        let mut settings_changed = false;
        let mut cube_result = OrientationCubeResult::default();
        let mut viewport_rect_logical = egui::Rect::NOTHING;

        let full_output = self.ctx.run(raw_input, |ctx| {
            layout::draw_top_panel(ctx, &mut active_workbench, &mut show_settings);
            layout::draw_left_panel(ctx, active_workbench, &mut active_tool);
            layout::draw_right_panel(ctx);
            settings_changed |= settings_panel::draw_settings_window(
                ctx,
                settings,
                &mut show_settings,
                &mut settings_tab,
                gpus,
                gpu_name,
            );
            layout::draw_bottom_panel(ctx, fps, hovered_point);

            viewport_rect_logical = ctx.available_rect();

            if let Some(input) = orientation_input {
                cube_result = orientation_cube::draw(ctx, input, &cube_config);
            }

            if let Some((px, py)) = pivot_screen_pos {
                layout::draw_pivot_indicator(ctx, px, py);
            }
        });

        self.active_workbench = active_workbench;
        self.active_tool = active_tool;
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

        UiFrameResult {
            submission: EguiSubmission {
                pixels_per_point: full_output.pixels_per_point,
                textures_delta: full_output.textures_delta,
                primitives,
            },
            settings_changed,
            active_tool,
            snap_to_view: cube_result.snap_to_view,
            rotate_delta: cube_result.rotate_delta,
            viewport,
        }
    }
}
