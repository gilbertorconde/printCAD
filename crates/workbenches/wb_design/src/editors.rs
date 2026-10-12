//! Per-feature parameter editors for the task panel.
//!
//! Every option maps 1:1 to a feature field. Editors return `true` when
//! the feature payload changed; the task applies edits live.
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

use crate::build::design_features_of_body;
use crate::feature::{
    ChamferMode, DesignFeature, EdgePick, EdgeSel, ExtrudeDirection, ExtrudeMode, FacePick,
    HelixMode, MirrorPlane, PatternAxis, PipeCorner, PipeOrientation, RevolveAxis, RevolveMode,
    SketchAxis, TransformStep,
};

mod borrow;
mod hole;

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
    ("Second offset", "offset2"),
    ("Angle", "angle"),
    ("Angle 2", "angle2"),
    ("Pitch", "pitch"),
    ("Height", "height"),
    ("Cone angle", "cone_angle"),
    ("Growth", "growth"),
    ("Binormal X", "binormal_x"),
    ("Binormal Y", "binormal_y"),
    ("Binormal Z", "binormal_z"),
    ("Diameter", "diameter"),
    ("Bore Ø", "counterbore_diameter"),
    ("Bore depth", "counterbore_depth"),
    ("Sink Ø", "countersink_diameter"),
    ("Sink angle", "countersink_angle"),
    ("Thread depth", "thread_depth"),
    ("Point angle", "point_angle"),
    ("Spot Ø", "spotface_diameter"),
    ("Spot depth", "spotface_depth"),
    ("Counterdrill Ø", "counterdrill_diameter"),
    ("Counterdrill depth", "counterdrill_depth"),
    ("Counterdrill angle", "counterdrill_angle"),
    ("Nut clearance", "nut_clearance"),
    ("Nut depth", "nut_depth"),
    ("Across flats", "nut_across_flats"),
    ("Nut turn", "nut_turn"),
    ("Turns", "turns"),
    ("Factor", "factor"),
    ("Radius", "radius"),
    ("Size", "size"),
    ("Size 2", "size2"),
    ("Thickness", "thickness"),
    ("Occurrences", "occurrences"),
    ("Offset X", "offset_x"),
    ("Offset Y", "offset_y"),
    ("Normal offset", "offset_z"),
    ("Rotation", "rotation"),
    ("Tilt about X", "tilt_x"),
    ("Tilt about Y", "tilt_y"),
];

/// What a feature's fields know of formulas: the feature's parameters,
/// which of them a formula sets, and the formula edits made this frame, for
/// the task to apply (`(key, formula)`, `None` taking one away).
pub(crate) struct Formulas<'a> {
    document: &'a core_document::Document,
    feature: FeatureId,
    params: Vec<core_document::Parameter>,
    pub edits: Vec<(String, Option<String>)>,
    /// Put before each name looked up: a multi-transform step's fields
    /// are `step2_length` and the like.
    prefix: Option<String>,
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
            prefix: None,
        }
    }

    /// The parameter an editor field labelled `label` shows.
    fn find(&self, label: &str) -> Option<core_document::Parameter> {
        let wanted = label.trim_end_matches(':');
        let name = LABEL_PARAMETERS
            .iter()
            .find(|(l, _)| *l == wanted)
            .map(|(_, name)| *name)?;
        let name = match &self.prefix {
            Some(prefix) => format!("{prefix}{name}"),
            None => name.to_string(),
        };
        let node = self.document.get_feature_meta(self.feature)?;
        self.params
            .iter()
            .find(|p| {
                p.name.as_deref() == Some(name.as_str()) && node.data.pointer(&p.pointer).is_some()
            })
            .cloned()
    }

    /// The field for `label` as a formula field, if it has a parameter:
    /// `Some(changed)` when it did, the value set by hand going into
    /// `value` and a formula into `edits`.
    fn show(&mut self, ui: &mut Ui, label: &str, value: f64) -> Option<(bool, f64)> {
        let p = self.find(label)?;
        Some(self.show_param(ui, label, p, value))
    }

    /// As [`Self::show`], for the parameter formulas call `name` (a
    /// primitive's `radius`, a step's `step2_length`) under `label`.
    fn show_named(
        &mut self,
        ui: &mut Ui,
        label: &str,
        name: &str,
        value: f64,
    ) -> Option<(bool, f64)> {
        let node = self.document.get_feature_meta(self.feature)?;
        let p = self
            .params
            .iter()
            .find(|p| p.name.as_deref() == Some(name) && node.data.pointer(&p.pointer).is_some())
            .cloned()?;
        Some(self.show_param(ui, label, p, value))
    }

    fn show_param(
        &mut self,
        ui: &mut Ui,
        label: &str,
        p: core_document::Parameter,
        value: f64,
    ) -> (bool, f64) {
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
        match edit {
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
        }
    }
}

/// A number formulas call `name`, as a formula field when the feature has
/// that parameter, else as `fallback` draws it.
fn named_f64(
    ui: &mut Ui,
    fx: &mut Formulas,
    value: &mut f64,
    (label, name): (&str, &str),
    fallback: impl FnOnce(&mut Ui, &mut f64) -> bool,
) -> bool {
    if let Some((changed, v)) = fx.show_named(ui, label, name, *value) {
        *value = v;
        return changed;
    }
    fallback(ui, value)
}

/// A helix's turns, a formula field where it has the parameter.
fn turns_field(ui: &mut Ui, fx: &mut Formulas, turns: &mut f32) -> bool {
    if let Some((changed, v)) = fx.show(ui, "Turns:", f64::from(*turns)) {
        *turns = (v as f32).clamp(0.1, 1000.0);
        return changed;
    }
    ui.horizontal(|ui| {
        label_cell(ui, "Turns");
        ui.add(egui::DragValue::new(turns).speed(0.1).range(0.1..=1000.0))
            .changed()
    })
    .inner
}

fn mm_drag(ui: &mut Ui, fx: &mut Formulas, value: &mut f32, label: &str) -> bool {
    if let Some((changed, v)) = fx.show(ui, label, f64::from(*value)) {
        *value = v as f32;
        return changed;
    }
    field(ui, label, |ui| QtyField::mm(value).speed(0.5).show(ui))
}

/// A length that may be zero or negative: a shift either way.
fn offset_drag(ui: &mut Ui, fx: &mut Formulas, value: &mut f32, label: &str) -> bool {
    if let Some((changed, v)) = fx.show(ui, label, f64::from(*value)) {
        *value = v as f32;
        return changed;
    }
    field(ui, label, |ui| QtyField::offset(value).speed(0.5).show(ui))
}

/// Which way a positive taper leans, under its field.
fn taper_note(ui: &mut Ui, text: &str) {
    ui.label(
        egui::RichText::new(text)
            .font(ui_kit::sans(ui_kit::tokens::FONT_XS))
            .color(ui_kit::tokens::TEXT3),
    );
}

/// A length that may be negative, its formula under `name` when it has one.
fn signed_mm(ui: &mut Ui, fx: &mut Formulas, value: &mut f32, label: &str, name: &str) -> bool {
    if let Some((changed, v)) = fx.show_named(ui, label, name, f64::from(*value)) {
        *value = v as f32;
        return changed;
    }
    field(ui, label, |ui| QtyField::offset(value).show(ui))
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
        *value = v.round().clamp(1.0, 1000.0) as u32;
        return changed;
    }
    field(ui, label, |ui| {
        let mut v = *value as f32;
        let changed = QtyField::new(&mut v)
            .decimals(0)
            .speed(0.1)
            .range(1.0..=1000.0)
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
    id_salt: (&'static str, FeatureId),
    current: Option<FeatureId>,
    label: &str,
) -> Option<FeatureId> {
    let sketches = crate::build::sketch_choices(ctx.document, body, id_salt.1, current);
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
    borrowed: &[(ExtrudeMode, String)],
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "Type");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(mode_name(*mode, borrowed))
            .show_ui(ui, |ui| {
                for (candidate, name) in mode_choices(&ExtrudeMode::ALL, borrowed) {
                    // Material-relative modes need an earlier solid.
                    if first_feature && needs_own_material(candidate, borrowed) {
                        continue;
                    }
                    if ui.selectable_label(*mode == candidate, name).clicked() && *mode != candidate
                    {
                        *mode = candidate;
                        changed = true;
                    }
                }
            });
    });
    changed
}

/// Whether `mode` ends only on the body's own material: up to shape stops
/// on borrowed faces too, when the body borrows any.
fn needs_own_material(mode: ExtrudeMode, targets: &[(ExtrudeMode, String)]) -> bool {
    let lends_faces = targets
        .iter()
        .any(|(t, _)| matches!(t, ExtrudeMode::UpToBorrowed(_)));
    mode.needs_material() && !(mode == ExtrudeMode::UpToShape && lends_faces)
}

/// The end conditions offered: `modes`, then the ones that name what they
/// stop on (a borrowed face, a plane).
fn mode_choices(
    modes: &[ExtrudeMode],
    targets: &[(ExtrudeMode, String)],
) -> Vec<(ExtrudeMode, String)> {
    modes
        .iter()
        .map(|m| (*m, m.label().to_string()))
        .chain(targets.iter().cloned())
        .collect()
}

/// What an end condition is called: one that names what it stops on by
/// it.
fn mode_name(mode: ExtrudeMode, targets: &[(ExtrudeMode, String)]) -> String {
    targets
        .iter()
        .find(|(c, _)| *c == mode)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| mode.label().to_string())
}

/// The end conditions that name what they stop on, for `body`: each face
/// it borrows, its own planes, and its datum planes (a coordinate system
/// by each of its planes).
fn end_targets(ctx: &WorkbenchRuntimeContext, body: BodyId) -> Vec<(ExtrudeMode, String)> {
    use crate::feature::PlaneTarget;
    use core_document::{BasePlane, DatumShape};
    let mut out: Vec<(ExtrudeMode, String)> = crate::borrow::faces_of_body(ctx.document, body)
        .into_iter()
        .map(|(r, name)| (ExtrudeMode::UpToBorrowed(r), format!("Up to {name}")))
        .collect();
    for plane in BasePlane::ALL {
        out.push((
            ExtrudeMode::UpToPlane(PlaneTarget::Base(plane)),
            format!("Up to the {} plane", plane.label()),
        ));
    }
    for (datum, name, made) in core_document::datums_of_body(ctx.document, body) {
        match made.shape {
            DatumShape::Plane { .. } => out.push((
                ExtrudeMode::UpToPlane(PlaneTarget::Datum { datum, plane: None }),
                format!("Up to {name}"),
            )),
            DatumShape::CoordinateSystem { .. } => {
                for plane in BasePlane::ALL {
                    let key = match plane {
                        BasePlane::XY => "XY",
                        BasePlane::XZ => "XZ",
                        BasePlane::YZ => "YZ",
                    };
                    out.push((
                        ExtrudeMode::UpToPlane(PlaneTarget::Datum {
                            datum,
                            plane: Some(plane),
                        }),
                        format!("Up to {name} {key}"),
                    ));
                }
            }
            _ => {}
        }
    }
    out
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
            *pick = Some(FacePick::of(face));
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
            if small_secondary_button(ui, "×").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        faces.remove(i);
        changed = true;
    }
    // Every face picked (Ctrl adds more) that the list does not hold yet.
    let fresh: Vec<FacePick> = picked_faces(ctx)
        .into_iter()
        .map(FacePick::of)
        .filter(|pick| !faces.iter().any(|f| f.same_place(pick)))
        .collect();
    let label = match fresh.len() {
        0 | 1 => "Add selected face".to_string(),
        n => format!("Add the {n} selected faces"),
    };
    if ui
        .add_enabled_ui(!fresh.is_empty(), |ui| accent_outline_button(ui, &label))
        .inner
        .on_hover_text("Click faces in the viewport first (Ctrl adds more), then press this")
        .clicked()
    {
        faces.extend(fresh);
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

/// The switch that makes a dress-up take the edges tangent to its own.
fn tangent_row(ui: &mut Ui, follow_tangent: &mut bool) -> bool {
    check_row(ui, follow_tangent, "Follow tangent edges")
        .on_hover_text(
            "Take every edge that runs on smoothly from a selected one, \
             such as the round of a filleted corner and the side past it",
        )
        .changed()
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
            if small_secondary_button(ui, "×").clicked() {
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
            let pick = EdgePick::of(edge);
            if !picks.contains(&pick) {
                picks.push(pick);
                changed = true;
            }
        }
    }
    changed
}

/// A picked edge, shown by where it was picked, and a button that takes
/// the edge picked in the viewport.
fn edge_pick_row(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    pick: &mut Option<EdgePick>,
    label: &str,
) -> bool {
    let mut changed = false;
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
        let has_selection = !ctx.selected_edges.is_empty();
        if ui
            .add_enabled_ui(has_selection, |ui| {
                accent_outline_button(ui, "Use selected edge")
            })
            .inner
            .on_hover_text("Click a straight edge in the viewport first, then press this")
            .clicked()
            && let Some(edge) = picked_edges(ctx).first()
        {
            *pick = Some(EdgePick::of(edge));
            changed = true;
        }
    });
    changed
}

/// The datums of `body` (a line along itself, a plane square to it) and the
/// lines of its sketches, as ways an extrusion may run.
fn direction_references(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
) -> Vec<(ExtrudeDirection, String)> {
    let mut out: Vec<(ExtrudeDirection, String)> =
        core_document::datums_of_body(ctx.document, body)
            .into_iter()
            .filter(|(_, _, d)| !matches!(d.shape, core_document::DatumShape::Point))
            .map(|(id, name, _)| (ExtrudeDirection::Datum(id), name))
            .collect();
    let mut sketches: Vec<(u64, FeatureId, String)> = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body == Some(body))
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, ..)| *seq);
    for (_, sketch, name) in sketches {
        let Some(feature) = ctx.document.feature_values(sketch).and_then(|data| {
            <wb_sketch::SketchFeature as core_document::WorkbenchFeature>::from_json(data).ok()
        }) else {
            continue;
        };
        let mut n = 0;
        for element in &feature.sketch.geometry {
            if let wb_sketch::sketch::GeometryElement::Line(line) = element {
                n += 1;
                out.push((
                    ExtrudeDirection::SketchLine {
                        sketch,
                        element: line.id,
                    },
                    format!("{name} › line {n}"),
                ));
            }
        }
    }
    out
}

/// Which way a pad or pocket runs: the profile's normal, a vector typed
/// in, or a picked edge.
fn extrude_direction_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    direction: &mut ExtrudeDirection,
    id_salt: impl egui::AsIdSalt,
) -> bool {
    let mut changed = false;
    let borrowed = crate::borrow::edges_of_body(ctx.document, body);
    let shown = match direction {
        ExtrudeDirection::Borrowed(r) => borrowed
            .iter()
            .find(|(c, _)| c == r)
            .map(|(_, name)| name.clone())
            .unwrap_or_else(|| direction.label().to_string()),
        ExtrudeDirection::Datum(_) | ExtrudeDirection::SketchLine { .. } => {
            direction_references(ctx, body)
                .into_iter()
                .find(|(c, _)| c == direction)
                .map(|(_, name)| name)
                .unwrap_or_else(|| direction.label().to_string())
        }
        _ => direction.label().to_string(),
    };
    ui.horizontal(|ui| {
        label_cell(ui, "Direction");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                let mut candidates = vec![
                    (
                        ExtrudeDirection::Normal,
                        ExtrudeDirection::Normal.label().to_string(),
                    ),
                    (
                        ExtrudeDirection::Custom([0.0, 0.0, 1.0]),
                        "Custom vector".to_string(),
                    ),
                    (
                        ExtrudeDirection::Edge(EdgePick {
                            faces: [0, 0],
                            point: [0.0; 3],
                            direction: [0.0, 0.0, 1.0],
                        }),
                        "Picked edge".to_string(),
                    ),
                ];
                candidates.extend(
                    borrowed
                        .iter()
                        .map(|(r, name)| (ExtrudeDirection::Borrowed(*r), name.clone())),
                );
                candidates.extend(
                    crate::feature::BaseAxis::ALL
                        .into_iter()
                        .map(|axis| (ExtrudeDirection::Axis(axis), axis.label().to_string())),
                );
                candidates.extend(direction_references(ctx, body));
                for (candidate, name) in candidates {
                    let is_current = match (&*direction, &candidate) {
                        (ExtrudeDirection::Custom(_), ExtrudeDirection::Custom(_))
                        | (ExtrudeDirection::Edge(_), ExtrudeDirection::Edge(_))
                        | (ExtrudeDirection::Normal, ExtrudeDirection::Normal) => true,
                        _ => *direction == candidate,
                    };
                    if ui.selectable_label(is_current, name).clicked() && !is_current {
                        // A picked edge starts from the one picked now.
                        *direction = match candidate {
                            ExtrudeDirection::Edge(_) => match picked_edges(ctx).first() {
                                Some(edge) => ExtrudeDirection::Edge(EdgePick::of(edge)),
                                None => candidate,
                            },
                            other => other,
                        };
                        changed = true;
                    }
                }
            });
    });
    match direction {
        ExtrudeDirection::Normal
        | ExtrudeDirection::Borrowed(_)
        | ExtrudeDirection::Datum(_)
        | ExtrudeDirection::SketchLine { .. }
        | ExtrudeDirection::Axis(_) => {}
        // Each component a number formulas can set: `Pad.direction_x`.
        ExtrudeDirection::Custom(v) => {
            for (c, (label, name)) in v.iter_mut().zip([
                ("Vector X:", "direction_x"),
                ("Vector Y:", "direction_y"),
                ("Vector Z:", "direction_z"),
            ]) {
                let mut value = f64::from(*c);
                let edited = named_f64(ui, fx, &mut value, (label, name), |ui, value| {
                    field(ui, label, |ui| {
                        ui.add(egui::DragValue::new(value).speed(0.05)).changed()
                    })
                });
                if edited {
                    *c = value as f32;
                    changed = true;
                }
            }
        }
        ExtrudeDirection::Edge(edge) => {
            let mut pick = Some(*edge);
            if edge_pick_row(ui, ctx, &mut pick, "Edge:")
                && let Some(new) = pick
            {
                *edge = new;
                changed = true;
            }
        }
    }
    changed
}

/// A pad's or pocket's less used settings: where it starts, the second
/// side's own taper, and a slanted length measured along the normal.
fn extrude_extras_rows(
    ui: &mut Ui,
    fx: &mut Formulas,
    extras: &mut crate::feature::ExtrudeExtras,
    two_sided: bool,
    direction: ExtrudeDirection,
) -> bool {
    let mut changed = offset_drag(ui, fx, &mut extras.start_offset, "Start offset:");
    if two_sided {
        let mut own = extras.taper2_deg.is_some();
        if check_row(ui, &mut own, "Second side's own taper").changed() {
            extras.taper2_deg = own.then_some(0.0);
            changed = true;
        }
        if let Some(taper) = &mut extras.taper2_deg {
            changed |= deg_drag(ui, fx, taper, "Second taper:", -85.0..=85.0);
        }
    }
    if direction != ExtrudeDirection::Normal {
        changed |= check_row(ui, &mut extras.along_normal, "Length along the normal")
            .on_hover_text("With a slanted direction, the length is measured square to the profile")
            .changed();
    }
    changed
}

/// The rows of one side's end condition, beyond its mode: the length, the
/// face or the faces it stops on and their offset.
#[expect(clippy::too_many_arguments)]
fn extrude_side_rows(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    mode: ExtrudeMode,
    length: (&mut f32, &str),
    face: &mut Option<FacePick>,
    offset: (&mut f32, &str),
    shape: &mut Vec<FacePick>,
    (body, borrowed): (BodyId, &mut Vec<crate::feature::BorrowedRef>),
) -> bool {
    let mut changed = false;
    match mode {
        ExtrudeMode::Dimension | ExtrudeMode::TwoLengths => {
            changed |= mm_drag(ui, fx, length.0, length.1);
        }
        ExtrudeMode::UpToFace => {
            changed |= face_pick_row(ui, ctx, face, "Target face:");
            changed |= offset_drag(ui, fx, offset.0, offset.1);
        }
        ExtrudeMode::UpToBorrowed(_) | ExtrudeMode::UpToPlane(_) => {
            changed |= offset_drag(ui, fx, offset.0, offset.1);
        }
        ExtrudeMode::UpToShape => {
            changed |= face_list_editor(ui, ctx, shape, "Stop faces:");
            changed |= borrowed_face_checks(ui, ctx, body, borrowed);
            changed |= offset_drag(ui, fx, offset.0, offset.1);
        }
        _ => {}
    }
    changed
}

/// A check row for each face `body` borrows: the ones ticked are stop
/// faces too.
fn borrowed_face_checks(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    borrowed: &mut Vec<crate::feature::BorrowedRef>,
) -> bool {
    let faces = crate::borrow::faces_of_body(ctx.document, body);
    if faces.is_empty() {
        return false;
    }
    let mut changed = false;
    label_cell(ui, "Borrowed stop faces:");
    for (r, name) in faces {
        let mut on = borrowed.contains(&r);
        if check_row(ui, &mut on, &name).changed() {
            if on {
                borrowed.push(r);
            } else {
                borrowed.retain(|b| *b != r);
            }
            changed = true;
        }
    }
    changed
}

/// The second side of a two-sided extrusion: none, or how it ends. Two
/// lengths always has one.
fn second_side_combo(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    first: ExtrudeMode,
    mode2: &mut Option<ExtrudeMode>,
    first_feature: bool,
    borrowed: &[(ExtrudeMode, String)],
) -> bool {
    let mut changed = false;
    let required = first == ExtrudeMode::TwoLengths;
    let shown = match *mode2 {
        None if required => ExtrudeMode::Dimension.label().to_string(),
        None => "None".to_string(),
        Some(mode) => mode_name(mode, borrowed),
    };
    ui.horizontal(|ui| {
        label_cell(ui, "Second side");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if !required && ui.selectable_label(mode2.is_none(), "None").clicked() {
                    changed |= mode2.take().is_some();
                }
                for (candidate, name) in mode_choices(&ExtrudeMode::SECOND_SIDE, borrowed) {
                    if first_feature && needs_own_material(candidate, borrowed) {
                        continue;
                    }
                    let current = *mode2 == Some(candidate)
                        || (required && mode2.is_none() && candidate == ExtrudeMode::Dimension);
                    if ui.selectable_label(current, name).clicked() && !current {
                        *mode2 = Some(candidate);
                        changed = true;
                    }
                }
            });
    });
    changed
}

/// The lines of a sketch and the datum lines of a body, as axis choices.
fn axis_choices(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    sketch: FeatureId,
) -> Vec<(RevolveAxis, String)> {
    let mut choices = Vec::new();
    if let Some(sketch) = ctx.document.feature_values(sketch).and_then(|data| {
        <wb_sketch::SketchFeature as core_document::WorkbenchFeature>::from_json(data).ok()
    }) {
        let mut n = 0;
        for element in &sketch.sketch.geometry {
            if let wb_sketch::sketch::GeometryElement::Line(line) = element {
                n += 1;
                let kind = if sketch.sketch.is_construction(line.id) {
                    "construction line"
                } else {
                    "line"
                };
                choices.push((
                    RevolveAxis::SketchLine(line.id),
                    format!("Sketch {kind} {n}"),
                ));
            }
        }
    }
    for (id, name, datum) in core_document::datums_of_body(ctx.document, body) {
        if matches!(datum.shape, core_document::DatumShape::Line { .. }) {
            choices.push((RevolveAxis::Datum(id), name));
        }
    }
    for (r, name) in crate::borrow::edges_of_body(ctx.document, body) {
        choices.push((RevolveAxis::Borrowed(r), name));
    }
    choices
}

/// What a revolution or helix spins about: the sketch's axes, a custom
/// axis, a line of the sketch, a datum line or a picked edge.
fn revolve_axis_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    sketch: FeatureId,
    axis: &mut RevolveAxis,
    id_salt: impl egui::AsIdSalt,
    helix: bool,
) -> bool {
    let mut changed = false;
    let references = axis_choices(ctx, body, sketch);
    let shown = references
        .iter()
        .find(|(choice, _)| choice == axis)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| axis.label().to_string());
    ui.horizontal(|ui| {
        label_cell(ui, "Axis");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                let picked_edge = picked_edges(ctx).first().map(EdgePick::of);
                let mut candidates = vec![
                    (
                        RevolveAxis::SketchY,
                        RevolveAxis::SketchY.label().to_string(),
                    ),
                    (
                        RevolveAxis::SketchX,
                        RevolveAxis::SketchX.label().to_string(),
                    ),
                    (
                        RevolveAxis::Custom {
                            origin: [0.0, 0.0],
                            dir: [1.0, 1.0],
                        },
                        "Custom axis".to_string(),
                    ),
                ];
                candidates.extend(
                    crate::feature::BaseAxis::ALL
                        .into_iter()
                        .map(|a| (RevolveAxis::Base(a), a.label().to_string())),
                );
                if helix {
                    candidates.push((
                        RevolveAxis::SketchNormal,
                        RevolveAxis::SketchNormal.label().to_string(),
                    ));
                }
                candidates.extend(references.iter().cloned());
                candidates.push((
                    RevolveAxis::Edge(picked_edge.unwrap_or(EdgePick {
                        faces: [0, 0],
                        point: [0.0; 3],
                        direction: [0.0, 0.0, 1.0],
                    })),
                    "Picked edge".to_string(),
                ));
                for (candidate, name) in candidates {
                    let is_current = match (&*axis, &candidate) {
                        (RevolveAxis::SketchLine(_), RevolveAxis::SketchLine(_))
                        | (RevolveAxis::Datum(_), RevolveAxis::Datum(_))
                        | (RevolveAxis::Borrowed(_), RevolveAxis::Borrowed(_))
                        | (RevolveAxis::Base(_), RevolveAxis::Base(_)) => *axis == candidate,
                        _ => std::mem::discriminant(axis) == std::mem::discriminant(&candidate),
                    };
                    if ui.selectable_label(is_current, name).clicked() && !is_current {
                        *axis = candidate;
                        changed = true;
                    }
                }
            });
    });
    if let RevolveAxis::Edge(edge) = axis {
        let mut pick = Some(*edge);
        if edge_pick_row(ui, ctx, &mut pick, "Edge:")
            && let Some(new) = pick
        {
            *edge = new;
            changed = true;
        }
    }
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

/// The axis taken as its shadow on the sketch plane, offered for the axes
/// from outside the sketch, which can stand tilted to it.
fn project_axis_row(ui: &mut Ui, axis: &RevolveAxis, project_axis: &mut bool) -> bool {
    matches!(
        axis,
        RevolveAxis::Edge(_)
            | RevolveAxis::Datum(_)
            | RevolveAxis::Borrowed(_)
            | RevolveAxis::Base(_)
    ) && check_row(ui, project_axis, "Project onto the sketch")
        .on_hover_text("An axis tilted to the sketch plane turns about its shadow on the plane")
        .changed()
}

/// A pad's or a pocket's face profile, when it has one: where it is, and a
/// flat face picked in the view to take its place.
fn face_profile_row(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    profile_face: &mut Option<FacePick>,
) -> bool {
    let Some(face) = profile_face else {
        return false;
    };
    ui.horizontal_wrapped(|ui| {
        label_cell(ui, "Profile");
        mono_label(
            ui,
            format!(
                "face at ({:.1}, {:.1}, {:.1})",
                face.point[0], face.point[1], face.point[2]
            ),
            FONT_XS,
            TEXT1,
        );
    });
    // Its own line: beside the face's position it runs past a narrow
    // panel.
    let picked = ctx.selected_face_in(body).filter(|f| {
        matches!(
            f.surface,
            None | Some(kernel_api::FaceSurface::Plane { .. })
        )
    });
    let clicked = ui
        .add_enabled_ui(picked.is_some(), |ui| {
            accent_outline_button(ui, "Use selected face")
        })
        .inner
        .on_hover_text("Click another flat face of the solid first, then press this")
        .clicked();
    match picked {
        Some(pick) if clicked => {
            *face = FacePick::of(pick);
            true
        }
        _ => false,
    }
}

/// The regions of the profile sketch a feature takes, a check each, shown
/// when the sketch's curves close more than one. With none chosen the
/// checks show the regions the whole profile takes; choosing those again
/// goes back to taking the whole profile.
fn regions_rows(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    sketch: FeatureId,
    chosen: &mut Vec<String>,
) -> bool {
    let regions = crate::build::sketch_regions(ctx.document, sketch);
    if regions.len() < 2 && chosen.is_empty() {
        return false;
    }
    let mut changed = false;
    let taken: Vec<bool> = regions
        .iter()
        .map(|r| {
            if chosen.is_empty() {
                r.default
            } else {
                chosen.contains(&r.name)
            }
        })
        .collect();
    ui.horizontal(|ui| label_cell(ui, "Regions:"));
    for (index, region) in regions.iter().enumerate() {
        let mut on = taken[index];
        let label = format!(
            "Region {} · {:.1} mm² at ({:.1}, {:.1})",
            index + 1,
            region.area,
            region.inside[0],
            region.inside[1]
        );
        if !check_row(ui, &mut on, &label).changed() {
            continue;
        }
        let mut now = taken.clone();
        now[index] = on;
        // Nothing taken is no profile: the last region stays.
        if !now.contains(&true) {
            continue;
        }
        let defaults: Vec<bool> = regions.iter().map(|r| r.default).collect();
        *chosen = if now == defaults {
            Vec::new()
        } else {
            regions
                .iter()
                .zip(&now)
                .filter(|(_, on)| **on)
                .map(|(r, _)| r.name.clone())
                .collect()
        };
        changed = true;
    }
    let gone = chosen
        .iter()
        .filter(|name| !regions.iter().any(|r| &r.name == *name))
        .count();
    if gone > 0 {
        taper_note(
            ui,
            &format!("{gone} chosen region(s) are no longer in the sketch."),
        );
        if small_secondary_button(ui, "Take the whole profile").clicked() {
            chosen.clear();
            changed = true;
        }
    }
    changed
}

/// What a pipe sweeps and along what: its profile a sketch or a picked
/// face, its path a sketch, picked edges of the solid or edges another body
/// lends.
fn pipe_inputs_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    (body, feature_id): (BodyId, FeatureId),
    spine: &mut FeatureId,
    profile_face: &mut Option<FacePick>,
    path_edges: &mut Vec<EdgePick>,
    path_borrowed: &mut Vec<crate::feature::BorrowedRef>,
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        label_cell(ui, "Profile");
        let shown = match profile_face {
            Some(p) => format!(
                "face at ({:.1}, {:.1}, {:.1})",
                p.point[0], p.point[1], p.point[2]
            ),
            None => "its sketch".to_string(),
        };
        mono_label(ui, shown, FONT_XS, TEXT1);
        let picked = ctx.selected_face_in(body).map(FacePick::of);
        if ui
            .add_enabled_ui(picked.is_some(), |ui| {
                accent_outline_button(ui, "Use selected face")
            })
            .inner
            .on_hover_text("Click a flat face of the solid first, then press this")
            .clicked()
            && let Some(pick) = picked
        {
            *profile_face = Some(pick);
            changed = true;
        }
        if profile_face.is_some() && small_secondary_button(ui, "Use the sketch").clicked() {
            *profile_face = None;
            changed = true;
        }
    });
    let along_edges = !path_edges.is_empty() || !path_borrowed.is_empty();
    if !along_edges
        && let Some(new) = sketch_combo(
            ui,
            ctx,
            body,
            ("pipe_spine", feature_id),
            Some(*spine),
            "Path:",
        )
    {
        *spine = new;
        changed = true;
    }
    ui.horizontal_wrapped(|ui| {
        label_cell(ui, "Path edges");
        let shown = if !path_edges.is_empty() {
            format!("{} of the solid", path_edges.len())
        } else if !path_borrowed.is_empty() {
            format!("{} borrowed", path_borrowed.len())
        } else {
            "none: the path sketch".to_string()
        };
        mono_label(ui, shown, FONT_XS, TEXT1);
        let picked: Vec<EdgePick> = picked_edges(ctx).iter().map(EdgePick::of).collect();
        if ui
            .add_enabled_ui(!picked.is_empty(), |ui| {
                accent_outline_button(ui, "Use selected edges")
            })
            .inner
            .on_hover_text("Click the edges of the solid in turn (Ctrl adds), then press this")
            .clicked()
        {
            *path_edges = picked;
            path_borrowed.clear();
            changed = true;
        }
        if along_edges && small_secondary_button(ui, "Use the sketch").clicked() {
            path_edges.clear();
            path_borrowed.clear();
            changed = true;
        }
    });
    let lent = crate::borrow::edges_of_body(ctx.document, body);
    if !lent.is_empty() {
        for (r, name) in &lent {
            let mut on = path_borrowed.contains(r);
            if check_row(ui, &mut on, &format!("Along {name}")).changed() {
                if on {
                    path_borrowed.push(*r);
                    path_edges.clear();
                } else {
                    path_borrowed.retain(|b| b != r);
                }
                changed = true;
            }
        }
    }
    changed
}

/// A pipe's orientation: which kind, and the path or direction it takes.
fn pipe_orientation_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    spine: FeatureId,
    orientation: &mut PipeOrientation,
) -> bool {
    let mut changed = false;
    // A binormal starts square to the path's plane, where it holds.
    let normal = crate::build::sketch_normal(ctx.document, spine).unwrap_or([0.0, 0.0, 1.0]);
    let kinds = [
        PipeOrientation::Standard,
        PipeOrientation::Frenet,
        PipeOrientation::Auxiliary { path: spine },
        PipeOrientation::Binormal {
            x: normal[0],
            y: normal[1],
            z: normal[2],
        },
        PipeOrientation::Fixed,
    ];
    ui.horizontal(|ui| {
        label_cell(ui, "Orientation");
        egui::ComboBox::from_id_salt(("pipe_orientation", feature_id))
            .selected_text(orientation.label())
            .show_ui(ui, |ui| {
                for candidate in kinds {
                    let same =
                        std::mem::discriminant(orientation) == std::mem::discriminant(&candidate);
                    if ui.selectable_label(same, candidate.label()).clicked() && !same {
                        *orientation = candidate;
                        changed = true;
                    }
                }
            });
    })
    .response
    .on_hover_text(
        "Standard keeps the section from twisting; Frenet turns it with the \
         path's curvature; an auxiliary path turns it to face a second path; \
         a binormal holds one direction of it fixed; fixed keeps it as drawn",
    );
    match orientation {
        PipeOrientation::Auxiliary { path } => {
            if let Some(new) = sketch_combo(
                ui,
                ctx,
                body,
                ("pipe_auxiliary", feature_id),
                Some(*path),
                "Auxiliary path:",
            ) {
                *path = new;
                changed = true;
            }
        }
        PipeOrientation::Binormal { x, y, z } => {
            for (value, label) in [(x, "Binormal X:"), (y, "Binormal Y:"), (z, "Binormal Z:")] {
                changed |= number_drag(ui, fx, value, label);
            }
        }
        PipeOrientation::Standard | PipeOrientation::Frenet | PipeOrientation::Fixed => {}
    }
    changed
}

/// A plain number, a formula field where the feature has its parameter.
fn number_drag(ui: &mut Ui, fx: &mut Formulas, value: &mut f32, label: &str) -> bool {
    if let Some((changed, v)) = fx.show(ui, label, f64::from(*value)) {
        *value = v as f32;
        return changed;
    }
    field(ui, label, |ui| QtyField::new(value).speed(0.05).show(ui))
}

/// The datum lines of a body and the axes of the sketches before
/// `feature`, as pattern axis choices.
fn pattern_axis_choices(
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    feature: FeatureId,
) -> Vec<(PatternAxis, String)> {
    let mut choices = Vec::new();
    for (id, name, datum) in core_document::datums_of_body(ctx.document, body) {
        if matches!(datum.shape, core_document::DatumShape::Line { .. }) {
            choices.push((PatternAxis::Datum(id), name));
        }
    }
    for (sketch, name) in crate::build::sketch_choices(ctx.document, body, feature, None) {
        for axis in SketchAxis::ALL {
            choices.push((
                PatternAxis::Sketch { sketch, axis },
                format!("{name} {}", axis.label()),
            ));
        }
    }
    choices
}

/// A picked edge as a pattern axis: a circular edge gives its axis
/// (centre and normal), a straight one itself.
fn edge_axis(edge: &core_document::EdgeRef) -> EdgePick {
    match edge.circle {
        Some(circle) => EdgePick {
            faces: edge.faces,
            point: circle.center,
            direction: circle.normal,
        },
        None => EdgePick::of(edge),
    }
}

/// What a pattern runs along or turns about: the body's axes, a custom
/// axis, a datum line, a sketch's axis or a picked edge.
fn pattern_axis_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    (body, feature): (BodyId, FeatureId),
    axis: &mut PatternAxis,
    id_salt: impl egui::AsIdSalt,
) -> bool {
    let mut changed = false;
    let references = pattern_axis_choices(ctx, body, feature);
    let shown = references
        .iter()
        .find(|(choice, _)| choice == axis)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| axis.label().to_string());
    ui.horizontal(|ui| {
        label_cell(ui, "Axis");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                let picked_edge = picked_edges(ctx).first().map(edge_axis);
                let mut candidates: Vec<(PatternAxis, String)> = [
                    PatternAxis::X,
                    PatternAxis::Y,
                    PatternAxis::Z,
                    PatternAxis::Custom {
                        origin: [0.0; 3],
                        dir: [0.0, 0.0, 1.0],
                    },
                ]
                .into_iter()
                .map(|c| (c, c.label().to_string()))
                .collect();
                candidates.extend(references.iter().cloned());
                candidates.push((
                    PatternAxis::Edge(picked_edge.unwrap_or(EdgePick {
                        faces: [0, 0],
                        point: [0.0; 3],
                        direction: [0.0, 0.0, 1.0],
                    })),
                    "Picked edge".to_string(),
                ));
                for (candidate, name) in candidates {
                    let is_current = match (&*axis, &candidate) {
                        (PatternAxis::Datum(_), PatternAxis::Datum(_))
                        | (PatternAxis::Sketch { .. }, PatternAxis::Sketch { .. }) => {
                            *axis == candidate
                        }
                        _ => std::mem::discriminant(axis) == std::mem::discriminant(&candidate),
                    };
                    if ui.selectable_label(is_current, name).clicked() && !is_current {
                        *axis = candidate;
                        changed = true;
                    }
                }
            });
    });
    if let PatternAxis::Edge(edge) = axis {
        ui.horizontal_wrapped(|ui| {
            label_cell(ui, "Edge:");
            mono_label(
                ui,
                format!(
                    "({:.1}, {:.1}, {:.1})",
                    edge.point[0], edge.point[1], edge.point[2]
                ),
                FONT_XS,
                TEXT1,
            );
            if ui
                .add_enabled_ui(!ctx.selected_edges.is_empty(), |ui| {
                    accent_outline_button(ui, "Use selected edge")
                })
                .inner
                .on_hover_text(
                    "Click a straight edge in the viewport, or a round one for its axis, \
                     then press this",
                )
                .clicked()
                && let Some(picked) = picked_edges(ctx).first()
            {
                *edge = edge_axis(picked);
                changed = true;
            }
        });
    }
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

/// How an uneven pattern's gaps show: the switch that turns them on, the
/// formula name and label of each (numbered from 1), and their unit.
struct Gaps {
    toggle: &'static str,
    name: &'static str,
    label: &'static str,
    degrees: bool,
}

/// An uneven pattern's gaps, one per step from an occurrence to the next.
/// Turned on, the list starts at the even gap and follows the occurrence
/// count; turned off, it empties and the pattern spaces evenly.
fn gaps_editor(
    ui: &mut Ui,
    fx: &mut Formulas,
    gaps: &mut Vec<f32>,
    (occurrences, even): (u32, f32),
    how: Gaps,
) -> bool {
    let mut changed = false;
    let mut uneven = !gaps.is_empty();
    if check_row(ui, &mut uneven, how.toggle)
        .on_hover_text("Set the gap from each occurrence to the next on its own")
        .changed()
    {
        gaps.clear();
        changed = true;
    }
    if !uneven {
        return changed;
    }
    let wanted = occurrences.saturating_sub(1).max(1) as usize;
    if gaps.len() != wanted {
        gaps.resize(wanted, even);
        changed = true;
    }
    for (i, gap) in gaps.iter_mut().enumerate() {
        let label = format!("{} {}:", how.label, i + 1);
        let name = format!("{}{}", how.name, i + 1);
        let mut value = f64::from(*gap);
        changed |= named_f64(ui, fx, &mut value, (&label, &name), |ui, v| {
            let mut shown = *v as f32;
            let edited = field(ui, &label, |ui| {
                if how.degrees {
                    QtyField::degrees(&mut shown).speed(1.0).show(ui)
                } else {
                    QtyField::mm(&mut shown).speed(0.5).show(ui)
                }
            });
            *v = f64::from(shown);
            edited
        });
        *gap = value as f32;
    }
    changed
}

/// The datum planes (a coordinate system by each of its planes) and the
/// sketches (each by its plane and its two axes) of the edited feature's
/// body, as mirror planes.
fn mirror_references(ctx: &WorkbenchRuntimeContext) -> Vec<(MirrorPlane, String)> {
    use crate::feature::SketchAxis;
    use core_document::{BasePlane, DatumShape};
    let Some(body) = edited_body(ctx) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (datum, name, made) in core_document::datums_of_body(ctx.document, body) {
        match made.shape {
            DatumShape::Plane { .. } => out.push((MirrorPlane::Datum { datum, plane: None }, name)),
            DatumShape::CoordinateSystem { .. } => {
                for (plane, key) in [
                    (BasePlane::XY, "XY"),
                    (BasePlane::XZ, "XZ"),
                    (BasePlane::YZ, "YZ"),
                ] {
                    out.push((
                        MirrorPlane::Datum {
                            datum,
                            plane: Some(plane),
                        },
                        format!("{name} {key}"),
                    ));
                }
            }
            _ => {}
        }
    }
    let mut sketches: Vec<(u64, FeatureId, String)> = ctx
        .document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "wb.sketch" && n.body == Some(body))
        .map(|(id, n)| (n.seq, *id, n.name.clone()))
        .collect();
    sketches.sort_by_key(|(seq, ..)| *seq);
    for (_, sketch, name) in sketches {
        for (axis, what) in [
            (None, "plane"),
            (Some(SketchAxis::Horizontal), "H axis"),
            (Some(SketchAxis::Vertical), "V axis"),
        ] {
            out.push((
                MirrorPlane::Sketch { sketch, axis },
                format!("{name} {what}"),
            ));
        }
    }
    out
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
        let shown = mirror_references(ctx)
            .into_iter()
            .find(|(c, _)| c == plane)
            .map(|(_, name)| name)
            .unwrap_or_else(|| plane.label().to_string());
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(shown)
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
                    *plane = MirrorPlane::Face(FacePick::of(face));
                    changed = true;
                }
                for (candidate, name) in mirror_references(ctx) {
                    if ui.selectable_label(*plane == candidate, name).clicked()
                        && *plane != candidate
                    {
                        *plane = candidate;
                        changed = true;
                    }
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

/// A draft's neutral plane (a picked face, one of the body's planes or a
/// datum plane) and what it pulls along (the neutral plane's normal, a
/// picked edge or a datum line).
fn draft_references_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    feature_id: FeatureId,
    neutral: &mut FacePick,
    neutral_plane: &mut Option<crate::feature::PlaneTarget>,
    pull: &mut Option<crate::feature::PullRef>,
) -> bool {
    use crate::feature::{PlaneTarget, PullRef};
    let mut changed = false;
    let planes: Vec<(PlaneTarget, String)> = end_targets(ctx, body)
        .into_iter()
        .filter_map(|(mode, name)| match mode {
            ExtrudeMode::UpToPlane(target) => {
                Some((target, name.trim_start_matches("Up to ").to_string()))
            }
            _ => None,
        })
        .collect();
    let shown = match neutral_plane {
        None => "Picked face".to_string(),
        Some(target) => planes
            .iter()
            .find(|(t, _)| t == target)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "(gone)".into()),
    };
    ui.horizontal(|ui| {
        label_cell(ui, "Neutral plane");
        egui::ComboBox::from_id_salt(("draft_neutral", feature_id))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(neutral_plane.is_none(), "Picked face")
                    .clicked()
                    && neutral_plane.is_some()
                {
                    *neutral_plane = None;
                    changed = true;
                }
                for (target, name) in &planes {
                    if ui
                        .selectable_label(*neutral_plane == Some(*target), name)
                        .clicked()
                        && *neutral_plane != Some(*target)
                    {
                        *neutral_plane = Some(*target);
                        changed = true;
                    }
                }
            });
    });
    if neutral_plane.is_none() {
        let mut neutral_opt = Some(*neutral);
        if face_pick_row(ui, ctx, &mut neutral_opt, "Neutral face:")
            && let Some(pick) = neutral_opt
        {
            *neutral = pick;
            changed = true;
        }
    }
    let lines: Vec<(FeatureId, String)> = core_document::datums_of_body(ctx.document, body)
        .into_iter()
        .filter(|(_, _, d)| matches!(d.shape, core_document::DatumShape::Line { .. }))
        .map(|(id, name, _)| (id, name))
        .collect();
    let shown = match pull {
        None => "Neutral plane's normal".to_string(),
        Some(PullRef::Edge(_)) => "Picked edge".to_string(),
        Some(PullRef::Datum(id)) => lines
            .iter()
            .find(|(l, _)| l == id)
            .map(|(_, n)| n.clone())
            .unwrap_or_else(|| "(gone)".into()),
    };
    ui.horizontal(|ui| {
        label_cell(ui, "Pull");
        egui::ComboBox::from_id_salt(("draft_pull", feature_id))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(pull.is_none(), "Neutral plane's normal")
                    .clicked()
                    && pull.is_some()
                {
                    *pull = None;
                    changed = true;
                }
                let picked = picked_edges(ctx).first().map(EdgePick::of);
                let is_edge = matches!(pull, Some(PullRef::Edge(_)));
                if ui
                    .add_enabled(
                        picked.is_some() || is_edge,
                        egui::Button::selectable(is_edge, "Picked edge"),
                    )
                    .on_disabled_hover_text("Click a straight edge first")
                    .clicked()
                    && let Some(edge) = picked
                {
                    *pull = Some(PullRef::Edge(edge));
                    changed = true;
                }
                for (id, name) in &lines {
                    let current = *pull == Some(PullRef::Datum(*id));
                    if ui.selectable_label(current, name).clicked() && !current {
                        *pull = Some(PullRef::Datum(*id));
                        changed = true;
                    }
                }
            });
    });
    changed
}

/// The body's Design features before this one, modifiers left out,
/// selectable as pattern originals.
fn originals_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    this_feature: FeatureId,
    originals: &mut Vec<FeatureId>,
) -> bool {
    let mut changed = false;
    label_cell(ui, "Originals (empty = whole body)");
    let features = design_features_of_body(ctx.document, body);
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

fn primitive_editor(ui: &mut Ui, fx: &mut Formulas, kind: &mut kernel_api::PrimitiveKind) -> bool {
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
                angle1_deg: -90.0,
                angle2_deg: 90.0,
                angle3_deg: 360.0,
            },
        ),
        (
            "Prism",
            P::Prism {
                sides: 6,
                circumradius: 5.0,
                height: 10.0,
                skew_x_deg: 0.0,
                skew_y_deg: 0.0,
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

    // Each number is the parameter formulas call by its field's name (an
    // angle's without `_deg`), a formula field where the feature has it.
    let name_of = |label: &str| -> String {
        label
            .trim_end_matches(':')
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_lowercase()
    };
    let mut num = |ui: &mut Ui, value: &mut f64, label: &str, angular: bool, lo: f64, hi: f64| {
        let changed = named_f64(ui, fx, value, (label, &name_of(label)), |ui, value| {
            ui.horizontal(|ui| {
                label_cell(ui, label);
                ui.add(
                    egui::DragValue::new(value)
                        .speed(if angular { 1.0 } else { 0.5 })
                        .range(lo..=hi)
                        .suffix(if angular { "°" } else { " mm" }),
                )
                .changed()
            })
            .inner
        });
        *value = value.clamp(lo, hi);
        changed
    };

    match kind {
        P::Box {
            length,
            width,
            height,
        } => {
            changed |= num(ui, length, "Length:", false, 0.01, 1.0e6);
            changed |= num(ui, width, "Width:", false, 0.01, 1.0e6);
            changed |= num(ui, height, "Height:", false, 0.01, 1.0e6);
        }
        P::Cylinder {
            radius,
            height,
            angle_deg,
        } => {
            changed |= num(ui, radius, "Radius:", false, 0.01, 1.0e6);
            changed |= num(ui, height, "Height:", false, 0.01, 1.0e6);
            changed |= num(ui, angle_deg, "Angle:", true, 1.0, 360.0);
        }
        P::Sphere {
            radius,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= num(ui, radius, "Radius:", false, 0.01, 1.0e6);
            changed |= num(ui, angle1_deg, "Angle 1:", true, -90.0, 90.0);
            changed |= num(ui, angle2_deg, "Angle 2:", true, -90.0, 90.0);
            changed |= num(ui, angle3_deg, "Angle 3:", true, 1.0, 360.0);
        }
        P::Cone {
            radius1,
            radius2,
            height,
            angle_deg,
        } => {
            changed |= num(ui, radius1, "Radius 1:", false, 0.0, 1.0e6);
            changed |= num(ui, radius2, "Radius 2:", false, 0.0, 1.0e6);
            changed |= num(ui, height, "Height:", false, 0.01, 1.0e6);
            changed |= num(ui, angle_deg, "Angle:", true, 1.0, 360.0);
        }
        P::Torus {
            radius1,
            radius2,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= num(ui, radius1, "Radius 1:", false, 0.01, 1.0e6);
            changed |= num(ui, radius2, "Radius 2:", false, 0.01, 1.0e6);
            changed |= num(ui, angle1_deg, "Angle 1:", true, -180.0, 180.0);
            changed |= num(ui, angle2_deg, "Angle 2:", true, -180.0, 180.0);
            changed |= num(ui, angle3_deg, "Angle 3:", true, 1.0, 360.0);
        }
        P::Ellipsoid {
            radius1,
            radius2,
            radius3,
            angle1_deg,
            angle2_deg,
            angle3_deg,
        } => {
            changed |= num(ui, radius1, "Radius 1:", false, 0.01, 1.0e6);
            changed |= num(ui, radius2, "Radius 2:", false, 0.01, 1.0e6);
            changed |= num(ui, radius3, "Radius 3:", false, 0.01, 1.0e6);
            changed |= num(ui, angle1_deg, "Angle 1:", true, -90.0, 90.0);
            changed |= num(ui, angle2_deg, "Angle 2:", true, -90.0, 90.0);
            changed |= num(ui, angle3_deg, "Angle 3:", true, 1.0, 360.0);
        }
        P::Prism {
            sides,
            circumradius,
            height,
            skew_x_deg,
            skew_y_deg,
        } => {
            changed |= num(ui, skew_x_deg, "Skew X:", true, -85.0, 85.0);
            changed |= num(ui, skew_y_deg, "Skew Y:", true, -85.0, 85.0);
            ui.horizontal(|ui| {
                label_cell(ui, "Sides");
                changed |= ui
                    .add(egui::DragValue::new(sides).speed(0.1).range(3..=64))
                    .changed();
            });
            changed |= num(ui, circumradius, "Circumradius:", false, 0.01, 1.0e6);
            changed |= num(ui, height, "Height:", false, 0.01, 1.0e6);
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
                changed |= num(ui, value, label, false, -1.0e6, 1.0e6);
            }
        }
    }
    changed
}

fn placement_editor(ui: &mut Ui, fx: &mut Formulas, placement: &mut kernel_api::Placement) -> bool {
    let mut changed = false;
    for (value, (label, name)) in placement.origin.iter_mut().zip([
        ("Position X", "x"),
        ("Position Y", "y"),
        ("Position Z", "z"),
    ]) {
        changed |= named_f64(ui, fx, value, (label, name), |ui, value| {
            ui.horizontal(|ui| {
                label_cell(ui, label);
                ui.add(egui::DragValue::new(value).speed(0.5).suffix(" mm"))
                    .changed()
            })
            .inner
        });
    }
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
    use core_document::DatumShape;
    let mut changed = false;

    match &mut datum.shape {
        DatumShape::Plane { size } => changed |= mm_drag(ui, fx, size, "Display size:"),
        DatumShape::Line { length } => changed |= mm_drag(ui, fx, length, "Display length:"),
        DatumShape::CoordinateSystem { size } => changed |= mm_drag(ui, fx, size, "Display size:"),
        DatumShape::Point => {}
    }

    changed |= crate::datum_panel::attachment_editor(ui, ctx, feature_id, datum);

    // One row each: side by side they are wider than the panel.
    let [x, y, n] = &mut datum.offset.translation;
    changed |= offset_drag(ui, fx, x, "Offset X:");
    changed |= offset_drag(ui, fx, y, "Offset Y:");
    changed |= offset_drag(ui, fx, n, "Normal offset:");
    changed |= deg_drag(
        ui,
        fx,
        &mut datum.offset.rotation_deg,
        "Rotation:",
        -180.0..=180.0,
    );
    let [about_x, about_y] = &mut datum.offset.tilt;
    changed |= deg_drag(ui, fx, about_x, "Tilt about X:", -180.0..=180.0);
    changed |= deg_drag(ui, fx, about_y, "Tilt about Y:", -180.0..=180.0);
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
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    // No feature before this one: nothing yet for a mode that needs
    // material to cut through or stop at.
    let first_feature = crate::build::design_features_of_body(ctx.document, body)
        .iter()
        .take_while(|(id, _)| *id != feature_id)
        .next()
        .is_none();
    match feature {
        DesignFeature::Base {} => {
            let from = ctx.document.base_geometry(body);
            let health = from.and_then(|g| g.health.as_ref());
            taper_note(
                ui,
                "The shape this body starts from, as it was imported or converted. \
                 The features after it change it.",
            );
            if let Some(health) = health.filter(|h| h.is_broken()) {
                taper_note(
                    ui,
                    &format!(
                        "The checker found {} problem(s) in it; Repair shape mends it.",
                        health.broken
                    ),
                );
            }
        }
        DesignFeature::Pad {
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            length,
            reversed,
            symmetric,
            mode,
            length2,
            taper_deg,
            up_to_face,
            up_to_offset,
            profile_face,
            profile_borrowed: _,
            direction,
            up_to_shape,
            mode2,
            up_to_face2,
            up_to_offset2,
            up_to_shape2,
            extras,
        } => {
            let borrowed = end_targets(ctx, body);
            changed |= face_profile_row(ui, ctx, body, profile_face);
            if let (Some(sketch), None) = (*sketch, profile_face.as_ref()) {
                changed |= regions_rows(ui, ctx, sketch, regions);
            }
            changed |=
                extrude_mode_combo(ui, ("pad_mode", feature_id), mode, first_feature, &borrowed);
            changed |= extrude_side_rows(
                ui,
                ctx,
                fx,
                *mode,
                (length, "Length:"),
                up_to_face,
                (up_to_offset, "Offset:"),
                up_to_shape,
                (body, &mut extras.up_to_shape_borrowed),
            );
            if *mode == ExtrudeMode::Dimension && mode2.is_none() {
                changed |= check_row(ui, symmetric, "Symmetric to plane").changed();
            }
            if !(*mode == ExtrudeMode::Dimension && *symmetric) {
                changed |= second_side_combo(
                    ui,
                    ("pad_mode2", feature_id),
                    *mode,
                    mode2,
                    first_feature,
                    &borrowed,
                );
                if let (_, Some(second)) = mode.sides(*mode2) {
                    changed |= extrude_side_rows(
                        ui,
                        ctx,
                        fx,
                        second,
                        (length2, "Second length:"),
                        up_to_face2,
                        (up_to_offset2, "Second offset:"),
                        up_to_shape2,
                        (body, &mut extras.up_to_shape_borrowed2),
                    );
                }
            }
            changed |=
                extrude_direction_editor(ui, ctx, fx, body, direction, ("pad_dir", feature_id));
            changed |= check_row(ui, reversed, "Reversed").changed();
            changed |= deg_drag(ui, fx, taper_deg, "Taper:", -85.0..=85.0);
            taper_note(ui, "Positive opens the pad out as it rises.");
            let two_sided = mode.sides(*mode2).1.is_some();
            changed |= extrude_extras_rows(ui, fx, extras, two_sided, *direction);
        }
        DesignFeature::Pocket {
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            depth,
            reversed,
            symmetric,
            through_all,
            mode,
            depth2,
            taper_deg,
            up_to_face,
            up_to_offset,
            profile_face,
            profile_borrowed: _,
            direction,
            up_to_shape,
            mode2,
            up_to_face2,
            up_to_offset2,
            up_to_shape2,
            extras,
        } => {
            // The flag and the ThroughAll mode are one setting: a file that
            // has only the flag set opens in that mode, and the flag
            // follows the mode picked.
            if *through_all && *mode != ExtrudeMode::ThroughAll {
                *mode = ExtrudeMode::ThroughAll;
                changed = true;
            }
            changed |= face_profile_row(ui, ctx, body, profile_face);
            if let (Some(sketch), None) = (*sketch, profile_face.as_ref()) {
                changed |= regions_rows(ui, ctx, sketch, regions);
            }
            let borrowed = end_targets(ctx, body);
            changed |= extrude_mode_combo(
                ui,
                ("pocket_mode", feature_id),
                mode,
                first_feature,
                &borrowed,
            );
            *through_all = *mode == ExtrudeMode::ThroughAll;
            changed |= extrude_side_rows(
                ui,
                ctx,
                fx,
                *mode,
                (depth, "Depth:"),
                up_to_face,
                (up_to_offset, "Offset:"),
                up_to_shape,
                (body, &mut extras.up_to_shape_borrowed),
            );
            if *mode == ExtrudeMode::Dimension && mode2.is_none() {
                changed |= check_row(ui, symmetric, "Symmetric to plane").changed();
            }
            if !(*mode == ExtrudeMode::Dimension && *symmetric) {
                changed |= second_side_combo(
                    ui,
                    ("pocket_mode2", feature_id),
                    *mode,
                    mode2,
                    first_feature,
                    &borrowed,
                );
                if let (_, Some(second)) = mode.sides(*mode2) {
                    changed |= extrude_side_rows(
                        ui,
                        ctx,
                        fx,
                        second,
                        (depth2, "Second depth:"),
                        up_to_face2,
                        (up_to_offset2, "Second offset:"),
                        up_to_shape2,
                        (body, &mut extras.up_to_shape_borrowed2),
                    );
                }
            }
            changed |=
                extrude_direction_editor(ui, ctx, fx, body, direction, ("pocket_dir", feature_id));
            changed |= check_row(ui, reversed, "Reversed")
                .on_hover_text("Cut along the sketch normal instead of against it")
                .changed();
            changed |= deg_drag(ui, fx, taper_deg, "Taper:", -85.0..=85.0);
            taper_note(ui, "Positive widens the pocket as it goes deeper.");
            let two_sided = mode.sides(*mode2).1.is_some();
            changed |= extrude_extras_rows(ui, fx, extras, two_sided, *direction);
        }
        DesignFeature::Revolution {
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            angle_deg,
            axis,
            project_axis,
            reversed,
            midplane,
            second_angle_deg,
            mode,
            up_to_face,
        }
        | DesignFeature::Groove {
            refine: _,
            boolean_tolerance: _,
            regions,
            sketch,
            angle_deg,
            axis,
            project_axis,
            reversed,
            midplane,
            second_angle_deg,
            mode,
            up_to_face,
        } => {
            changed |= regions_rows(ui, ctx, *sketch, regions);
            // Each face the body borrows is a target of its own.
            let targets: Vec<(RevolveMode, String)> =
                crate::borrow::faces_of_body(ctx.document, body)
                    .into_iter()
                    .map(|(r, name)| (RevolveMode::UpToBorrowed(r), format!("Up to {name}")))
                    .collect();
            let shown = targets
                .iter()
                .find(|(t, _)| t == mode)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| mode.label().to_string());
            ui.horizontal(|ui| {
                label_cell(ui, "Type");
                egui::ComboBox::from_id_salt(("rev_mode", feature_id))
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        let choices = RevolveMode::ALL
                            .iter()
                            .map(|m| (*m, m.label().to_string()))
                            .chain(targets.iter().cloned());
                        for (candidate, name) in choices {
                            // Every mode but an angle or a borrowed face
                            // stops on the body's own material.
                            if first_feature
                                && !matches!(
                                    candidate,
                                    RevolveMode::Angle | RevolveMode::UpToBorrowed(_)
                                )
                            {
                                continue;
                            }
                            if ui.selectable_label(*mode == candidate, name).clicked()
                                && *mode != candidate
                            {
                                *mode = candidate;
                                changed = true;
                            }
                        }
                    });
            });
            match mode {
                RevolveMode::Angle => {
                    changed |= deg_drag(ui, fx, angle_deg, "Angle:", 0.1..=360.0);
                }
                RevolveMode::UpToFace => {
                    changed |= face_pick_row(ui, ctx, up_to_face, "Target face:");
                }
                RevolveMode::ToFirst | RevolveMode::ToLast | RevolveMode::UpToBorrowed(_) => {}
            }
            changed |= revolve_axis_editor(
                ui,
                ctx,
                body,
                *sketch,
                axis,
                ("rev_axis", feature_id),
                false,
            );
            changed |= project_axis_row(ui, axis, project_axis);
            if *mode == RevolveMode::Angle {
                changed |= check_row(ui, midplane, "Midplane").changed();
                let mut two_sided = second_angle_deg.is_some();
                if check_row(ui, &mut two_sided, "Second angle").changed() {
                    *second_angle_deg = two_sided.then_some(90.0);
                    changed = true;
                }
                if let Some(second) = second_angle_deg {
                    changed |= deg_drag(ui, fx, second, "Angle 2:", 0.1..=360.0);
                }
            }
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        DesignFeature::Loft {
            refine: _,
            boolean_tolerance: _,
            sections,
            ruled,
            closed,
            subtractive,
        } => {
            use crate::feature::LoftSection;
            label_cell(ui, "Sections (in order)");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = match section {
                    LoftSection::Feature(id) => ctx
                        .document
                        .get_feature_meta(*id)
                        .map(|n| n.name.clone())
                        .unwrap_or_else(|| "(missing)".into()),
                    LoftSection::Face(pick) => format!(
                        "face at ({:.1}, {:.1}, {:.1})",
                        pick.point[0], pick.point[1], pick.point[2]
                    ),
                };
                ui.horizontal(|ui| {
                    mono_label(ui, format!("{}. {name}", i + 1), FONT_XS, TEXT1);
                    if small_secondary_button(ui, "×").clicked() && sections.len() > 1 {
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
            ) && !sections.contains(&LoftSection::Feature(new))
            {
                sections.push(LoftSection::Feature(new));
                changed = true;
            }
            // A datum point closes the loft to it, at either end.
            let points: Vec<(FeatureId, String)> =
                core_document::datums_of_body(ctx.document, body)
                    .into_iter()
                    .filter(|(_, _, d)| matches!(d.shape, core_document::DatumShape::Point))
                    .map(|(id, name, _)| (id, name))
                    .collect();
            if !points.is_empty() {
                ui.horizontal(|ui| {
                    label_cell(ui, "Add point:");
                    egui::ComboBox::from_id_salt(("loft_point", feature_id))
                        .selected_text("A datum point…")
                        .show_ui(ui, |ui| {
                            for (id, name) in &points {
                                let section = LoftSection::Feature(*id);
                                if ui.selectable_label(false, name).clicked()
                                    && !sections.contains(&section)
                                {
                                    sections.push(section);
                                    changed = true;
                                }
                            }
                        });
                });
            }
            let picked = ctx.selected_face_in(body).map(FacePick::of);
            ui.horizontal(|ui| {
                label_cell(ui, "Add face:");
                if ui
                    .add_enabled_ui(picked.is_some(), |ui| {
                        accent_outline_button(ui, "Use selected face")
                    })
                    .inner
                    .on_hover_text("Click a flat face of the solid first, then press this")
                    .clicked()
                    && let Some(pick) = picked
                {
                    sections.push(LoftSection::Face(pick));
                    changed = true;
                }
            });
            changed |= check_row(ui, ruled, "Ruled (straight transitions)").changed();
            changed |= check_row(ui, closed, "Closed (loop back)").changed();
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        DesignFeature::Pipe {
            refine: _,
            boolean_tolerance: _,
            profile,
            spine,
            orientation,
            corner,
            sections,
            subtractive,
            profile_face,
            path_edges,
            path_borrowed,
        } => {
            changed |= pipe_inputs_editor(
                ui,
                ctx,
                (body, feature_id),
                spine,
                profile_face,
                path_edges,
                path_borrowed,
            );
            changed |= pipe_orientation_editor(ui, ctx, fx, body, feature_id, *spine, orientation);
            ui.horizontal(|ui| {
                label_cell(ui, "Corners");
                egui::ComboBox::from_id_salt(("pipe_corner", feature_id))
                    .selected_text(corner.label())
                    .show_ui(ui, |ui| {
                        for candidate in PipeCorner::ALL {
                            if ui
                                .selectable_label(*corner == candidate, candidate.label())
                                .clicked()
                                && *corner != candidate
                            {
                                *corner = candidate;
                                changed = true;
                            }
                        }
                    });
            });
            label_cell(ui, "Sections along the path (in order)");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = ctx
                    .document
                    .get_feature_meta(*section)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| "(missing)".into());
                ui.horizontal(|ui| {
                    mono_label(ui, format!("{}. {name}", i + 1), FONT_XS, TEXT1);
                    if small_secondary_button(ui, "×").clicked() {
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
                ("pipe_section_add", feature_id),
                None,
                "Add section:",
            ) && new != *profile
                && new != *spine
                && !sections.contains(&new)
            {
                sections.push(new);
                changed = true;
            }
            // A datum point last closes the pipe to it.
            let points: Vec<(FeatureId, String)> =
                core_document::datums_of_body(ctx.document, body)
                    .into_iter()
                    .filter(|(_, _, d)| matches!(d.shape, core_document::DatumShape::Point))
                    .map(|(id, name, _)| (id, name))
                    .collect();
            if !points.is_empty() {
                ui.horizontal(|ui| {
                    label_cell(ui, "End at point:");
                    egui::ComboBox::from_id_salt(("pipe_point", feature_id))
                        .selected_text("A datum point…")
                        .show_ui(ui, |ui| {
                            for (id, name) in &points {
                                if ui.selectable_label(false, name).clicked()
                                    && !sections.contains(id)
                                {
                                    sections.push(*id);
                                    changed = true;
                                }
                            }
                        });
                });
            }
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        DesignFeature::Helix {
            refine: _,
            boolean_tolerance: _,
            sketch,
            axis,
            project_axis,
            mode,
            pitch,
            height,
            turns,
            left_handed,
            cone_angle_deg,
            reversed,
            subtractive,
            growth,
            keep_inside,
        } => {
            changed |= revolve_axis_editor(
                ui,
                ctx,
                body,
                *sketch,
                axis,
                ("helix_axis", feature_id),
                true,
            );
            changed |= project_axis_row(ui, axis, project_axis);
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
                    changed |= turns_field(ui, fx, turns);
                }
                HelixMode::HeightTurns => {
                    changed |= mm_drag(ui, fx, height, "Height:");
                    changed |= turns_field(ui, fx, turns);
                }
                HelixMode::HeightTurnsGrowth => {
                    // A height of 0 is a flat spiral; below it is nothing.
                    if offset_drag(ui, fx, height, "Height:") {
                        *height = height.max(0.0);
                        changed = true;
                    }
                    changed |= turns_field(ui, fx, turns);
                    changed |= offset_drag(ui, fx, growth, "Growth:");
                }
            }
            if *mode != HelixMode::HeightTurnsGrowth {
                changed |= deg_drag(ui, fx, cone_angle_deg, "Cone angle:", -85.0..=85.0);
            }
            changed |= check_row(ui, left_handed, "Left handed").changed();
            changed |= check_row(ui, reversed, "Reversed").changed();
            changed |= check_row(ui, subtractive, "Subtractive").changed();
            if *subtractive {
                changed |= check_row(ui, keep_inside, "Keep inside")
                    .on_hover_text("Keep what the helix shares with the body instead of cutting it")
                    .changed();
            }
        }
        DesignFeature::Primitive {
            refine: _,
            boolean_tolerance: _,
            kind,
            placement,
            subtractive,
            attached,
        } => {
            changed |= primitive_editor(ui, fx, kind);
            let mut attach = attached.is_some();
            if check_row(ui, &mut attach, "Attached")
                .on_hover_text(
                    "Placed by a mode on what is picked, as a datum is, and following it",
                )
                .changed()
            {
                *attached = attach.then(|| {
                    Box::new(crate::feature::Attached {
                        attachment: core_document::DatumAttachment::BasePlane(
                            core_document::BasePlane::XY,
                        ),
                        offset: core_document::AttachmentOffset::default(),
                    })
                });
                changed = true;
            }
            match attached {
                Some(a) => {
                    let mut datum = a.datum();
                    if crate::datum_panel::attachment_editor(ui, ctx, feature_id, &mut datum) {
                        **a = crate::feature::Attached::from_datum(&datum);
                        changed = true;
                    }
                    let [x, y, n] = &mut a.offset.translation;
                    changed |= offset_drag(ui, fx, x, "Offset X:");
                    changed |= offset_drag(ui, fx, y, "Offset Y:");
                    changed |= offset_drag(ui, fx, n, "Normal offset:");
                    changed |= deg_drag(
                        ui,
                        fx,
                        &mut a.offset.rotation_deg,
                        "Rotation:",
                        -180.0..=180.0,
                    );
                    changed |= check_row(ui, &mut a.offset.flip, "Flip side").changed();
                }
                None => changed |= placement_editor(ui, fx, placement),
            }
            changed |= check_row(ui, subtractive, "Subtractive").changed();
        }
        DesignFeature::Hole { .. } => {
            changed |= hole::hole_editor(ui, ctx, fx, body, feature_id, feature);
        }
        DesignFeature::Fillet {
            radius,
            edges,
            follow_tangent,
        } => {
            changed |= mm_drag(ui, fx, radius, "Radius:");
            changed |= edge_sel_editor(ui, ctx, edges, ("fillet_edges", feature_id));
            changed |= tangent_row(ui, follow_tangent);
        }
        DesignFeature::Chamfer {
            size,
            mode,
            size2,
            angle_deg,
            flip,
            edges,
            follow_tangent,
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
            changed |= tangent_row(ui, follow_tangent);
        }
        DesignFeature::Draft {
            angle_deg,
            neutral,
            faces,
            reversed,
            neutral_plane,
            pull,
        } => {
            changed |= deg_drag(ui, fx, angle_deg, "Angle:", 0.1..=45.0);
            changed |=
                draft_references_editor(ui, ctx, body, feature_id, neutral, neutral_plane, pull);
            changed |= face_list_editor(ui, ctx, faces, "Faces to draft:");
            changed |= check_row(ui, reversed, "Reversed pull").changed();
        }
        DesignFeature::OffsetFaces { faces, distance } => {
            changed |= face_list_editor(ui, ctx, faces, "Faces to offset:");
            changed |= signed_mm(ui, fx, distance, "Distance", "distance");
            taper_note(
                ui,
                "Along each face's outward normal; negative moves it into the material. A \
                 bore's face offset in widens it.",
            );
        }
        DesignFeature::MoveFaces {
            faces,
            translation,
            angle_deg,
            axis_point,
            axis_dir,
        } => {
            changed |= face_list_editor(ui, ctx, faces, "Faces to move:");
            for (i, (label, name)) in [("Move X", "x"), ("Move Y", "y"), ("Move Z", "z")]
                .into_iter()
                .enumerate()
            {
                changed |= signed_mm(ui, fx, &mut translation[i], label, name);
            }
            changed |= deg_drag(ui, fx, angle_deg, "Angle", -360.0..=360.0);
            if angle_deg.abs() > 1e-6 {
                for (i, label) in ["Axis point X", "Axis point Y", "Axis point Z"]
                    .into_iter()
                    .enumerate()
                {
                    changed |= field(ui, label, |ui| {
                        QtyField::offset(&mut axis_point[i]).show(ui)
                    });
                }
                for (i, label) in ["Axis X", "Axis Y", "Axis Z"].into_iter().enumerate() {
                    changed |= field(ui, label, |ui| {
                        QtyField::new(&mut axis_dir[i]).speed(0.05).show(ui)
                    });
                }
            }
        }
        DesignFeature::DeleteFaces { faces } => {
            changed |= face_list_editor(ui, ctx, faces, "Faces to delete:");
            taper_note(
                ui,
                "The faces around each opening grow to close it: a bore, a boss or a round \
                 taken away.",
            );
        }
        DesignFeature::Thickness {
            value,
            faces,
            inward,
            join,
            both_sides,
        } => {
            changed |= mm_drag(ui, fx, value, "Thickness:");
            changed |= face_list_editor(ui, ctx, faces, "Faces to open:");
            changed |= check_row(ui, both_sides, "Both sides")
                .on_hover_text("Walls on both sides of the faces, the thickness each way")
                .changed();
            if !*both_sides {
                changed |= check_row(ui, inward, "Inward").changed();
            }
            ui.horizontal(|ui| {
                label_cell(ui, "Join");
                egui::ComboBox::from_id_salt(("thickness_join", feature_id))
                    .selected_text(join.label())
                    .show_ui(ui, |ui| {
                        for candidate in kernel_api::ThicknessJoin::ALL {
                            if ui
                                .selectable_label(*join == candidate, candidate.label())
                                .clicked()
                                && *join != candidate
                            {
                                *join = candidate;
                                changed = true;
                            }
                        }
                    });
            });
        }
        DesignFeature::Mirrored {
            originals,
            plane,
            refine: _,
            boolean_tolerance: _,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= mirror_plane_editor(ui, ctx, plane, ("mirror_plane", feature_id));
        }
        DesignFeature::LinearPattern {
            refine: _,
            boolean_tolerance: _,
            originals,
            axis,
            length,
            occurrences,
            spacing_mode,
            reversed,
            spacings,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(
                ui,
                ctx,
                (body, feature_id),
                axis,
                ("linear_axis", feature_id),
            );
            changed |= count_drag(ui, fx, occurrences, "Occurrences:");
            changed |= check_row(ui, spacing_mode, "Length is spacing")
                .on_hover_text("Off: length is the overall span")
                .changed();
            changed |= mm_drag(ui, fx, length, "Length:");
            let even = if *spacing_mode || *occurrences < 2 {
                *length
            } else {
                *length / (*occurrences - 1) as f32
            };
            changed |= gaps_editor(
                ui,
                fx,
                spacings,
                (*occurrences, even),
                Gaps {
                    toggle: "Uneven spacing",
                    name: "spacing",
                    label: "Spacing",
                    degrees: false,
                },
            );
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        DesignFeature::PolarPattern {
            refine: _,
            boolean_tolerance: _,
            originals,
            axis,
            angle_deg,
            occurrences,
            reversed,
            step_mode,
            angles,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(
                ui,
                ctx,
                (body, feature_id),
                axis,
                ("polar_axis", feature_id),
            );
            changed |= count_drag(ui, fx, occurrences, "Occurrences:");
            changed |= check_row(ui, step_mode, "Angle is the step")
                .on_hover_text(
                    "On: the angle between one occurrence and the next. \
                     Off: the angle all of them span",
                )
                .changed();
            changed |= deg_drag(ui, fx, angle_deg, "Angle:", 1.0..=360.0);
            let even = if *step_mode || *occurrences < 2 {
                *angle_deg
            } else if (*angle_deg - 360.0).abs() < 1e-6 {
                *angle_deg / *occurrences as f32
            } else {
                *angle_deg / (*occurrences - 1) as f32
            };
            changed |= gaps_editor(
                ui,
                fx,
                angles,
                (*occurrences, even),
                Gaps {
                    toggle: "Uneven steps",
                    name: "step_angle",
                    label: "Step angle",
                    degrees: true,
                },
            );
            changed |= check_row(ui, reversed, "Reversed").changed();
        }
        DesignFeature::MultiTransform {
            originals,
            steps,
            refine: _,
            boolean_tolerance: _,
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
                    if small_secondary_button(ui, "×").clicked() {
                        remove = Some(i);
                    }
                });
                // This step's numbers are `step{n}_…` to formulas.
                fx.prefix = Some(format!("step{}_", i + 1));
                match step {
                    TransformStep::Linear {
                        axis,
                        length,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(
                            ui,
                            ctx,
                            (body, feature_id),
                            axis,
                            ("mt_lin", feature_id, i),
                        );
                        changed |= mm_drag(ui, fx, length, "Length:");
                        changed |= count_drag(ui, fx, occurrences, "Occurrences:");
                    }
                    TransformStep::Polar {
                        axis,
                        angle_deg,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(
                            ui,
                            ctx,
                            (body, feature_id),
                            axis,
                            ("mt_pol", feature_id, i),
                        );
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
                        if let Some((edited, v)) = fx.show(ui, "Factor:", f64::from(*factor)) {
                            *factor = (v as f32).clamp(0.01, 100.0);
                            changed |= edited;
                        } else {
                            ui.horizontal(|ui| {
                                label_cell(ui, "Factor");
                                changed |= ui
                                    .add(
                                        egui::DragValue::new(factor)
                                            .speed(0.05)
                                            .range(0.01..=100.0),
                                    )
                                    .changed();
                            });
                        }
                        ui.horizontal_wrapped(|ui| {
                            label_cell(ui, "Center");
                            for v in center.iter_mut() {
                                changed |= ui.add(egui::DragValue::new(v).speed(0.5)).changed();
                            }
                            let picked = picked_point(ctx);
                            if ui
                                .add_enabled_ui(picked.is_some(), |ui| {
                                    accent_outline_button(ui, "Use selected")
                                })
                                .inner
                                .on_hover_text(
                                    "A circular edge's centre, else where an edge or a face was \
                                     picked",
                                )
                                .clicked()
                                && let Some(at) = picked
                            {
                                *center = at;
                                changed = true;
                            }
                        });
                        let points: Vec<(String, [f32; 3])> =
                            core_document::datums_of_body(ctx.document, body)
                                .into_iter()
                                .filter(|(_, _, d)| {
                                    matches!(d.shape, core_document::DatumShape::Point)
                                })
                                .map(|(_, name, d)| (name, d.frame().origin))
                                .collect();
                        if !points.is_empty() {
                            ui.horizontal(|ui| {
                                label_cell(ui, "");
                                egui::ComboBox::from_id_salt(("mt_scale_point", feature_id, i))
                                    .selected_text("A datum point…")
                                    .show_ui(ui, |ui| {
                                        for (name, at) in &points {
                                            if ui.selectable_label(false, name).clicked() {
                                                *center = *at;
                                                changed = true;
                                            }
                                        }
                                    });
                            });
                        }
                        changed |= count_drag(ui, fx, occurrences, "Occurrences:");
                    }
                }
            }
            fx.prefix = None;
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
        DesignFeature::Clone { source } => {
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
        DesignFeature::Borrow {
            source,
            frozen,
            options,
        } => {
            changed |= borrow::borrow_editor(ui, ctx, body, feature_id, source, frozen, options);
        }
        DesignFeature::BodyBoolean {
            tool_body,
            kind,
            more_tools,
            refine: _,
            boolean_tolerance: _,
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
            // Further tools, taken the same way in turn.
            let mut remove = None;
            for (i, tool) in more_tools.iter().enumerate() {
                let name = bodies
                    .iter()
                    .find(|(id, _)| id == tool)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| "(gone)".into());
                ui.horizontal(|ui| {
                    label_cell(ui, "");
                    mono_label(ui, name, FONT_XS, TEXT1);
                    if small_secondary_button(ui, "×").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                more_tools.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                label_cell(ui, "Add tool");
                egui::ComboBox::from_id_salt(("bool_more", feature_id))
                    .selected_text("Another body…")
                    .show_ui(ui, |ui| {
                        for (id, name) in &bodies {
                            if id != tool_body
                                && !more_tools.contains(id)
                                && ui.selectable_label(false, name).clicked()
                            {
                                more_tools.push(*id);
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
        changed |= advanced_editor(ui, fx, feature_id, feature);
    }
    changed
}

/// The Advanced group of a feature that fuses or cuts: the tolerance its
/// boolean runs at.
fn advanced_editor(
    ui: &mut Ui,
    fx: &mut Formulas,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new("Advanced")
        .id_salt(("design_advanced", feature_id))
        .default_open(feature.boolean_tolerance().is_some())
        .show_unindented(ui, |ui| {
            changed |= tolerance_editor(ui, fx, feature_id, feature);
            taper_note(
                ui,
                "Faces this close to touching are joined by the fuse or cut, closing \
                 a gap or a sliver between them. Empty leaves it automatic.",
            );
        });
    changed
}

/// The boolean tolerance: a length while one is stated, with a button
/// putting it back to automatic; else an empty field, a length typed
/// into it stating one.
fn tolerance_editor(
    ui: &mut Ui,
    fx: &mut Formulas,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    const LABEL: &str = "Tolerance";
    const NAME: &str = "boolean_tolerance";
    if let Some(stated) = feature.boolean_tolerance() {
        let mut changed = false;
        let mut value = stated;
        let edited = match fx.show_named(ui, LABEL, NAME, f64::from(stated)) {
            Some((edited, v)) => {
                value = v as f32;
                edited
            }
            None => field(ui, LABEL, |ui| {
                QtyField::mm(&mut value).speed(0.001).decimals(3).show(ui)
            }),
        };
        if edited {
            feature.set_boolean_tolerance(Some(value));
            changed = true;
        }
        if field(ui, "", |ui| {
            small_secondary_button(ui, "Automatic").clicked()
        }) {
            if let Some(key) = fx
                .params
                .iter()
                .find(|p| p.name.as_deref() == Some(NAME))
                .map(|p| p.key.clone())
            {
                fx.edits.push((key, None));
            }
            feature.set_boolean_tolerance(None);
            changed = true;
        }
        return changed;
    }
    let id = egui::Id::new(("boolean_tolerance_draft", feature_id));
    let mut draft: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let mut stated = None;
    field(ui, LABEL, |ui| {
        let response = ui.add(
            egui::TextEdit::singleline(&mut draft)
                .hint_text("Automatic")
                .desired_width(120.0),
        );
        if response.lost_focus() {
            stated = parse_tolerance(&draft);
        }
        false
    });
    if stated.is_some() {
        draft.clear();
    }
    ui.data_mut(|d| d.insert_temp(id, draft));
    if stated.is_some() {
        feature.set_boolean_tolerance(stated);
        return true;
    }
    false
}

/// A tolerance as typed: a positive length in mm, the unit optional.
fn parse_tolerance(text: &str) -> Option<f32> {
    text.trim()
        .trim_end_matches("mm")
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|t| t.is_finite() && *t > 0.0)
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

/// Every picked face in the edited feature's body frame, in the order
/// picked.
fn picked_faces(ctx: &WorkbenchRuntimeContext) -> Vec<core_document::FaceRef> {
    match edited_body(ctx) {
        Some(body) => ctx.selected_faces_in(body),
        None => ctx.picked_faces(),
    }
}

/// A point picked in the viewport, in the edited feature's body frame: a
/// circular edge's centre, else where an edge was picked, else a face.
fn picked_point(ctx: &WorkbenchRuntimeContext) -> Option<[f32; 3]> {
    let edges = picked_edges(ctx);
    edges
        .first()
        .map(|e| e.circle.map_or(e.point, |c| c.center))
        .or_else(|| picked_face(ctx).map(|f| f.point))
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
    fn every_field_label_names_a_parameter_design_lists() {
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
            json!({"Pad": {"sketch": s, "length": 10.0, "reversed": false, "mode": "UpToShape",
                "mode2": "UpToFace", "direction": {"Custom": [0.0, 1.0, 1.0]},
                "up_to_shape": [{"point": [1.0, 2.0, 3.0], "normal": [0.0, 0.0, 1.0]}]}}),
            json!({"Pocket": {"sketch": null, "depth": 5.0, "reversed": false, "mode": "TwoLengths",
                "mode2": "UpToShape",
                "profile_face": {"point": [100.0, 200.0, 300.0], "normal": [0.0, 0.0, 1.0]},
                "direction": {"Edge": {"point": [100.0, 200.0, 300.0], "direction": [0.0, 0.0, 1.0]}}}}),
            json!({"Revolution": {"sketch": s, "angle_deg": 360.0, "mode": "UpToFace",
                "axis": {"Edge": {"point": [100.0, 200.0, 300.0], "direction": [0.0, 1.0, 0.0]}}}}),
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
            json!({"Pipe": {"profile": s, "spine": s, "subtractive": false,
                "orientation": {"Binormal": {"x": 0.0, "y": 0.0, "z": 1.0}},
                "corner": "Round", "sections": [s]}}),
            json!({"Pipe": {"profile": s, "spine": s, "subtractive": false,
                "orientation": {"Auxiliary": {"path": s}}}}),
            json!({"Helix": {"sketch": s, "axis": "SketchY", "mode": "HeightTurnsGrowth",
                "pitch": 2.0, "height": 0.0, "turns": 3.0, "growth": 2.0, "left_handed": false,
                "cone_angle_deg": 0.0, "reversed": false, "subtractive": true,
                "keep_inside": true}}),
            json!({"Fillet": {"radius": 1.0}}),
            json!({"Chamfer": {"size": 1.0, "mode": "DistanceAngle"}}),
            json!({"Draft": {"angle_deg": 3.0, "neutral": {"point": [0.0, 0.0, 0.0], "normal": [0.0, 0.0, 1.0]}, "faces": []}}),
            json!({"Thickness": {"value": 1.0, "faces": []}}),
            json!({"Mirrored": {"originals": [], "plane": "XY"}}),
            json!({"MultiTransform": {"originals": [], "steps": [
                {"Scale": {"factor": 2.0, "center": [0.0, 0.0, 0.0], "occurrences": 2}},
                {"Mirror": {"plane": "XZ"}}]}}),
            json!({"Borrow": {"source": {"Sketch": s}}}),
            json!({"Borrow": {"source": {"Solid": {"body": body.0.to_string(),
                "faces": [{"point": [100.0, 200.0, 300.0], "normal": [0.0, 0.0, 1.0]}],
                "edges": [{"point": [100.0, 200.0, 300.0], "direction": [0.0, 0.0, 1.0]}]}}}}),
            json!({"Primitive": {
                "kind": {"Cylinder": {"radius": 5.0, "height": 10.0, "angle_deg": 360.0}},
                "placement": {"origin": [0.0, 0.0, 0.0], "x_axis": [1.0, 0.0, 0.0], "z_axis": [0.0, 0.0, 1.0]},
                "subtractive": false}}),
        ];
        for data in features {
            let feature = DesignFeature::from_json(&data).unwrap();
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
