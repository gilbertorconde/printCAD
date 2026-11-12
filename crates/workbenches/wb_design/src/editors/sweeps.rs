//! The Revolution, Groove, Loft, Pipe and Helix task panels: the axis
//! turned about, the sections lofted through, the path swept along and
//! how the profile turns on it.

use bench_api::Select;
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{
    DesignFeature, EdgePick, FacePick, HelixMode, PipeCorner, PipeOrientation, RevolveAxis,
    RevolveMode,
};
use crate::form::Form;

use super::{
    Bounds, CHOICE_WIDTH, Formulas, Kind, bare, choice_shown, coords, deg_drag, edge_pick_row,
    face_pick_row, label_row, mm_drag, mono_value, number_drag, offset_drag, pick, picked_edges,
    quantity, regions_rows, set_f32, sketch_combo, small_button, use_button,
};

/// The Revolution, Groove, Loft, Pipe and Helix editors; `true` when it
/// changed the feature. Any other kind is left alone.
pub(super) fn sweep_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
    first_feature: bool,
) -> bool {
    let mut changed = false;
    match feature {
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
            start_angle_deg,
            mode,
            up_to_face,
        }
        | DesignFeature::Groove {
            keep_inside: _,
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
            start_angle_deg,
            mode,
            up_to_face,
        } => {
            changed |= regions_rows(f, ctx, *sketch, regions);
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
            let offered: Vec<(RevolveMode, String)> = RevolveMode::ALL
                .iter()
                .map(|m| (*m, m.label().to_string()))
                .chain(targets.iter().cloned())
                // Every mode but an angle or a borrowed face stops on the body's own
                // material.
                .filter(|(candidate, _)| {
                    !first_feature
                        || matches!(candidate, RevolveMode::Angle | RevolveMode::UpToBorrowed(_))
                })
                .collect();
            let at = offered.iter().position(|(c, _)| c == mode);
            let names = offered.iter().map(|(_, n)| n.clone()).collect();
            if let Some(i) = choice_shown(f, "Type", shown, names, at) {
                *mode = offered[i].0;
                changed = true;
            }
            match mode {
                RevolveMode::Angle => {
                    changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 0.1..=360.0);
                }
                RevolveMode::UpToFace => {
                    changed |= face_pick_row(f, ctx, up_to_face, "Target face:");
                }
                RevolveMode::ToFirst | RevolveMode::ToLast | RevolveMode::UpToBorrowed(_) => {}
            }
            changed |= revolve_axis_editor(f, ctx, body, *sketch, axis, false);
            changed |= project_axis_row(f, axis, project_axis);
            if *mode == RevolveMode::Angle {
                changed |= f.check(midplane, "Midplane", "");
                let mut two_sided = second_angle_deg.is_some();
                if f.check(&mut two_sided, "Second angle", "") {
                    *second_angle_deg = two_sided.then_some(90.0);
                    changed = true;
                }
                if let Some(second) = second_angle_deg {
                    changed |= deg_drag(f, fx, second, ("Angle 2:", "angle2"), 0.1..=360.0);
                }
            }
            changed |= f.check(reversed, "Reversed", "");
            changed |= deg_drag(
                f,
                fx,
                start_angle_deg,
                ("Start angle:", "start_angle"),
                -360.0..=360.0,
            );
        }
        DesignFeature::Loft {
            keep_inside: _,
            refine: _,
            boolean_tolerance: _,
            sections,
            ruled,
            closed,
            subtractive,
        } => {
            use crate::feature::LoftSection;
            label_row(f, "Sections (in order)");
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
                f.flow("", false, |f| {
                    mono_value(f, &format!("{}. {name}", i + 1), true);
                    if small_button(f, "×", "") && sections.len() > 1 {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                sections.remove(i);
                changed = true;
            }
            if let Some(new) = sketch_combo(f, ctx, body, feature_id, None, "Add section:")
                && !sections.contains(&LoftSection::Feature(new))
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
                let names = points.iter().map(|(_, n)| n.clone()).collect();
                if let Some(i) = choice_shown(f, "Add point:", "A datum point…".into(), names, None)
                {
                    let section = LoftSection::Feature(points[i].0);
                    if !sections.contains(&section) {
                        sections.push(section);
                        changed = true;
                    }
                }
            }
            let picked = ctx.selected_face_in(body).map(FacePick::of);
            f.flow("Add face", false, |f| {
                if use_button(
                    f,
                    "Use selected face",
                    picked.is_some(),
                    "Click a flat face of the solid first, then press this",
                ) && let Some(pick) = picked
                {
                    sections.push(LoftSection::Face(pick));
                    changed = true;
                }
            });
            changed |= f.check(ruled, "Ruled (straight transitions)", "");
            changed |= f.check(closed, "Closed (loop back)", "");
            changed |= f.check(subtractive, "Subtractive", "");
        }
        DesignFeature::Pipe {
            keep_inside: _,
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
            path_curves,
        } => {
            changed |= pipe_inputs_editor(
                f,
                ctx,
                (body, feature_id),
                spine,
                profile_face,
                (path_edges, path_borrowed, path_curves),
            );
            changed |= pipe_orientation_editor(f, ctx, fx, (body, feature_id), *spine, orientation);
            changed |= pick(f, "Corners", &PipeCorner::ALL, corner, |candidate| {
                candidate.label()
            });
            label_row(f, "Sections along the path (in order)");
            let mut remove = None;
            for (i, section) in sections.iter().enumerate() {
                let name = ctx
                    .document
                    .get_feature_meta(*section)
                    .map(|n| n.name.clone())
                    .unwrap_or_else(|| "(missing)".into());
                f.flow("", false, |f| {
                    mono_value(f, &format!("{}. {name}", i + 1), true);
                    if small_button(f, "×", "") {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                sections.remove(i);
                changed = true;
            }
            if let Some(new) = sketch_combo(f, ctx, body, feature_id, None, "Add section:")
                && new != *profile
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
                let names = points.iter().map(|(_, n)| n.clone()).collect();
                if let Some(i) =
                    choice_shown(f, "End at point:", "A datum point…".into(), names, None)
                    && !sections.contains(&points[i].0)
                {
                    sections.push(points[i].0);
                    changed = true;
                }
            }
            changed |= f.check(subtractive, "Subtractive", "");
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
            changed |= revolve_axis_editor(f, ctx, body, *sketch, axis, true);
            changed |= project_axis_row(f, axis, project_axis);
            changed |= pick(f, "Mode", &HelixMode::ALL, mode, |candidate| {
                candidate.label()
            });
            match mode {
                HelixMode::PitchHeight => {
                    changed |= mm_drag(f, fx, pitch, ("Pitch:", "pitch"));
                    changed |= mm_drag(f, fx, height, ("Height:", "height"));
                }
                HelixMode::PitchTurns => {
                    changed |= mm_drag(f, fx, pitch, ("Pitch:", "pitch"));
                    changed |= turns_field(f, fx, turns);
                }
                HelixMode::HeightTurns => {
                    changed |= mm_drag(f, fx, height, ("Height:", "height"));
                    changed |= turns_field(f, fx, turns);
                }
                HelixMode::HeightTurnsGrowth => {
                    // A height of 0 is a flat spiral; below it is nothing.
                    if offset_drag(f, fx, height, ("Height:", "height")) {
                        *height = height.max(0.0);
                        changed = true;
                    }
                    changed |= turns_field(f, fx, turns);
                    changed |= offset_drag(f, fx, growth, ("Growth:", "growth"));
                }
            }
            if *mode != HelixMode::HeightTurnsGrowth {
                changed |= deg_drag(
                    f,
                    fx,
                    cone_angle_deg,
                    ("Cone angle:", "cone_angle"),
                    -85.0..=85.0,
                );
            }
            changed |= f.check(left_handed, "Left handed", "");
            changed |= f.check(reversed, "Reversed", "");
            changed |= f.check(subtractive, "Subtractive", "");
            if *subtractive {
                changed |= f.check(
                    keep_inside,
                    "Keep inside",
                    "Keep what the helix shares with the body instead of cutting it",
                );
            }
        }
        _ => {}
    }
    changed
}

/// A helix's turns, a formula field where it has the parameter.
fn turns_field(f: &mut Form, fx: &mut Formulas, turns: &mut f32) -> bool {
    let bounds = Bounds::within(0.1, 1000.0);
    if let Some((changed, v)) = fx.show_within(f, ("Turns:", "turns"), f64::from(*turns), bounds) {
        *turns = (v as f32).clamp(0.1, 1000.0);
        return changed;
    }
    let mut q = quantity("Turns", f64::from(*turns), Kind::Plain, 0.1);
    (q.min, q.max) = (Some(0.1), Some(1000.0));
    q.bare = true;
    match f.quantity(q) {
        Some(v) => {
            *turns = v as f32;
            true
        }
        None => false,
    }
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
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    sketch: FeatureId,
    axis: &mut RevolveAxis,
    helix: bool,
) -> bool {
    let mut changed = false;
    let references = axis_choices(ctx, body, sketch);
    let shown = references
        .iter()
        .find(|(choice, _)| choice == axis)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| axis.label().to_string());
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
    let at = candidates
        .iter()
        .position(|(candidate, _)| match (&*axis, candidate) {
            (RevolveAxis::SketchLine(_), RevolveAxis::SketchLine(_))
            | (RevolveAxis::Datum(_), RevolveAxis::Datum(_))
            | (RevolveAxis::Borrowed(_), RevolveAxis::Borrowed(_))
            | (RevolveAxis::Base(_), RevolveAxis::Base(_)) => axis == candidate,
            _ => std::mem::discriminant(axis) == std::mem::discriminant(candidate),
        });
    let names = candidates.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "Axis", shown, names, at) {
        *axis = candidates[i].0;
        changed = true;
    }
    if let RevolveAxis::Edge(edge) = axis {
        let mut pick = Some(*edge);
        if edge_pick_row(f, ctx, &mut pick, "Edge:")
            && let Some(new) = pick
        {
            *edge = new;
            changed = true;
        }
    }
    if let RevolveAxis::Custom { origin, dir } = axis {
        f.flow("", false, |f| {
            f.flow("Origin", false, |f| {
                for v in origin.iter_mut() {
                    changed |= set_f32(v, bare(f, "", f64::from(*v), 0.5, ""));
                }
            });
            f.flow("Dir", false, |f| {
                for v in dir.iter_mut() {
                    changed |= set_f32(v, bare(f, "", f64::from(*v), 0.1, ""));
                }
            });
        });
    }
    changed
}

/// The axis taken as its shadow on the sketch plane, offered for the axes
/// from outside the sketch, which can stand tilted to it.
fn project_axis_row(f: &mut Form, axis: &RevolveAxis, project_axis: &mut bool) -> bool {
    matches!(
        axis,
        RevolveAxis::Edge(_)
            | RevolveAxis::Datum(_)
            | RevolveAxis::Borrowed(_)
            | RevolveAxis::Base(_)
    ) && f.check(
        project_axis,
        "Project onto the sketch",
        "An axis tilted to the sketch plane turns about its shadow on the plane",
    )
}

/// What a pipe sweeps and along what: its profile a sketch or a picked
/// face, its path a sketch (all its curves or those ticked), picked edges
/// of the solid or edges another body lends.
fn pipe_inputs_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    (body, feature_id): (BodyId, FeatureId),
    spine: &mut FeatureId,
    profile_face: &mut Option<FacePick>,
    (path_edges, path_borrowed, path_curves): (
        &mut Vec<EdgePick>,
        &mut Vec<crate::feature::BorrowedRef>,
        &mut Vec<uuid::Uuid>,
    ),
) -> bool {
    let mut changed = false;
    f.flow("Profile", true, |f| {
        let shown = match profile_face {
            Some(p) => format!("face at {}", coords(p.point)),
            None => "its sketch".to_string(),
        };
        mono_value(f, &shown, true);
        let picked = ctx.selected_face_in(body).map(FacePick::of);
        if use_button(
            f,
            "Use selected face",
            picked.is_some(),
            "Click a flat face of the solid first, then press this",
        ) && let Some(pick) = picked
        {
            *profile_face = Some(pick);
            changed = true;
        }
        if profile_face.is_some() && small_button(f, "Use the sketch", "") {
            *profile_face = None;
            changed = true;
        }
    });
    let along_edges = !path_edges.is_empty() || !path_borrowed.is_empty();
    if !along_edges && let Some(new) = sketch_combo(f, ctx, body, feature_id, Some(*spine), "Path:")
    {
        *spine = new;
        path_curves.clear();
        changed = true;
    }
    if !along_edges {
        changed |= path_curves_editor(f, ctx, *spine, path_curves);
    }
    f.flow("Path edges", true, |f| {
        let shown = if !path_edges.is_empty() {
            format!("{} of the solid", path_edges.len())
        } else if !path_borrowed.is_empty() {
            format!("{} borrowed", path_borrowed.len())
        } else {
            "none: the path sketch".to_string()
        };
        mono_value(f, &shown, true);
        let picked: Vec<EdgePick> = picked_edges(ctx).iter().map(EdgePick::of).collect();
        if use_button(
            f,
            "Use selected edges",
            !picked.is_empty(),
            "Click the edges of the solid in turn (Ctrl adds), then press this",
        ) {
            *path_edges = picked;
            path_borrowed.clear();
            changed = true;
        }
        if along_edges && small_button(f, "Use the sketch", "") {
            path_edges.clear();
            path_borrowed.clear();
            changed = true;
        }
    });
    for (r, name) in &crate::borrow::edges_of_body(ctx.document, body) {
        let mut on = path_borrowed.contains(r);
        if f.check(&mut on, &format!("Along {name}"), "") {
            if on {
                path_borrowed.push(*r);
                path_edges.clear();
            } else {
                path_borrowed.retain(|b| b != r);
            }
            changed = true;
        }
    }
    changed
}

/// The curves of the path sketch a pipe runs along: a row for each, every
/// one ticked while the pipe takes the whole sketch. The last one ticked
/// stays.
fn path_curves_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    spine: FeatureId,
    path_curves: &mut Vec<uuid::Uuid>,
) -> bool {
    use wb_sketch::sketch::GeometryElement;
    let Ok(drawn) = crate::build::load_sketch(ctx.document, spine) else {
        return false;
    };
    let sketch = &drawn.sketch;
    let curves: Vec<uuid::Uuid> = sketch
        .geometry
        .iter()
        .filter(|e| !matches!(e, GeometryElement::Point(_)) && !sketch.is_construction(e.id()))
        .map(|e| e.id())
        .collect();
    let mut changed = false;
    f.flow("Path curves", true, |f| {
        let shown = if path_curves.is_empty() {
            "the whole sketch".to_string()
        } else {
            format!("{} of {}", path_curves.len(), curves.len())
        };
        mono_value(f, &shown, true);
        if !path_curves.is_empty() && small_button(f, "Whole sketch", "") {
            path_curves.clear();
            changed = true;
        }
    });
    if curves.len() < 2 {
        return changed;
    }
    for id in &curves {
        let mut on = path_curves.is_empty() || path_curves.contains(id);
        let name = wb_sketch::style::element_name(sketch, *id);
        if f.check(&mut on, &name, "") {
            let taken: Vec<uuid::Uuid> = curves
                .iter()
                .copied()
                .filter(|c| {
                    if c == id {
                        on
                    } else {
                        path_curves.is_empty() || path_curves.contains(c)
                    }
                })
                .collect();
            if taken.is_empty() {
                continue;
            }
            *path_curves = if taken.len() == curves.len() {
                Vec::new()
            } else {
                taken
            };
            changed = true;
        }
    }
    changed
}

/// A pipe's orientation: which kind, and the path or direction it takes.
fn pipe_orientation_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    (body, feature_id): (BodyId, FeatureId),
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
    let at = kinds
        .iter()
        .position(|k| std::mem::discriminant(orientation) == std::mem::discriminant(k));
    let hint = "Standard keeps the section from twisting; Frenet turns it with the \
                path's curvature; an auxiliary path turns it to face a second path; \
                a binormal holds one direction of it fixed; fixed keeps it as drawn";
    let picked = f.select_hinted(
        "Orientation",
        Select {
            label: "Orientation".into(),
            options: kinds.iter().map(|k| k.label().to_string()).collect(),
            selected: at.unwrap_or(kinds.len()),
            width: Some(CHOICE_WIDTH),
            text: Some(orientation.label().to_string()),
            ..Default::default()
        },
        hint,
    );
    if let Some(i) = picked {
        *orientation = kinds[i];
        changed = true;
    }
    match orientation {
        PipeOrientation::Auxiliary { path } => {
            if let Some(new) =
                sketch_combo(f, ctx, body, feature_id, Some(*path), "Auxiliary path:")
            {
                *path = new;
                changed = true;
            }
        }
        PipeOrientation::Binormal { x, y, z } => {
            for (value, label) in [
                (x, ("Binormal X:", "binormal_x")),
                (y, ("Binormal Y:", "binormal_y")),
                (z, ("Binormal Z:", "binormal_z")),
            ] {
                changed |= number_drag(f, fx, value, label);
            }
        }
        PipeOrientation::Standard | PipeOrientation::Frenet | PipeOrientation::Fixed => {}
    }
    changed
}
