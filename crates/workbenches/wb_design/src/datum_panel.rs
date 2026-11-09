//! The datum task's attachment: the mode selector and a row for each
//! reference the mode takes, filled from what is picked in the viewport.

use bench_api::{ButtonStyle, GapSize, NoteKind, Quantity, Select};
use core_document::{
    BasePlane, BodyId, DatumAttachment, DatumFeature, DatumShape, EdgeAnchor, EdgeSpot, FeatureId,
    FrameAnchor, LineAnchor, PlaneAnchor, PointAnchor, WorkbenchFeature, WorkbenchRuntimeContext,
};

use crate::editors::{CHOICE_WIDTH, LABEL_BLANK};
use crate::form::Form;
use core_document::attach::{
    PICK_MODES as MODES, Picked, candidate, current_edge, mode_label, settle,
};

fn coords(p: [f32; 3]) -> String {
    format!("({:.1}, {:.1}, {:.1})", p[0], p[1], p[2])
}

/// A choice in the label column; the option picked.
fn choice(f: &mut Form, label: &str, select: Select) -> Option<usize> {
    f.select(
        label,
        Select {
            label: label.into(),
            width: Some(CHOICE_WIDTH),
            ..select
        },
    )
}

/// A reference row: its label, what it holds, and a button that takes the
/// pick for it. Answers whether the button was pressed with a pick there.
fn reference_row(f: &mut Form, label: &str, shown: String, can_pick: bool, button: &str) -> bool {
    f.flow(label, true, |f| {
        f.reading(&shown);
        f.button(
            button,
            ButtonStyle::Outline,
            can_pick,
            "Click it in the viewport first, then press this",
        ) && can_pick
    })
}

fn spot_row(f: &mut Form, spot: &mut EdgeSpot, edge: &EdgeAnchor) -> bool {
    let unknown = EdgeSpot::ALL
        .iter()
        .position(|c| *c == EdgeSpot::Centre && edge.circle.is_none());
    let picked = choice(
        f,
        "At",
        Select {
            options: EdgeSpot::ALL
                .iter()
                .map(|c| c.label().to_string())
                .collect(),
            selected: EdgeSpot::ALL
                .iter()
                .position(|c| c == spot)
                .unwrap_or(EdgeSpot::ALL.len()),
            disabled: unknown.into_iter().collect(),
            ..Default::default()
        },
    );
    match picked {
        Some(i) => {
            *spot = EdgeSpot::ALL[i];
            true
        }
        None => false,
    }
}

/// A choice offering `options`, `text` in its closed field and none
/// chosen, under a blank label: a list a pick adds from.
fn adding_choice(f: &mut Form, text: &str, options: Vec<String>) -> Option<usize> {
    choice(
        f,
        LABEL_BLANK,
        Select {
            selected: options.len(),
            options,
            text: Some(text.into()),
            ..Default::default()
        },
    )
}

/// One of the base planes side by side under `label`; the one picked.
fn base_planes(f: &mut Form, label: &str, current: BasePlane) -> Option<usize> {
    f.select(
        label,
        Select {
            label: label.into(),
            options: BasePlane::ALL
                .iter()
                .map(|p| p.label().to_string())
                .collect(),
            selected: BasePlane::ALL
                .iter()
                .position(|p| *p == current)
                .unwrap_or(BasePlane::ALL.len()),
            segmented: true,
            ..Default::default()
        },
    )
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
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    index: usize,
    line: &mut LineAnchor,
    picked: &Picked,
    refs: &FeatureRefs,
) -> bool {
    let mut changed = false;
    if reference_row(
        f,
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
        let names = refs.lines.iter().map(|(l, _)| l.clone()).collect();
        if let Some(i) = adding_choice(f, "A datum or sketch line…", names) {
            *line = refs.lines[i].1;
            changed = true;
        }
    }
    changed
}

/// A point reference taken from a datum or sketch point made before.
fn feature_point_row(f: &mut Form, point: &mut PointAnchor, refs: &FeatureRefs) -> bool {
    if refs.points.is_empty() {
        return false;
    }
    let names = refs.points.iter().map(|(l, _)| l.clone()).collect();
    match adding_choice(f, "A datum or sketch point…", names) {
        Some(i) => {
            *point = refs.points[i].1;
            true
        }
        None => false,
    }
}

/// The datum plane or coordinate system (and which of its planes) the
/// datum stands on.
fn on_datum_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
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
    let chosen = choice(
        f,
        "Datum",
        Select {
            options: picked.datums.iter().map(|(id, ..)| name(*id)).collect(),
            selected: picked
                .datums
                .iter()
                .position(|(id, ..)| id == datum)
                .unwrap_or(picked.datums.len()),
            text: Some(name(*datum)),
            ..Default::default()
        },
    );
    if let Some(i) = chosen {
        let (id, found, system) = &picked.datums[i];
        *datum = *id;
        *plane = system.then_some(plane.unwrap_or(BasePlane::XY));
        *frame = pick_frame(*found, *plane).into();
        changed = true;
    }
    if let Some(which) = plane
        && let Some(i) = base_planes(f, "Its plane", *which)
    {
        let candidate = BasePlane::ALL[i];
        *which = candidate;
        if let Some((_, found, _)) = picked.datums.iter().find(|(id, ..)| id == datum) {
            *frame = pick_frame(*found, Some(candidate)).into();
        }
        changed = true;
    }
    changed
}

/// The other body and which of its origin planes the datum stands on.
fn other_body_rows(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    own: BodyId,
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
    let chosen = choice(
        f,
        "Body",
        Select {
            options: bodies.iter().map(|(_, n)| n.clone()).collect(),
            selected: bodies
                .iter()
                .position(|(id, _)| id == body)
                .unwrap_or(bodies.len()),
            text: Some(shown),
            ..Default::default()
        },
    );
    if let Some(i) = chosen {
        *body = bodies[i].0;
        changed = true;
    }
    if let Some(i) = base_planes(f, "Plane", *plane) {
        *plane = BasePlane::ALL[i];
        changed = true;
    }
    if changed && let Some(found) = core_document::body_plane_in(ctx.document, own, *body, *plane) {
        *frame = found.into();
    }
    changed
}

fn plane_row(
    f: &mut Form,
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
    let mut options: Vec<String> = BasePlane::ALL
        .iter()
        .map(|b| b.label().to_string())
        .collect();
    options.extend(datums.iter().map(|(_, name, _)| name.clone()));
    let at = BasePlane::ALL
        .iter()
        .position(|b| *plane == PlaneAnchor::Base(*b))
        .or_else(|| {
            datums
                .iter()
                .position(
                    |(id, ..)| matches!(plane, PlaneAnchor::Datum { datum: d, .. } if d == id),
                )
                .map(|i| BasePlane::ALL.len() + i)
        });
    let chosen = f.flow("", true, |f| {
        choice(
            f,
            &format!("Plane {}", index + 1),
            Select {
                selected: at.unwrap_or(options.len()),
                options,
                text: Some(shown),
                ..Default::default()
            },
        )
    });
    match chosen {
        Some(i) if i < BasePlane::ALL.len() => {
            *plane = PlaneAnchor::Base(BasePlane::ALL[i]);
            changed = true;
        }
        Some(i) => {
            let (id, _, datum) = &datums[i - BasePlane::ALL.len()];
            let frame = datum.frame();
            *plane = PlaneAnchor::Datum {
                datum: *id,
                origin: frame.origin,
                normal: frame.normal,
            };
            changed = true;
        }
        None => {}
    }
    // On a line of its own: beside a long face reading it wraps a letter a line.
    let take = f.flow(LABEL_BLANK, false, |f| {
        f.button(
            "Use selected face",
            ButtonStyle::Outline,
            picked.face.is_some(),
            "Click a face in the viewport first, then press this",
        )
    });
    if take && let Some(face) = picked.face {
        *plane = PlaneAnchor::Face { face };
        changed = true;
    }
    changed
}

/// The attachment part of the datum task: the mode, and the references
/// it takes. Answers whether the datum changed.
pub fn attachment_editor(
    f: &mut Form,
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

    {
        // The base planes, the flat face it was made on (shown chosen, not
        // offered), then the modes, each offered once what it needs is
        // picked.
        let mut options: Vec<String> = BasePlane::ALL
            .iter()
            .map(|p| p.label().to_string())
            .collect();
        let mut selected = BasePlane::ALL
            .iter()
            .position(|p| datum.attachment == DatumAttachment::BasePlane(*p));
        if matches!(datum.attachment, DatumAttachment::FlatFace { .. }) {
            selected = Some(options.len());
            options.push(datum.attachment.label().to_string());
        }
        let first_mode = options.len();
        let mut disabled = Vec::new();
        let mut hints = vec![String::new(); first_mode];
        let mut made = Vec::new();
        for (i, (mode, needs)) in MODES.iter().enumerate() {
            let current = datum.attachment.mode() == *mode;
            let built = candidate(
                mode,
                ctx,
                body,
                datum.frame(),
                current_edge(&datum.attachment),
                &picked,
            );
            if current {
                selected = Some(first_mode + i);
            } else if built.is_none() {
                disabled.push(first_mode + i);
            }
            options.push(mode_label(mode).to_string());
            hints.push(needs.to_string());
            made.push(built);
        }
        let chosen = choice(
            f,
            "Attached to",
            Select {
                selected: selected.unwrap_or(options.len()),
                options,
                text: Some(datum.attachment.label().to_string()),
                disabled,
                hints,
                separated: vec![first_mode],
                ..Default::default()
            },
        );
        match chosen {
            Some(i) if i < BasePlane::ALL.len() => {
                datum.attachment = DatumAttachment::BasePlane(BasePlane::ALL[i]);
                changed = true;
            }
            Some(i) if i >= first_mode => {
                if let Some(built) = made[i - first_mode].take() {
                    datum.attachment = built;
                    picks_changed = true;
                }
            }
            _ => {}
        }
    }

    match &mut datum.attachment {
        DatumAttachment::Face { face } => {
            if reference_row(
                f,
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
            picks_changed |= point_rows(f, points, &picked);
            for point in points.iter_mut() {
                picks_changed |= feature_point_row(f, point, &refs);
            }
        }
        DatumAttachment::TwoPoints { points } => {
            picks_changed |= point_rows(f, points, &picked);
            for point in points.iter_mut() {
                picks_changed |= feature_point_row(f, point, &refs);
            }
        }
        DatumAttachment::NormalToEdge { edge, spot, along } => {
            picks_changed |= edge_row(f, edge, &picked);
            f.flow("Along", false, |f| {
                let mut on = along.is_some();
                if f.check(
                    &mut on,
                    "A share of the way",
                    "The plane stands this far along the edge from its start",
                ) {
                    *along = on.then_some(0.5);
                    picks_changed = true;
                }
                if let Some(share) = along {
                    let set = f.quantity(Quantity {
                        label: String::new(),
                        value: f64::from(*share * 100.0),
                        unit: Some("%".into()),
                        min: Some(0.0),
                        max: Some(100.0),
                        speed: Some(0.1),
                        ..Default::default()
                    });
                    if let Some(percent) = set {
                        *share = percent as f32 / 100.0;
                        picks_changed = true;
                    }
                }
            });
            if along.is_none() {
                changed |= spot_row(f, spot, edge);
            }
        }
        DatumAttachment::TangentToEdge { edge, spot } => {
            picks_changed |= edge_row(f, edge, &picked);
            changed |= spot_row(f, spot, edge);
        }
        DatumAttachment::FaceNormal { face } => {
            if reference_row(
                f,
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
            picks_changed |= on_datum_rows(f, ctx, (datum, plane, frame), &picked);
        }
        DatumAttachment::OtherBody {
            body: other,
            plane,
            frame,
        } => {
            picks_changed |= other_body_rows(f, ctx, body, (other, plane, frame));
        }
        DatumAttachment::LineAndPoint { line, point } => {
            picks_changed |= line_row(f, ctx, 0, line, &picked, &refs);
            picks_changed |= point_rows(f, std::slice::from_mut(point), &picked);
            picks_changed |= feature_point_row(f, point, &refs);
        }
        DatumAttachment::LineMeetsPlane { line, plane } => {
            picks_changed |= line_row(f, ctx, 0, line, &picked, &refs);
            picks_changed |= plane_row(f, ctx, (feature_id, 0), body, plane, &picked);
        }
        DatumAttachment::TwoLines { lines } => {
            for (i, line) in lines.iter_mut().enumerate() {
                picks_changed |= line_row(f, ctx, i, line, &picked, &refs);
            }
        }
        DatumAttachment::AlongEdge { edge } | DatumAttachment::CurveCentre { edge } => {
            picks_changed |= edge_row(f, edge, &picked);
        }
        DatumAttachment::PlaneIntersection { planes } | DatumAttachment::Midplane { planes } => {
            for (i, plane) in planes.iter_mut().enumerate() {
                picks_changed |= plane_row(f, ctx, (feature_id, i), body, plane, &picked);
            }
        }
        DatumAttachment::Inertia { centre, .. } => {
            f.flow("Centre", false, |f| {
                f.reading(&coords(*centre));
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
        f.gap(GapSize::Small);
        f.note(NoteKind::Warning, None, &problem);
    }
    changed
}

fn edge_row(f: &mut Form, edge: &mut EdgeAnchor, picked: &Picked) -> bool {
    let shown = match edge.circle {
        Some(circle) => format!("circle R{:.2} at {}", circle.radius, coords(circle.center)),
        None => coords(edge.point),
    };
    if reference_row(
        f,
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

fn point_rows(f: &mut Form, points: &mut [PointAnchor], picked: &Picked) -> bool {
    let mut changed = false;
    let from_pick = picked.points().first().copied();
    for (i, point) in points.iter_mut().enumerate() {
        if reference_row(
            f,
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
            changed |= spot_row(f, spot, &edge);
        }
    }
    changed
}
