//! The task panels that change a solid's faces and edges: Fillet,
//! Chamfer, Draft, Offset and Move faces, Offset solid, Scale, Delete
//! faces and Thickness, with the edge and face pickers only they use.

use bench_api::Select;
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{ChamferMode, DesignFeature, EdgePick, EdgeSel, ExtrudeMode, FacePick};
use crate::form::Form;

use super::{
    CHOICE_WIDTH, Formulas, Kind, choice_shown, deg_drag, end_targets, face_list_editor,
    face_pick_row, label_row, mm_drag, mm_drag_upto, number, number_drag, pick, picked_edges,
    picked_row, set_f32, signed_mm, small_button, taper_note, use_button,
};

/// The Fillet, Chamfer, Draft, Offset faces, Offset solid, Scale, Move
/// faces, Delete faces and Thickness editors; `true` when it changed the
/// feature. Any other kind is left alone.
pub(super) fn dressup_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    match feature {
        DesignFeature::Fillet {
            radius,
            edges,
            follow_tangent,
        } => {
            let most = crate::size_limit::drag_limits(ctx.document, feature_id);
            changed |= mm_drag_upto(f, fx, radius, ("Radius:", "radius"), most.map(|m| m.first));
            changed |= edge_sel_editor(f, ctx, edges, true);
            if !matches!(edges, EdgeSel::BetweenFaces(_)) {
                changed |= tangent_row(f, follow_tangent);
            }
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
            changed |= pick(f, "Type", &ChamferMode::ALL, mode, |candidate| {
                candidate.label()
            });
            let most = crate::size_limit::drag_limits(ctx.document, feature_id);
            changed |= mm_drag_upto(f, fx, size, ("Size:", "size"), most.map(|m| m.first));
            match mode {
                ChamferMode::EqualDistance => {}
                ChamferMode::TwoDistances => {
                    let most = most.and_then(|m| m.second);
                    changed |= mm_drag_upto(f, fx, size2, ("Size 2:", "size2"), most);
                    changed |= f.check(flip, "Flip direction", "");
                }
                ChamferMode::DistanceAngle => {
                    changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 1.0..=89.0);
                    changed |= f.check(flip, "Flip direction", "");
                }
            }
            changed |= edge_sel_editor(f, ctx, edges, false);
            changed |= tangent_row(f, follow_tangent);
        }
        DesignFeature::Draft {
            angle_deg,
            neutral,
            faces,
            reversed,
            neutral_plane,
            pull,
        } => {
            changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 0.1..=45.0);
            changed |= draft_references_editor(f, ctx, body, neutral, neutral_plane, pull);
            changed |= face_list_editor(f, ctx, faces, "Faces to draft:");
            changed |= f.check(reversed, "Reversed pull", "");
        }
        DesignFeature::OffsetFaces { faces, distance } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to offset:");
            changed |= signed_mm(f, fx, distance, "Distance", "distance");
            taper_note(
                f,
                "Along each face's outward normal; negative moves it into the material. A \
                 bore's face offset in widens it.",
            );
        }
        DesignFeature::OffsetSolid { distance } => {
            changed |= signed_mm(f, fx, distance, "Distance", "distance");
            taper_note(
                f,
                "Every face moves this far along its outward normal, the faces around it \
                 following: positive grows the solid, negative shrinks it. A box grows twice \
                 the distance each way; a hole in it narrows.",
            );
        }
        DesignFeature::Scale {
            factor,
            per_axis,
            factors,
            about,
        } => {
            changed |= scale_editor(f, ctx, fx, body, factor, per_axis, factors, about);
        }
        DesignFeature::MoveFaces {
            faces,
            translation,
            angle_deg,
            axis_point,
            axis_dir,
        } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to move:");
            for (i, (label, name)) in [("Move X", "x"), ("Move Y", "y"), ("Move Z", "z")]
                .into_iter()
                .enumerate()
            {
                changed |= signed_mm(f, fx, &mut translation[i], label, name);
            }
            changed |= deg_drag(f, fx, angle_deg, ("Angle", "angle"), -360.0..=360.0);
            if angle_deg.abs() > 1e-6 {
                for (i, label) in ["Axis point X", "Axis point Y", "Axis point Z"]
                    .into_iter()
                    .enumerate()
                {
                    let set = number(f, label, f64::from(axis_point[i]), Kind::Offset, 0.1);
                    changed |= set_f32(&mut axis_point[i], set);
                }
                for (i, label) in ["Axis X", "Axis Y", "Axis Z"].into_iter().enumerate() {
                    let set = number(f, label, f64::from(axis_dir[i]), Kind::Plain, 0.05);
                    changed |= set_f32(&mut axis_dir[i], set);
                }
            }
        }
        DesignFeature::DeleteFaces { faces } => {
            changed |= face_list_editor(f, ctx, faces, "Faces to delete:");
            taper_note(
                f,
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
            changed |= mm_drag(f, fx, value, ("Thickness:", "thickness"));
            changed |= face_list_editor(f, ctx, faces, "Faces to open:");
            changed |= f.check(
                both_sides,
                "Both sides",
                "Walls on both sides of the faces, the thickness each way",
            );
            if !*both_sides {
                changed |= f.check(inward, "Inward", "");
            }
            changed |= pick(
                f,
                "Join",
                &kernel_api::ThicknessJoin::ALL,
                join,
                |candidate| candidate.label(),
            );
        }
        _ => {}
    }
    changed
}

/// What a dress-up rounds or bevels: every edge, the edges of picked
/// faces, picked edges, or (a fillet's, when `between` is set) the round
/// between two picked faces.
fn edge_sel_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    edges: &mut EdgeSel,
    between: bool,
) -> bool {
    let mut changed = false;
    let shown = match edges {
        EdgeSel::All => "All edges".to_string(),
        EdgeSel::Faces(f) => format!("{} face(s)", f.len()),
        EdgeSel::Edges(e) => format!("{} edge(s)", e.len()),
        EdgeSel::BetweenFaces(f) => format!("Between {} of 2 faces", f.len()),
    };
    let mut options = vec![
        "All edges".to_string(),
        "Edges of picked faces".to_string(),
        "Picked edges".to_string(),
    ];
    let mut hints = vec![String::new(); 3];
    if between {
        options.push("Between two faces".to_string());
        hints.push(
            "The round between two faces that need share no edge, \
             such as a step across a groove"
                .to_string(),
        );
    }
    let at = match edges {
        EdgeSel::All => 0,
        EdgeSel::Faces(_) => 1,
        EdgeSel::Edges(_) => 2,
        EdgeSel::BetweenFaces(_) => 3,
    };
    let picked = f.select(
        "Edges",
        Select {
            label: "Edges".into(),
            options,
            selected: at,
            width: Some(CHOICE_WIDTH),
            text: Some(shown),
            hints,
            ..Default::default()
        },
    );
    match picked {
        Some(0) => {
            *edges = EdgeSel::All;
            changed = true;
        }
        Some(1) => {
            *edges = EdgeSel::Faces(faces_of_selection(edges));
            changed = true;
        }
        Some(2) => {
            *edges = EdgeSel::Edges(Vec::new());
            changed = true;
        }
        Some(3) => {
            let mut faces = faces_of_selection(edges);
            faces.truncate(2);
            *edges = EdgeSel::BetweenFaces(faces);
            changed = true;
        }
        _ => {}
    }
    match edges {
        EdgeSel::Faces(faces) => changed |= face_list_editor(f, ctx, faces, "Faces:"),
        EdgeSel::Edges(picks) => changed |= edge_list_editor(f, ctx, picks),
        EdgeSel::BetweenFaces(faces) => {
            changed |= face_list_editor(f, ctx, faces, "Faces:");
            if faces.len() > 2 {
                faces.drain(..faces.len() - 2);
            }
        }
        EdgeSel::All => {}
    }
    changed
}

/// The faces a selection picked, kept when it changes kind.
fn faces_of_selection(edges: &EdgeSel) -> Vec<crate::feature::FacePick> {
    match edges {
        EdgeSel::Faces(faces) | EdgeSel::BetweenFaces(faces) => faces.clone(),
        EdgeSel::All | EdgeSel::Edges(_) => Vec::new(),
    }
}

/// The switch that makes a dress-up take the edges tangent to its own.
fn tangent_row(f: &mut Form, follow_tangent: &mut bool) -> bool {
    f.check(
        follow_tangent,
        "Follow tangent edges",
        "Take every edge that runs on smoothly from a selected one, \
             such as the round of a filleted corner and the side past it",
    )
}

/// The picked edges, each removable, and a button that adds whatever is
/// picked in the viewport.
fn edge_list_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    picks: &mut Vec<EdgePick>,
) -> bool {
    let mut changed = false;
    label_row(f, "Edges:");
    let mut remove = None;
    for (i, pick) in picks.iter().enumerate() {
        if picked_row(f, "line", "Edge", i, pick.point) {
            remove = Some(i);
        }
    }
    if let Some(i) = remove {
        picks.remove(i);
        changed = true;
    }
    let has_selection = !ctx.selected_edges.is_empty();
    if use_button(
        f,
        "Add selected edges",
        has_selection,
        "Click edges in the viewport first (Ctrl adds), then press this",
    ) {
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

/// A draft's neutral plane (a picked face, one of the body's planes or a
/// datum plane) and what it pulls along (the neutral plane's normal, a
/// picked edge or a datum line).
fn draft_references_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
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
    let mut options = vec!["Picked face".to_string()];
    options.extend(planes.iter().map(|(_, n)| n.clone()));
    let at = match neutral_plane {
        None => Some(0),
        Some(target) => planes.iter().position(|(t, _)| t == target).map(|i| i + 1),
    };
    match choice_shown(f, "Neutral plane", shown, options, at) {
        Some(0) => {
            *neutral_plane = None;
            changed = true;
        }
        Some(i) => {
            *neutral_plane = Some(planes[i - 1].0);
            changed = true;
        }
        None => {}
    }
    if neutral_plane.is_none() {
        let mut neutral_opt = Some(*neutral);
        if face_pick_row(f, ctx, &mut neutral_opt, "Neutral face:")
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
    let picked = picked_edges(ctx).first().map(EdgePick::of);
    let is_edge = matches!(pull, Some(PullRef::Edge(_)));
    let mut options = vec![
        "Neutral plane's normal".to_string(),
        "Picked edge".to_string(),
    ];
    options.extend(lines.iter().map(|(_, n)| n.clone()));
    let mut hints = vec![String::new(), "Click a straight edge first".to_string()];
    hints.resize(options.len(), String::new());
    let at = match pull {
        None => Some(0),
        Some(PullRef::Edge(_)) => Some(1),
        Some(PullRef::Datum(id)) => lines.iter().position(|(l, _)| l == id).map(|i| i + 2),
    };
    let chosen = f.select(
        "Pull",
        Select {
            label: "Pull".into(),
            selected: at.unwrap_or(options.len()),
            options,
            width: Some(CHOICE_WIDTH),
            text: Some(shown),
            disabled: if picked.is_some() || is_edge {
                Vec::new()
            } else {
                vec![1]
            },
            hints,
            ..Default::default()
        },
    );
    match chosen {
        Some(0) => {
            *pull = None;
            changed = true;
        }
        Some(1) => {
            if let Some(edge) = picked {
                *pull = Some(PullRef::Edge(edge));
                changed = true;
            }
        }
        Some(i) => {
            *pull = Some(PullRef::Datum(lines[i - 2].0));
            changed = true;
        }
        None => {}
    }
    changed
}

/// A Scale's factors (one, or one per axis), the point it scales about
/// and the unit presets.
#[allow(clippy::too_many_arguments)]
fn scale_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    factor: &mut f32,
    per_axis: &mut bool,
    factors: &mut [f32; 3],
    about: &mut crate::feature::ScaleCentre,
) -> bool {
    use crate::feature::ScaleCentre;
    let mut changed = f.check(
        per_axis,
        "Per axis",
        "A factor along each of the body's X, Y and Z",
    );
    if *per_axis {
        for (value, label) in factors.iter_mut().zip([
            ("Factor X", "factor_x"),
            ("Factor Y", "factor_y"),
            ("Factor Z", "factor_z"),
        ]) {
            changed |= number_drag(f, fx, value, label);
        }
    } else {
        changed |= number_drag(f, fx, factor, ("Factor", "factor"));
    }
    f.flow("Presets", false, |f| {
        for (label, value, hint) in [
            (
                "inch → mm",
                25.4,
                "A part modelled in inches, made millimetres",
            ),
            (
                "mm → inch",
                1.0 / 25.4,
                "A part modelled in millimetres, made inches",
            ),
        ] {
            if small_button(f, label, hint) {
                *per_axis = false;
                *factor = value;
                changed = true;
            }
        }
    });
    let mut centres: Vec<(ScaleCentre, String)> = [ScaleCentre::Origin, ScaleCentre::BodyCentre]
        .map(|c| (c, c.label().to_string()))
        .into();
    let datums = core_document::datums_of_body(ctx.document, body)
        .into_iter()
        .map(|(datum, name, _)| (ScaleCentre::Datum { datum }, name));
    centres.extend(datums);
    let shown = centres
        .iter()
        .skip(2)
        .find(|(c, _)| c == about)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| about.label().to_string());
    let at = centres.iter().position(|(c, _)| c == about);
    let names = centres.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "About", shown, names, at) {
        *about = centres[i].0;
        changed = true;
    }
    taper_note(
        f,
        "The whole solid scales about the point: the body's origin, the middle of its \
         bounding box, or a datum's origin. Factors above 1 enlarge it.",
    );
    changed
}
