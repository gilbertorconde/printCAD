//! The Pad and Pocket task panels: the profile, the end conditions of
//! each side, the direction swept along and the extras (taper, start
//! offset).

use bench_api::Select;
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{DesignFeature, EdgePick, ExtrudeDirection, ExtrudeMode, FacePick};
use crate::form::Form;

use super::{
    Bounds, CHOICE_WIDTH, Formulas, bare, choice_shown, clean, coords, deg_drag, edge_pick_row,
    end_targets, face_list_editor, face_pick_row, label_row, mm_drag, mono_value, named_f64,
    offset_drag, picked_edges, picked_face, regions_rows, taper_note, use_button,
};

/// The Pad and Pocket editors; `true` when it changed the feature. Any
/// other kind is left alone.
pub(super) fn extrude_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature: &mut DesignFeature,
    first_feature: bool,
) -> bool {
    let mut changed = false;
    match feature {
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
            changed |= face_profile_row(f, ctx, body, profile_face);
            if let (Some(sketch), None) = (*sketch, profile_face.as_ref()) {
                changed |= regions_rows(f, ctx, sketch, regions);
            }
            changed |= extrude_mode_combo(f, mode, first_feature, &borrowed);
            changed |= extrude_side_rows(
                f,
                ctx,
                fx,
                *mode,
                (length, ("Length:", "length")),
                up_to_face,
                (up_to_offset, ("Offset:", "offset")),
                up_to_shape,
                (body, &mut extras.up_to_shape_borrowed),
            );
            if *mode == ExtrudeMode::Dimension && mode2.is_none() {
                changed |= f.check(symmetric, "Symmetric to plane", "");
            }
            if !(*mode == ExtrudeMode::Dimension && *symmetric) {
                changed |= second_side_combo(f, *mode, mode2, first_feature, &borrowed);
                if let (_, Some(second)) = mode.sides(*mode2) {
                    changed |= extrude_side_rows(
                        f,
                        ctx,
                        fx,
                        second,
                        (length2, ("Second length:", "length2")),
                        up_to_face2,
                        (up_to_offset2, ("Second offset:", "offset2")),
                        up_to_shape2,
                        (body, &mut extras.up_to_shape_borrowed2),
                    );
                }
            }
            changed |= extrude_direction_editor(f, ctx, fx, body, direction);
            changed |= f.check(reversed, "Reversed", "");
            changed |= deg_drag(f, fx, taper_deg, ("Taper:", "taper"), -85.0..=85.0);
            taper_note(f, "Positive opens the pad out as it rises.");
            let two_sided = mode.sides(*mode2).1.is_some();
            changed |= extrude_extras_rows(f, ctx, body, fx, extras, two_sided, *direction);
        }
        DesignFeature::Pocket {
            keep_inside: _,
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
            changed |= face_profile_row(f, ctx, body, profile_face);
            if let (Some(sketch), None) = (*sketch, profile_face.as_ref()) {
                changed |= regions_rows(f, ctx, sketch, regions);
            }
            let borrowed = end_targets(ctx, body);
            changed |= extrude_mode_combo(f, mode, first_feature, &borrowed);
            *through_all = *mode == ExtrudeMode::ThroughAll;
            changed |= extrude_side_rows(
                f,
                ctx,
                fx,
                *mode,
                (depth, ("Depth:", "depth")),
                up_to_face,
                (up_to_offset, ("Offset:", "offset")),
                up_to_shape,
                (body, &mut extras.up_to_shape_borrowed),
            );
            if *mode == ExtrudeMode::Dimension && mode2.is_none() {
                changed |= f.check(symmetric, "Symmetric to plane", "");
            }
            if !(*mode == ExtrudeMode::Dimension && *symmetric) {
                changed |= second_side_combo(f, *mode, mode2, first_feature, &borrowed);
                if let (_, Some(second)) = mode.sides(*mode2) {
                    changed |= extrude_side_rows(
                        f,
                        ctx,
                        fx,
                        second,
                        (depth2, ("Second depth:", "depth2")),
                        up_to_face2,
                        (up_to_offset2, ("Second offset:", "offset2")),
                        up_to_shape2,
                        (body, &mut extras.up_to_shape_borrowed2),
                    );
                }
            }
            changed |= extrude_direction_editor(f, ctx, fx, body, direction);
            changed |= f.check(
                reversed,
                "Reversed",
                "Cut along the sketch normal instead of against it",
            );
            changed |= deg_drag(f, fx, taper_deg, ("Taper:", "taper"), -85.0..=85.0);
            taper_note(f, "Positive widens the pocket as it goes deeper.");
            let two_sided = mode.sides(*mode2).1.is_some();
            changed |= extrude_extras_rows(f, ctx, body, fx, extras, two_sided, *direction);
        }
        _ => {}
    }
    changed
}

fn extrude_mode_combo(
    f: &mut Form,
    mode: &mut ExtrudeMode,
    first_feature: bool,
    borrowed: &[(ExtrudeMode, String)],
) -> bool {
    // Material-relative modes need an earlier solid.
    let offered: Vec<(ExtrudeMode, String)> = mode_choices(&ExtrudeMode::ALL, borrowed)
        .into_iter()
        .filter(|(candidate, _)| !(first_feature && needs_own_material(*candidate, borrowed)))
        .collect();
    let at = offered.iter().position(|(c, _)| c == mode);
    let names = offered.iter().map(|(_, n)| n.clone()).collect();
    match choice_shown(f, "Type", mode_name(*mode, borrowed), names, at) {
        Some(i) => {
            *mode = offered[i].0;
            true
        }
        None => false,
    }
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
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    direction: &mut ExtrudeDirection,
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
    let at = candidates
        .iter()
        .position(|(candidate, _)| match (&*direction, candidate) {
            (ExtrudeDirection::Custom(_), ExtrudeDirection::Custom(_))
            | (ExtrudeDirection::Edge(_), ExtrudeDirection::Edge(_))
            | (ExtrudeDirection::Normal, ExtrudeDirection::Normal) => true,
            _ => direction == candidate,
        });
    let names = candidates.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "Direction", shown, names, at) {
        // A picked edge starts from the one picked now.
        *direction = match candidates[i].0 {
            ExtrudeDirection::Edge(_) => match picked_edges(ctx).first() {
                Some(edge) => ExtrudeDirection::Edge(EdgePick::of(edge)),
                None => candidates[i].0,
            },
            other => other,
        };
        changed = true;
    }
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
                let edited = named_f64(
                    f,
                    fx,
                    &mut value,
                    (label, name, Bounds::default()),
                    |f, value| match bare(f, clean(label), *value, 0.05, "") {
                        Some(v) => {
                            *value = v;
                            true
                        }
                        None => false,
                    },
                );
                if edited {
                    *c = value as f32;
                    changed = true;
                }
            }
        }
        ExtrudeDirection::Edge(edge) => {
            let mut pick = Some(*edge);
            if edge_pick_row(f, ctx, &mut pick, "Edge:")
                && let Some(new) = pick
            {
                *edge = new;
                changed = true;
            }
        }
    }
    changed
}

/// Where a feature starts: the profile's own plane, a datum plane's,
/// another sketch's or a flat face of the solid, and the offset from there.
pub(super) fn start_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    fx: &mut Formulas,
    start_at: &mut Option<FeatureId>,
    start_face: &mut Option<FacePick>,
    start_offset: &mut f32,
) -> bool {
    let mut changed = false;
    let mut planes: Vec<(Option<FeatureId>, String)> = vec![(None, "Profile".to_string())];
    planes.extend(
        core_document::datums_of_body(ctx.document, body)
            .into_iter()
            .filter(|(_, _, d)| {
                !matches!(
                    d.shape,
                    core_document::DatumShape::Line { .. } | core_document::DatumShape::Point
                )
            })
            .map(|(id, name, _)| (Some(id), name)),
    );
    planes.extend(
        crate::build::sketches_of_body(ctx.document, body)
            .into_iter()
            .map(|(id, name)| (Some(id), name)),
    );
    let shown = if start_face.is_some() {
        "Face".to_string()
    } else {
        planes
            .iter()
            .find(|(id, _)| id == start_at)
            .map_or_else(|| "(gone)".to_string(), |(_, name)| name.clone())
    };
    // A flat face picked in the view.
    let flat = picked_face(ctx).filter(|f| {
        f.surface
            .is_none_or(|s| matches!(s, kernel_api::FaceSurface::Plane { .. }))
    });
    let at = if start_face.is_some() {
        planes.len()
    } else {
        planes
            .iter()
            .position(|(id, _)| id == start_at)
            .unwrap_or(planes.len() + 1)
    };
    let mut options: Vec<String> = planes.iter().map(|(_, n)| n.clone()).collect();
    options.push("Selected face".to_string());
    let mut hints = vec![String::new(); planes.len()];
    hints.push("Click a flat face in the viewport first".to_string());
    let picked = f.select(
        "Start at",
        Select {
            label: "Start at".into(),
            options,
            selected: at,
            width: Some(CHOICE_WIDTH),
            text: Some(shown),
            disabled: if flat.is_some() {
                Vec::new()
            } else {
                vec![planes.len()]
            },
            hints,
            ..Default::default()
        },
    );
    match picked {
        Some(i) if i < planes.len() => {
            *start_at = planes[i].0;
            *start_face = None;
            changed = true;
        }
        Some(_) => {
            if let Some(face) = &flat {
                *start_face = Some(FacePick::of(face));
                *start_at = None;
                changed = true;
            }
        }
        None => {}
    }
    if start_face.is_some() {
        changed |= face_pick_row(f, ctx, start_face, "Start face:");
    }
    changed |= offset_drag(f, fx, start_offset, ("Start offset:", "start_offset"));
    changed
}

/// A pad's or pocket's less used settings: where it starts, the second
/// side's own taper, and a slanted length measured along the normal.
fn extrude_extras_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    fx: &mut Formulas,
    extras: &mut crate::feature::ExtrudeExtras,
    two_sided: bool,
    direction: ExtrudeDirection,
) -> bool {
    let mut changed = start_rows(
        f,
        ctx,
        body,
        fx,
        &mut extras.start_at,
        &mut extras.start_face,
        &mut extras.start_offset,
    );
    if two_sided {
        let mut own = extras.taper2_deg.is_some();
        if f.check(&mut own, "Second side's own taper", "") {
            extras.taper2_deg = own.then_some(0.0);
            changed = true;
        }
        if let Some(taper) = &mut extras.taper2_deg {
            changed |= deg_drag(f, fx, taper, ("Second taper:", ""), -85.0..=85.0);
        }
    }
    if direction != ExtrudeDirection::Normal {
        changed |= f.check(
            &mut extras.along_normal,
            "Length along the normal",
            "With a slanted direction, the length is measured square to the profile",
        );
    }
    changed
}

/// The rows of one side's end condition, beyond its mode: the length, the
/// face or the faces it stops on and their offset.
#[expect(clippy::too_many_arguments)]
fn extrude_side_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    mode: ExtrudeMode,
    length: (&mut f32, (&str, &str)),
    face: &mut Option<FacePick>,
    offset: (&mut f32, (&str, &str)),
    shape: &mut Vec<FacePick>,
    (body, borrowed): (BodyId, &mut Vec<crate::feature::BorrowedRef>),
) -> bool {
    let mut changed = false;
    match mode {
        ExtrudeMode::Dimension | ExtrudeMode::TwoLengths => {
            changed |= mm_drag(f, fx, length.0, length.1);
        }
        ExtrudeMode::UpToFace => {
            changed |= face_pick_row(f, ctx, face, "Target face:");
            changed |= offset_drag(f, fx, offset.0, offset.1);
        }
        ExtrudeMode::UpToBorrowed(_) | ExtrudeMode::UpToPlane(_) => {
            changed |= offset_drag(f, fx, offset.0, offset.1);
        }
        ExtrudeMode::UpToShape => {
            changed |= face_list_editor(f, ctx, shape, "Stop faces:");
            changed |= borrowed_face_checks(f, ctx, body, borrowed);
            changed |= offset_drag(f, fx, offset.0, offset.1);
        }
        _ => {}
    }
    changed
}

/// A check row for each face `body` borrows: the ones ticked are stop
/// faces too.
fn borrowed_face_checks(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    borrowed: &mut Vec<crate::feature::BorrowedRef>,
) -> bool {
    let faces = crate::borrow::faces_of_body(ctx.document, body);
    if faces.is_empty() {
        return false;
    }
    let mut changed = false;
    label_row(f, "Borrowed stop faces:");
    for (r, name) in faces {
        let mut on = borrowed.contains(&r);
        if f.check(&mut on, &name, "") {
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
    f: &mut Form,
    first: ExtrudeMode,
    mode2: &mut Option<ExtrudeMode>,
    first_feature: bool,
    borrowed: &[(ExtrudeMode, String)],
) -> bool {
    let required = first == ExtrudeMode::TwoLengths;
    let shown = match *mode2 {
        None if required => ExtrudeMode::Dimension.label().to_string(),
        None => "None".to_string(),
        Some(mode) => mode_name(mode, borrowed),
    };
    let mut offered: Vec<(Option<ExtrudeMode>, String)> = Vec::new();
    if !required {
        offered.push((None, "None".to_string()));
    }
    for (candidate, name) in mode_choices(&ExtrudeMode::SECOND_SIDE, borrowed) {
        if first_feature && needs_own_material(candidate, borrowed) {
            continue;
        }
        offered.push((Some(candidate), name));
    }
    let at = offered.iter().position(|(c, _)| {
        *c == *mode2 || (required && mode2.is_none() && *c == Some(ExtrudeMode::Dimension))
    });
    let names = offered.iter().map(|(_, n)| n.clone()).collect();
    match choice_shown(f, "Second side", shown, names, at) {
        Some(i) => {
            *mode2 = offered[i].0;
            true
        }
        None => false,
    }
}

/// A pad's or a pocket's face profile, when it has one: where it is, and a
/// flat face picked in the view to take its place.
fn face_profile_row(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    profile_face: &mut Option<FacePick>,
) -> bool {
    let Some(face) = profile_face else {
        return false;
    };
    f.flow("Profile", true, |f| {
        mono_value(f, &format!("face at {}", coords(face.point)), true);
    });
    // Its own line: beside the face's position it runs past a narrow
    // panel.
    let picked = ctx.selected_face_in(body).filter(|f| {
        matches!(
            f.surface,
            None | Some(kernel_api::FaceSurface::Plane { .. })
        )
    });
    let clicked = use_button(
        f,
        "Use selected face",
        picked.is_some(),
        "Click another flat face of the solid first, then press this",
    );
    match picked {
        Some(pick) if clicked => {
            *face = FacePick::of(pick);
            true
        }
        _ => false,
    }
}
