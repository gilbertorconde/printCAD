use axes::AxisPreset;
use core_document::{Document, Unit};
use egui::{self, Color32, Context, Ui};
use settings::{
    LightSource, NavigationStyle, OrbitYawAxis, ProjectionMode, UserSettings,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SettingsTab {
    Document,
    Camera,
    Lighting,
    Input,
    Rendering,
    About,
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 6] = [
        SettingsTab::Document,
        SettingsTab::Camera,
        SettingsTab::Lighting,
        SettingsTab::Input,
        SettingsTab::Rendering,
        SettingsTab::About,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            SettingsTab::Document => "Document",
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
    document: &mut Document,
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
                    SettingsTab::Document => {
                        changed |= document_settings_ui(right, document);
                    }
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

fn document_settings_ui(ui: &mut Ui, document: &mut Document) -> bool {
    let mut changed = false;

    ui.label("Display unit");
    ui.weak(
        "Controls how lengths and coordinates are presented. \
         Internal storage is always in millimetres, so switching units \
         never alters geometry.",
    );

    let current = document.display_unit();
    let mut selected = current;
    egui::ComboBox::from_id_salt("document_display_unit_combo")
        .width(260.0)
        .selected_text(current.long_label())
        .show_ui(ui, |ui| {
            for unit in Unit::ALL {
                if ui
                    .selectable_value(&mut selected, unit, unit.long_label())
                    .clicked()
                {
                    // selectable_value already mutates `selected`; no extra work.
                }
            }
        });

    if selected != current {
        document.set_display_unit(selected);
        changed = true;
    }

    changed
}

fn camera_settings_ui(ui: &mut Ui, settings: &mut UserSettings) -> bool {
    let camera = &mut settings.camera;
    let mut changed = false;

    egui::ComboBox::from_id_salt("nav_style_panel")
        .width(260.0)
        .selected_text(match camera.navigation_style {
            NavigationStyle::Gesture => "Gesture navigation",
            NavigationStyle::Cad => "CAD (future)",
        })
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(&mut camera.navigation_style, NavigationStyle::Gesture, "Gesture")
                .changed()
            {
                changed = true;
            }
            if ui
                .selectable_value(&mut camera.navigation_style, NavigationStyle::Cad, "CAD")
                .changed()
            {
                changed = true;
            }
        });

    changed |= ui
        .checkbox(&mut camera.zoom_to_cursor, "Zoom to cursor")
        .changed();
    changed |= ui.checkbox(&mut camera.invert_zoom, "Invert zoom").changed();
    changed |= ui
        .add(egui::Slider::new(&mut camera.wheel_zoom_factor, 0.7..=0.995).text("Wheel step factor"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut camera.orbit_sensitivity, 0.05..=2.0).text("Orbit sensitivity"))
        .changed();
    changed |= ui
        .add(egui::Slider::new(&mut camera.pan_sensitivity, 0.1..=3.0).text("Pan sensitivity"))
        .changed();

    egui::ComboBox::from_id_salt("orbit_yaw_panel")
        .width(260.0)
        .selected_text(match camera.orbit_yaw_axis {
            OrbitYawAxis::WorldUp => "Orbit yaw: world up",
            OrbitYawAxis::CameraUp => "Orbit yaw: camera up",
        })
        .show_ui(ui, |ui| {
            if ui
                .selectable_value(
                    &mut camera.orbit_yaw_axis,
                    OrbitYawAxis::WorldUp,
                    "World up",
                )
                .changed()
            {
                changed = true;
            }
            if ui
                .selectable_value(
                    &mut camera.orbit_yaw_axis,
                    OrbitYawAxis::CameraUp,
                    "Camera up",
                )
                .changed()
            {
                changed = true;
            }
        });

    changed |= ui
        .add(
            egui::Slider::new(&mut camera.min_focal_distance, 0.1..=50.0)
                .text("Min focal distance (mm)"),
        )
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut camera.max_focal_distance, 50.0..=500_000.0)
                .text("Max focal distance (mm)"),
        )
        .changed();

    ui.separator();
    ui.label("Axis preset");
    egui::ComboBox::from_id_salt("axis_preset_combo")
        .width(260.0)
        .selected_text(camera.axis_preset.label())
        .show_ui(ui, |ui| {
            for preset in AxisPreset::ALL {
                if ui
                    .selectable_value(&mut camera.axis_preset, preset, preset.label())
                    .changed()
                {
                    changed = true;
                }
            }
        });
    ui.weak(camera.axis_preset.description());

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
    } else {
        changed |= ui
            .add(
                egui::Slider::new(&mut camera.ortho_height_mm, 1.0..=500_000.0)
                    .text("Ortho height (mm)"),
            )
            .changed();
    }

    ui.separator();
    changed |= ui
        .checkbox(&mut camera.auto_near_far, "Auto near/far from scene bounds")
        .changed();
    if camera.auto_near_far {
        changed |= ui
            .add(
                egui::Slider::new(&mut camera.near_far_near_ratio, 0.00001..=0.1)
                    .text("Near distance ratio (× focal distance)"),
            )
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut camera.near_far_depth_ratio_cap, 1000.0..=500_000.0)
                    .text("Far/near ratio cap"),
            )
            .changed();
        changed |= ui
            .add(
                egui::Slider::new(&mut camera.near_far_margin, 1.0..=10_000.0)
                    .text("Far-plane margin (mm)"),
            )
            .changed();
    }

    changed |= ui
        .add(egui::Slider::new(&mut camera.view_transition_ms, 120.0..=1200.0).text(
            "View transition (ms)",
        ))
        .changed();
    changed |= ui
        .add(
            egui::Slider::new(&mut camera.click_drag_threshold_px, 2.0..=12.0)
                .text("Click ↔ drag threshold (px)"),
        )
        .changed();

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

    ui.add_space(10.0);
    ui.separator();
    ui.label("Face edge lines");
    ui.horizontal(|ui| {
        ui.label("Color:");
        let mut color = Color32::from_rgb(
            (lighting.edge_line_color[0] * 255.0) as u8,
            (lighting.edge_line_color[1] * 255.0) as u8,
            (lighting.edge_line_color[2] * 255.0) as u8,
        );
        if ui.color_edit_button_srgba(&mut color).changed() {
            lighting.edge_line_color = [
                color.r() as f32 / 255.0,
                color.g() as f32 / 255.0,
                color.b() as f32 / 255.0,
            ];
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Width (px):");
        changed |= ui
            .add(
                egui::Slider::new(&mut lighting.edge_line_width, 0.5..=8.0).show_value(true),
            )
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
    ui.label("Debugging");
    changed |= ui
        .checkbox(
            &mut settings.rendering.show_log_panel,
            "Show in-app log panel at bottom",
        )
        .changed();

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
