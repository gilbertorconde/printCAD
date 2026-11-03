//! The task panel of a surface step and the bench's Preferences page.
//!
//! A step's inputs are lists: each row names a sketch or an edge, with a
//! cross to take it out; "Add the selection" takes what is selected now
//! (a sketch in the tree, edges and faces in the view; a list of curves or
//! edges takes a face's edges). Every
//! edit writes the step at once and it builds again.

use core_document::{FeatureId, WorkbenchRuntimeContext};
use egui::{RichText, Ui};
use kernel_api::Continuity;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    Note, PrefRow, QtyField, check_row, field_label, icon_button, note_card, pref_group,
    select_field, small_secondary_button,
};
use ui_kit::{sans, sans_medium};

use crate::Options;
use crate::feature::{Axis, CurveRef, Direction, EdgePick, FacePick, PlaneRef, SurfaceFeature};

const FIELD: f32 = 150.0;

/// What is selected now: curves and faces a step could take, and the
/// edges round the faces.
pub type Selection = (Vec<CurveRef>, Vec<FacePick>, Vec<CurveRef>);

/// Draw the step's editor; whether anything changed.
pub fn editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    id: FeatureId,
    feature: &mut SurfaceFeature,
    selection: Option<Selection>,
) -> bool {
    let (picked, faces, outline) = selection.unwrap_or_default();
    // A list of curves or edges takes a picked face's edges too.
    let mut around = picked.clone();
    around.extend(outline.into_iter().filter(|c| !picked.contains(c)));
    let mut changed = false;
    if let Some(waits) = feature.kind().waits {
        note_card(
            ui,
            Note::Warning,
            Some("Waits on the geometry kernel"),
            &format!("{waits}. The step keeps its settings and builds once the kernel can."),
        );
    }
    if let Some(error) = ctx
        .document
        .get_feature_meta(id)
        .and_then(|n| n.error.clone())
    {
        note_card(ui, Note::Error, Some("Does not build"), &error);
    }
    ui.add_space(SPACE_1);
    match feature {
        SurfaceFeature::Extrude {
            curves,
            direction,
            length,
            symmetric,
            reversed,
        } => {
            changed |= curve_list(ui, ctx, "Curves", curves, &around);
            changed |= length_row(ui, "Length", length);
            changed |= direction_row(ui, direction);
            changed |= check_row(ui, symmetric, "Both ways, half each").changed();
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        SurfaceFeature::Revolve {
            curves,
            axis,
            angle_deg,
        } => {
            changed |= curve_list(ui, ctx, "Curves", curves, &around);
            changed |= axis_row(ui, axis);
            changed |= row(ui, "Angle", |ui| {
                QtyField::degrees(angle_deg).range(0.1..=360.0).show(ui)
            });
        }
        SurfaceFeature::PlanarFill { curves } => {
            hint(
                ui,
                "Closed loops in one plane; a loop inside another is a hole.",
            );
            changed |= curve_list(ui, ctx, "Loops", curves, &around);
        }
        SurfaceFeature::Fill {
            boundary,
            continuity,
        } => {
            hint(
                ui,
                "Curves meeting end to end round the hole: four today. Pick edges of the \
                 body's sheets to fill between them.",
            );
            changed |= curve_list(ui, ctx, "Boundary", boundary, &around);
            changed |= continuity_row(ui, continuity);
        }
        SurfaceFeature::Ruled { first, second } => {
            changed |= one_curve(ui, ctx, "First curve", first, &picked);
            changed |= one_curve(ui, ctx, "Second curve", second, &picked);
        }
        SurfaceFeature::Loft {
            sections,
            closed,
            guides,
        } => {
            hint(
                ui,
                "One curve per section, in the order the surface passes them.",
            );
            changed |= curve_list(ui, ctx, "Sections", sections, &picked);
            hint(
                ui,
                "Guides shape the surface between the sections: each crosses every \
                 section once, in order. A guided loft is one face, through sections of \
                 one curve each, and does not close.",
            );
            changed |= curve_list(ui, ctx, "Guides", guides, &picked);
            changed |= check_row(ui, closed, "Closed back to the first").changed();
        }
        SurfaceFeature::Sweep {
            profile,
            path,
            second_rail,
        } => {
            changed |= curve_list(ui, ctx, "Profile", profile, &picked);
            changed |= curve_list(ui, ctx, "Path", path, &picked);
            hint(
                ui,
                "With a second rail, the profile runs from the path's start to the \
                 rail's and rides both, scaled to the width between them.",
            );
            changed |= curve_list(ui, ctx, "Second rail", second_rail, &picked);
        }
        SurfaceFeature::Offset { faces: f, distance } => {
            changed |= face_list(ui, "Faces", f, &faces);
            changed |= length_row(ui, "Distance", distance);
        }
        SurfaceFeature::Extend {
            edges,
            length,
            continuity,
        } => {
            changed |= edge_list(ui, "Edges", edges, &around);
            changed |= length_row(ui, "Length", length);
            changed |= continuity_row(ui, continuity);
        }
        SurfaceFeature::Blend {
            first,
            second,
            continuity,
        } => {
            changed |= one_edge(ui, "First edge", first, &picked);
            changed |= one_edge(ui, "Second edge", second, &picked);
            changed |= continuity_row(ui, continuity);
        }
        SurfaceFeature::Split { faces: f, curves } => {
            changed |= face_list(ui, "Faces", f, &faces);
            changed |= curve_list(ui, ctx, "Cut along", curves, &picked);
        }
        SurfaceFeature::Sew { gap } => {
            hint(
                ui,
                "Joins the body's surfaces where their edges meet. A shell that closes \
                 becomes a solid.",
            );
            changed |= row(ui, "Close gaps up to", |ui| {
                QtyField::new(gap).unit("mm").range(0.0..=5.0).show(ui)
            });
        }
        SurfaceFeature::Fillet { edges, radius } => {
            hint(ui, "Edges where two faces of the body's surfaces meet.");
            changed |= edge_list(ui, "Edges", edges, &around);
            changed |= length_row(ui, "Radius", radius);
        }
        SurfaceFeature::FilletFaces {
            first,
            second,
            radius,
            flip_first,
            flip_second,
        } => {
            hint(
                ui,
                "Two faces that share no edge, such as two separate surfaces. The round \
                 rolls on the side each face's normal points to; flip a face to roll it on \
                 the other.",
            );
            changed |= one_face(ui, "First face", first, &faces);
            changed |= check_row(ui, flip_first, "Round on its other side").changed();
            // The second slot offers the face picked last.
            let last: Vec<FacePick> = faces.iter().rev().copied().collect();
            changed |= one_face(ui, "Second face", second, &last);
            changed |= check_row(ui, flip_second, "Round on its other side").changed();
            changed |= length_row(ui, "Radius", radius);
        }
        SurfaceFeature::Thicken {
            thickness,
            both_sides,
        } => {
            changed |= length_row(ui, "Thickness", thickness);
            changed |= check_row(ui, both_sides, "Both sides, half each").changed();
        }
        SurfaceFeature::Trim {
            plane,
            offset,
            flip,
        } => {
            changed |= plane_row(ui, plane);
            changed |= length_row(ui, "Offset", offset);
            changed |= check_row(ui, flip, "Keep the other side").changed();
        }
        SurfaceFeature::Mirror { plane, offset } => {
            changed |= plane_row(ui, plane);
            changed |= length_row(ui, "Offset", offset);
        }
        SurfaceFeature::TrimScan {
            tolerance, share, ..
        } => {
            hint(
                ui,
                "The body's surfaces are cut where they cross, and each face kept where enough \
                 of it lies on the scan; what is kept is sewn, a closed shell a solid.",
            );
            let mut near = tolerance.unwrap_or(0.0);
            if length_row(ui, "Within (0: from the fits)", &mut near) {
                *tolerance = (near > 0.0).then_some(near);
                changed = true;
            }
            let mut percent = *share * 100.0;
            if row(ui, "Of its area", |ui| {
                QtyField::new(&mut percent)
                    .unit("%")
                    .range(1.0..=100.0)
                    .show(ui)
            }) {
                *share = percent / 100.0;
                changed = true;
            }
        }
        SurfaceFeature::FitNetwork {
            curves,
            polylines,
            patches,
            tolerance,
            ..
        } => {
            hint(
                ui,
                &format!(
                    "{} patches over {} curves, fitted to the scan together: they meet \
                     across every curve without a gap, tangent unless it is a crease.",
                    patches.len(),
                    curves.len() + polylines.len()
                ),
            );
            changed |= length_row(ui, "Tolerance", tolerance);
        }
        SurfaceFeature::FitScanFree {
            points, tolerance, ..
        } => {
            hint(
                ui,
                &format!(
                    "A B-spline fitted to {} points of the scan, refined until it keeps within \
                     the tolerance of them or only noise is left.",
                    points.len()
                ),
            );
            changed |= length_row(ui, "Tolerance", tolerance);
        }
        SurfaceFeature::FitScan { sigma, extend, .. } => {
            hint(
                ui,
                &format!(
                    "Fitted to the scan's points within {sigma:.3} mm RMS; it reaches past them \
                     by the share below, so neighbouring fits cross and can be trimmed."
                ),
            );
            let mut share = *extend * 100.0;
            if row(ui, "Extend", |ui| {
                QtyField::new(&mut share)
                    .unit("%")
                    .range(0.0..=200.0)
                    .show(ui)
            }) {
                *extend = share / 100.0;
                changed = true;
            }
        }
    }
    changed
}

fn hint(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).font(sans(FONT_XS)).color(TEXT3)).wrap());
    ui.add_space(SPACE_1);
}

fn row(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> bool) -> bool {
    ui.horizontal(|ui| {
        field_label(ui, label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control)
            .inner
    })
    .inner
}

fn length_row(ui: &mut Ui, label: &str, value: &mut f32) -> bool {
    row(ui, label, |ui| QtyField::new(value).unit("mm").show(ui))
}

fn continuity_row(ui: &mut Ui, continuity: &mut Continuity) -> bool {
    row(ui, "Continuity", |ui| {
        select_field(
            ui,
            "surface_continuity",
            continuity,
            &[
                (Continuity::G0, "Touching (G0)"),
                (Continuity::G1, "Tangent (G1)"),
                (Continuity::G2, "Curvature (G2)"),
            ],
            FIELD,
        )
    })
}

fn direction_row(ui: &mut Ui, direction: &mut Direction) -> bool {
    let mut choice = match *direction {
        Direction::Custom(_) => Direction::SketchNormal,
        other => other,
    };
    let custom = matches!(direction, Direction::Custom(_));
    let changed = row(ui, "Direction", |ui| {
        select_field(
            ui,
            "surface_direction",
            &mut choice,
            &[
                (Direction::SketchNormal, "Square to the sketch"),
                (Direction::X, "X"),
                (Direction::Y, "Y"),
                (Direction::Z, "Z"),
            ],
            FIELD,
        )
    });
    if changed || !custom {
        *direction = choice;
    }
    changed
}

fn axis_row(ui: &mut Ui, axis: &mut Axis) -> bool {
    if matches!(axis, Axis::Custom { .. }) {
        hint(ui, "A custom axis, set by a script.");
    }
    let mut choice = *axis;
    let changed = row(ui, "Axis", |ui| {
        select_field(
            ui,
            "surface_axis",
            &mut choice,
            &[
                (Axis::SketchVertical, "Sketch's vertical"),
                (Axis::SketchHorizontal, "Sketch's horizontal"),
                (Axis::X, "X"),
                (Axis::Y, "Y"),
                (Axis::Z, "Z"),
            ],
            FIELD,
        )
    });
    if changed {
        *axis = choice;
    }
    changed
}

fn plane_row(ui: &mut Ui, plane: &mut PlaneRef) -> bool {
    if matches!(plane, PlaneRef::Custom { .. }) {
        hint(ui, "A custom plane, set by a script.");
    }
    let mut choice = *plane;
    let changed = row(ui, "Plane", |ui| {
        select_field(
            ui,
            "surface_plane",
            &mut choice,
            &[
                (PlaneRef::YZ, "YZ"),
                (PlaneRef::XZ, "XZ"),
                (PlaneRef::XY, "XY"),
            ],
            FIELD,
        )
    });
    if changed {
        *plane = choice;
    }
    changed
}

fn curve_name(ctx: &WorkbenchRuntimeContext, curve: &CurveRef) -> String {
    match curve {
        CurveRef::Sketch(id) => ctx
            .document
            .get_feature_meta(*id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "A sketch that is gone".into()),
        CurveRef::Edge(edge) => edge_name(edge),
        CurveRef::BodyEdge { body, edge } => {
            let name = ctx
                .document
                .bodies()
                .iter()
                .find(|b| b.id == *body)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "a body that is gone".into());
            format!("{} of {name}", edge_name(edge))
        }
    }
}

fn edge_name(edge: &EdgePick) -> String {
    if edge.length > 0.0 {
        format!("Edge, {:.2} mm", edge.length)
    } else {
        "Edge".into()
    }
}

/// A heading with an "Add the selection" button, then one row per item with
/// a cross to take it out. Whether the list changed.
fn list<T: Clone + PartialEq>(
    ui: &mut Ui,
    title: &str,
    items: &mut Vec<T>,
    selection: &[T],
    name: impl Fn(&T) -> String,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(sans_medium(FONT_SM)).color(TEXT1));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let fresh: Vec<T> = selection
                .iter()
                .filter(|s| !items.contains(s))
                .cloned()
                .collect();
            if ui
                .add_enabled_ui(!fresh.is_empty(), |ui| {
                    small_secondary_button(ui, "Add the selection")
                })
                .inner
                .on_disabled_hover_text(
                    "Select a sketch in the tree, or edges or a face in the view",
                )
                .clicked()
            {
                items.extend(fresh);
                changed = true;
            }
        });
    });
    if items.is_empty() {
        hint(ui, "Nothing picked yet.");
    }
    let mut remove = None;
    for (index, item) in items.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.add_space(SPACE_2);
            ui.label(RichText::new(name(item)).font(sans(FONT_SM)).color(TEXT2));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, "close", "Take it out").clicked() {
                    remove = Some(index);
                }
            });
        });
    }
    if let Some(index) = remove {
        items.remove(index);
        changed = true;
    }
    ui.add_space(SPACE_1);
    changed
}

fn curve_list(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    title: &str,
    curves: &mut Vec<CurveRef>,
    picked: &[CurveRef],
) -> bool {
    list(ui, title, curves, picked, |c| curve_name(ctx, c))
}

fn edge_list(ui: &mut Ui, title: &str, edges: &mut Vec<EdgePick>, picked: &[CurveRef]) -> bool {
    let picked: Vec<EdgePick> = picked
        .iter()
        .filter_map(|c| match c {
            CurveRef::Edge(e) => Some(*e),
            CurveRef::Sketch(_) | CurveRef::BodyEdge { .. } => None,
        })
        .collect();
    list(ui, title, edges, &picked, edge_name)
}

fn face_list(ui: &mut Ui, title: &str, faces: &mut Vec<FacePick>, picked: &[FacePick]) -> bool {
    list(ui, title, faces, picked, |_| "Face".to_string())
}

/// A single curve slot, filled from the selection's first curve.
fn one_curve(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    title: &str,
    slot: &mut Option<CurveRef>,
    picked: &[CurveRef],
) -> bool {
    let mut items: Vec<CurveRef> = slot.iter().copied().collect();
    let first: Vec<CurveRef> = picked.iter().take(1).copied().collect();
    let before = items.clone();
    if list(ui, title, &mut items, &first, |c| curve_name(ctx, c)) {
        *slot = items.last().copied();
    }
    items != before
}

fn one_edge(ui: &mut Ui, title: &str, slot: &mut Option<EdgePick>, picked: &[CurveRef]) -> bool {
    let mut items: Vec<EdgePick> = slot.iter().copied().collect();
    let first: Vec<EdgePick> = picked
        .iter()
        .find_map(|c| match c {
            CurveRef::Edge(e) => Some(*e),
            CurveRef::Sketch(_) | CurveRef::BodyEdge { .. } => None,
        })
        .into_iter()
        .collect();
    let before = items.clone();
    if list(ui, title, &mut items, &first, edge_name) {
        *slot = items.last().copied();
    }
    items != before
}

/// A single face slot, filled from the first of `picked`.
fn one_face(ui: &mut Ui, title: &str, slot: &mut Option<FacePick>, picked: &[FacePick]) -> bool {
    let mut items: Vec<FacePick> = slot.iter().copied().collect();
    let first: Vec<FacePick> = picked.iter().take(1).copied().collect();
    let before = items.clone();
    if list(ui, title, &mut items, &first, |_| "Face".to_string()) {
        *slot = items.last().copied();
    }
    items != before
}

/// A continuity check's findings: how many edges, how many meet tangent,
/// and each edge from the sharpest crease down.
pub fn continuity(
    ui: &mut Ui,
    body: &str,
    joins: &[([f32; 3], kernel_api::EdgeContinuity)],
    tangent_deg: f64,
    apart_mm: f64,
) {
    let apart = joins.iter().filter(|(_, j)| j.gap > apart_mm).count();
    let tangent = joins
        .iter()
        .filter(|(_, j)| j.gap <= apart_mm && j.angle_deg < tangent_deg)
        .count();
    hint(
        ui,
        &format!(
            "{body}: {} shared edge(s); {tangent} tangent, {} creased, {apart} apart. \
             The view labels each edge.",
            joins.len(),
            joins.len() - tangent - apart,
        ),
    );
    let mut sorted: Vec<&kernel_api::EdgeContinuity> = joins.iter().map(|(_, j)| j).collect();
    sorted.sort_by(|a, b| b.angle_deg.total_cmp(&a.angle_deg));
    egui::Grid::new("surface_continuity")
        .num_columns(3)
        .spacing(egui::vec2(SPACE_3, SPACE_1))
        .show(ui, |ui| {
            for title in ["Edge at", "Crease", "Gap"] {
                ui.label(RichText::new(title).font(sans_medium(FONT_XS)).color(TEXT2));
            }
            ui.end_row();
            for join in sorted {
                let [x, y, z] = join.point;
                ui.label(
                    RichText::new(format!("{x:.1}, {y:.1}, {z:.1}"))
                        .font(ui_kit::mono(FONT_XS))
                        .color(TEXT2),
                );
                ui.label(
                    RichText::new(format!("{:.2}°", join.angle_deg))
                        .font(ui_kit::mono(FONT_XS))
                        .color(TEXT1),
                );
                ui.label(
                    RichText::new(format!("{:.4} mm", join.gap))
                        .font(ui_kit::mono(FONT_XS))
                        .color(TEXT1),
                );
                ui.end_row();
            }
        });
}

/// An analysis's settings and, for a curvature map, its legend; the
/// display changed, when it did.
pub fn analysis(
    ui: &mut Ui,
    body: &str,
    display: crate::analysis::Display,
    range: Option<(f64, f64)>,
    palette: &core_document::SketchPalette,
) -> Option<crate::analysis::Display> {
    use crate::analysis::{Display, Measure, StripeAxis};
    let mut next = display;
    match &mut next {
        Display::Curvature { measure, limit } => {
            let sign = if *measure == Measure::Gaussian {
                "positive on a dome or in a bowl, negative on a saddle, none where a \
                 surface unrolls flat"
            } else {
                "negative where they bulge out, positive in a hollow"
            };
            hint(
                ui,
                &format!("{body}, painted by how its surfaces bend: {sign}."),
            );
            row(ui, "Curvature", |ui| {
                let options: Vec<(Measure, &str)> =
                    Measure::ALL.iter().map(|m| (*m, m.label())).collect();
                select_field(ui, "surface_curvature_measure", measure, &options, FIELD)
            });
            let mut auto = *limit <= 0.0;
            if check_row(ui, &mut auto, "Colour range from the body").changed() {
                *limit = if auto {
                    0.0
                } else {
                    range.map_or(0.1, |(lo, hi)| lo.abs().max(hi.abs()).max(1e-6))
                };
            }
            if !auto {
                let mut value = *limit as f32;
                if row(ui, "Colours reach", |ui| {
                    QtyField::new(&mut value)
                        .unit(measure.unit())
                        .range(1e-6..=1e3)
                        .show(ui)
                }) {
                    *limit = f64::from(value);
                }
            }
            ui.add_space(SPACE_1);
            ui.horizontal(|ui| {
                for (color, text) in [
                    (palette.analysis_low, "−"),
                    (palette.analysis_mid, "0"),
                    (palette.analysis_high, "+"),
                ] {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(SPACE_3, SPACE_3), egui::Sense::hover());
                    ui.painter().rect_filled(rect, RADIUS_SM, color32(color));
                    ui.label(RichText::new(text).font(ui_kit::mono(FONT_XS)).color(TEXT2));
                }
            });
            if let Some((low, high)) = range {
                hint(
                    ui,
                    &format!(
                        "From {low:.4} to {high:.4} {} over the body.",
                        measure.unit()
                    ),
                );
            } else {
                hint(ui, "The kernel read no curvature of the body.");
            }
        }
        Display::Zebra { stripes, axis } => {
            hint(
                ui,
                &format!(
                    "{body}, striped by the way its surfaces face. A stripe kinks where \
                     faces meet at a crease (G0), bends sharply where they meet tangent \
                     (G1), and runs on smoothly where they bend alike (G2)."
                ),
            );
            let mut count = *stripes as f32;
            if row(ui, "Stripes", |ui| {
                QtyField::new(&mut count).range(1.0..=64.0).show(ui)
            }) {
                *stripes = count.round().clamp(1.0, 64.0) as u32;
            }
            row(ui, "Along", |ui| {
                let options: Vec<(StripeAxis, &str)> =
                    StripeAxis::ALL.iter().map(|a| (*a, a.label())).collect();
                select_field(ui, "surface_zebra_axis", axis, &options, FIELD)
            });
        }
    }
    (next != display).then_some(next)
}

fn color32(c: [f32; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(
        (c[0] * 255.0).round() as u8,
        (c[1] * 255.0).round() as u8,
        (c[2] * 255.0).round() as u8,
    )
}

/// The Surface preferences page.
pub fn settings(ui: &mut Ui, options: &mut Options, filter: &str) {
    pref_group(
        ui,
        "New surfaces",
        vec![
            PrefRow::toggle(
                "Hide the sketch a surface uses",
                &mut options.hide_used_sketches,
            )
            .hint("Keep the sketches a new surface is built from out of the view"),
        ],
        filter,
    );
}
