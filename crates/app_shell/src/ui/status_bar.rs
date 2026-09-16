//! The status bar: activity or workbench state on the left, selection
//! summary, and mono readouts on the right (coordinates, dimensions,
//! navigation style, frame counters, the document server).

use axes::AxisSystem;
use core_document::{StatusItems, Unit, format_length_mm};
use egui::{RichText, Vec2};
use glam::Vec3;
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{mono_label, small_secondary_button, vseparator};

use super::overlays::rgb;

/// Everything the status bar reads this frame.
pub struct StatusBarInputs<'a> {
    pub fps: Option<f32>,
    pub scene_redraws_per_s: u32,
    pub hovered_point: Option<[f32; 3]>,
    pub axis_system: AxisSystem,
    pub display_unit: Unit,
    pub pending_imports: u32,
    pub pending_document_open: u32,
    pub kernel_status: Option<&'a str>,
    pub kernel_cancellable: bool,
    pub kernel_progress: Option<(u64, u64)>,
    pub server_label: &'a str,
    pub document_saving: bool,
    pub nav_style: &'a str,
    pub items: Option<&'a StatusItems>,
    pub preselect: Option<&'a str>,
    /// "w × h × d" of the selection, already formatted.
    pub dimensions: Option<&'a str>,
}

/// Returns true when the user asked to stop the running kernel job.
pub fn draw_status_bar(ui: &mut egui::Ui, inputs: &StatusBarInputs<'_>) -> bool {
    let mut cancel_requested = false;
    egui::Panel::bottom("status_bar")
        .exact_size(STATUS_BAR)
        .frame(
            egui::Frame::new()
                .fill(BG0)
                .inner_margin(egui::Margin::symmetric(10, 0))
                .stroke(egui::Stroke::NONE),
        )
        .show(ui, |ui| {
            let rect = ui.max_rect();
            ui.painter()
                .hline(rect.x_range(), rect.top(), egui::Stroke::new(1.0, BORDER));
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_4;
                draw_activity(ui, inputs, &mut cancel_requested);

                if let Some(sel) = inputs.items.and_then(|i| i.selection.as_deref()) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new("Selected:").font(sans(FONT_XS)).color(TEXT2));
                        ui.label(RichText::new(sel).font(sans(FONT_XS)).color(TEXT1));
                    });
                } else if let Some(pre) = inputs.preselect {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(
                            RichText::new("Preselected:")
                                .font(sans(FONT_XS))
                                .color(TEXT2),
                        );
                        ui.label(RichText::new(pre).font(sans(FONT_XS)).color(TEXT1));
                    });
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = SPACE_3;
                    if let Some(mode) = inputs.items.and_then(|i| i.mode.as_deref()) {
                        ui.label(RichText::new(mode).font(sans(FONT_XS)).color(TEXT2));
                        vseparator(ui, 14.0);
                    }
                    ui.label(
                        RichText::new(format!("Nav: {}", inputs.nav_style))
                            .font(sans(FONT_XS))
                            .color(TEXT2),
                    );
                    vseparator(ui, 14.0);
                    // Two numbers because they are two things: UI frames
                    // presented, and how often the 3D scene was re-rendered
                    // under them (cached otherwise).
                    let scene = if inputs.fps.is_none() || inputs.scene_redraws_per_s == 0 {
                        "scene: cached".to_string()
                    } else {
                        format!("scene: {}/s", inputs.scene_redraws_per_s)
                    };
                    mono_label(ui, scene, FONT_XS, TEXT3);
                    let fps = match inputs.fps {
                        Some(fps) if fps > 0.0 => format!("FPS {fps:.0}"),
                        Some(_) => "FPS …".to_string(),
                        // The loop is about to sleep; a frozen number would
                        // read as a live measurement.
                        None => "FPS idle".to_string(),
                    };
                    mono_label(ui, fps, FONT_XS, TEXT3);
                    mono_label(ui, inputs.server_label, FONT_XS, TEXT3);
                    vseparator(ui, 14.0);
                    if let Some(dim) = inputs.dimensions {
                        mono_label(ui, format!("Dim: {dim}"), FONT_XS, TEXT2);
                    }
                    mono_label(ui, coords_text(inputs), FONT_XS, TEXT2);
                });
            });
        });
    cancel_requested
}

fn coords_text(inputs: &StatusBarInputs<'_>) -> String {
    if let Some(coords) = inputs.items.and_then(|i| i.coords.as_deref()) {
        return coords.to_owned();
    }
    let axes = [
        ("H", inputs.axis_system.horizontal()),
        ("V", inputs.axis_system.vertical()),
        ("D", inputs.axis_system.depth()),
    ];
    match inputs.hovered_point {
        Some(pos) => {
            let canonical = inputs
                .axis_system
                .world_to_canonical(Vec3::from_array(pos))
                .to_array();
            axes.iter()
                .enumerate()
                .map(|(idx, (role, axis))| {
                    // Stored coordinates are millimetres; format through the
                    // document's display unit.
                    format!(
                        "{}({}) {}",
                        role,
                        axis.signed_label(),
                        format_length_mm(canonical[idx], inputs.display_unit, 2)
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ")
        }
        None => {
            let suffix = inputs.display_unit.short_label();
            axes.iter()
                .map(|(role, axis)| format!("{}({}) — {}", role, axis.signed_label(), suffix))
                .collect::<Vec<_>>()
                .join(" · ")
        }
    }
}

/// The far-left slot: a state dot with a title, or — while the kernel,
/// an open or a save is busy — a spinner or determinate bar with the
/// announced stage and a Cancel button.
fn draw_activity(ui: &mut egui::Ui, inputs: &StatusBarInputs<'_>, cancel: &mut bool) {
    let busy = inputs.pending_imports > 0 || inputs.pending_document_open > 0;
    if !busy {
        let (color, title) = match inputs.items.and_then(|i| i.state.clone()) {
            Some((rgb_color, title)) => (rgb(rgb_color, 1.0), title),
            None => (SUCCESS, "Ready".to_string()),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(6.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 3.0, color);
            ui.label(RichText::new(title).font(sans(FONT_XS)).color(TEXT2));
        });
        return;
    }

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        // A stage that announced its counts earns a real bar; anything
        // else keeps the honest spinner.
        match inputs.kernel_progress {
            Some((done, total)) if total > 0 => {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(120.0, 6.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 3.0, BG4);
                let mut fill = rect;
                fill.set_width(rect.width() * (done as f32 / total as f32));
                ui.painter().rect_filled(fill, 3.0, ACCENT);
                mono_label(ui, format!("{done}/{total}"), FONT_XS, TEXT3);
            }
            _ => {
                ui.add(egui::Spinner::new().size(12.0).color(ACCENT));
            }
        }
        let mut parts = Vec::new();
        match inputs.kernel_status {
            Some(status) => parts.push(status.to_owned()),
            // Nothing announced yet: say only what is certain, which is
            // how many jobs are outstanding.
            None if inputs.pending_imports == 1 => parts.push("Working…".to_string()),
            None if inputs.pending_imports > 1 => {
                parts.push(format!("Working… ({} jobs)", inputs.pending_imports));
            }
            None => {}
        }
        if inputs.document_saving {
            parts.push("Saving document…".to_string());
        } else if inputs.pending_document_open > 0 {
            parts.push("Opening document…".to_string());
        }
        ui.label(
            RichText::new(parts.join(" · "))
                .font(sans(FONT_XS))
                .color(TEXT1),
        );
        if inputs.kernel_cancellable && small_secondary_button(ui, "Cancel").clicked() {
            *cancel = true;
        }
    });
}
