//! The Mirrored, pattern and Multi transform task panels: the features
//! repeated, the plane, axis, points or path they repeat by, and the
//! occurrences skipped.

use bench_api::{ButtonStyle, Tone};
use core_document::{BodyId, FeatureId, WorkbenchRuntimeContext};

use crate::feature::{
    DesignFeature, EdgePick, MirrorPlane, PatternAxis, SketchAxis, TransformStep,
};
use crate::form::Form;

use super::patterns;
use super::{
    Bounds, Formulas, Kind, LABEL_BLANK, bare, choice_shown, coords, count_drag, deg_drag,
    edited_body, label_row, mm_drag, mono_value, named_f64, number, picked_edges, picked_point,
    plane_editor, plane_references, quantity, set_f32, sketch_combo, small_button, taper_note,
    use_button,
};
use crate::build::design_features_of_body;

/// The Mirrored, pattern and Multi transform editors; `true` when it
/// changed the feature. Any other kind is left alone.
pub(super) fn transform_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    fx: &mut Formulas,
    body: BodyId,
    feature_id: FeatureId,
    feature: &mut DesignFeature,
) -> bool {
    let mut changed = false;
    match feature {
        DesignFeature::Mirrored {
            originals,
            plane,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= mirror_plane_editor(f, ctx, plane);
            changed |= skipped_editor(f, skipped);
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
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
            changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
            changed |= f.check(
                spacing_mode,
                "Length is spacing",
                "Off: length is the overall span",
            );
            changed |= mm_drag(f, fx, length, ("Length:", "length"));
            let even = if *spacing_mode || *occurrences < 2 {
                *length
            } else {
                *length / (*occurrences - 1) as f32
            };
            changed |= gaps_editor(
                f,
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
            changed |= f.check(reversed, "Reversed", "");
            changed |= skipped_editor(f, skipped);
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
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
            changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
            changed |= f.check(
                step_mode,
                "Angle is the step",
                "On: the angle between one occurrence and the next. \
                     Off: the angle all of them span",
            );
            changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 1.0..=360.0);
            let even = if *step_mode || *occurrences < 2 {
                *angle_deg
            } else if (*angle_deg - 360.0).abs() < 1e-6 {
                *angle_deg / *occurrences as f32
            } else {
                *angle_deg / (*occurrences - 1) as f32
            };
            changed |= gaps_editor(
                f,
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
            changed |= f.check(reversed, "Reversed", "");
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::CircularPattern {
            originals,
            axis,
            outward,
            spacing,
            counts,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= patterns::rings_editor(
                f,
                ctx,
                fx,
                (body, feature_id),
                (axis, outward),
                (spacing, counts),
                true,
            );
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::PointPattern {
            originals,
            sketch,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            if let Some(picked) = sketch_combo(f, ctx, body, feature_id, Some(*sketch), "Points") {
                *sketch = picked;
                changed = true;
            }
            taper_note(
                f,
                "A copy at each of the sketch's points after its first, moved by how far \
                 that point is from the first; construction points left out.",
            );
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::PathPattern {
            originals,
            path,
            layout,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            changed |= patterns::path_editor(f, ctx, fx, (body, feature_id), path, layout);
            taper_note(
                f,
                "The original stands for the path's start: each copy is moved from there to \
                 its place along the path.",
            );
            changed |= skipped_editor(f, skipped);
        }
        DesignFeature::MultiTransform {
            originals,
            steps,
            refine: _,
            boolean_tolerance: _,
            skipped,
        } => {
            changed |= originals_editor(f, ctx, body, feature_id, originals);
            f.caption(
                "Steps, each applied to every result of the ones before",
                Tone::Neutral,
                false,
                false,
            );
            let mut remove = None;
            for (i, step) in steps.iter_mut().enumerate() {
                let label = match step {
                    TransformStep::Linear { .. } => "Linear",
                    TransformStep::Polar { .. } => "Polar",
                    TransformStep::Mirror { .. } => "Mirror",
                    TransformStep::Scale { .. } => "Scale",
                    TransformStep::Circular { .. } => "Circular",
                    TransformStep::Points { .. } => "Points",
                    TransformStep::Path { .. } => "Path",
                };
                f.flow("", false, |f| {
                    mono_value(f, &format!("{}. {label}", i + 1), true);
                    if small_button(f, "×", "") {
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
                        changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
                        changed |= mm_drag(f, fx, length, ("Length:", "length"));
                        changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
                    }
                    TransformStep::Polar {
                        axis,
                        angle_deg,
                        occurrences,
                    } => {
                        changed |= pattern_axis_editor(f, ctx, (body, feature_id), axis);
                        changed |= deg_drag(f, fx, angle_deg, ("Angle:", "angle"), 1.0..=360.0);
                        changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
                    }
                    TransformStep::Mirror { plane } => {
                        changed |= mirror_plane_editor(f, ctx, plane);
                    }
                    TransformStep::Scale {
                        factor,
                        center,
                        occurrences,
                    } => {
                        if let Some((edited, v)) = fx.show_within(
                            f,
                            ("Factor:", "factor"),
                            f64::from(*factor),
                            Bounds::within(0.01, 100.0),
                        ) {
                            *factor = (v as f32).clamp(0.01, 100.0);
                            changed |= edited;
                        } else {
                            let mut q = quantity("Factor", f64::from(*factor), Kind::Plain, 0.05);
                            (q.min, q.max, q.bare) = (Some(0.01), Some(100.0), true);
                            changed |= set_f32(factor, f.quantity(q));
                        }
                        f.flow("Center", true, |f| {
                            for v in center.iter_mut() {
                                changed |= set_f32(v, bare(f, "", f64::from(*v), 0.5, ""));
                            }
                            let picked = picked_point(ctx);
                            if use_button(
                                f,
                                "Use selected",
                                picked.is_some(),
                                "A circular edge's centre, else where an edge or a face was picked",
                            ) && let Some(at) = picked
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
                            let names = points.iter().map(|(n, _)| n.clone()).collect();
                            if let Some(k) =
                                choice_shown(f, LABEL_BLANK, "A datum point…".into(), names, None)
                            {
                                *center = points[k].1;
                                changed = true;
                            }
                        }
                        changed |= count_drag(f, fx, occurrences, ("Occurrences:", "occurrences"));
                    }
                    TransformStep::Circular {
                        axis,
                        outward,
                        spacing,
                        counts,
                    } => {
                        changed |= patterns::rings_editor(
                            f,
                            ctx,
                            fx,
                            (body, feature_id),
                            (axis, outward),
                            (spacing, counts),
                            false,
                        );
                    }
                    TransformStep::Path { path, layout } => {
                        changed |=
                            patterns::path_editor(f, ctx, fx, (body, feature_id), path, layout);
                    }
                    TransformStep::Points { sketch } => {
                        if let Some(picked) =
                            sketch_combo(f, ctx, body, feature_id, Some(*sketch), "Points")
                        {
                            *sketch = picked;
                            changed = true;
                        }
                    }
                }
            }
            fx.prefix = None;
            if let Some(i) = remove {
                steps.remove(i);
                changed = true;
            }
            f.flow("", true, |f| {
                let add =
                    |f: &mut Form, label: &str| f.button(label, ButtonStyle::Secondary, true, "");
                if add(f, "+ Linear") {
                    steps.push(TransformStep::Linear {
                        axis: PatternAxis::X,
                        length: 10.0,
                        occurrences: 2,
                    });
                    changed = true;
                }
                if add(f, "+ Polar") {
                    steps.push(TransformStep::Polar {
                        axis: PatternAxis::Z,
                        angle_deg: 360.0,
                        occurrences: 4,
                    });
                    changed = true;
                }
                if add(f, "+ Mirror") {
                    steps.push(TransformStep::Mirror {
                        plane: MirrorPlane::YZ,
                    });
                    changed = true;
                }
            });
            f.flow("", true, |f| {
                let add =
                    |f: &mut Form, label: &str| f.button(label, ButtonStyle::Secondary, true, "");
                if add(f, "+ Circular") {
                    steps.push(TransformStep::Circular {
                        axis: PatternAxis::Z,
                        outward: PatternAxis::X,
                        spacing: 10.0,
                        counts: vec![6, 12],
                    });
                    changed = true;
                }
                if let Some((sketch, _)) =
                    crate::build::sketches_of_body(ctx.document, body).first()
                    && add(f, "+ Points")
                {
                    steps.push(TransformStep::Points { sketch: *sketch });
                    changed = true;
                }
                if let Some(path) = patterns::add_path_step(f, ctx, body) {
                    steps.push(TransformStep::Path {
                        path,
                        layout: Default::default(),
                    });
                    changed = true;
                }
                if add(f, "+ Scale") {
                    steps.push(TransformStep::Scale {
                        factor: 2.0,
                        center: [0.0; 3],
                        occurrences: 2,
                    });
                    changed = true;
                }
            });
            changed |= skipped_editor(f, skipped);
        }
        _ => {}
    }
    changed
}

/// The occurrences a pattern leaves out, each brought back by its button,
/// all of them by Clear; the view's dots leave one out or bring it back.
fn skipped_editor(f: &mut Form, skipped: &mut Vec<u32>) -> bool {
    let mut changed = false;
    f.flow("Skipped", false, |f| {
        if skipped.is_empty() {
            f.caption("None", Tone::Neutral, false, false);
        } else if small_button(f, "Clear", "Bring every occurrence back") {
            skipped.clear();
            changed = true;
        }
    });
    if !skipped.is_empty() {
        let mut back = None;
        f.flow(LABEL_BLANK, true, |f| {
            for (at, index) in skipped.iter().enumerate() {
                let name = format!("{} ×", crate::instances::occurrence_name(*index));
                if small_button(f, &name, "Bring it back") {
                    back = Some(at);
                }
            }
        });
        if let Some(at) = back {
            skipped.remove(at);
            changed = true;
        }
    }
    f.caption(
        "Click an occurrence's dot in the view to leave it out or bring it back",
        Tone::Neutral,
        false,
        false,
    );
    changed
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
pub(super) fn pattern_axis_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    (body, feature): (BodyId, FeatureId),
    axis: &mut PatternAxis,
) -> bool {
    let mut changed = false;
    let references = pattern_axis_choices(ctx, body, feature);
    let shown = references
        .iter()
        .find(|(choice, _)| choice == axis)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| axis.label().to_string());
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
    let at = candidates
        .iter()
        .position(|(candidate, _)| match (&*axis, candidate) {
            (PatternAxis::Datum(_), PatternAxis::Datum(_))
            | (PatternAxis::Sketch { .. }, PatternAxis::Sketch { .. }) => axis == candidate,
            _ => std::mem::discriminant(axis) == std::mem::discriminant(candidate),
        });
    let names = candidates.iter().map(|(_, n)| n.clone()).collect();
    if let Some(i) = choice_shown(f, "Axis", shown, names, at) {
        *axis = candidates[i].0;
        changed = true;
    }
    if let PatternAxis::Edge(edge) = axis {
        f.flow("Edge", true, |f| {
            mono_value(f, &coords(edge.point), true);
            if use_button(
                f,
                "Use selected edge",
                !ctx.selected_edges.is_empty(),
                "Click a straight edge in the viewport, or a round one for its axis, \
                 then press this",
            ) && let Some(picked) = picked_edges(ctx).first()
            {
                *edge = edge_axis(picked);
                changed = true;
            }
        });
    }
    if let PatternAxis::Custom { origin, dir } = axis {
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
    f: &mut Form,
    fx: &mut Formulas,
    gaps: &mut Vec<f32>,
    (occurrences, even): (u32, f32),
    how: Gaps,
) -> bool {
    let mut changed = false;
    let mut uneven = !gaps.is_empty();
    if f.check(
        &mut uneven,
        how.toggle,
        "Set the gap from each occurrence to the next on its own",
    ) {
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
        changed |= named_f64(
            f,
            fx,
            &mut value,
            (&label, &name, Bounds::default()),
            |f, v| {
                let (kind, speed) = if how.degrees {
                    (Kind::Degrees, 1.0)
                } else {
                    (Kind::Mm, 0.5)
                };
                match number(f, &label, *v, kind, speed) {
                    Some(set) => {
                        *v = set;
                        true
                    }
                    None => false,
                }
            },
        );
        *gap = value as f32;
    }
    changed
}

/// The datum planes (a coordinate system by each of its planes) and the
/// sketches (each by its plane and its two axes) of the edited feature's
/// body, as mirror planes.
/// The planes of the edited body's datums and sketches.
fn mirror_references(ctx: &WorkbenchRuntimeContext) -> Vec<(MirrorPlane, String)> {
    edited_body(ctx)
        .map(|body| plane_references(ctx, body))
        .unwrap_or_default()
}

fn mirror_plane_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    plane: &mut MirrorPlane,
) -> bool {
    let references = mirror_references(ctx);
    plane_editor(f, ctx, plane, &references)
}

/// The body's Design features before this one, modifiers left out,
/// selectable as pattern originals.
fn originals_editor(
    f: &mut Form,
    ctx: &WorkbenchRuntimeContext,
    body: BodyId,
    this_feature: FeatureId,
    originals: &mut Vec<FeatureId>,
) -> bool {
    let mut changed = false;
    label_row(f, "Originals (empty = whole body)");
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
        if f.check(&mut included, &name, "") {
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
