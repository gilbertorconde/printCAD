use crate::settings::{LightSource, ProjectionMode, UserSettings};
use egui::{self, Color32, Context, Ui};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    Camera,
    Lighting,
    Input,
    Rendering,
    About,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 5] = [
        SettingsTab::Camera,
        SettingsTab::Lighting,
        SettingsTab::Input,
        SettingsTab::Rendering,
        SettingsTab::About,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SettingsTab::Camera => "Camera",
            SettingsTab::Lighting => "Lighting",
            SettingsTab::Input => "Input",
            SettingsTab::Rendering => "Rendering",
            SettingsTab::About => "About",
        }
    }
}

pub(super) fn draw_settings_window(
    ctx: &Context,
    settings: &mut UserSettings,
    show_settings: &mut bool,
    settings_tab: &mut SettingsTab,
    gpus: &[String],
    gpu_name: Option<&str>,
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
                        changed |= camera_settings_ui(right, settings);
                    }
                    SettingsTab::Lighting => {
                        changed |= lighting_settings_ui(right, settings);
                    }
                    SettingsTab::Input => {
                        right.label("Input settings coming soon.");
                    }
                    SettingsTab::Rendering => {
                        changed |= render_settings_ui(right, settings, gpus);
                    }
                    SettingsTab::About => {
                        about_ui(right, gpu_name);
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
        .add(egui::Slider::new(&mut camera.orbit_sensitivity, 0.05..=2.0).text("Orbit sensitivity"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut camera.zoom_sensitivity, 0.01..=0.5).text("Zoom sensitivity"))
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

    egui::Grid::new("light_sources_grid")
        .num_columns(5)
        .spacing([10.0, 8.0])
        .show(ui, |ui| {
            ui.label("");
            ui.label("Horizontal");
            ui.label("Vertical");
            ui.label("Color");
            ui.label("Intensity");
            ui.end_row();

            changed |= light_source_row(ui, "Main light", &mut lighting.main_light);
            ui.end_row();

            changed |= light_source_row(ui, "Backlight", &mut lighting.backlight);
            ui.end_row();

            changed |= light_source_row(ui, "Fill light", &mut lighting.fill_light);
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

    egui::ComboBox::from_label("(App restart required)")
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
                settings.fps_cap
            };
            if (parsed - settings.fps_cap).abs() > f32::EPSILON {
                settings.fps_cap = parsed;
                changed = true;
            }
        }
    });

    ui.add_space(12.0);
    ui.separator();
    ui.label("Anti-aliasing");

    let msaa_options = [(1, "Off"), (2, "2x MSAA"), (4, "4x MSAA"), (8, "8x MSAA")];
    let current_msaa = settings.rendering.msaa_samples;
    let current_label = msaa_options
        .iter()
        .find(|(v, _)| *v == current_msaa)
        .map(|(_, l)| *l)
        .unwrap_or("4x MSAA");

    ui.horizontal(|ui| {
        ui.label("MSAA (requires restart):");
        egui::ComboBox::from_id_salt("msaa_combo")
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                for (value, label) in msaa_options {
                    if ui.selectable_label(current_msaa == value, label).clicked() {
                        settings.rendering.msaa_samples = value;
                        changed = true;
                    }
                }
            });
    });

    changed
}

fn light_source_row(ui: &mut Ui, label: &str, light: &mut LightSource) -> bool {
    let mut changed = false;

    changed |= ui.checkbox(&mut light.enabled, label).changed();
    changed |= ui
        .add(
            egui::DragValue::new(&mut light.horizontal_angle)
                .range(-180.0..=180.0)
                .speed(1.0)
                .suffix("°"),
        )
        .changed();
    changed |= ui
        .add(
            egui::DragValue::new(&mut light.vertical_angle)
                .range(-90.0..=90.0)
                .speed(1.0)
                .suffix("°"),
        )
        .changed();

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

    changed |= ui
        .add(
            egui::Slider::new(&mut light.intensity, 0.0..=1.0)
                .show_value(true)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        )
        .changed();

    changed
}

fn about_ui(ui: &mut Ui, gpu_name: Option<&str>) {
    ui.label("printCAD");
    ui.label("A parametric 3D CAD application");
    ui.add_space(12.0);
    ui.separator();
    ui.label("System Information");
    ui.add_space(4.0);
    if let Some(name) = gpu_name {
        ui.label(format!("GPU: {}", name));
    } else {
        ui.label("GPU: Unknown");
    }
    ui.add_space(12.0);
    ui.separator();
    ui.label("Version: 0.1.0 (pre-alpha)");
}
