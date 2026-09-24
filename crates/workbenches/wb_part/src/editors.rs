//! Per-feature parameter editors for the task panel.
//!
//! Every dialog option maps 1:1 to a feature field; editing either path
//! recomputes. Editors return `true` when the feature payload changed.
//! Rows are a fixed-width label column beside a control, the way the
//! design lays out its parameter cards.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use egui::{RichText, Ui};
use ui_kit::sans;
use ui_kit::tokens::*;
use ui_kit::widgets::{
    QtyField, accent_outline_button, check_row, mono_label, secondary_button,
    small_secondary_button,
};

use crate::build::{part_features_of_body, sketches_of_body};
use crate::feature::{
    ChamferMode, EdgePick, EdgeSel, ExtrudeMode, FacePick, HelixMode, HoleCut, HoleFit,
    METRIC_SIZES, MirrorPlane, PartFeature, PatternAxis, RevolveAxis, TransformStep,
};

/// The label column of a parameter row.
pub(crate) fn label_cell(ui: &mut Ui, label: &str) {
    // A long label is cut to the column, and whole on hover.
    let text = label.trim_end_matches(':');
    ui.add_sized(
        [96.0, INPUT],
        egui::Label::new(RichText::new(text).font(sans(FONT_SM)).color(TEXT2)).truncate(),
    )
    .on_hover_text(text);
}

/// One parameter row: the label column, then `add` draws the control and
/// reports whether it changed the value.
fn field(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> bool) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = SPACE_2;
        label_cell(ui, label);
        add(ui)
    })
    .inner
}

/// The parameter each editor field shows, by the field's label.
const LABEL_PARAMETERS: &[(&str, &str)] = &[
    ("Length", "length"),
    ("Second length", "length2"),
    ("Depth", "depth"),
    ("Second depth", "depth2"),
    ("Taper", "taper"),
    ("Offset", "offset"),
    ("Angle", "angle"),
    ("Angle 2", "angle2"),
    ("Pitch", "pitch"),
    ("Height", "height"),
    ("Cone angle", "cone_angle"),
    ("Diameter", "diameter"),
    ("Bore Ø", "counterbore_diameter"),
    ("Bore depth", "counterbore_depth"),
    ("Sink Ø", "countersink_diameter"),
    ("Sink angle", "countersink_angle"),
    ("Radius", "radius"),
    ("Size", "size"),
    ("Size 2", "size2"),
    ("Thickness", "thickness"),
    ("Occurrences", "occurrences"),
    ("Offset X", "offset_x"),
    ("Offset Y", "offset_y"),
    ("Normal offset", "offset_z"),
    ("Rotation", "rotation"),
];

/// What a feature's fields know of formulas: the feature's parameters,
/// which of them a formula sets, and the formula edits made this frame, for
/// the task to apply (`(key, formula)`, `None` taking one away).
pub(crate) struct Formulas<'a> {
    document: &'a core_document::Document,
    feature: FeatureId,
    params: Vec<core_document::Parameter>,
    pub edits: Vec<(String, Option<String>)>,
}

impl<'a> Formulas<'a> {
    pub(crate) fn of(document: &'a core_document::Document, feature: FeatureId) -> Self {
        let params = document
            .get_feature_meta(feature)
            .map(|node| {
                if node.workbench_id.as_str() == "core.datum" {
                    crate::params::datum_parameters()
                } else {
                    crate::params::feature_parameters(node)
                }
            })
            .unwrap_or_default();
        Self {
            document,
            feature,
            params,
            edits: Vec::new(),
        }
    }

    /// The parameter an editor field labelled `label` shows.
    fn find(&self, label: &str) -> Option<core_document::Parameter> {
        let wanted = label.trim_end_matches(':');
        let name = LABEL_PARAMETERS
            .iter()
            .find(|(l, _)| *l == wanted)
            .map(|(_, name)| *name)?;
        let node = self.document.get_feature_meta(self.feature)?;
        self.params
            .iter()
            .find(|p| p.name.as_deref() == Some(name) && node.data.pointer(&p.pointer).is_some())
            .cloned()
    }

    /// The field for `label` as a formula field, if it has a parameter:
    /// `Some(changed)` when it did, the value set by hand going into
    /// `value` and a formula into `edits`.
    fn show(&mut self, ui: &mut Ui, label: &str, value: f64) -> Option<(bool, f64)> {
        let p = self.find(label)?;
        let formula = self.document.feature_formula(self.feature, &p.key);
        let slot = self
            .document
            .evaluated_slots(self.feature)
            .iter()
            .find(|s| s.key == p.key);
        let shown = match (formula, slot.map(|s| &s.result)) {
            (Some(_), Some(Ok(q))) => q.value,
            _ => value,
        };
        let error = slot
            .and_then(|s| s.result.as_ref().err())
            .map(String::as_str);
        let host = core_document::DocumentFormulas {
            document: self.document,
            dim: p.dim,
        };
        let edit = ui
            .horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = SPACE_2;
                label_cell(ui, label);
                ui_kit::widgets::FormulaField::new(
                    egui::Id::new(("part_field", self.feature, &p.key)),
                    shown,
                    &host,
                )
                .formula(formula)
                .error(error)
                .unit(match p.dim {
                    core_document::expr::Dim::LENGTH => "mm",
                    core_document::expr::Dim::ANGLE => "°",
                    _ => "",
                })
                .decimals(if p.integer { 0 } else { 2 })
                .speed(if p.integer { 0.05 } else { 0.5 })
                .show(ui)
            })
            .inner;
        Some(match edit {
            Some(ui_kit::widgets::FormulaEdit::Value(v)) => {
                if formula.is_some() {
                    self.edits.push((p.key, None));
                }
                (true, v)
            }
            Some(ui_kit::widgets::FormulaEdit::Formula(text)) => {
                self.edits.push((p.key, Some(text)));
                (false, value)
            }
            None => (false, value),
        })
    }
}

fn mm_drag(ui: &mut Ui, fx: &mut Formulas, value: &mut f32, label: &str) -> bool {
    if let Some((changed, v)) = fx.show(ui, label, f64::from(*value)) {
        *value = v as f32;
        return changed;
    }
    field(ui, label, |ui| QtyField::mm(value).speed(0.5).show(ui))
}

fn deg_drag(
    ui: &mut Ui,
    fx: &mut Formulas,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    if let Some((changed, v)) = fx.show(ui, label, f64::from(*value)) {
        *value = (v as f32).clamp(*range.start(), *range.end());
        return changed;
    }
    let range = (*range.start() as f64)..=(*range.end() as f64);
    field(ui, label, |ui| {
        QtyField::degrees(value).speed(1.0).range(range).show(ui)
    })
}

fn count_drag(ui: &mut Ui, fx: &mut Formulas, value: &mut u32, label: &str) -> bool {
    if let Some((changed, v)) = fx.show(ui, label, f64::from(*value)) {
        *value = v.round().clamp(2.0, 1000.0) as u32;
        return changed;
    }
    field(ui, label, |ui| {
        let mut v = *value as f32;
        let changed = QtyField::new(&mut v)
            .decimals(0)
            .speed(0.1)
            .range(2.0..=1000.0)
            .show(ui);
        if changed {
            *value = v.round() as u32;
        }
        changed
    })
}

fn sketch_combo(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    id_salt: impl egui::AsIdSalt,
    current: Option<FeatureId>,
    label: &str,
) -> Option<FeatureId> {
    let sketches = sketches_of_body(ctx.document, body);
    let current_name = current
        .and_then(|id| {
            sketches
                .iter()
                .find(|(sid, _)| *sid == id)
                .map(|(_, n)| n.clone())
        })
        .unwrap_or_else(|| "(pick)".to_string());
    let mut picked = None;
    ui.horizontal(|ui| {
        label_cell(ui, label);
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(current_name)
            .show_ui(ui, |ui| {
                for (id, name) in &sketches {
                    if ui.selectable_label(current == Some(*id), name).clicked()
                        && current != Some(*id)
                    {
                        picked = Some(*id);
                    }
                }
            });
    });
    picked
}

fn extrude_mode_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    mode: &mut ExtrudeMode,
    first_feature: bool,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Type");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(mode.label())
            .show_ui(ui, |ui| {
                for candidate in ExtrudeMode::ALL {
                    // Material-relative modes need an earlier solid.
                    let needs_material = matches!(
                        candidate,
                        ExtrudeMode::ThroughAll | ExtrudeMode::ToFirst | ExtrudeMode::ToLast
                    );
                    if first_feature && needs_material {
                        continue;
                    }
                    if ui
                        .selectable_label(*mode == candidate, candidate.label())
                        .clicked()
                        && *mode != candidate
                    {
                        *mode = candidate;
                        changed = true;
                    }
                }
            });
    });
    changed
}

/// "Use selected face" picker row. Shows the current pick and captures the
/// viewport's selected face on click.
fn face_pick_row(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    pick: &mut Option<FacePick>,
    label: &str,
) -> bool {
    let mut changed = false;
    // The button goes under the point when the row is narrow.
    ui.horizontal_wrapped(|ui| {
        label_cell(ui, label);
        match pick {
            Some(p) => {
                mono_label(
                    ui,
                    format!("({:.1}, {:.1}, {:.1})", p.point[0], p.point[1], p.point[2]),
                    FONT_XS,
                    TEXT1,
                );
            }
            None => {
                mono_label(ui, "(none)", FONT_XS, TEXT3);
            }
        }
        let has_selection = ctx.selected_face.is_some();
        if ui
            .add_enabled_ui(has_selection, |ui| {
                accent_outline_button(ui, "Use selected face")
            })
            .inner
            .on_hover_text("Click a face in the viewport first, then press this")
            .clicked()
            && let Some(face) = picked_face(ctx)
        {
            *pick = Some(FacePick {
                point: face.point,
                normal: face.normal,
            });
            changed = true;
        }
    });
    changed
}

fn face_list_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    faces: &mut Vec<FacePick>,
    label: &str,
) -> bool {
    let mut changed = false;
    label_cell(ui, label);
    let mut remove = None;
    for (i, face) in faces.iter().enumerate() {
        ui.horizontal(|ui| {
            mono_label(
                ui,
                format!(
                    "Face @ ({:.1}, {:.1}, {:.1})",
                    face.point[0], face.point[1], face.point[2]
                ),
                FONT_XS,
                TEXT1,
            );
            if small_secondary_button(ui, "✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        faces.remove(i);
        changed = true;
    }
    let has_selection = ctx.selected_face.is_some();
    if ui
        .add_enabled_ui(has_selection, |ui| {
            accent_outline_button(ui, "Add selected face")
        })
        .inner
        .on_hover_text("Click a face in the viewport first, then press this")
        .clicked()
        && let Some(face) = picked_face(ctx)
    {
        faces.push(FacePick {
            point: face.point,
            normal: face.normal,
        });
        changed = true;
    }
    changed
}

fn edge_sel_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    edges: &mut EdgeSel,
    id_salt: impl egui::AsIdSalt,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Edges");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(match edges {
                EdgeSel::All => "All edges".to_string(),
                EdgeSel::Faces(f) => format!("{} face(s)", f.len()),
                EdgeSel::Edges(e) => format!("{} edge(s)", e.len()),
            })
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(matches!(edges, EdgeSel::All), "All edges")
                    .clicked()
                    && !matches!(edges, EdgeSel::All)
                {
                    *edges = EdgeSel::All;
                    changed = true;
                }
                if ui
                    .selectable_label(matches!(edges, EdgeSel::Faces(_)), "Edges of picked faces")
                    .clicked()
                    && !matches!(edges, EdgeSel::Faces(_))
                {
                    *edges = EdgeSel::Faces(Vec::new());
                    changed = true;
                }
                if ui
                    .selectable_label(matches!(edges, EdgeSel::Edges(_)), "Picked edges")
                    .clicked()
                    && !matches!(edges, EdgeSel::Edges(_))
                {
                    *edges = EdgeSel::Edges(Vec::new());
                    changed = true;
                }
            });
    });
    match edges {
        EdgeSel::Faces(faces) => changed |= face_list_editor(ui, ctx, faces, "Faces:"),
        EdgeSel::Edges(picks) => changed |= edge_list_editor(ui, ctx, picks),
        EdgeSel::All => {}
    }
    changed
}

/// The picked edges, each removable, and a button that adds whatever is
/// picked in the viewport.
fn edge_list_editor(ui: &mut Ui, ctx: &WorkbenchRuntimeContext, picks: &mut Vec<EdgePick>) -> bool {
    let mut changed = false;
    label_cell(ui, "Edges:");
    let mut remove = None;
    for (i, pick) in picks.iter().enumerate() {
        ui.horizontal(|ui| {
            mono_label(
                ui,
                format!(
                    "Edge @ ({:.1}, {:.1}, {:.1})",
                    pick.point[0], pick.point[1], pick.point[2]
                ),
                FONT_XS,
                TEXT1,
            );
            if small_secondary_button(ui, "✕").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        picks.remove(i);
        changed = true;
    }
    let has_selection = !ctx.selected_edges.is_empty();
    if ui
        .add_enabled_ui(has_selection, |ui| {
            accent_outline_button(ui, "Add selected edges")
        })
        .inner
        .on_hover_text("Click edges in the viewport first (Ctrl adds), then press this")
        .clicked()
    {
        for edge in &picked_edges(ctx) {
            let pick = EdgePick {
                point: edge.point,
                direction: edge.direction,
            };
            if !picks.contains(&pick) {
                picks.push(pick);
                changed = true;
            }
        }
    }
    changed
}

fn revolve_axis_editor(ui: &mut Ui, axis: &mut RevolveAxis, id_salt: impl egui::AsIdSalt) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Axis");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(axis.label())
            .show_ui(ui, |ui| {
                for candidate in [
                    RevolveAxis::SketchY,
                    RevolveAxis::SketchX,
                    RevolveAxis::Custom {
                        origin: [0.0, 0.0],
                        dir: [1.0, 1.0],
                    },
                ] {
                    let is_current =
                        std::mem::discriminant(axis) == std::mem::discriminant(&candidate);
                    if ui.selectable_label(is_current, candidate.label()).clicked() && !is_current {
                        *axis = candidate;
                        changed = true;
                    }
                }
            });
    });
    if let RevolveAxis::Custom { origin, dir } = axis {
        ui.horizontal(|ui| {
            label_cell(ui, "Origin");
            changed |= ui
                .add(egui::DragValue::new(&mut origin[0]).speed(0.5))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut origin[1]).speed(0.5))
                .changed();
            label_cell(ui, "Dir");
            changed |= ui
                .add(egui::DragValue::new(&mut dir[0]).speed(0.1))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut dir[1]).speed(0.1))
                .changed();
        });
    }
    changed
}

fn pattern_axis_editor(ui: &mut Ui, axis: &mut PatternAxis, id_salt: impl egui::AsIdSalt) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Axis");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(axis.label())
            .show_ui(ui, |ui| {
                for candidate in [
                    PatternAxis::X,
                    PatternAxis::Y,
                    PatternAxis::Z,
                    PatternAxis::Custom {
                        origin: [0.0; 3],
                        dir: [0.0, 0.0, 1.0],
                    },
                ] {
                    let is_current =
                        std::mem::discriminant(axis) == std::mem::discriminant(&candidate);
                    if ui.selectable_label(is_current, candidate.label()).clicked() && !is_current {
                        *axis = candidate;
                        changed = true;
                    }
                }
            });
    });
    if let PatternAxis::Custom { origin, dir } = axis {
        ui.horizontal(|ui| {
            label_cell(ui, "Origin");
            for v in origin.iter_mut() {
                changed |= ui.add(egui::DragValue::new(v).speed(0.5)).changed();
            }
        });
        ui.horizontal(|ui| {
            label_cell(ui, "Dir");
            for v in dir.iter_mut() {
                changed |= ui.add(egui::DragValue::new(v).speed(0.1)).changed();
            }
        });
    }
    changed
}

fn mirror_plane_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    plane: &mut MirrorPlane,
    id_salt: impl egui::AsIdSalt,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Plane");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(plane.label())
            .show_ui(ui, |ui| {
                for candidate in MirrorPlane::BASE {
                    if ui
                        .selectable_label(*plane == candidate, candidate.label())
                        .clicked()
                        && *plane != candidate
                    {
                        *plane = candidate;
                        changed = true;
                    }
                }
                let is_face = matches!(plane, MirrorPlane::Face(_));
                if ui.selectable_label(is_face, "Picked face").clicked()
                    && !is_face
                    && let Some(face) = picked_face(ctx)
                {
                    *plane = MirrorPlane::Face(FacePick {
                        point: face.point,
                        normal: face.normal,
                    });
                    changed = true;
                }
            });
    });
    if let MirrorPlane::Face(pick) = plane {
        let mut opt = Some(*pick);
        if face_pick_row(ui, ctx, &mut opt, "Face:")
            && let Some(new_pick) = opt
        {
            *pick = new_pick;
            changed = true;
        }
    }
    changed
}

/// Earlier part features selectable as pattern originals.
fn originals_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    this_feature: FeatureId,
    originals: &mut Vec<FeatureId>,
) -> bool {
    let mut changed = false;
    label_cell(ui, "Originals (empty = whole body)");
    let features = part_features_of_body(ctx.document, body);
    for (id, feature) in &features {
        if *id == this_feature {
            break;
        }
        if feature.is_modifier() {
            continue;
        }
        let name = ctx
            .document
            .get_feature_meta(*id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| feature.kind_label().to_string());
        let mut included = originals.contains(id);
        if check_row(ui, &mut included, &name).changed() {
            if included {
                originals.push(*id);
            } else {
                originals.retain(|o| o != id);
            }
            changed = true;
        }
    }
    changed
}

fn primitive_editor(ui: &mut Ui, kind: &mut kernel_api::PrimitiveKind) -> bool {
    use kernel_api::PrimitiveKind as P;
    let mut changed = false;
    let variants: [(&str, P); 8] = [
        (
            "Box",
            P::Box {
                length: 10.0,
                width: 10.0,
                height: 10.0,
            },
        ),
        (
            "Cylinder",
            P::Cylinder {
                radius: 5.0,
                height: 10.0,
                angle_deg: 360.0,
            },
        ),
        (
            "Sphere",
            P::Sphere {
                radius: 5.0,
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
        ),
        (
            "Cone",
            P::Cone {
                radius1: 5.0,
                radius2: 2.0,
                height: 10.0,
                angle_deg: 360.0,
            },
        ),
        (
            "Torus",
            P::Torus {
                radius1: 10.0,
                radius2: 2.0,
                angle1_deg: -180.0,
                angle2_deg: 180.0,
                angle3_deg: 360.0,
            },
        ),
        (
            "Ellipsoid",
            P::Ellipsoid {
                radius1: 8.0,
                radius2: 5.0,
                radius3: 3.0,
            },
        ),
        (
            "Prism",
            P::Prism {
                sides: 6,
                circumradius: 5.0,
                height: 10.0,
            },
        ),
        (
            "Wedge",
            P::Wedge {
                xmin: 0.0,
                xmax: 10.0,
                ymin: 0.0,
                ymax: 10.0,
                zmin: 0.0,
                zmax: 10.0,
                x2min: 2.0,
                x2max: 8.0,
                z2min: 2.0,
                z2max: 8.0,
            },
        ),
    ];
    let current_label = variants
        .iter()
        .find(|(_, v)| std::mem::discriminant(kind) == std::mem::discriminant(v))
        .map(|(l, _)| *l)
        .unwrap_or("?");
    ui.horizontal(|ui| {
        label_cell(ui, "Shape");
        egui::ComboBox::from_id_salt("primitive_kind")
            .selected_text(current_label)
            .show_ui(ui, |ui| {
                for (label, template) in &variants {
                    let is_current =
                        std::mem::discriminant(kind) == std::mem::discriminant(template);
                    if ui.selectable_label(is_current, *label).clicked() && !is_current {
                        *kind = *template;
                        changed = true;
                    }
                }
            });
    });

    let dim = |ui: &mut Ui, value: &mut f64, label: &str, min: f64| {
        ui.horizontal(|ui| {
            label_cell(ui, label);
            ui.add(
                egui::DragValue::new(value)
                    .speed(0.5)
                    .range(min..=1.0e6)
                    .suffix(" mm"),
            )
            .changed()
        })
        .inner
    };
    let ang = |ui: &mut Ui, value: &mut f64, label: &str, lo: f64, hi: f64| {
        ui.horizontal(|ui| {
            label_cell(ui, label);
            ui.add(
                egui::DragValue::new(value)
                    .speed(1.0)
                    .range(lo..=hi)
                    .suffix("°"),
            )
            .changed()
        })
        .inner
    };

    match kind {
        P::Box {
            length,
            width,
            height,
        } => {
            changed |= dim(ui, length, "Length:", 0.01);
            changed |= dim(ui, width, "Width:", 0.01);
            changed |= dim(ui, height, "Height:", 0.01);
        }
        P::Cylinder {
            radius,
            height,
            angle_deg,
        } => {
            changed |= dim(ui, radius, "Radius:", 0.01);
            changed |= dim(ui, height, "Height:", 0.01);
            changed |= ang(ui, angle_deg, "Angle:", 1.0, 360.0);
        }
        P::Sphere {
            radius,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= dim(ui, radius, "Radius:", 0.01);
            changed |= ang(ui, angle1_deg, "Angle 1:", -90.0, 90.0);
            changed |= ang(ui, angle2_deg, "Angle 2:", -90.0, 90.0);
            changed |= ang(ui, angle3_deg, "Angle 3:", 1.0, 360.0);
        }
        P::Cone {
            radius1,
            radius2,
            height,
            angle_deg,
        } => {
            changed |= dim(ui, radius1, "Radius 1:", 0.0);
            changed |= dim(ui, radius2, "Radius 2:", 0.0);
            changed |= dim(ui, height, "Height:", 0.01);
            changed |= ang(ui, angle_deg, "Angle:", 1.0, 360.0);
        }
        P::Torus {
            radius1,
            radius2,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= dim(ui, radius1, "Radius 1:", 0.01);
            changed |= dim(ui, radius2, "Radius 2:", 0.01);
            changed |= ang(ui, angle1_deg, "Angle 1:", -180.0, 180.0);
            changed |= ang(ui, angle2_deg, "Angle 2:", -180.0, 180.0);
            changed |= ang(ui, angle3_deg, "Angle 3:", 1.0, 360.0);
        }
        P::Ellipsoid {
            radius1,
            radius2,
            radius3,
        } => {
            changed |= dim(ui, radius1, "Radius 1:", 0.01);
            changed |= dim(ui, radius2, "Radius 2:", 0.01);
            changed |= dim(ui, radius3, "Radius 3:", 0.01);
        }
        P::Prism {
            sides,
            circumradius,
            height,
        } => {
            ui.horizontal(|ui| {
                label_cell(ui, "Sides");
                changed |= ui
                    .add(egui::DragValue::new(sides).speed(0.1).range(3..=64))
                    .changed();
            });
            changed |= dim(ui, circumradius, "Circumradius:", 0.01);
            changed |= dim(ui, height, "Height:", 0.01);
        }
        P::Wedge {
            xmin,
            xmax,
            ymin,
            ymax,
            zmin,
            zmax,
            x2min,
            x2max,
            z2min,
            z2max,
        } => {
            for (value, label) in [
                (xmin, "X min:"),
                (xmax, "X max:"),
                (ymin, "Y min:"),
                (ymax, "Y max:"),
                (zmin, "Z min:"),
                (zmax, "Z max:"),
                (x2min, "X2 min:"),
                (x2max, "X2 max:"),
                (z2min, "Z2 min:"),
                (z2max, "Z2 max:"),
            ] {
                ui.horizontal(|ui| {
                    label_cell(ui, label);
                    changed |= ui
                        .add(
                            egui::DragValue::new(value)
                                .speed(0.5)
                                .range(-1.0e6..=1.0e6)
                                .suffix(" mm"),
                        )
                        .changed();
                });
            }
        }
    }
    changed
}

fn placement_editor(ui: &mut Ui, placement: &mut kernel_api::Placement) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Position");
        for v in placement.origin.iter_mut() {
            changed |= ui
                .add(egui::DragValue::new(v).speed(0.5).suffix(" mm"))
                .changed();
        }
    });
    changed
}

/// Settings editor for a datum feature. Returns true when the payload
/// changed.
pub fn datum_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    feature_id: FeatureId,
    datum: &mut core_document::DatumFeature,
) -> bool {
    use core_document::{BasePlane, DatumAttachment, DatumShape};
    let mut changed = false;

    match &mut datum.shape {
        DatumShape::Plane { size } => changed |= mm_drag(ui, fx, size, "Display size:"),
        DatumShape::Line { length } => changed |= mm_drag(ui, fx, length, "Display length:"),
        DatumShape::CoordinateSystem { size } => changed |= mm_drag(ui, fx, size, "Display size:"),
        DatumShape::Point => {}
    }

    ui.horizontal(|ui| {
        label_cell(ui, "Attached to");
        egui::ComboBox::from_id_salt(("datum_attach", feature_id))
            .selected_text(datum.attachment.label())
            .show_ui(ui, |ui| {
                for plane in BasePlane::ALL {
                    let candidate = DatumAttachment::BasePlane(plane);
                    if ui
                        .selectable_label(datum.attachment == candidate, plane.label())
                        .clicked()
                        && datum.attachment != candidate
                    {
                        datum.attachment = candidate;
                        changed = true;
                    }
                }
                let is_face = matches!(datum.attachment, DatumAttachment::FlatFace { .. });
                let can_pick = ctx.selected_face.is_some();
                if ui
                    .add_enabled(
                        can_pick || is_face,
                        egui::Button::selectable(is_face, "Picked face"),
                    )
                    .on_hover_text("Click a face in the viewport first")
                    .clicked()
                    && let Some(face) = picked_face(ctx)
                {
                    datum.attachment = DatumAttachment::FlatFace {
                        point: face.point,
                        normal: face.normal,
                    };
                    changed = true;
                }
            });
    });
    if matches!(datum.attachment, DatumAttachment::FlatFace { .. })
        && ctx.selected_face.is_some()
        && ui
            .button("Re-pick from selected face")
            .on_hover_text("Move the attachment to the currently selected face")
            .clicked()
        && let Some(face) = picked_face(ctx)
    {
        datum.attachment = DatumAttachment::FlatFace {
            point: face.point,
            normal: face.normal,
        };
        changed = true;
    }

    // One row each: side by side they are wider than the panel.
    let [x, y, n] = &mut datum.offset.translation;
    changed |= mm_drag(ui, fx, x, "Offset X:");
    changed |= mm_drag(ui, fx, y, "Offset Y:");
    changed |= mm_drag(ui, fx, n, "Normal offset:");
    changed |= deg_drag(
        ui,
        fx,
        &mut datum.offset.rotation_deg,
        "Rotation:",
        -180.0..=180.0,
    );
    changed |= check_row(ui, &mut datum.offset.flip, "Flip side").changed();
    changed
}

/// The full settings editor for one feature. Returns true when the payload
/// changed and needs a rebuild.
pub fn feature_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut PartFeature,
) -> bool {
    let mut changed = false;
    match feature {
        PartFeature::Pad {
            refine: _,
            sketch,
            length,
            reversed,
            symmetric,
            mode,
            length2,
            taper_deg,
            up_to_face,
            up_to_offset,
        } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("pad_sketch", feature_id),
                Some(*sketch),
                "Profile:",
            ) {
                *sketch = new;
                changed = true;
            }
            changed |= extrude_mode_combo(ui, ("pad_mode", feature_id), mode, false);
            match mode {
                ExtrudeMode::Dimension => {
                    changed |= mm_drag(ui, fx, length, "Length:");
                    changed |= check_row(ui, symmetric, "Symmetric to plane").changed();
                }
                ExtrudeMode::TwoLengths => {
                    changed |= mm_drag(ui, fx, length, "Length:");
                    changed |= mm_drag(ui, fx, length2, "Second length:");
                }
                ExtrudeMode::UpToFace => {
                    changed |= face_pick_row(ui, ctx, up_to_face, "Target face:");
                    changed |= mm_drag(ui, fx, up_to_offset, "Offset:");
                }
                _ => {}
            }
            changed |= check_row(ui, reversed, "Reversed").changed();
            changed |= deg_drag(ui, fx, taper_deg, "Taper:", -85.0..=85.0);
        }
        PartFeature::Pocket {
            refine: _,
            sketch,
            depth,
            reversed,
            through_all,
            mode,
            depth2,
            taper_deg,
            up_to_face,
            up_to_offset,
        } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("pocket_sketch", feature_id),
                Some(*sketch),
                "Profile:",
            ) {
                *sketch = new;
                changed = true;
            }
            // Legacy flag folds into the mode picker.
            if *through_all {
                *mode = ExtrudeMode::ThroughAll;
                *through_all = false;
                changed = true;
            }
            changed |= extrude_mode_combo(ui, ("pocket_mode", feature_id), mode, false);
            match mode {
                ExtrudeMode::Dimension => changed |= mm_drag(ui, fx, depth, "Depth:"),
                ExtrudeMode::TwoLengths => {
                    changed |= mm_drag(ui, fx, depth, "Depth:");
                    changed |= mm_drag(ui, fx, depth2, "Second depth:");
                }
                ExtrudeMode::UpToFace => {
                    changed |= face_pick_row(ui, ctx, up_to_face, "Target face:");
                    changed |= mm_drag(ui, fx, up_to_offset, "Offset:");
                }
                _ => {}
            }
            changed |= check_row(ui, reversed, "Reversed")
                .on_hover_text("Cut along the sketch normal instead of against it")
                .changed();
            changed |= deg_drag(ui, fx, taper_deg, "Taper:", -85.0..=85.0);
        }
        PartFeature::Revolution {
            refine: _,
            sketch,
            angle_deg,
            axis,
            reversed,
            midplane,
            second_angle_deg,
        }
        | PartFeature::Groove {
            refine: _,
            sketch,
            angle_deg,
            axis,
            reversed,
            midplane,
            second_angle_deg,
        } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("rev_sketch", feature_id),
                Some(*sketch),
                "Profile:",
            ) {
                *sketch = new;
                changed = true;
            }
            changed |= deg_drag(ui, fx, angle_deg, "Angle:", 0.1..=360.0);
            changed |= revolve_axis_editor(ui, axis, ("rev_axis", feature_id));
            changed |= check_row(ui, midplane, "Midplane").changed();
            let mut two_sided = second_angle_deg.is_some();
            if check_row(ui, &mut two_sided, "Second angle").changed() {
                *second_angle_deg = two_sided.then_some(90.0);
                changed = true;
            }
            if let Some(second) = second_angle_deg {
                changed |= deg_drag(ui, fx, second, "Angle 2:", 0.1..=360.0);
            }
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        PartFeature::Loft {
            refine: _,
            sections,
            ruled,
            closed,
            subtractive,
        } => {
            label_cell(ui, "Sections (in order)");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = ctx
                    .document
                    .get_feature_meta(*section)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| "(missing)".into());
                ui.horizontal(|ui| {
                    mono_label(ui, format!("{}. {name}", i + 1), FONT_XS, TEXT1);
                    if small_secondary_button(ui, "✕").clicked() && sections.len() > 1 {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                sections.remove(i);
                changed = true;
            }
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("loft_add", feature_id),
                None,
                "Add section:",
            ) && !sections.contains(&new)
            {
                sections.push(new);
                changed = true;
            }
            changed |= check_row(ui, ruled, "Ruled (straight transitions)").changed();
            changed |= check_row(ui, closed, "Closed (loop back)").changed();
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        PartFeature::Pipe {
            refine: _,
            profile,
            spine,
            frenet,
            subtractive,
        } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("pipe_profile", feature_id),
                Some(*profile),
                "Profile:",
            ) {
                *profile = new;
                changed = true;
            }
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("pipe_spine", feature_id),
                Some(*spine),
                "Path:",
            ) {
                *spine = new;
                changed = true;
            }
            changed |= check_row(ui, frenet, "Frenet orientation")
                .on_hover_text("Rotate the profile with the path's curvature frame")
                .changed();
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        PartFeature::Helix {
            refine: _,
            sketch,
            axis,
            mode,
            pitch,
            height,
            turns,
            left_handed,
            cone_angle_deg,
            reversed,
            subtractive,
        } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("helix_sketch", feature_id),
                Some(*sketch),
                "Profile:",
            ) {
                *sketch = new;
                changed = true;
            }
            changed |= revolve_axis_editor(ui, axis, ("helix_axis", feature_id));
            ui.horizontal(|ui| {
                label_cell(ui, "Mode");
                egui::ComboBox::from_id_salt(("helix_mode", feature_id))
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for candidate in HelixMode::ALL {
                            if ui
                                .selectable_label(*mode == candidate, candidate.label())
                                .clicked()
                                && *mode != candidate
                            {
                                *mode = candidate;
                                changed = true;
                            }
                        }
                    });
            });
            match mode {
                HelixMode::PitchHeight => {
                    changed |= mm_drag(ui, fx, pitch, "Pitch:");
                    changed |= mm_drag(ui, fx, height, "Height:");
                }
                HelixMode::PitchTurns => {
                    changed |= mm_drag(ui, fx, pitch, "Pitch:");
                    ui.horizontal(|ui| {
                        label_cell(ui, "Turns");
                        changed |= ui
                            .add(egui::DragValue::new(turns).speed(0.1).range(0.1..=1000.0))
                            .changed();
                    });
                }
                HelixMode::HeightTurns => {
                    changed |= mm_drag(ui, fx, height, "Height:");
                    ui.horizontal(|ui| {
                        label_cell(ui, "Turns");
                        changed |= ui
                            .add(egui::DragValue::new(turns).speed(0.1).range(0.1..=1000.0))
                            .changed();
                    });
                }
            }
            changed |= deg_drag(ui, fx, cone_angle_deg, "Cone angle:", -85.0..=85.0);
            changed |= check_row(ui, left_handed, "Left handed").changed();
            changed |= check_row(ui, reversed, "Reversed").changed();
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        PartFeature::Primitive {
            refine: _,
            kind,
            placement,
            subtractive,
        } => {
            changed |= primitive_editor(ui, kind);
            changed |= placement_editor(ui, placement);
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        PartFeature::Hole {
            refine: _,
            sketch,
            diameter,
            depth,
            through_all,
            cut,
            metric_index,
            threaded,
            fit,
            reversed,
        } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("hole_sketch", feature_id),
                Some(*sketch),
                "Positions:",
            ) {
                *sketch = new;
                changed = true;
            }
            ui.horizontal(|ui| {
                label_cell(ui, "Size");
                let current = metric_index
                    .and_then(|i| METRIC_SIZES.get(i).map(|(name, ..)| *name))
                    .unwrap_or("Custom");
                egui::ComboBox::from_id_salt(("hole_size", feature_id))
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(metric_index.is_none(), "Custom")
                            .clicked()
                            && metric_index.is_some()
                        {
                            *metric_index = None;
                            changed = true;
                        }
                        for (i, (name, ..)) in METRIC_SIZES.iter().enumerate() {
                            if ui
                                .selectable_label(*metric_index == Some(i), *name)
                                .clicked()
                                && *metric_index != Some(i)
                            {
                                *metric_index = Some(i);
                                changed = true;
                            }
                        }
                    });
            });
            if metric_index.is_some() {
                changed |= check_row(ui, threaded, "Threaded (tap drill)")
                    .on_hover_text("Use the tap-drill diameter for later thread cutting")
                    .changed();
                if !*threaded {
                    ui.horizontal(|ui| {
                        label_cell(ui, "Fit");
                        egui::ComboBox::from_id_salt(("hole_fit", feature_id))
                            .selected_text(fit.label())
                            .show_ui(ui, |ui| {
                                for candidate in HoleFit::ALL {
                                    if ui
                                        .selectable_label(*fit == candidate, candidate.label())
                                        .clicked()
                                        && *fit != candidate
                                    {
                                        *fit = candidate;
                                        changed = true;
                                    }
                                }
                            });
                    });
                }
                mono_label(
                    ui,
                    format!(
                        "Drill Ø {:.2} mm",
                        crate::build::hole_diameter(&PartFeature::Hole {
                            refine: false,
                            sketch: *sketch,
                            diameter: *diameter,
                            depth: *depth,
                            through_all: *through_all,
                            cut: *cut,
                            metric_index: *metric_index,
                            threaded: *threaded,
                            fit: *fit,
                            reversed: *reversed,
                        })
                    ),
                    FONT_SM,
                    TEXT2,
                );
            } else {
                changed |= mm_drag(ui, fx, diameter, "Diameter:");
            }
            changed |= check_row(ui, through_all, "Through all").changed();
            if !*through_all {
                changed |= mm_drag(ui, fx, depth, "Depth:");
            }
            ui.horizontal(|ui| {
                label_cell(ui, "Hole cut");
                egui::ComboBox::from_id_salt(("hole_cut", feature_id))
                    .selected_text(cut.label())
                    .show_ui(ui, |ui| {
                        let options = [
                            HoleCut::None,
                            HoleCut::Counterbore {
                                diameter: *diameter * 2.0,
                                depth: 2.0,
                            },
                            HoleCut::Countersink {
                                diameter: *diameter * 2.0,
                                angle_deg: 90.0,
                            },
                        ];
                        for candidate in options {
                            let is_current =
                                std::mem::discriminant(cut) == std::mem::discriminant(&candidate);
                            if ui.selectable_label(is_current, candidate.label()).clicked()
                                && !is_current
                            {
                                *cut = candidate;
                                changed = true;
                            }
                        }
                    });
            });
            match cut {
                HoleCut::None => {}
                HoleCut::Counterbore { diameter, depth } => {
                    changed |= mm_drag(ui, fx, diameter, "Bore Ø:");
                    changed |= mm_drag(ui, fx, depth, "Bore depth:");
                }
                HoleCut::Countersink {
                    diameter,
                    angle_deg,
                } => {
                    changed |= mm_drag(ui, fx, diameter, "Sink Ø:");
                    changed |= deg_drag(ui, fx, angle_deg, "Sink angle:", 10.0..=170.0);
                }
            }
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        PartFeature::Fillet { radius, edges } => {
            changed |= mm_drag(ui, fx, radius, "Radius:");
            changed |= edge_sel_editor(ui, ctx, edges, ("fillet_edges", feature_id));
        }
        PartFeature::Chamfer {
            size,
            mode,
            size2,
            angle_deg,
            flip,
            edges,
        } => {
            ui.horizontal(|ui| {
                label_cell(ui, "Type");
                egui::ComboBox::from_id_salt(("chamfer_mode", feature_id))
                    .selected_text(mode.label())
                    .show_ui(ui, |ui| {
                        for candidate in ChamferMode::ALL {
                            if ui
                                .selectable_label(*mode == candidate, candidate.label())
                                .clicked()
                                && *mode != candidate
                            {
                                *mode = candidate;
                                changed = true;
                            }
                        }
                    });
            });
            changed |= mm_drag(ui, fx, size, "Size:");
            match mode {
                ChamferMode::EqualDistance => {}
                ChamferMode::TwoDistances => {
                    changed |= mm_drag(ui, fx, size2, "Size 2:");
                    changed |= check_row(ui, flip, "Flip direction").changed();
                }
                ChamferMode::DistanceAngle => {
                    changed |= deg_drag(ui, fx, angle_deg, "Angle:", 1.0..=89.0);
                    changed |= check_row(ui, flip, "Flip direction").changed();
                }
            }
            changed |= edge_sel_editor(ui, ctx, edges, ("chamfer_edges", feature_id));
        }
        PartFeature::Draft {
            angle_deg,
            neutral,
            faces,
            reversed,
        } => {
            changed |= deg_drag(ui, fx, angle_deg, "Angle:", 0.1..=45.0);
            let mut neutral_opt = Some(*neutral);
            if face_pick_row(ui, ctx, &mut neutral_opt, "Neutral plane:")
                && let Some(pick) = neutral_opt
            {
                *neutral = pick;
                changed = true;
            }
            changed |= face_list_editor(ui, ctx, faces, "Faces to draft:");
            changed |= check_row(ui, reversed, "Reversed pull").changed();
        }
        PartFeature::Thickness {
            value,
            faces,
            inward,
        } => {
            changed |= mm_drag(ui, fx, value, "Thickness:");
            changed |= face_list_editor(ui, ctx, faces, "Faces to open:");
            changed |= check_row(ui, inward, "Inward").changed();
        }
        PartFeature::Mirrored {
            originals,
            plane,
            refine: _,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= mirror_plane_editor(ui, ctx, plane, ("mirror_plane", feature_id));
        }
        PartFeature::LinearPattern {
            refine: _,
            originals,
            axis,
            length,
            occurrences,
            spacing_mode,
            reversed,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(ui, axis, ("linear_axis", feature_id));
            changed |= count_drag(ui, fx, occurrences, "Occurrences:");
            changed |= check_row(ui, spacing_mode, "Length is spacing")
                .on_hover_text("Off: length is the overall span")
                .changed();
            changed |= mm_drag(ui, fx, length, "Length:");
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        PartFeature::PolarPattern {
            refine: _,
            originals,
            axis,
            angle_deg,
            occurrences,
            reversed,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(ui, axis, ("polar_axis", feature_id));
            changed |= count_drag(ui, fx, occurrences, "Occurrences:");
            changed |= deg_drag(ui, fx, angle_deg, "Angle:", 1.0..=360.0);
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        PartFeature::MultiTransform {
            originals,
            steps,
            refine: _,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            ui.label(
                RichText::new("Steps, each applied to every result of the ones before")
                    .font(sans(FONT_SM))
                    .color(TEXT2),
            );
            let mut remove = None;
            for (i, step) in steps.iter_mut().enumerate() {
                let label = match step {
                    TransformStep::Linear { .. } => "Linear",
                    TransformStep::Polar { .. } => "Polar",
                    TransformStep::Mirror { .. } => "Mirror",
                    TransformStep::Scale { .. } => "Scale",
                };
                ui.horizontal(|ui| {
                    mono_label(ui, format!("{}. {label}", i + 1), FONT_XS, TEXT1);
                    if small_secondary_button(ui, "✕").clicked() {
                        remove = Some(i);
                    }
                });
                match step {
                    TransformStep::Linear {
                        axis,
                        length,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(ui, axis, ("mt_lin", feature_id, i));
                        changed |= mm_drag(ui, fx, length, "Length:");
                        changed |= count_drag(ui, fx, occurrences, "Occurrences:");
                    }
                    TransformStep::Polar {
                        axis,
                        angle_deg,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(ui, axis, ("mt_pol", feature_id, i));
                        changed |= deg_drag(ui, fx, angle_deg, "Angle:", 1.0..=360.0);
                        changed |= count_drag(ui, fx, occurrences, "Occurrences:");
                    }
                    TransformStep::Mirror { plane } => {
                        changed |= mirror_plane_editor(ui, ctx, plane, ("mt_mir", feature_id, i));
                    }
                    TransformStep::Scale {
                        factor,
                        center,
                        occurrences,
                    } => {
                        ui.horizontal(|ui| {
                            label_cell(ui, "Factor");
                            changed |= ui
                                .add(egui::DragValue::new(factor).speed(0.05).range(0.01..=100.0))
                                .changed();
                        });
                        ui.horizontal_wrapped(|ui| {
                            label_cell(ui, "Center");
                            for v in center.iter_mut() {
                                changed |= ui.add(egui::DragValue::new(v).speed(0.5)).changed();
                            }
                        });
                        changed |= count_drag(ui, fx, occurrences, "Occurrences:");
                    }
                }
            }
            if let Some(i) = remove {
                steps.remove(i);
                changed = true;
            }
            ui.horizontal_wrapped(|ui| {
                if secondary_button(ui, "+ Linear").clicked() {
                    steps.push(TransformStep::Linear {
                        axis: PatternAxis::X,
                        length: 10.0,
                        occurrences: 2,
                    });
                    changed = true;
                }
                if secondary_button(ui, "+ Polar").clicked() {
                    steps.push(TransformStep::Polar {
                        axis: PatternAxis::Z,
                        angle_deg: 360.0,
                        occurrences: 4,
                    });
                    changed = true;
                }
                if secondary_button(ui, "+ Mirror").clicked() {
                    steps.push(TransformStep::Mirror {
                        plane: MirrorPlane::YZ,
                    });
                    changed = true;
                }
                if secondary_button(ui, "+ Scale").clicked() {
                    steps.push(TransformStep::Scale {
                        factor: 2.0,
                        center: [0.0; 3],
                        occurrences: 2,
                    });
                    changed = true;
                }
            });
        }
        PartFeature::Clone { source } => {
            let bodies: Vec<(BodyId, String)> = ctx
                .document
                .bodies()
                .iter()
                .filter(|b| b.id != body)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            let current = bodies
                .iter()
                .find(|(id, _)| id == source)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "(pick body)".into());
            ui.horizontal(|ui| {
                label_cell(ui, "Source body");
                egui::ComboBox::from_id_salt(("clone_body", feature_id))
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (id, name) in &bodies {
                            if ui.selectable_label(source == id, name).clicked() && source != id {
                                *source = *id;
                                changed = true;
                            }
                        }
                    });
            });
        }
        PartFeature::BodyBoolean {
            tool_body,
            kind,
            refine: _,
        } => {
            let bodies: Vec<(BodyId, String)> = ctx
                .document
                .bodies()
                .iter()
                .filter(|b| b.id != body)
                .map(|b| (b.id, b.name.clone()))
                .collect();
            let current = bodies
                .iter()
                .find(|(id, _)| id == tool_body)
                .map(|(_, n)| n.clone())
                .unwrap_or_else(|| "(pick body)".into());
            ui.horizontal(|ui| {
                label_cell(ui, "Tool body");
                egui::ComboBox::from_id_salt(("bool_body", feature_id))
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (id, name) in &bodies {
                            if ui.selectable_label(tool_body == id, name).clicked()
                                && tool_body != id
                            {
                                *tool_body = *id;
                                changed = true;
                            }
                        }
                    });
            });
            ui.horizontal(|ui| {
                label_cell(ui, "Operation");
                for (candidate, label) in [
                    (kernel_api::BoolKind::Fuse, "Fuse"),
                    (kernel_api::BoolKind::Cut, "Cut"),
                    (kernel_api::BoolKind::Common, "Common"),
                ] {
                    if ui.selectable_label(*kind == candidate, label).clicked()
                        && *kind != candidate
                    {
                        *kind = candidate;
                        changed = true;
                    }
                }
            });
        }
    }
    // Every feature that fuses or cuts can merge the coplanar faces it
    // leaves; new ones take the preference, this switch changes one.
    if feature.can_refine() {
        let mut refine = feature.refine();
        if check_row(ui, &mut refine, "Refine result").changed() {
            feature.set_refine(refine);
            changed = true;
        }
    }
    changed
}

/// The body of the feature the task panel edits.
fn edited_body(ctx: &WorkbenchRuntimeContext) -> Option<core_document::BodyId> {
    ctx.active_document_object
        .and_then(|id| ctx.document.get_feature_meta(id))
        .and_then(|node| node.body)
}

/// The picked face in the edited feature's body frame, where the feature
/// keeps its references.
fn picked_face(ctx: &WorkbenchRuntimeContext) -> Option<core_document::FaceRef> {
    match edited_body(ctx) {
        Some(body) => ctx.selected_face_in(body),
        None => ctx.selected_face,
    }
}

/// The picked edges in the edited feature's body frame.
fn picked_edges(ctx: &WorkbenchRuntimeContext) -> Vec<core_document::EdgeRef> {
    match edited_body(ctx) {
        Some(body) => ctx.selected_edges_in(body),
        None => ctx.selected_edges.clone(),
    }
}

#[cfg(test)]
mod formula_fields {
    use super::LABEL_PARAMETERS;

    #[test]
    fn every_field_label_names_a_parameter_part_design_lists() {
        let listed = crate::params::every_name();
        for (label, name) in LABEL_PARAMETERS {
            assert!(
                listed.contains(name),
                "{label} maps to {name}, which no feature lists"
            );
        }
    }
}

#[cfg(test)]
mod panel_width {
    use super::*;
    use core_document::{Document, WorkbenchFeature};
    use serde_json::json;

    const WIDTH: f32 = 300.0;

    /// How wide `draw` lays out in a column `WIDTH` wide.
    fn used_width(
        doc: &mut Document,
        feature: FeatureId,
        draw: &dyn Fn(&mut Ui, &WorkbenchRuntimeContext, &mut Formulas),
    ) -> f32 {
        let ctx = egui::Context::default();
        ui_kit::apply_theme(&ctx);
        let wctx = WorkbenchRuntimeContext::new(doc, [0.0; 3], [0.0; 3], (0, 0, 800, 600));
        let shown: &WorkbenchRuntimeContext = &wctx;
        let mut width = 0.0;
        for _ in 0..2 {
            let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut column = ui.new_child(egui::UiBuilder::new().max_rect(
                    egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(WIDTH, 3000.0)),
                ));
                let mut fx = Formulas::of(shown.document, feature);
                draw(&mut column, shown, &mut fx);
                width = column.min_rect().width();
            });
            output.textures_delta.clear();
        }
        width
    }

    #[test]
    fn every_editor_fits_a_narrow_task_panel() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let sketch = doc
            .add_feature_in_body(
                wb_sketch::SketchFeature::from_sketch(wb_sketch::sketch::Sketch::new("s")),
                "Sketch".into(),
                Some(body),
            )
            .unwrap();
        let s = sketch.0.to_string();
        let features = [
            json!({"Pad": {"sketch": s, "length": 10.0, "reversed": false}}),
            json!({"Pocket": {"sketch": s, "depth": 5.0, "reversed": false}}),
            json!({"Hole": {"sketch": s, "diameter": 5.0, "depth": 8.0, "through_all": false,
                "cut": {"Counterbore": {"diameter": 9.0, "depth": 2.0}}}}),
            json!({"Chamfer": {"size": 1.0}}),
            json!({"LinearPattern": {"originals": [], "axis": "X", "length": 20.0, "occurrences": 3}}),
            json!({"PolarPattern": {"originals": [], "axis": "Z", "angle_deg": 360.0, "occurrences": 6}}),
            json!({"MultiTransform": {"originals": [], "steps": [
                {"Linear": {"axis": "X", "length": 20.0, "occurrences": 3}}]}}),
            json!({"Revolution": {"sketch": s, "angle_deg": 360.0}}),
            json!({"Groove": {"sketch": s, "angle_deg": 360.0}}),
            json!({"Helix": {"sketch": s, "axis": "SketchY", "mode": "PitchHeight", "pitch": 2.0,
                "height": 10.0, "turns": 5.0, "left_handed": false, "cone_angle_deg": 0.0,
                "reversed": false, "subtractive": false}}),
            json!({"Loft": {"sections": [s], "ruled": false, "closed": false, "subtractive": false}}),
            json!({"Pipe": {"profile": s, "spine": s, "frenet": false, "subtractive": false}}),
            json!({"Fillet": {"radius": 1.0}}),
            json!({"Chamfer": {"size": 1.0, "mode": "DistanceAngle"}}),
            json!({"Draft": {"angle_deg": 3.0, "neutral": {"point": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0]}, "faces": []}}),
            json!({"Thickness": {"value": 1.0, "faces": []}}),
            json!({"Mirrored": {"originals": [], "plane": "XY"}}),
            json!({"MultiTransform": {"originals": [], "steps": [
                {"Scale": {"factor": 2.0, "center": [0.0, 0.0, 0.0], "occurrences": 2}},
                {"Mirror": {"plane": "XZ"}}]}}),
            json!({"Primitive": {
                "kind": {"Cylinder": {"radius": 5.0, "height": 10.0, "angle_deg": 360.0}},
                "placement": {"origin": [0.0, 0.0, 0.0], "x_axis": [1.0, 0.0, 0.0], "z_axis": [0.0, 0.0, 1.0]},
                "subtractive": false}}),
        ];
        for data in features {
            let feature = PartFeature::from_json(&data).unwrap();
            let id = doc
                .add_feature_in_body(feature.clone(), "Feature".into(), Some(body))
                .unwrap();
            let width = used_width(&mut doc, id, &|ui, ctx, fx| {
                let mut f = feature.clone();
                feature_editor(ui, ctx, fx, body, id, &mut f);
            });
            assert!(width <= WIDTH + 0.5, "{data} lays out {width} px wide");
        }
        let datum = core_document::DatumFeature {
            shape: core_document::DatumShape::Plane { size: 30.0 },
            attachment: core_document::DatumAttachment::BasePlane(core_document::BasePlane::XY),
            offset: Default::default(),
        };
        let id = doc
            .add_feature_in_body(datum, "Plane".into(), Some(body))
            .unwrap();
        let width = used_width(&mut doc, id, &|ui, ctx, fx| {
            let mut d = datum;
            datum_editor(ui, ctx, fx, id, &mut d);
        });
        assert!(
            width <= WIDTH + 0.5,
            "the datum editor lays out {width} px wide"
        );
    }
}
