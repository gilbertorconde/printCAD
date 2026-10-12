//! The datum task's attachment: the mode selector and a row for each
//! reference the mode takes, filled from what is picked in the viewport.

use core_document::{
    BasePlane, BodyId, DatumAttachment, DatumFeature, DatumShape, EdgeAnchor, EdgeSpot, FeatureId,
    FrameAnchor, LineAnchor, PlaneAnchor, PointAnchor, WorkbenchFeature, WorkbenchRuntimeContext,
};
use egui::Ui;
use ui_kit::tokens::*;
use ui_kit::widgets::{Note, accent_outline_button, mono_label, note_card};

use crate::editors::label_cell;
use core_document::attach::{
    PICK_MODES as MODES, Picked, candidate, current_edge, mode_label, settle,
};

fn coords(p: [f32; 3]) -> String {
    format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2])
}

/// A reference row: its label, what it holds, and a button that takes the
/// pick for it. Answers whether the button was pressed with a pick there.
fn reference_row(ui: &mut Ui, label: &str, shown: String, can_pick: bool, button: &str) -> bool {
    ui.horizontal_wrapped(|ui| {
        label_cell(ui, label);
        mono_label(ui, shown, FONT_XS, TEXT1);
        ui.add_enabled_ui(can_pick, |ui| accent_outline_button(ui, button))
            .inner
            .on_hover_text("Click it in the viewport first, then press this")
            .clicked()
            && can_pick
    })
    .inner
}

fn spot_row(ui: &mut Ui, salt: (FeatureId, usize), spot: &mut EdgeSpot, edge: &EdgeAnchor) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "At");
        egui::ComboBox::from_id_salt(("datum_spot", salt))
            .selected_text(spot.label())
            .show_ui(ui, |ui| {
                for candidate in EdgeSpot::ALL {
                    let known = match candidate {
                        EdgeSpot::Centre => edge.circle.is_some(),
                        _ => true,
                    };
                    if ui
                        .add_enabled(
                            known,
                            egui::Button::selectable(*spot == candidate, candidate.label()),
                        )
                        .clicked()
                        && *spot != candidate
                    {
                        *spot = candidate;
                        changed = true;
                    }
                }
            });
    });
    changed
}

fn point_text(point: &PointAnchor) -> String {
    let at = coords(point.point());
    match point {
        PointAnchor::At { .. } => at,
        PointAnchor::Face { .. } => format!("{at} on a face"),
        PointAnchor::Edge { .. } => format!("{at} on an edge"),
        PointAnchor::Datum { .. } => format!("{at}, a datum's"),
        PointAnchor::Sketch { .. } => format!("{at} in a sketch"),
    }
}

/// The body's features made before this datum that it may take points and
/// lines from: datum points and lines, and the points and lines of its
/// sketches, each with a label.
struct FeatureRefs {
    points: Vec<(String, PointAnchor)>,
    lines: Vec<(String, LineAnchor)>,
}

/// The most sketch elements a reference list offers.
const MOST_SKETCH_REFS: usize = 200;

fn feature_refs(ctx: &WorkbenchRuntimeContext, body: BodyId, own_seq: Option<u64>) -> FeatureRefs {
    let mut refs = FeatureRefs {
        points: Vec::new(),
        lines: Vec::new(),
    };
    let before = |id: FeatureId| {
        ctx.document
            .get_feature_meta(id)
            .is_some_and(|n| Some(n.seq) < own_seq)
    };
    for (id, name, datum) in core_document::datums_of_body(ctx.document, body) {
        if !before(id) {
            continue;
        }
        let frame = datum.frame();
        match datum.shape {
            DatumShape::Point => refs.points.push((
                name,
                PointAnchor::Datum {
                    datum: id,
                    point: frame.origin,
                },
            )),
            DatumShape::Line { .. } => refs.lines.push((
                name,
                LineAnchor::Datum {
                    datum: id,
                    origin: frame.origin,
                    direction: frame.x_axis,
                },
            )),
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
        if !before(sketch) {
            continue;
        }
        let Some(feature) = ctx
            .document
            .feature_values(sketch)
            .and_then(|v| wb_sketch::SketchFeature::from_json(v).ok())
        else {
            continue;
        };
        let (mut points, mut lines) = (0, 0);
        for element in &feature.sketch.geometry {
            if refs.points.len() + refs.lines.len() >= MOST_SKETCH_REFS {
                break;
            }
            use wb_sketch::sketch::GeometryElement;
            let id = element.id();
            let Some(at) = crate::datum_refs::sketch_points_of(&feature, id) else {
                continue;
            };
            match (element, at.as_slice()) {
                (GeometryElement::Point(_), [point]) => {
                    points += 1;
                    refs.points.push((
                        format!("{name} › point {points}"),
                        PointAnchor::Sketch {
                            sketch,
                            element: id,
                            point: *point,
                        },
                    ));
                }
                (GeometryElement::Line(_), [start, end]) => {
                    lines += 1;
                    refs.lines.push((
                        format!("{name} › line {lines}"),
                        LineAnchor::Sketch {
                            sketch,
                            element: id,
                            start: *start,
                            end: *end,
                        },
                    ));
                }
                _ => {}
            }
        }
    }
    refs
}

fn line_text(ctx: &WorkbenchRuntimeContext, line: &LineAnchor) -> String {
    let (at, _) = line.line();
    match line {
        LineAnchor::Edge { .. } => format!("edge at {}", coords(at)),
        LineAnchor::Datum { datum, .. } | LineAnchor::Sketch { sketch: datum, .. } => ctx
            .document
            .get_feature_meta(*datum)
            .map(|n| format!("{} at {}", n.name, coords(at)))
            .unwrap_or_else(|| "(gone)".into()),
    }
}

/// A line reference: what it holds, the picked edge, or a datum or sketch
/// line made before.
fn line_row(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    (salt, index): (FeatureId, usize),
    line: &mut LineAnchor,
    picked: &Picked,
    refs: &FeatureRefs,
) -> bool {
    let mut changed = false;
    if reference_row(
        ui,
        &format!("Line {}", index + 1),
        line_text(ctx, line),
        !picked.edges.is_empty(),
        "Use selected edge",
    ) && let Some(edge) = picked.edges.get(index.min(picked.edges.len() - 1))
    {
        *line = LineAnchor::Edge { edge: *edge };
        changed = true;
    }
    if !refs.lines.is_empty() {
        ui.horizontal(|ui| {
            label_cell(ui, "");
            egui::ComboBox::from_id_salt(("datum_line_ref", salt, index))
                .selected_text("A datum or sketch line…")
                .show_ui(ui, |ui| {
                    for (label, candidate) in &refs.lines {
                        if ui.selectable_label(line == candidate, label).clicked() {
                            *line = *candidate;
                            changed = true;
                        }
                    }
                });
        });
    }
    changed
}

/// A point reference taken from a datum or sketch point made before.
fn feature_point_row(
    ui: &mut Ui,
    salt: (FeatureId, usize),
    point: &mut PointAnchor,
    refs: &FeatureRefs,
) -> bool {
    if refs.points.is_empty() {
        return false;
    }
    let mut changed = false;
    ui.horizontal(|ui| {
        label_cell(ui, "");
        egui::ComboBox::from_id_salt(("datum_point_ref", salt))
            .selected_text("A datum or sketch point…")
            .show_ui(ui, |ui| {
                for (label, candidate) in &refs.points {
                    if ui.selectable_label(point == candidate, label).clicked() {
                        *point = *candidate;
                        changed = true;
                    }
                }
            });
    });
    changed
}

/// The datum plane or coordinate system (and which of its planes) the
/// datum stands on.
fn on_datum_rows(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    salt: FeatureId,
    (datum, plane, frame): (&mut FeatureId, &mut Option<BasePlane>, &mut FrameAnchor),
    picked: &Picked,
) -> bool {
    let mut changed = false;
    let name = |id: FeatureId| {
        ctx.document
            .get_feature_meta(id)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "(gone)".into())
    };
    let pick_frame = |found: core_document::DatumFrame, which: Option<BasePlane>| match which {
        Some(which) => found
            .planes()
            .into_iter()
            .zip(BasePlane::ALL)
            .find(|(_, p)| *p == which)
            .map(|((_, f), _)| f)
            .unwrap_or(found),
        None => found,
    };
    ui.horizontal(|ui| {
        label_cell(ui, "Datum");
        egui::ComboBox::from_id_salt(("datum_on", salt))
            .selected_text(name(*datum))
            .show_ui(ui, |ui| {
                for (id, found, system) in &picked.datums {
                    if ui.selectable_label(*datum == *id, name(*id)).clicked() && *datum != *id {
                        *datum = *id;
                        *plane = system.then_some(plane.unwrap_or(BasePlane::XY));
                        *frame = pick_frame(*found, *plane).into();
                        changed = true;
                    }
                }
            });
    });
    if let Some(which) = plane {
        ui.horizontal(|ui| {
            label_cell(ui, "Its plane");
            for candidate in BasePlane::ALL {
                if ui
                    .selectable_label(*which == candidate, candidate.label())
                    .clicked()
                    && *which != candidate
                {
                    *which = candidate;
                    if let Some((_, found, _)) = picked.datums.iter().find(|(id, ..)| id == datum) {
                        *frame = pick_frame(*found, Some(candidate)).into();
                    }
                    changed = true;
                }
            }
        });
    }
    changed
}

/// The other body and which of its origin planes the datum stands on.
fn other_body_rows(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    (salt, own): (FeatureId, BodyId),
    (body, plane, frame): (&mut BodyId, &mut BasePlane, &mut FrameAnchor),
) -> bool {
    let mut changed = false;
    let bodies: Vec<(BodyId, String)> = ctx
        .document
        .bodies()
        .iter()
        .filter(|b| b.id != own)
        .map(|b| (b.id, b.name.clone()))
        .collect();
    let shown = bodies
        .iter()
        .find(|(id, _)| id == body)
        .map(|(_, n)| n.clone())
        .unwrap_or_else(|| "(gone)".into());
    ui.horizontal(|ui| {
        label_cell(ui, "Body");
        egui::ComboBox::from_id_salt(("datum_other_body", salt))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (id, name) in &bodies {
                    if ui.selectable_label(body == id, name).clicked() && body != id {
                        *body = *id;
                        changed = true;
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        label_cell(ui, "Plane");
        for candidate in BasePlane::ALL {
            if ui
                .selectable_label(*plane == candidate, candidate.label())
                .clicked()
                && *plane != candidate
            {
                *plane = candidate;
                changed = true;
            }
        }
    });
    if changed && let Some(found) = core_document::body_plane_in(ctx.document, own, *body, *plane) {
        *frame = found.into();
    }
    changed
}

fn plane_row(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    (feature_id, index): (FeatureId, usize),
    body: BodyId,
    plane: &mut PlaneAnchor,
    picked: &Picked,
) -> bool {
    let mut changed = false;
    let shown = match plane {
        PlaneAnchor::Base(base) => base.label().to_string(),
        PlaneAnchor::Datum { datum, .. } => ctx
            .document
            .get_feature_meta(*datum)
            .map(|n| n.name.clone())
            .unwrap_or_else(|| "(gone)".into()),
        PlaneAnchor::Face { face } => format!("face at {}", coords(face.point)),
    };
    // Only datums made before this one: none of them can be made from it.
    let own_seq = ctx.document.get_feature_meta(feature_id).map(|n| n.seq);
    let datums: Vec<(FeatureId, String, DatumFeature)> =
        core_document::datums_of_body(ctx.document, body)
            .into_iter()
            .filter(|(id, _, datum)| {
                crate::datum_refs::has_plane(datum)
                    && ctx
                        .document
                        .get_feature_meta(*id)
                        .is_some_and(|n| Some(n.seq) < own_seq)
            })
            .collect();
    ui.horizontal_wrapped(|ui| {
        label_cell(ui, &format!("Plane {}", index + 1));
        egui::ComboBox::from_id_salt(("datum_plane", feature_id, index))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for base in BasePlane::ALL {
                    let candidate = PlaneAnchor::Base(base);
                    if ui
                        .selectable_label(*plane == candidate, base.label())
                        .clicked()
                    {
                        *plane = candidate;
                        changed = true;
                    }
                }
                for (id, name, datum) in &datums {
                    let chosen = matches!(plane, PlaneAnchor::Datum { datum: d, .. } if d == id);
                    if ui.selectable_label(chosen, name).clicked() && !chosen {
                        let frame = datum.frame();
                        *plane = PlaneAnchor::Datum {
                            datum: *id,
                            origin: frame.origin,
                            normal: frame.normal,
                        };
                        changed = true;
                    }
                }
            });
    });
    // On a line of its own: beside a long face reading it wraps a letter a line.
    ui.horizontal(|ui| {
        label_cell(ui, "");
        if ui
            .add_enabled_ui(picked.face.is_some(), |ui| {
                accent_outline_button(ui, "Use selected face")
            })
            .inner
            .on_hover_text("Click a face in the viewport first, then press this")
            .clicked()
            && let Some(face) = picked.face
        {
            *plane = PlaneAnchor::Face { face };
            changed = true;
        }
    });
    changed
}

/// The attachment part of the datum task: the mode, and the references
/// it takes. Answers whether the datum changed.
pub fn attachment_editor(
    ui: &mut Ui,
    ctx: &WorkbenchRuntimeContext,
    feature_id: FeatureId,
    datum: &mut DatumFeature,
) -> bool {
    let Some(body) = ctx
        .document
        .get_feature_meta(feature_id)
        .and_then(|n| n.body)
    else {
        return false;
    };
    let own_seq = ctx.document.get_feature_meta(feature_id).map(|n| n.seq);
    let before = |id: FeatureId| {
        ctx.document
            .get_feature_meta(id)
            .is_some_and(|n| Some(n.seq) < own_seq)
    };
    let picked = Picked::of(ctx, body).with_datums(ctx, body, before);
    let refs = feature_refs(ctx, body, own_seq);
    let mut changed = false;
    let mut picks_changed = false;

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
                if matches!(datum.attachment, DatumAttachment::FlatFace { .. }) {
                    let _ = ui.selectable_label(true, datum.attachment.label());
                }
                ui.separator();
                for (mode, needs) in MODES {
                    let current = datum.attachment.mode() == *mode;
                    let made = candidate(
                        mode,
                        ctx,
                        body,
                        datum.frame(),
                        current_edge(&datum.attachment),
                        &picked,
                    );
                    let response = ui.add_enabled(
                        current || made.is_some(),
                        egui::Button::selectable(current, mode_label(mode)),
                    );
                    let response = if needs.is_empty() {
                        response
                    } else {
                        response.on_disabled_hover_text(*needs)
                    };
                    if response.clicked()
                        && !current
                        && let Some(made) = made
                    {
                        datum.attachment = made;
                        picks_changed = true;
                    }
                }
            });
    });

    let salt = feature_id;
    match &mut datum.attachment {
        DatumAttachment::Face { face } => {
            if reference_row(
                ui,
                "Face",
                coords(face.point),
                picked.face.is_some(),
                "Use selected face",
            ) && let Some(new) = picked.face
            {
                *face = new;
                picks_changed = true;
            }
        }
        DatumAttachment::ThreePoints { points } => {
            picks_changed |= point_rows(ui, salt, points, &picked);
            for (i, point) in points.iter_mut().enumerate() {
                picks_changed |= feature_point_row(ui, (salt, i + 10), point, &refs);
            }
        }
        DatumAttachment::TwoPoints { points } => {
            picks_changed |= point_rows(ui, salt, points, &picked);
            for (i, point) in points.iter_mut().enumerate() {
                picks_changed |= feature_point_row(ui, (salt, i + 10), point, &refs);
            }
        }
        DatumAttachment::NormalToEdge { edge, spot, along } => {
            picks_changed |= edge_row(ui, edge, &picked);
            ui.horizontal(|ui| {
                label_cell(ui, "Along");
                let mut on = along.is_some();
                if ui
                    .checkbox(&mut on, "A share of the way")
                    .on_hover_text("The plane stands this far along the edge from its start")
                    .changed()
                {
                    *along = on.then_some(0.5);
                    picks_changed = true;
                }
                if let Some(share) = along {
                    let mut percent = *share * 100.0;
                    if ui_kit::widgets::QtyField::new(&mut percent)
                        .unit("%")
                        .range(0.0..=100.0)
                        .show(ui)
                    {
                        *share = percent / 100.0;
                        picks_changed = true;
                    }
                }
            });
            if along.is_none() {
                changed |= spot_row(ui, (salt, 0), spot, edge);
            }
        }
        DatumAttachment::TangentToEdge { edge, spot } => {
            picks_changed |= edge_row(ui, edge, &picked);
            changed |= spot_row(ui, (salt, 0), spot, edge);
        }
        DatumAttachment::FaceNormal { face } => {
            if reference_row(
                ui,
                "Face",
                coords(face.point),
                picked.face.is_some(),
                "Use selected face",
            ) && let Some(new) = picked.face
            {
                *face = new;
                picks_changed = true;
            }
        }
        DatumAttachment::OnDatum {
            datum,
            plane,
            frame,
        } => {
            picks_changed |= on_datum_rows(ui, ctx, salt, (datum, plane, frame), &picked);
        }
        DatumAttachment::OtherBody {
            body: other,
            plane,
            frame,
        } => {
            picks_changed |= other_body_rows(ui, ctx, (salt, body), (other, plane, frame));
        }
        DatumAttachment::LineAndPoint { line, point } => {
            picks_changed |= line_row(ui, ctx, (salt, 0), line, &picked, &refs);
            picks_changed |= point_rows(ui, salt, std::slice::from_mut(point), &picked);
            picks_changed |= feature_point_row(ui, (salt, 0), point, &refs);
        }
        DatumAttachment::LineMeetsPlane { line, plane } => {
            picks_changed |= line_row(ui, ctx, (salt, 0), line, &picked, &refs);
            picks_changed |= plane_row(ui, ctx, (salt, 0), body, plane, &picked);
        }
        DatumAttachment::TwoLines { lines } => {
            for (i, line) in lines.iter_mut().enumerate() {
                picks_changed |= line_row(ui, ctx, (salt, i), line, &picked, &refs);
            }
        }
        DatumAttachment::AlongEdge { edge } | DatumAttachment::CurveCentre { edge } => {
            picks_changed |= edge_row(ui, edge, &picked);
        }
        DatumAttachment::PlaneIntersection { planes } | DatumAttachment::Midplane { planes } => {
            for (i, plane) in planes.iter_mut().enumerate() {
                picks_changed |= plane_row(ui, ctx, (salt, i), body, plane, &picked);
            }
        }
        DatumAttachment::Inertia { centre, .. } => {
            ui.horizontal(|ui| {
                label_cell(ui, "Centre");
                mono_label(ui, coords(*centre), FONT_XS, TEXT1);
            });
        }
        DatumAttachment::BasePlane(_) | DatumAttachment::FlatFace { .. } => {}
    }

    let mut problem = None;
    if picks_changed {
        if let Err(e) = settle(ctx, body, datum) {
            problem = Some(e);
        }
        changed = true;
    }
    if let Some(problem) = problem.or_else(|| datum.attachment.problem().map(str::to_string)) {
        ui.add_space(SPACE_1);
        note_card(ui, Note::Warning, None, &problem);
    }
    changed
}

fn edge_row(ui: &mut Ui, edge: &mut EdgeAnchor, picked: &Picked) -> bool {
    let shown = match edge.circle {
        Some(circle) => format!("circle R{:.2} at {}", circle.radius, coords(circle.center)),
        None => coords(edge.point),
    };
    if reference_row(
        ui,
        "Edge",
        shown,
        !picked.edges.is_empty(),
        "Use selected edge",
    ) && let Some(new) = picked.edges.first()
    {
        *edge = *new;
        return true;
    }
    false
}

fn point_rows(ui: &mut Ui, salt: FeatureId, points: &mut [PointAnchor], picked: &Picked) -> bool {
    let mut changed = false;
    let from_pick = picked.points().first().copied();
    for (i, point) in points.iter_mut().enumerate() {
        if reference_row(
            ui,
            &format!("Point {}", i + 1),
            point_text(point),
            from_pick.is_some(),
            "Use selected",
        ) && let Some(new) = from_pick
        {
            *point = new;
            changed = true;
        }
        if let PointAnchor::Edge { edge, spot } = point {
            let edge = *edge;
            changed |= spot_row(ui, (salt, i + 1), spot, &edge);
        }
    }
    changed
}
