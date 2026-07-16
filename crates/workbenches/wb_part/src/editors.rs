//! Per-feature settings editors for the left panel.
//!
//! Every dialog option maps 1:1 to a feature field; editing either path
//! recomputes. Editors return `true` when the feature payload changed.

use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};
use egui::Ui;

use crate::build::{part_features_of_body, sketches_of_body};
use crate::feature::{
    ChamferMode, EdgeSel, ExtrudeMode, FacePick, HelixMode, HoleCut, HoleFit, MirrorPlane,
    PartFeature, PatternAxis, RevolveAxis, TransformStep, METRIC_SIZES,
};

fn mm_drag(ui: &mut Ui, value: &mut f32, label: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(value)
                .speed(0.5)
                .range(0.01..=1.0e6)
                .suffix(" mm"),
        )
        .changed()
    })
    .inner
}

fn deg_drag(
    ui: &mut Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(
            egui::DragValue::new(value)
                .speed(1.0)
                .range(range)
                .suffix("°"),
        )
        .changed()
    })
    .inner
}

fn count_drag(ui: &mut Ui, value: &mut u32, label: &str) -> bool {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(value).speed(0.1).range(2..=1000))
            .changed()
    })
    .inner
}

fn sketch_combo(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    id_salt: impl std::hash::Hash,
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
        ui.label(label);
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
    id_salt: impl std::hash::Hash,
    mode: &mut ExtrudeMode,
    first_feature: bool,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Type:");
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
    ui.horizontal(|ui| {
        ui.label(label);
        match pick {
            Some(p) => {
                ui.label(format!(
                    "({:.1}, {:.1}, {:.1})",
                    p.point[0], p.point[1], p.point[2]
                ));
            }
            None => {
                ui.label("(none)");
            }
        }
        let has_selection = ctx.selected_face.is_some();
        if ui
            .add_enabled(has_selection, egui::Button::new("Use selected face"))
            .on_hover_text("Click a face in the viewport first, then press this")
            .clicked()
        {
            if let Some(face) = ctx.selected_face {
                *pick = Some(FacePick {
                    point: face.point,
                    normal: face.normal,
                });
                changed = true;
            }
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
    ui.label(label);
    let mut remove = None;
    for (i, face) in faces.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(format!(
                "· ({:.1}, {:.1}, {:.1})",
                face.point[0], face.point[1], face.point[2]
            ));
            if ui.small_button("✕").clicked() {
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
        .add_enabled(has_selection, egui::Button::new("Add selected face"))
        .on_hover_text("Click a face in the viewport first, then press this")
        .clicked()
    {
        if let Some(face) = ctx.selected_face {
            faces.push(FacePick {
                point: face.point,
                normal: face.normal,
            });
            changed = true;
        }
    }
    changed
}

fn edge_sel_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    edges: &mut EdgeSel,
    id_salt: impl std::hash::Hash,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Edges:");
        egui::ComboBox::from_id_salt(id_salt)
            .selected_text(match edges {
                EdgeSel::All => "All edges".to_string(),
                EdgeSel::Faces(f) => format!("{} face(s)", f.len()),
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
            });
    });
    if let EdgeSel::Faces(faces) = edges {
        changed |= face_list_editor(ui, ctx, faces, "Faces:");
    }
    changed
}

fn revolve_axis_editor(ui: &mut Ui, axis: &mut RevolveAxis, id_salt: impl std::hash::Hash) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Axis:");
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
            ui.label("Origin:");
            changed |= ui
                .add(egui::DragValue::new(&mut origin[0]).speed(0.5))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut origin[1]).speed(0.5))
                .changed();
            ui.label("Dir:");
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

fn pattern_axis_editor(ui: &mut Ui, axis: &mut PatternAxis, id_salt: impl std::hash::Hash) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Axis:");
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
            ui.label("Origin:");
            for v in origin.iter_mut() {
                changed |= ui.add(egui::DragValue::new(v).speed(0.5)).changed();
            }
        });
        ui.horizontal(|ui| {
            ui.label("Dir:");
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
    id_salt: impl std::hash::Hash,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Plane:");
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
                if ui.selectable_label(is_face, "Picked face").clicked() && !is_face {
                    if let Some(face) = ctx.selected_face {
                        *plane = MirrorPlane::Face(FacePick {
                            point: face.point,
                            normal: face.normal,
                        });
                        changed = true;
                    }
                }
            });
    });
    if let MirrorPlane::Face(pick) = plane {
        let mut opt = Some(*pick);
        if face_pick_row(ui, ctx, &mut opt, "Face:") {
            if let Some(new_pick) = opt {
                *pick = new_pick;
                changed = true;
            }
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
    ui.label("Originals (empty = whole body):");
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
        if ui.checkbox(&mut included, name).changed() {
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
        ui.label("Shape:");
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
            ui.label(label);
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
            ui.label(label);
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
                ui.label("Sides:");
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
                    ui.label(label);
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
        ui.label("Position:");
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
    feature_id: FeatureId,
    datum: &mut core_document::DatumFeature,
) -> bool {
    use core_document::{BasePlane, DatumAttachment, DatumShape};
    let mut changed = false;

    match &mut datum.shape {
        DatumShape::Plane { size } => changed |= mm_drag(ui, size, "Display size:"),
        DatumShape::Line { length } => changed |= mm_drag(ui, length, "Display length:"),
        DatumShape::Point => {}
    }

    ui.horizontal(|ui| {
        ui.label("Attached to:");
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
                {
                    if let Some(face) = ctx.selected_face {
                        datum.attachment = DatumAttachment::FlatFace {
                            point: face.point,
                            normal: face.normal,
                        };
                        changed = true;
                    }
                }
            });
    });
    if matches!(datum.attachment, DatumAttachment::FlatFace { .. })
        && ctx.selected_face.is_some()
        && ui
            .button("Re-pick from selected face")
            .on_hover_text("Move the attachment to the currently selected face")
            .clicked()
    {
        if let Some(face) = ctx.selected_face {
            datum.attachment = DatumAttachment::FlatFace {
                point: face.point,
                normal: face.normal,
            };
            changed = true;
        }
    }

    ui.label("Attachment offset:");
    ui.horizontal(|ui| {
        for (value, label) in datum.offset.translation.iter_mut().zip(["x", "y", "n"]) {
            ui.label(label);
            changed |= ui
                .add(egui::DragValue::new(value).speed(0.5).suffix(" mm"))
                .changed();
        }
    });
    changed |= deg_drag(
        ui,
        &mut datum.offset.rotation_deg,
        "Rotation:",
        -180.0..=180.0,
    );
    changed |= ui.checkbox(&mut datum.offset.flip, "Flip side").changed();
    changed
}

/// The full settings editor for one feature. Returns true when the payload
/// changed and needs a rebuild.
pub fn feature_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut PartFeature,
) -> bool {
    let mut changed = false;
    match feature {
        PartFeature::Pad {
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
                    changed |= mm_drag(ui, length, "Length:");
                    changed |= ui.checkbox(symmetric, "Symmetric to plane").changed();
                }
                ExtrudeMode::TwoLengths => {
                    changed |= mm_drag(ui, length, "Length:");
                    changed |= mm_drag(ui, length2, "Second length:");
                }
                ExtrudeMode::UpToFace => {
                    changed |= face_pick_row(ui, ctx, up_to_face, "Target face:");
                    changed |= mm_drag(ui, up_to_offset, "Offset:");
                }
                _ => {}
            }
            changed |= ui.checkbox(reversed, "Reversed").changed();
            changed |= deg_drag(ui, taper_deg, "Taper:", -85.0..=85.0);
        }
        PartFeature::Pocket {
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
                ExtrudeMode::Dimension => changed |= mm_drag(ui, depth, "Depth:"),
                ExtrudeMode::TwoLengths => {
                    changed |= mm_drag(ui, depth, "Depth:");
                    changed |= mm_drag(ui, depth2, "Second depth:");
                }
                ExtrudeMode::UpToFace => {
                    changed |= face_pick_row(ui, ctx, up_to_face, "Target face:");
                    changed |= mm_drag(ui, up_to_offset, "Offset:");
                }
                _ => {}
            }
            changed |= ui
                .checkbox(reversed, "Reversed")
                .on_hover_text("Cut along the sketch normal instead of against it")
                .changed();
            changed |= deg_drag(ui, taper_deg, "Taper:", -85.0..=85.0);
        }
        PartFeature::Revolution {
            sketch,
            angle_deg,
            axis,
            reversed,
            midplane,
            second_angle_deg,
        }
        | PartFeature::Groove {
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
            changed |= deg_drag(ui, angle_deg, "Angle:", 0.1..=360.0);
            changed |= revolve_axis_editor(ui, axis, ("rev_axis", feature_id));
            changed |= ui.checkbox(midplane, "Midplane").changed();
            let mut two_sided = second_angle_deg.is_some();
            if ui.checkbox(&mut two_sided, "Second angle").changed() {
                *second_angle_deg = two_sided.then_some(90.0);
                changed = true;
            }
            if let Some(second) = second_angle_deg {
                changed |= deg_drag(ui, second, "Angle 2:", 0.1..=360.0);
            }
            changed |= ui.checkbox(reversed, "Reversed").changed();
        }
        PartFeature::Loft {
            sections,
            ruled,
            closed,
            subtractive,
        } => {
            ui.label("Sections (in order):");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = ctx
                    .document
                    .get_feature_meta(*section)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| "(missing)".into());
                ui.horizontal(|ui| {
                    ui.label(format!("{}. {name}", i + 1));
                    if ui.small_button("✕").clicked() && sections.len() > 1 {
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
            ) {
                if !sections.contains(&new) {
                    sections.push(new);
                    changed = true;
                }
            }
            changed |= ui.checkbox(ruled, "Ruled (straight transitions)").changed();
            changed |= ui.checkbox(closed, "Closed (loop back)").changed();
            changed |= ui.checkbox(subtractive, "Subtractive").changed();
        }
        PartFeature::Pipe {
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
            changed |= ui
                .checkbox(frenet, "Frenet orientation")
                .on_hover_text("Rotate the profile with the path's curvature frame")
                .changed();
            changed |= ui.checkbox(subtractive, "Subtractive").changed();
        }
        PartFeature::Helix {
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
                ui.label("Mode:");
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
                    changed |= mm_drag(ui, pitch, "Pitch:");
                    changed |= mm_drag(ui, height, "Height:");
                }
                HelixMode::PitchTurns => {
                    changed |= mm_drag(ui, pitch, "Pitch:");
                    ui.horizontal(|ui| {
                        ui.label("Turns:");
                        changed |= ui
                            .add(egui::DragValue::new(turns).speed(0.1).range(0.1..=1000.0))
                            .changed();
                    });
                }
                HelixMode::HeightTurns => {
                    changed |= mm_drag(ui, height, "Height:");
                    ui.horizontal(|ui| {
                        ui.label("Turns:");
                        changed |= ui
                            .add(egui::DragValue::new(turns).speed(0.1).range(0.1..=1000.0))
                            .changed();
                    });
                }
            }
            changed |= deg_drag(ui, cone_angle_deg, "Cone angle:", -85.0..=85.0);
            changed |= ui.checkbox(left_handed, "Left handed").changed();
            changed |= ui.checkbox(reversed, "Reversed").changed();
            changed |= ui.checkbox(subtractive, "Subtractive").changed();
        }
        PartFeature::Primitive {
            kind,
            placement,
            subtractive,
        } => {
            changed |= primitive_editor(ui, kind);
            changed |= placement_editor(ui, placement);
            changed |= ui.checkbox(subtractive, "Subtractive").changed();
        }
        PartFeature::Hole {
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
                ui.label("Size:");
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
                changed |= ui
                    .checkbox(threaded, "Threaded (tap drill)")
                    .on_hover_text("Use the tap-drill diameter for later thread cutting")
                    .changed();
                if !*threaded {
                    ui.horizontal(|ui| {
                        ui.label("Fit:");
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
                ui.label(format!(
                    "Drill Ø {:.2} mm",
                    crate::build::hole_diameter(&PartFeature::Hole {
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
                ));
            } else {
                changed |= mm_drag(ui, diameter, "Diameter:");
            }
            changed |= ui.checkbox(through_all, "Through all").changed();
            if !*through_all {
                changed |= mm_drag(ui, depth, "Depth:");
            }
            ui.horizontal(|ui| {
                ui.label("Hole cut:");
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
                    changed |= mm_drag(ui, diameter, "Bore Ø:");
                    changed |= mm_drag(ui, depth, "Bore depth:");
                }
                HoleCut::Countersink {
                    diameter,
                    angle_deg,
                } => {
                    changed |= mm_drag(ui, diameter, "Sink Ø:");
                    changed |= deg_drag(ui, angle_deg, "Sink angle:", 10.0..=170.0);
                }
            }
            changed |= ui.checkbox(reversed, "Reversed").changed();
        }
        PartFeature::Fillet { radius, edges } => {
            changed |= mm_drag(ui, radius, "Radius:");
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
                ui.label("Type:");
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
            changed |= mm_drag(ui, size, "Size:");
            match mode {
                ChamferMode::EqualDistance => {}
                ChamferMode::TwoDistances => {
                    changed |= mm_drag(ui, size2, "Size 2:");
                    changed |= ui.checkbox(flip, "Flip direction").changed();
                }
                ChamferMode::DistanceAngle => {
                    changed |= deg_drag(ui, angle_deg, "Angle:", 1.0..=89.0);
                    changed |= ui.checkbox(flip, "Flip direction").changed();
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
            changed |= deg_drag(ui, angle_deg, "Angle:", 0.1..=45.0);
            let mut neutral_opt = Some(*neutral);
            if face_pick_row(ui, ctx, &mut neutral_opt, "Neutral plane:") {
                if let Some(pick) = neutral_opt {
                    *neutral = pick;
                    changed = true;
                }
            }
            changed |= face_list_editor(ui, ctx, faces, "Faces to draft:");
            changed |= ui.checkbox(reversed, "Reversed pull").changed();
        }
        PartFeature::Thickness {
            value,
            faces,
            inward,
        } => {
            changed |= mm_drag(ui, value, "Thickness:");
            changed |= face_list_editor(ui, ctx, faces, "Faces to open:");
            changed |= ui.checkbox(inward, "Inward").changed();
        }
        PartFeature::Mirrored { originals, plane } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= mirror_plane_editor(ui, ctx, plane, ("mirror_plane", feature_id));
        }
        PartFeature::LinearPattern {
            originals,
            axis,
            length,
            occurrences,
            spacing_mode,
            reversed,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(ui, axis, ("linear_axis", feature_id));
            changed |= count_drag(ui, occurrences, "Occurrences:");
            changed |= ui
                .checkbox(spacing_mode, "Length is spacing")
                .on_hover_text("Off: length is the overall span")
                .changed();
            changed |= mm_drag(ui, length, "Length:");
            changed |= ui.checkbox(reversed, "Reversed").changed();
        }
        PartFeature::PolarPattern {
            originals,
            axis,
            angle_deg,
            occurrences,
            reversed,
        } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(ui, axis, ("polar_axis", feature_id));
            changed |= count_drag(ui, occurrences, "Occurrences:");
            changed |= deg_drag(ui, angle_deg, "Angle:", 1.0..=360.0);
            changed |= ui.checkbox(reversed, "Reversed").changed();
        }
        PartFeature::MultiTransform { originals, steps } => {
            changed |= originals_editor(ui, ctx, body, feature_id, originals);
            ui.label("Steps (each applies to all previous results):");
            let mut remove = None;
            for (i, step) in steps.iter_mut().enumerate() {
                let label = match step {
                    TransformStep::Linear { .. } => "Linear",
                    TransformStep::Polar { .. } => "Polar",
                    TransformStep::Mirror { .. } => "Mirror",
                    TransformStep::Scale { .. } => "Scale",
                };
                ui.horizontal(|ui| {
                    ui.label(format!("{}. {label}", i + 1));
                    if ui.small_button("✕").clicked() {
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
                        changed |= mm_drag(ui, length, "Length:");
                        changed |= count_drag(ui, occurrences, "Occurrences:");
                    }
                    TransformStep::Polar {
                        axis,
                        angle_deg,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(ui, axis, ("mt_pol", feature_id, i));
                        changed |= deg_drag(ui, angle_deg, "Angle:", 1.0..=360.0);
                        changed |= count_drag(ui, occurrences, "Occurrences:");
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
                            ui.label("Factor:");
                            changed |= ui
                                .add(egui::DragValue::new(factor).speed(0.05).range(0.01..=100.0))
                                .changed();
                        });
                        ui.horizontal(|ui| {
                            ui.label("Center:");
                            for v in center.iter_mut() {
                                changed |= ui.add(egui::DragValue::new(v).speed(0.5)).changed();
                            }
                        });
                        changed |= count_drag(ui, occurrences, "Occurrences:");
                    }
                }
            }
            if let Some(i) = remove {
                steps.remove(i);
                changed = true;
            }
            ui.horizontal(|ui| {
                if ui.button("+ Linear").clicked() {
                    steps.push(TransformStep::Linear {
                        axis: PatternAxis::X,
                        length: 10.0,
                        occurrences: 2,
                    });
                    changed = true;
                }
                if ui.button("+ Polar").clicked() {
                    steps.push(TransformStep::Polar {
                        axis: PatternAxis::Z,
                        angle_deg: 360.0,
                        occurrences: 4,
                    });
                    changed = true;
                }
                if ui.button("+ Mirror").clicked() {
                    steps.push(TransformStep::Mirror {
                        plane: MirrorPlane::YZ,
                    });
                    changed = true;
                }
                if ui.button("+ Scale").clicked() {
                    steps.push(TransformStep::Scale {
                        factor: 2.0,
                        center: [0.0; 3],
                        occurrences: 2,
                    });
                    changed = true;
                }
            });
        }
        PartFeature::BodyBoolean { tool_body, kind } => {
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
                ui.label("Tool body:");
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
                ui.label("Operation:");
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
    changed
}
