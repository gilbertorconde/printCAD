//! The Display page: camera, lighting and rendering.

use super::*;

pub(super) fn display_page(
    ui: &mut Ui,
    state: &mut PreferencesState,
    inputs: &PreferencesInputs<'_>,
    filter: &str,
) {
    let draft = &mut state.draft;
    match state.tab {
        0 => {
            let camera = &mut draft.camera;
            let preset_hint = camera.axis_preset.description();
            pref_group(
                ui,
                "Camera",
                vec![
                    PrefRow::select(
                        "Projection",
                        "prefs_projection",
                        &mut camera.projection,
                        &[
                            (ProjectionMode::Perspective, "Perspective"),
                            (ProjectionMode::Orthographic, "Orthographic"),
                        ],
                    ),
                    PrefRow::qty(
                        "Field of view",
                        QtyField::degrees(&mut camera.fov_degrees).range(10.0..=120.0),
                    )
                    .hint("Vertical, for perspective"),
                    PrefRow::qty(
                        "Orthographic height",
                        QtyField::mm(&mut camera.ortho_height_mm).range(1.0..=500_000.0),
                    ),
                    PrefRow::qty(
                        "Minimum focal distance",
                        QtyField::mm(&mut camera.min_focal_distance).range(0.1..=50.0),
                    ),
                    PrefRow::qty(
                        "Maximum focal distance",
                        QtyField::mm(&mut camera.max_focal_distance).range(50.0..=500_000.0),
                    ),
                    PrefRow::toggle("Auto near / far planes", &mut camera.auto_near_far)
                        .hint("Clip planes follow the scene bounds"),
                    PrefRow::qty(
                        "Near distance ratio",
                        QtyField::new(&mut camera.near_far_near_ratio)
                            .range(0.00001..=0.1)
                            .speed(0.0001)
                            .decimals(5),
                    )
                    .hint("Times the focal distance"),
                    PrefRow::qty(
                        "Far / near ratio cap",
                        QtyField::new(&mut camera.near_far_depth_ratio_cap)
                            .range(1000.0..=500_000.0)
                            .speed(100.0)
                            .decimals(0),
                    ),
                    PrefRow::qty(
                        "Far plane margin",
                        QtyField::mm(&mut camera.near_far_margin).range(1.0..=10_000.0),
                    ),
                    PrefRow::qty(
                        "View transition",
                        QtyField::new(&mut camera.view_transition_ms)
                            .unit("ms")
                            .range(120.0..=1200.0)
                            .speed(10.0)
                            .decimals(0),
                    ),
                    PrefRow::select(
                        "Axis preset",
                        "prefs_axis_preset",
                        &mut camera.axis_preset,
                        &[
                            (AxisPreset::ALL[0], AxisPreset::ALL[0].label()),
                            (AxisPreset::ALL[1], AxisPreset::ALL[1].label()),
                            (AxisPreset::ALL[2], AxisPreset::ALL[2].label()),
                        ],
                    )
                    .hint(preset_hint),
                ],
                filter,
            );
        }
        1 => {
            let lighting = &mut draft.lighting;
            let mut rows = Vec::new();
            for (label, light) in [
                ("Main light", &mut lighting.main_light),
                ("Backlight", &mut lighting.backlight),
                ("Fill light", &mut lighting.fill_light),
            ] {
                rows.push(
                    PrefRow::new(label, move |ui| {
                        let mut changed = false;
                        changed |= QtyField::new(&mut light.intensity)
                            .range(0.0..=1.0)
                            .speed(0.01)
                            .width(70.0)
                            .show(ui);
                        let mut color = egui::Color32::from_rgb(
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
                        changed |= QtyField::degrees(&mut light.vertical_angle)
                            .range(-90.0..=90.0)
                            .decimals(0)
                            .width(70.0)
                            .show(ui);
                        changed |= QtyField::degrees(&mut light.horizontal_angle)
                            .range(-180.0..=180.0)
                            .decimals(0)
                            .width(70.0)
                            .show(ui);
                        changed |= ui_kit::widgets::toggle(ui, &mut light.enabled, label).changed();
                        changed
                    })
                    .hint("On · horizontal · vertical · color · intensity"),
                );
            }
            pref_group(ui, "Light sources", rows, filter);
            pref_group(
                ui,
                "Ambient and specular",
                vec![
                    PrefRow::color("Ambient color", &mut lighting.ambient_color),
                    PrefRow::qty(
                        "Ambient intensity",
                        QtyField::new(&mut lighting.ambient_intensity)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    ),
                    PrefRow::qty(
                        "Specular shininess",
                        QtyField::new(&mut lighting.specular_shininess)
                            .range(8.0..=128.0)
                            .speed(1.0)
                            .decimals(0),
                    )
                    .hint("Larger is a tighter highlight"),
                    PrefRow::qty(
                        "Specular intensity",
                        QtyField::new(&mut lighting.specular_intensity)
                            .range(0.0..=1.0)
                            .speed(0.01),
                    ),
                ],
                filter,
            );
            pref_group(
                ui,
                "Edge lines",
                vec![
                    PrefRow::color("Edge color", &mut lighting.edge_line_color),
                    PrefRow::qty(
                        "Edge width",
                        QtyField::new(&mut lighting.edge_line_width)
                            .unit("px")
                            .range(0.5..=8.0)
                            .speed(0.1)
                            .decimals(1),
                    ),
                ],
                filter,
            );
        }
        _ => {
            let gpus = inputs.gpus;
            let preferred_gpu = &mut draft.preferred_gpu;
            let msaa = &mut draft.rendering.msaa_samples;
            let mut gpu_rows = vec![
                PrefRow::new("Preferred GPU", move |ui| {
                    let current = preferred_gpu
                        .clone()
                        .unwrap_or_else(|| "Automatic".to_string());
                    let mut selected = current.clone();
                    let options: Vec<(String, String)> = std::iter::once("Automatic".to_string())
                        .chain(gpus.iter().cloned())
                        .map(|g| (g.clone(), g))
                        .collect();
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("prefs_gpu")
                        .width(220.0)
                        .selected_text(RichText::new(&current).font(sans(FONT_SM)))
                        .show_ui(ui, |ui| {
                            for (value, label) in &options {
                                if ui
                                    .selectable_value(&mut selected, value.clone(), label)
                                    .clicked()
                                {
                                    changed = true;
                                }
                            }
                        });
                    if changed {
                        *preferred_gpu = (selected != "Automatic").then_some(selected);
                    }
                    changed
                })
                .hint("Takes effect after a restart"),
            ];
            gpu_rows.push(
                PrefRow::select(
                    "Anti-aliasing",
                    "prefs_msaa",
                    msaa,
                    &[(1, "Off"), (2, "2× MSAA"), (4, "4× MSAA"), (8, "8× MSAA")],
                )
                .hint("Takes effect after a restart"),
            );
            gpu_rows.push(
                PrefRow::qty(
                    "Curve smoothness",
                    QtyField::new(&mut draft.rendering.curve_step_deg)
                        .unit("°")
                        .range(2.0..=45.0)
                        .speed(0.5)
                        .decimals(1),
                )
                .hint("The turn between two drawn facets of a curved face; smaller is rounder"),
            );
            pref_group(ui, "Rendering", gpu_rows, filter);
            pref_group(
                ui,
                "Scene guides",
                vec![
                    PrefRow::toggle("Grid", &mut draft.rendering.show_grid),
                    PrefRow::select(
                        "Grid plane",
                        "prefs_grid_plane",
                        &mut draft.rendering.grid_plane,
                        &[
                            (settings::GridPlane::FollowView, "Follow the view"),
                            (settings::GridPlane::Ground, "Ground plane only"),
                        ],
                    ),
                    PrefRow::select(
                        "Grid lines",
                        "prefs_grid_style",
                        &mut draft.rendering.grid_style,
                        &[
                            (settings::GridStyle::Dotted, "Dotted"),
                            (settings::GridStyle::Dashed, "Dashed"),
                            (settings::GridStyle::Solid, "Solid"),
                        ],
                    ),
                    PrefRow::color("Grid colour", &mut draft.rendering.grid_color),
                    PrefRow::qty(
                        "Grid opacity",
                        QtyField::new(&mut draft.rendering.grid_opacity)
                            .range(0.05..=1.0)
                            .speed(0.01)
                            .decimals(2),
                    )
                    .hint("How strongly the grid shows"),
                    PrefRow::toggle("Origin planes", &mut draft.rendering.show_origin_planes),
                    PrefRow::toggle("Origin lines", &mut draft.rendering.show_origin_lines)
                        .hint("The world axes through the origin on the grid's plane"),
                    PrefRow::toggle("Solid section", &mut draft.rendering.solid_section_caps).hint(
                        "A part cut by the clipping plane shows a hatched cap of material; \
                             off, its inside shows through the cut",
                    ),
                ],
                filter,
            );

            let selection_color = &mut draft.rendering.selection_color;
            pref_group(
                ui,
                "Selection",
                vec![
                    PrefRow::new("Face colour", move |ui| {
                        let mut color = egui::Color32::from_rgb(
                            (selection_color[0] * 255.0) as u8,
                            (selection_color[1] * 255.0) as u8,
                            (selection_color[2] * 255.0) as u8,
                        );
                        let changed = ui.color_edit_button_srgba(&mut color).changed();
                        if changed {
                            *selection_color = [
                                color.r() as f32 / 255.0,
                                color.g() as f32 / 255.0,
                                color.b() as f32 / 255.0,
                            ];
                        }
                        changed
                    })
                    .hint("What a selected face is painted"),
                    PrefRow::qty(
                        "Face opacity",
                        QtyField::new(&mut draft.rendering.selection_opacity)
                            .range(0.1..=f64::from(settings::MAX_SELECTION_OPACITY))
                            .speed(0.01)
                            .decimals(2),
                    )
                    .hint("How much paint goes over a selected face or body"),
                ],
                filter,
            );

            let preview_color = &mut draft.rendering.preview_color;
            pref_group(
                ui,
                "Feature preview",
                vec![
                    PrefRow::new("Colour", move |ui| {
                        let mut color = egui::Color32::from_rgb(
                            (preview_color[0] * 255.0) as u8,
                            (preview_color[1] * 255.0) as u8,
                            (preview_color[2] * 255.0) as u8,
                        );
                        let changed = ui.color_edit_button_srgba(&mut color).changed();
                        if changed {
                            *preview_color = [
                                color.r() as f32 / 255.0,
                                color.g() as f32 / 255.0,
                                color.b() as f32 / 255.0,
                            ];
                        }
                        changed
                    })
                    .hint("What a feature being edited adds or takes is drawn in"),
                    PrefRow::qty(
                        "Opacity",
                        QtyField::new(&mut draft.rendering.preview_opacity)
                            .range(0.05..=f64::from(settings::MAX_SELECTION_OPACITY))
                            .speed(0.01)
                            .decimals(2),
                    )
                    .hint("How solid its faces look; its edges are drawn whole"),
                ],
                filter,
            );
        }
    }
}
