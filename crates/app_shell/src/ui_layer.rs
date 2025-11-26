use egui::{self, Color32, Context, Ui};
use egui_winit::{egui as egui_core, State};
use render_vk::EguiSubmission;
use settings::{LightSource, ProjectionMode, UserSettings};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Camera,
    Lighting,
    Input,
    Rendering,
}

impl SettingsTab {
    const ALL: [SettingsTab; 4] = [
        SettingsTab::Camera,
        SettingsTab::Lighting,
        SettingsTab::Input,
        SettingsTab::Rendering,
    ];

    fn label(&self) -> &'static str {
        match self {
            SettingsTab::Camera => "Camera",
            SettingsTab::Lighting => "Lighting",
            SettingsTab::Input => "Input",
            SettingsTab::Rendering => "Rendering",
        }
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
    settings_tab: SettingsTab,
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
            settings_tab: SettingsTab::Camera,
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

    pub fn run(
        &mut self,
        window: &Window,
        settings: &mut UserSettings,
        orientation_input: Option<&OrientationCubeInput>,
        fps: f32,
        gpu_name: Option<&str>,
        gpus: &[String],
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
            Self::draw_top_panel(ctx, &mut active_workbench, &mut show_settings);
            Self::draw_left_panel(ctx, active_workbench, &mut active_tool);
            Self::draw_right_panel(ctx);
            settings_changed |= Self::draw_settings_window(
                ctx,
                settings,
                &mut show_settings,
                &mut settings_tab,
                gpus,
            );
            Self::draw_bottom_panel(ctx, fps, gpu_name);

            viewport_rect_logical = ctx.available_rect();

            if let Some(input) = orientation_input {
                cube_result = orientation_cube::draw(ctx, input, &cube_config);
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

    fn draw_top_panel(
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
                    ui.selectable_value(
                        active_workbench,
                        ActiveWorkbench::PartDesign,
                        "Part Design",
                    );
                    ui.add_space(12.0);
                    if ui.button("Settings").clicked() {
                        *show_settings = true;
                    }
                });
            });
    }

    fn draw_left_panel(
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

    fn draw_right_panel(ctx: &Context) {
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

    fn draw_settings_window(
        ctx: &Context,
        settings: &mut UserSettings,
        show_settings: &mut bool,
        settings_tab: &mut SettingsTab,
        gpus: &[String],
    ) -> bool {
        if !*show_settings {
            return false;
        }

        let mut changed = false;
        egui::Window::new("Settings")
            .open(show_settings)
            .default_width(520.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.columns(2, |columns| {
                    let left = &mut columns[0];
                    left.set_min_width(140.0);
                    left.heading("Tabs");
                    left.separator();
                    for tab in SettingsTab::ALL {
                        let selected = *settings_tab == tab;
                        if left.selectable_label(selected, tab.label()).clicked() {
                            *settings_tab = tab;
                        }
                    }

                    let right = &mut columns[1];
                    right.heading(settings_tab.label());
                    right.separator();
                    match settings_tab {
                        SettingsTab::Camera => {
                            changed |= Self::camera_settings_ui(right, settings);
                        }
                        SettingsTab::Lighting => {
                            changed |= Self::lighting_settings_ui(right, settings);
                        }
                        SettingsTab::Input => {
                            right.label("Input settings coming soon.");
                        }
                        SettingsTab::Rendering => {
                            changed |= Self::render_settings_ui(right, settings, gpus);
                        }
                    }
                });
            });
        changed
    }

    fn camera_settings_ui(ui: &mut Ui, settings: &mut UserSettings) -> bool {
        let camera = &mut settings.camera;
        let mut changed = false;

        changed |= ui
            .add(
                egui::Slider::new(&mut camera.orbit_sensitivity, 0.05..=2.0)
                    .text("Orbit sensitivity"),
            )
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut camera.zoom_sensitivity, 0.01..=0.5)
                    .text("Zoom sensitivity"),
            )
            .changed();
        changed |= ui
            .checkbox(&mut camera.invert_zoom, "Invert zoom")
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut camera.min_distance, 0.05..=5.0).text("Min distance"))
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut camera.max_distance, 5.0..=2000.0).text("Max distance"))
            .changed();

        ui.separator();
        ui.label("Projection");
        ui.horizontal(|ui| {
            changed |= ui
                .radio_value(
                    &mut camera.projection,
                    ProjectionMode::Perspective,
                    "Perspective",
                )
                .changed();
            changed |= ui
                .radio_value(
                    &mut camera.projection,
                    ProjectionMode::Orthographic,
                    "Orthographic",
                )
                .changed();
        });

        if camera.projection == ProjectionMode::Perspective {
            ui.separator();
            ui.label("Field of view");
            changed |= ui
                .add(
                    egui::Slider::new(&mut camera.fov_degrees, 10.0..=120.0)
                        .text("Vertical FOV (degrees)"),
                )
                .changed();
        }

        changed
    }

    fn lighting_settings_ui(ui: &mut Ui, settings: &mut UserSettings) -> bool {
        let lighting = &mut settings.lighting;
        let mut changed = false;

        ui.label("Light Sources");
        ui.separator();

        // Create a grid layout similar to FreeCAD
        egui::Grid::new("light_sources_grid")
            .num_columns(5)
            .spacing([10.0, 8.0])
            .show(ui, |ui| {
                // Header row
                ui.label("");
                ui.label("Horizontal");
                ui.label("Vertical");
                ui.label("Color");
                ui.label("Intensity");
                ui.end_row();

                // Main light
                changed |= Self::light_source_row(ui, "Main light", &mut lighting.main_light);
                ui.end_row();

                // Backlight
                changed |= Self::light_source_row(ui, "Backlight", &mut lighting.backlight);
                ui.end_row();

                // Fill light
                changed |= Self::light_source_row(ui, "Fill light", &mut lighting.fill_light);
                ui.end_row();
            });

        ui.add_space(10.0);
        ui.separator();
        ui.label("Ambient Light");

        ui.horizontal(|ui| {
            ui.label("Color:");
            let mut color = Color32::from_rgb(
                (lighting.ambient_color[0] * 255.0) as u8,
                (lighting.ambient_color[1] * 255.0) as u8,
                (lighting.ambient_color[2] * 255.0) as u8,
            );
            if ui.color_edit_button_srgba(&mut color).changed() {
                lighting.ambient_color = [
                    color.r() as f32 / 255.0,
                    color.g() as f32 / 255.0,
                    color.b() as f32 / 255.0,
                ];
                changed = true;
            }
        });

        ui.horizontal(|ui| {
            ui.label("Intensity:");
            changed |= ui
                .add(egui::Slider::new(&mut lighting.ambient_intensity, 0.0..=1.0).show_value(true))
                .changed();
        });

        changed
    }

    fn render_settings_ui(ui: &mut Ui, settings: &mut UserSettings, gpus: &[String]) -> bool {
        let mut changed = false;
        ui.label("GPU");
        ui.separator();

        let current = settings
            .preferred_gpu
            .as_deref()
            .unwrap_or("Automatic")
            .to_string();
        let mut selected = current.clone();

        egui::ComboBox::from_label("(App restarted required)")
            .selected_text(&selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut selected, "Automatic".to_string(), "Automatic");
                for name in gpus {
                    ui.selectable_value(&mut selected, name.clone(), name);
                }
            });

        if selected != current {
            if selected == "Automatic" {
                settings.preferred_gpu = None;
            } else {
                settings.preferred_gpu = Some(selected);
            }
            changed = true;
        }

        if gpus.is_empty() {
            ui.label("No Vulkan-capable GPUs detected.");
        }

        ui.add_space(12.0);
        ui.separator();
        ui.label("Frame pacing");

        // Plain numeric input for FPS cap (0 = uncapped, no explicit max)
        let mut cap_str = if settings.fps_cap <= 0.0 {
            String::new()
        } else {
            format!("{:.0}", settings.fps_cap)
        };

        ui.horizontal(|ui| {
            ui.label("FPS cap (0 = uncapped):");
            let response = ui.add(
                egui::TextEdit::singleline(&mut cap_str)
                    .desired_width(80.0)
                    .hint_text("0"),
            );
            if response.changed() {
                let s = cap_str.trim();
                let parsed = if s.is_empty() {
                    0.0
                } else if let Ok(v) = s.parse::<f32>() {
                    v.max(0.0)
                } else {
                    // Invalid input: don't change the setting yet
                    settings.fps_cap
                };
                if (parsed - settings.fps_cap).abs() > f32::EPSILON {
                    settings.fps_cap = parsed;
                    changed = true;
                }
            }
        });

        changed
    }

    fn light_source_row(ui: &mut Ui, label: &str, light: &mut LightSource) -> bool {
        let mut changed = false;

        // Enabled checkbox with label
        changed |= ui.checkbox(&mut light.enabled, label).changed();

        // Horizontal angle
        changed |= ui
            .add(
                egui::DragValue::new(&mut light.horizontal_angle)
                    .range(-180.0..=180.0)
                    .speed(1.0)
                    .suffix("°"),
            )
            .changed();

        // Vertical angle
        changed |= ui
            .add(
                egui::DragValue::new(&mut light.vertical_angle)
                    .range(-90.0..=90.0)
                    .speed(1.0)
                    .suffix("°"),
            )
            .changed();

        // Color picker
        let mut color = Color32::from_rgb(
            (light.color[0] * 255.0) as u8,
            (light.color[1] * 255.0) as u8,
            (light.color[2] * 255.0) as u8,
        );
        if ui.color_edit_button_srgba(&mut color).changed() {
            light.color = [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
            ];
            changed = true;
        }

        // Intensity slider
        changed |= ui
            .add(
                egui::Slider::new(&mut light.intensity, 0.0..=1.0)
                    .show_value(true)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
            )
            .changed();

        changed
    }

    fn draw_bottom_panel(ctx: &Context, fps: f32, gpu_name: Option<&str>) {
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let fps_text = if fps > 0.0 {
                    format!("FPS: {:.1}", fps)
                } else {
                    "FPS: …".to_string()
                };
                ui.label(fps_text);
                ui.separator();
                if let Some(name) = gpu_name {
                    ui.label(format!("GPU: {}", name));
                } else {
                    ui.label("GPU: Unknown");
                }
            });
        });
    }
}
