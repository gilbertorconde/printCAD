//! Transform tools over the current selection: translate, rotate, scale,
//! mirror and array. Copies deep-copy points with fresh ids while preserving
//! point sharing *within* the copied set.

use std::collections::{HashMap, HashSet};

use glam::{Mat2, Vec2};
use uuid::Uuid;

use super::{ToolEffect, ToolParams, ToolState};
use crate::sketch::{
    Arc, BSpline, Circle, Conic, ConstraintKind, Ellipse, GeometryElement, Line, Point, Sketch,
    Vec2D,
};
use crate::snap;

/// A 2D similarity transform `p' = m·p + t` (rotation/scale/mirror plus
/// translation). Also drives the overlay ghost previews.
#[derive(Debug, Clone, Copy)]
pub struct Similarity {
    m: Mat2,
    t: Vec2,
}

impl Similarity {
    pub fn translation(d: Vec2) -> Self {
        Self {
            m: Mat2::IDENTITY,
            t: d,
        }
    }

    pub fn rotation_about(center: Vec2, angle: f32) -> Self {
        let m = Mat2::from_angle(angle);
        Self {
            m,
            t: center - m * center,
        }
    }

    pub fn scale_about(center: Vec2, factor: f32) -> Self {
        let m = Mat2::from_diagonal(Vec2::splat(factor));
        Self {
            m,
            t: center - m * center,
        }
    }

    /// Reflection through the point `c`: a half turn about it.
    pub fn point_reflection(c: Vec2) -> Self {
        Self {
            m: -Mat2::IDENTITY,
            t: 2.0 * c,
        }
    }

    /// Reflection about the line through `a` and `b`.
    pub fn mirror_about(a: Vec2, b: Vec2) -> Self {
        let u = (b - a).normalize();
        // Householder-style reflection: 2·uuᵀ − I.
        let m = Mat2::from_cols(
            Vec2::new(2.0 * u.x * u.x - 1.0, 2.0 * u.x * u.y),
            Vec2::new(2.0 * u.x * u.y, 2.0 * u.y * u.y - 1.0),
        );
        Self { m, t: a - m * a }
    }

    /// The transform `p' = m·p + t`, for a map worked out elsewhere (one
    /// sketch plane's coordinates onto another's).
    pub fn linear(m: Mat2, t: Vec2) -> Self {
        Self { m, t }
    }

    /// Whether the transform only moves: no turn, scale or mirror, so
    /// anything stated against the sketch axes (horizontal, a distance in
    /// x) still holds after it.
    pub fn is_translation(&self) -> bool {
        (self.m - Mat2::IDENTITY)
            .abs()
            .to_cols_array()
            .iter()
            .all(|v| *v < 1e-5)
    }

    pub fn apply(&self, p: Vec2D) -> Vec2D {
        Vec2D::from_glam(self.m * p.to_glam() + self.t)
    }

    /// Linear part only (for direction vectors like an ellipse major axis).
    pub fn apply_vec(&self, v: Vec2D) -> Vec2D {
        Vec2D::from_glam(self.m * v.to_glam())
    }

    /// Uniform scale factor of the linear part.
    pub fn scale_factor(&self) -> f32 {
        self.m.determinant().abs().sqrt()
    }

    /// Whether the transform flips orientation (mirror): CCW arcs must swap
    /// their endpoints to stay CCW.
    pub fn flips_orientation(&self) -> bool {
        self.m.determinant() < 0.0
    }
}

/// Every point id the selection touches: selected point elements plus the
/// points referenced by selected curves (shared points included: the
/// solver re-solves neighbours afterwards).
fn selection_point_ids(sketch: &Sketch, selected: &HashSet<Uuid>) -> HashSet<Uuid> {
    let mut pts = HashSet::new();
    for geom in &sketch.geometry {
        if !selected.contains(&geom.id()) {
            continue;
        }
        match geom {
            GeometryElement::Point(p) => {
                pts.insert(p.id);
            }
            other => pts.extend(Sketch::curve_point_ids(other)),
        }
    }
    pts
}

/// Transform the selected geometry in place. Returns the touched point
/// count (0 = empty selection).
pub(super) fn apply_to_selection(
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) -> usize {
    let pts = selection_point_ids(sketch, selected);
    let scale = xf.scale_factor();
    for geom in &mut sketch.geometry {
        match geom {
            GeometryElement::Point(p) if pts.contains(&p.id) => {
                p.position = xf.apply(p.position);
            }
            GeometryElement::Circle(c) if selected.contains(&c.id) => c.radius *= scale,
            GeometryElement::Arc(a) if selected.contains(&a.id) => a.radius *= scale,
            GeometryElement::Ellipse(e) if selected.contains(&e.id) => {
                e.major = xf.apply_vec(e.major);
            }
            GeometryElement::Conic(c) if selected.contains(&c.id) => {
                c.axis = xf.apply_vec(c.axis);
                c.minor *= scale;
            }
            _ => {}
        }
    }
    let moved: HashSet<Uuid> = pts.iter().chain(selected.iter()).copied().collect();
    adapt_constraints(sketch, &moved, xf);
    pts.len()
}

/// After the geometry in `moved` (points and elements) went through `xf`,
/// the constraints that bind nothing else say what the shape now is, so
/// the solver keeps the move rather than undoing it: a dimension takes the
/// value it measures after the move, a fixed point is fixed where it has
/// gone, and horizontal and vertical follow the turn. They stay as they
/// are when the axes do, swap on a quarter turn, and at any other angle
/// become parallel or perpendicular to one of the lines, which keeps a
/// rectangle a rectangle at any angle.
pub(crate) fn adapt_constraints(sketch: &mut Sketch, moved: &HashSet<Uuid>, xf: &Similarity) {
    use crate::sketch::{ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID, measured_value, with_dimension_value};
    let bound = |kind: &ConstraintKind| {
        let refs = crate::sketch::constraint_refs(kind);
        refs.iter().any(|r| moved.contains(r))
            && refs
                .iter()
                .all(|r| moved.contains(r) || [ORIGIN_ID, X_AXIS_ID, Y_AXIS_ID].contains(r))
    };
    // Where the x axis goes: along x, along y, or elsewhere.
    let x_image = xf
        .apply_vec(Vec2D::new(1.0, 0.0))
        .to_glam()
        .normalize_or_zero();
    let keeps_axes = x_image.y.abs() < 1e-4;
    let swaps_axes = x_image.x.abs() < 1e-4;
    let mut unaligner = Unaligner::default();
    let mut dropped: Vec<Uuid> = Vec::new();
    let snapshot = sketch.clone();
    for constraint in &mut sketch.constraints {
        if !bound(&constraint.kind) {
            continue;
        }
        match constraint.kind {
            ConstraintKind::FixedPoint { point, .. } => {
                if let Some(position) = snapshot.point_position(point) {
                    constraint.kind = ConstraintKind::FixedPoint { point, position };
                }
            }
            ConstraintKind::Horizontal { element } | ConstraintKind::Vertical { element } => {
                let horizontal = matches!(constraint.kind, ConstraintKind::Horizontal { .. });
                if keeps_axes {
                    continue;
                }
                if swaps_axes {
                    constraint.kind = if horizontal {
                        ConstraintKind::Vertical { element }
                    } else {
                        ConstraintKind::Horizontal { element }
                    };
                    continue;
                }
                match unaligner.relate(element, horizontal) {
                    Some(kind) => constraint.kind = kind,
                    None => dropped.push(constraint.id),
                }
            }
            // Two points' alignment turns with them: kept or swapped with
            // the axes, gone at any other angle.
            ConstraintKind::HorizontalPoints { point1, point2 }
            | ConstraintKind::VerticalPoints { point1, point2 } => {
                let horizontal = matches!(constraint.kind, ConstraintKind::HorizontalPoints { .. });
                if keeps_axes {
                    continue;
                }
                if swaps_axes {
                    constraint.kind = if horizontal {
                        ConstraintKind::VerticalPoints { point1, point2 }
                    } else {
                        ConstraintKind::HorizontalPoints { point1, point2 }
                    };
                    continue;
                }
                dropped.push(constraint.id);
            }
            // A pin to the origin or an axis the move took the point off
            // would pull it back: it goes.
            ConstraintKind::Coincident { point1, point2 }
                if point1 == ORIGIN_ID || point2 == ORIGIN_ID =>
            {
                let point = if point1 == ORIGIN_ID { point2 } else { point1 };
                if snapshot
                    .point_position(point)
                    .is_none_or(|p| p.to_glam().length() > 1e-4)
                {
                    dropped.push(constraint.id);
                }
            }
            ConstraintKind::PointOnLine { point, line }
                if line == X_AXIS_ID || line == Y_AXIS_ID =>
            {
                let off = snapshot
                    .point_position(point)
                    .map(|p| if line == X_AXIS_ID { p.y } else { p.x });
                if off.is_none_or(|o| o.abs() > 1e-4) {
                    dropped.push(constraint.id);
                }
            }
            ref kind => {
                if let Some(now) = measured_value(&snapshot, kind) {
                    constraint.kind = with_dimension_value(kind, now);
                }
            }
        }
    }
    sketch.constraints.retain(|c| !dropped.contains(&c.id));
}

/// Horizontal and vertical constraints made relative to one another: the
/// first line met keeps the turn free (its own constraint goes, having no
/// counterpart), and each other holds to it as it held to the axes,
/// parallel or perpendicular.
#[derive(Debug, Default)]
pub(crate) struct Unaligner {
    reference: Option<(Uuid, bool)>,
}

impl Unaligner {
    /// What a horizontal (or vertical) constraint on `element` becomes;
    /// `None` when it goes.
    pub(crate) fn relate(&mut self, element: Uuid, horizontal: bool) -> Option<ConstraintKind> {
        let Some((line, line_horizontal)) = self.reference else {
            self.reference = Some((element, horizontal));
            return None;
        };
        Some(if horizontal == line_horizontal {
            ConstraintKind::Parallel {
                line1: element,
                line2: line,
            }
        } else {
            ConstraintKind::Perpendicular {
                line1: element,
                line2: line,
            }
        })
    }
}

/// Turn the horizontal and vertical constraints on the lines among
/// `elements` into parallel and perpendicular ones among themselves, so
/// the group keeps its shape and is free to turn as a whole. How many
/// constraints changed or went.
pub fn remove_axis_alignment(sketch: &mut Sketch, elements: &HashSet<Uuid>) -> usize {
    let lines: HashSet<Uuid> = elements
        .iter()
        .copied()
        .filter(|id| matches!(sketch.get_geometry(*id), Some(GeometryElement::Line(_))))
        .collect();
    let mut unaligner = Unaligner::default();
    let mut dropped: Vec<Uuid> = Vec::new();
    let mut changed = 0;
    for constraint in &mut sketch.constraints {
        let (element, horizontal) = match constraint.kind {
            ConstraintKind::Horizontal { element } => (element, true),
            ConstraintKind::Vertical { element } => (element, false),
            _ => continue,
        };
        if !lines.contains(&element) {
            continue;
        }
        changed += 1;
        match unaligner.relate(element, horizontal) {
            Some(kind) => constraint.kind = kind,
            None => dropped.push(constraint.id),
        }
    }
    sketch.constraints.retain(|c| !dropped.contains(&c.id));
    changed
}

/// Add a transformed deep copy of the selection: fresh point ids, sharing
/// preserved within the copy. Returns the number of copied elements.
pub(super) fn copy_selection(
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) -> usize {
    let source = sketch.clone();
    copy_from(&source, sketch, selected, xf)
}

/// The selection repeated in a grid: `rows` × `cols` copies stepped by
/// `dx` and `dy`, the original in the corner. `linked`, every copy stays
/// the original's size and one pitch along the rows and one down the
/// columns space them.
pub fn array(
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    rows: u32,
    cols: u32,
    dx: f32,
    dy: f32,
    linked: bool,
) -> ToolEffect {
    if selected.is_empty() || rows.max(cols) < 2 {
        return ToolEffect::none();
    }
    let (rows, cols) = (rows.max(1), cols.max(1));
    let source = sketch.clone();
    let reference = reference_point(&source, selected);
    let mut copies = 0;
    let mut maps = Vec::new();
    let mut members = Vec::new();
    for row in 0..rows {
        for col in 0..cols {
            if row == 0 && col == 0 {
                members.extend(reference);
                continue;
            }
            let delta = Vec2::new(col as f32 * dx, row as f32 * dy);
            let map = copy_linked(
                &source,
                sketch,
                selected,
                &Similarity::translation(delta),
                linked,
            );
            members.extend(reference.and_then(|r| map.get(&r).copied()));
            maps.push(map);
            copies += 1;
        }
    }
    if linked {
        equal_sizes(sketch, &source, selected, &maps);
        if members.len() == (rows * cols) as usize {
            for (across, step, count, way) in [
                (false, dx, cols, Vec2D::new(dx.signum(), 0.0)),
                (true, dy, rows, Vec2D::new(0.0, dy.signum())),
            ] {
                if count >= 2 && step.abs() > 1e-6 {
                    sketch.add_constraint(ConstraintKind::Pitch {
                        points: members.clone(),
                        columns: cols,
                        distance: step.abs(),
                        across,
                        direction: Some(way),
                    });
                }
            }
        }
    }
    ToolEffect::changed(format!("Rectangular array: {copies} copies"))
}

/// Copy as [`copy_from`] does, returning the old → new ids. A `linked`
/// copy is placed and sized by what ties it to the original: what pinned
/// the original where it is stays behind, and its dimensions come along
/// as reference ones.
fn copy_linked(
    source: &Sketch,
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
    linked: bool,
) -> HashMap<Uuid, Uuid> {
    let map = copy_mapped(source, sketch, selected, xf);
    let first = sketch.constraints.len();
    copy_constraints(source, sketch, &map, xf);
    if linked {
        let mut index = 0;
        sketch.constraints.retain(|c| {
            index += 1;
            index <= first
                || !matches!(
                    c.kind,
                    ConstraintKind::FixedPoint { .. }
                        | ConstraintKind::Block { .. }
                        | ConstraintKind::DistanceX { b: None, .. }
                        | ConstraintKind::DistanceY { b: None, .. }
                )
        });
        for c in sketch.constraints.iter_mut().skip(first) {
            if c.kind.is_dimensional() {
                c.driving = false;
            }
        }
    }
    let copied: HashSet<Uuid> = map.values().copied().collect();
    adapt_constraints(sketch, &copied, xf);
    map
}

/// The point of the selection that stands for it in a pitch: its first
/// point in the sketch's order.
fn reference_point(source: &Sketch, selected: &HashSet<Uuid>) -> Option<Uuid> {
    let points = selection_point_ids(source, selected);
    source
        .geometry
        .iter()
        .flat_map(|g| match g {
            GeometryElement::Point(p) => vec![p.id],
            other => Sketch::curve_point_ids(other),
        })
        .find(|id| points.contains(id))
}

/// Hold every copy's lines, circles, arcs and ellipses the size of the
/// originals they copy.
fn equal_sizes(
    sketch: &mut Sketch,
    source: &Sketch,
    selected: &HashSet<Uuid>,
    maps: &[HashMap<Uuid, Uuid>],
) {
    for geom in source
        .geometry
        .iter()
        .filter(|g| selected.contains(&g.id()))
    {
        let id = geom.id();
        for map in maps {
            let Some(&copy) = map.get(&id) else {
                continue;
            };
            let kind = match geom {
                GeometryElement::Line(_) => ConstraintKind::EqualLength {
                    line1: id,
                    line2: copy,
                },
                GeometryElement::Circle(_) | GeometryElement::Arc(_) => {
                    ConstraintKind::EqualRadius {
                        circle1: id,
                        circle2: copy,
                    }
                }
                GeometryElement::Ellipse(_) => ConstraintKind::EqualEllipse {
                    ellipse1: id,
                    ellipse2: copy,
                },
                _ => continue,
            };
            sketch.add_constraint(kind);
        }
    }
}

/// Copy `selected` elements of `source` into `target` under `xf`: fresh
/// point ids, sharing preserved within the copy. `source` and `target`
/// may be the same sketch cloned, or two sketches (a clipboard, a mirror).
/// Returns the number of copied elements.
pub fn copy_from(
    source: &Sketch,
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) -> usize {
    let map = copy_mapped(source, sketch, selected, xf);
    // The copy holds its shape as the original does.
    copy_constraints(source, sketch, &map, xf);
    let copied: HashSet<Uuid> = map.values().copied().collect();
    adapt_constraints(sketch, &copied, xf);
    map.len()
}

/// Copy as [`copy_from`] does, and return the old → new id of every copied
/// point and curve.
pub fn copy_mapped(
    source: &Sketch,
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    xf: &Similarity,
) -> HashMap<Uuid, Uuid> {
    let pts = selection_point_ids(source, selected);
    let mut map: HashMap<Uuid, Uuid> = HashMap::new();
    for pid in &pts {
        let Some(pos) = source.point_position(*pid) else {
            continue;
        };
        let new_id = sketch.add_geometry(GeometryElement::Point(Point::new(xf.apply(pos))));
        sketch.set_construction(new_id, source.is_construction(*pid));
        map.insert(*pid, new_id);
    }

    let scale = xf.scale_factor();
    let flip = xf.flips_orientation();
    let originals: Vec<GeometryElement> = source
        .geometry
        .iter()
        .filter(|g| selected.contains(&g.id()) && !matches!(g, GeometryElement::Point(_)))
        .cloned()
        .collect();
    for geom in originals {
        // Skip curves with dangling references rather than panic.
        if !Sketch::curve_point_ids(&geom)
            .iter()
            .all(|pid| map.contains_key(pid))
        {
            continue;
        }
        let copy = match &geom {
            GeometryElement::Line(l) => {
                GeometryElement::Line(Line::new(map[&l.start], map[&l.end]))
            }
            GeometryElement::Arc(a) => {
                // A mirrored CCW arc reads CW: swap endpoints to stay CCW.
                let (s, e) = if flip {
                    (a.end, a.start)
                } else {
                    (a.start, a.end)
                };
                GeometryElement::Arc(Arc::new(map[&a.center], map[&s], map[&e], a.radius * scale))
            }
            GeometryElement::Circle(c) => {
                GeometryElement::Circle(Circle::new(map[&c.center], c.radius * scale))
            }
            GeometryElement::Ellipse(e) => {
                let mut copy = Ellipse::new(map[&e.center], xf.apply_vec(e.major), e.ratio);
                // A mirror turns the arc's direction: swap its ends to keep
                // it counter-clockwise, as for a circular arc.
                copy.arc = e.arc.map(|arc| {
                    let (s, end) = if flip {
                        (arc.end, arc.start)
                    } else {
                        (arc.start, arc.end)
                    };
                    crate::sketch::EllipseArcEnds {
                        start: map[&s],
                        end: map[&end],
                    }
                });
                GeometryElement::Ellipse(copy)
            }
            // A similarity keeps a spline's parameters: its degree, knots
            // and the parameters of the points it passes through carry over.
            GeometryElement::BSpline(b) => GeometryElement::BSpline(
                BSpline {
                    control_points: b.control_points.iter().map(|pid| map[pid]).collect(),
                    fit_points: b.fit_points.iter().map(|pid| map[pid]).collect(),
                    ..BSpline::new(Vec::new(), b.periodic)
                }
                .with_shape_of(b),
            ),
            // A mirror turns the curve over: the axis stays, the ends swap
            // sides of it, and the arc keeps them.
            GeometryElement::Conic(c) => GeometryElement::Conic(Conic {
                id: Uuid::new_v4(),
                center: map[&c.center],
                axis: xf.apply_vec(c.axis),
                minor: c.minor * scale,
                start: map[&c.start],
                end: map[&c.end],
                ..c.clone()
            }),
            GeometryElement::Point(_) => continue,
        };
        let flag = source.is_construction(geom.id());
        let new_id = sketch.add_geometry(copy);
        sketch.set_construction(new_id, flag);
        map.insert(geom.id(), new_id);
    }
    copy_texts(source, sketch, &mut map, xf);
    map
}

/// The text blocks whose outlines were all copied stay text in the copy:
/// a block of their own, on its point carried by `xf` (made when its point
/// was not among the copied), turned and sized with the outlines. A mirror
/// leaves the copied outlines plain curves, since no text reads backwards.
fn copy_texts(
    source: &Sketch,
    sketch: &mut Sketch,
    map: &mut HashMap<Uuid, Uuid>,
    xf: &Similarity,
) {
    if xf.flips_orientation() {
        return;
    }
    let turn = xf.apply_vec(Vec2D::new(1.0, 0.0)).to_glam();
    let turn = turn.y.atan2(turn.x).to_degrees();
    for block in &source.texts {
        if block.elements.is_empty() || !block.elements.iter().all(|e| map.contains_key(e)) {
            continue;
        }
        let anchor = match map.get(&block.anchor) {
            Some(id) => *id,
            None => {
                let Some(at) = source.point_position(block.anchor) else {
                    continue;
                };
                let id = sketch.add_geometry(GeometryElement::Point(Point::new(xf.apply(at))));
                sketch.set_construction(id, source.is_construction(block.anchor));
                map.insert(block.anchor, id);
                id
            }
        };
        sketch.texts.push(crate::sketch::TextBlock {
            id: Uuid::new_v4(),
            anchor,
            elements: block.elements.iter().map(|e| map[e]).collect(),
            placed: xf.apply(block.placed),
            size: block.size * xf.scale_factor(),
            spacing: block.spacing * xf.scale_factor(),
            angle: block.angle + turn,
            ..block.clone()
        });
    }
}

/// Copy the constraints of `source` among the elements `map` copied into
/// `sketch`, their references renamed through `map`; a constraint that
/// names anything not copied stays behind. A fixed point's position goes
/// through `xf` as its point did. Returns how many were copied.
pub fn copy_constraints(
    source: &Sketch,
    sketch: &mut Sketch,
    map: &HashMap<Uuid, Uuid>,
    xf: &Similarity,
) -> usize {
    fn rename(value: &mut serde_json::Value, map: &HashMap<Uuid, Uuid>) {
        match value {
            serde_json::Value::String(text) => {
                if let Some(new) = text.parse::<Uuid>().ok().and_then(|old| map.get(&old)) {
                    *text = new.to_string();
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(|v| rename(v, map)),
            serde_json::Value::Object(fields) => fields.values_mut().for_each(|v| rename(v, map)),
            _ => {}
        }
    }
    let mut count = 0;
    for constraint in &source.constraints {
        let refs = crate::sketch::constraint_refs(&constraint.kind);
        if refs.is_empty() || !refs.iter().all(|r| map.contains_key(r)) {
            continue;
        }
        let Ok(mut value) = serde_json::to_value(&constraint.kind) else {
            continue;
        };
        rename(&mut value, map);
        let Ok(mut kind) = serde_json::from_value::<ConstraintKind>(value) else {
            continue;
        };
        if let ConstraintKind::FixedPoint { position, .. } = &mut kind {
            *position = xf.apply(*position);
        }
        sketch.add_constraint(kind);
        count += 1;
    }
    count
}

/// A turn as the counter-clockwise angle from 0 to a full turn that the
/// angular pitch holds.
fn geom_angle(angle: f32) -> f32 {
    angle.rem_euclid(std::f32::consts::TAU)
}

/// Cursor snapped to an existing point's *position* only.
fn snapped_pos(sketch: &Sketch, cursor: Vec2D, snap_tol: f32) -> Vec2D {
    snap::snap_to_point(sketch, cursor, snap_tol, &[])
        .position(sketch)
        .unwrap_or(cursor)
}

/// Where a translate or rotate pattern puts its copies: each one's
/// multiple of the clicked shift or turn, in order along the pattern
/// (negative before the original). `None` when the tool moves the
/// selection instead of copying it.
///
/// `copies` counts the copies, or with `copies_total` every element, the
/// original included, the click then giving the whole span. With
/// `copies_symmetric` the copies repeat on the other side of the original
/// as well.
pub(crate) fn pattern_multiples(params: &ToolParams) -> Option<Vec<f32>> {
    let (per_side, step) = if params.copies_total {
        let n = params.copies.checked_sub(1).filter(|n| *n > 0)?;
        (n, 1.0 / n as f32)
    } else if params.copies > 0 {
        (params.copies, 1.0)
    } else {
        return None;
    };
    let after = (1..=per_side as i32).map(|k| k as f32 * step);
    Some(if params.copies_symmetric {
        after.clone().rev().map(|m| -m).chain(after).collect()
    } else {
        after.collect()
    })
}

/// A full turn's copies: the elements, the original included, spread
/// evenly around, as turns from the original in radians. `None` when
/// there are no copies.
pub(crate) fn full_turn_angles(params: &ToolParams) -> Option<Vec<f32>> {
    let elements = if params.copies_total {
        params.copies
    } else {
        params.copies + 1
    };
    (elements >= 2).then(|| {
        let step = std::f32::consts::TAU / elements as f32;
        (1..elements).map(|k| k as f32 * step).collect()
    })
}

/// The reference point and its copies in pattern order, the original at
/// its place among them: what a linked pattern's pitch runs through.
fn pitch_points(
    reference: Option<Uuid>,
    multiples: &[f32],
    maps: &[HashMap<Uuid, Uuid>],
) -> Option<Vec<Uuid>> {
    let r = reference?;
    let mut ordered: Vec<(f32, Uuid)> = multiples
        .iter()
        .zip(maps)
        .map(|(m, map)| Some((*m, *map.get(&r)?)))
        .collect::<Option<_>>()?;
    ordered.push((0.0, r));
    ordered.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(ordered.into_iter().map(|(_, id)| id).collect())
}

pub(super) fn translate(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
    params: &ToolParams,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::TranslateFrom { base } => {
            *state = ToolState::Idle;
            let delta = (pos - base).to_glam();
            if selected.is_empty() || delta.length() < 1e-6 {
                return ToolEffect::none();
            }
            let Some(multiples) = pattern_multiples(params) else {
                let n = apply_to_selection(sketch, selected, &Similarity::translation(delta));
                return ToolEffect::changed(format!("Moved selection ({n} points)"));
            };
            let source = sketch.clone();
            let reference = reference_point(&source, selected);
            let linked = params.copies_linked;
            let maps: Vec<_> = multiples
                .iter()
                .map(|m| {
                    let step = Similarity::translation(delta * *m);
                    copy_linked(&source, sketch, selected, &step, linked)
                })
                .collect();
            if linked {
                equal_sizes(sketch, &source, selected, &maps);
                if let Some(points) = pitch_points(reference, &multiples, &maps) {
                    let pitch = delta
                        * multiples
                            .iter()
                            .copied()
                            .fold(f32::MAX, |a, m| a.min(m.abs()));
                    sketch.add_constraint(ConstraintKind::Pitch {
                        columns: points.len() as u32,
                        points,
                        distance: pitch.length(),
                        across: false,
                        direction: Some(Vec2D::from_glam(delta.normalize())),
                    });
                }
            }
            ToolEffect::changed(format!("Created {} translated cop(ies)", maps.len()))
        }
        _ => {
            *state = ToolState::TranslateFrom { base: pos };
            ToolEffect::none()
        }
    }
}

pub(super) fn rotate(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
    params: &ToolParams,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::RotateCenter { center } => {
            if (pos - center).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::RotateRef {
                center,
                reference: pos,
            };
            ToolEffect::none()
        }
        ToolState::RotateRef { center, reference } => {
            *state = ToolState::Idle;
            let c = center.to_glam();
            let to = pos.to_glam() - c;
            if selected.is_empty() || to.length() < 1e-6 {
                return ToolEffect::none();
            }
            let angle = (reference.to_glam() - c).angle_to(to);
            match pattern_multiples(params) {
                Some(multiples) => {
                    let angles: Vec<f32> = multiples.iter().map(|m| angle * m).collect();
                    rotated_copies(sketch, center, selected, &angles, params.copies_linked)
                }
                None => {
                    apply_to_selection(sketch, selected, &Similarity::rotation_about(c, angle));
                    ToolEffect::changed(format!("Rotated selection by {:.1}°", angle.to_degrees()))
                }
            }
        }
        // A full turn needs only its centre: the copies go evenly around.
        _ if params.rotate_full_turn
            && let Some(angles) = full_turn_angles(params) =>
        {
            *state = ToolState::Idle;
            if selected.is_empty() {
                return ToolEffect::none();
            }
            rotated_copies(sketch, pos, selected, &angles, params.copies_linked)
        }
        _ => {
            *state = ToolState::RotateCenter { center: pos };
            ToolEffect::none()
        }
    }
}

/// Copies of the selection turned about `center` by each of `angles`, in
/// pattern order; linked, held to one polar pitch about the centre.
fn rotated_copies(
    sketch: &mut Sketch,
    center: Vec2D,
    selected: &HashSet<Uuid>,
    angles: &[f32],
    linked: bool,
) -> ToolEffect {
    let c = center.to_glam();
    let source = sketch.clone();
    let reference = reference_point(&source, selected);
    let maps: Vec<_> = angles
        .iter()
        .map(|a| {
            let step = Similarity::rotation_about(c, *a);
            copy_linked(&source, sketch, selected, &step, linked)
        })
        .collect();
    if linked {
        equal_sizes(sketch, &source, selected, &maps);
        if let Some(points) = pitch_points(reference, angles, &maps) {
            // The pivot as a point the pitch turns about: the one there,
            // or a construction point put there.
            let pivot = match snap::snap_to_point(sketch, center, 1e-5, &[]) {
                snap::SnapTarget::Existing(id) => id,
                _ => {
                    let id = sketch.add_geometry(GeometryElement::Point(Point::new(center)));
                    sketch.set_construction(id, true);
                    id
                }
            };
            // In pattern order each point is one step counter-clockwise
            // of the one before.
            let pitch = angles.iter().copied().fold(f32::MAX, |a, m| a.min(m.abs()));
            sketch.add_constraint(ConstraintKind::PolarPitch {
                center: pivot,
                points,
                angle_rad: geom_angle(pitch),
            });
        }
    }
    ToolEffect::changed(format!("Created {} rotated cop(ies)", maps.len()))
}

pub(super) fn scale(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
    copies: u32,
) -> ToolEffect {
    let pos = snapped_pos(sketch, cursor, snap_tol);
    match *state {
        ToolState::ScaleBase { base } => {
            if (pos - base).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            *state = ToolState::ScaleRef {
                base,
                reference: pos,
            };
            ToolEffect::none()
        }
        ToolState::ScaleRef { base, reference } => {
            *state = ToolState::Idle;
            let b = base.to_glam();
            let factor = (pos.to_glam() - b).length() / (reference.to_glam() - b).length();
            if selected.is_empty() || factor < 1e-4 {
                return ToolEffect::none();
            }
            if copies == 0 {
                apply_to_selection(sketch, selected, &Similarity::scale_about(b, factor));
                return ToolEffect::changed(format!("Scaled selection by {factor:.3}"));
            }
            // The original stays; each copy is scaled once more than the one
            // before it.
            let source = sketch.clone();
            for k in 1..=copies {
                let step = Similarity::scale_about(b, factor.powi(k as i32));
                copy_from(&source, sketch, selected, &step);
            }
            ToolEffect::changed(format!("Scaled {copies} copies by {factor:.3} each"))
        }
        _ => {
            *state = ToolState::ScaleBase { base: pos };
            ToolEffect::none()
        }
    }
}

/// The line element under the cursor and its endpoints, for one-click
/// mirror axes.
fn line_under_cursor(sketch: &Sketch, cursor: Vec2D, tol: f32) -> Option<(Vec2D, Vec2D, Uuid)> {
    sketch
        .geometry
        .iter()
        .filter_map(|g| match g {
            GeometryElement::Line(l) => {
                let d = snap::distance_to_element(sketch, g, cursor)?;
                (d <= tol).then(|| {
                    (
                        sketch.point_position(l.start),
                        sketch.point_position(l.end),
                        d,
                        l.id,
                    )
                })
            }
            _ => None,
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .and_then(|(a, b, _, id)| Some((a?, b?, id)))
}

/// What a mirror reflects about.
#[derive(Debug, Clone, Copy)]
enum MirrorAbout {
    /// The line through two points; `element` when a line was clicked.
    Line {
        a: Vec2D,
        b: Vec2D,
        element: Option<Uuid>,
    },
    /// A point; `point` when an existing one was clicked.
    Point { at: Vec2D, point: Option<Uuid> },
}

fn mirror_now(
    state: &mut ToolState,
    sketch: &mut Sketch,
    selected: &HashSet<Uuid>,
    about: MirrorAbout,
    params: &super::ToolParams,
) -> ToolEffect {
    *state = ToolState::Idle;
    if selected.is_empty() {
        return ToolEffect::none();
    }
    let xf = match about {
        MirrorAbout::Line { a, b, .. } => {
            if (b - a).to_glam().length() < 1e-6 {
                return ToolEffect::none();
            }
            Similarity::mirror_about(a.to_glam(), b.to_glam())
        }
        MirrorAbout::Point { at, .. } => Similarity::point_reflection(at.to_glam()),
    };
    // A linked image follows the original, which must stay for it to.
    let linked = params.mirror_linked && params.mirror_keep;
    let n = if linked {
        let source = sketch.clone();
        let map = copy_mapped(&source, sketch, selected, &xf);
        link_image(sketch, &source, &map, about);
        map.len()
    } else {
        copy_selection(sketch, selected, &xf)
    };
    if !params.mirror_keep {
        let originals: Vec<Uuid> = selected.iter().copied().collect();
        crate::commands::delete_items(sketch, &originals);
    }
    ToolEffect::changed(format!("Mirrored {n} element(s)"))
}

/// The mirror's line or centre as an element to hold symmetry to: the one
/// clicked, or else a construction line or point made and blocked where
/// the clicks put it.
fn mirror_reference(sketch: &mut Sketch, about: MirrorAbout) -> (Uuid, bool) {
    let point_at = |sketch: &mut Sketch, at: Vec2D| match snap::snap_to_point(sketch, at, 1e-5, &[])
    {
        snap::SnapTarget::Existing(id) => id,
        _ => sketch.add_geometry(GeometryElement::Point(Point::new(at))),
    };
    match about {
        MirrorAbout::Line {
            element: Some(line),
            ..
        } => (line, true),
        MirrorAbout::Line { a, b, .. } => {
            let (pa, pb) = (point_at(sketch, a), point_at(sketch, b));
            let line = sketch.add_geometry(GeometryElement::Line(Line::new(pa, pb)));
            sketch.set_construction(line, true);
            sketch.add_constraint(ConstraintKind::Block { element: line });
            (line, true)
        }
        MirrorAbout::Point { point: Some(p), .. } => (p, false),
        MirrorAbout::Point { at, .. } => {
            let p = point_at(sketch, at);
            sketch.set_construction(p, true);
            sketch.add_constraint(ConstraintKind::Block { element: p });
            (p, false)
        }
    }
}

/// Hold each copied point symmetric to its original, and each copied
/// circle, arc and ellipse the size of its original, so the image follows
/// the original's edits.
fn link_image(sketch: &mut Sketch, source: &Sketch, map: &HashMap<Uuid, Uuid>, about: MirrorAbout) {
    let (reference, is_line) = mirror_reference(sketch, about);
    for (old, new) in map {
        match source.get_geometry(*old) {
            Some(GeometryElement::Point(_)) => {
                sketch.add_constraint(if is_line {
                    ConstraintKind::Symmetric {
                        point1: *old,
                        point2: *new,
                        line: reference,
                    }
                } else {
                    ConstraintKind::SymmetricAboutPoint {
                        point1: *old,
                        point2: *new,
                        center: reference,
                    }
                });
            }
            Some(GeometryElement::Circle(_) | GeometryElement::Arc(_)) => {
                sketch.add_constraint(ConstraintKind::EqualRadius {
                    circle1: *old,
                    circle2: *new,
                });
            }
            Some(GeometryElement::Ellipse(_)) => {
                sketch.add_constraint(ConstraintKind::EqualEllipse {
                    ellipse1: *old,
                    ellipse2: *new,
                });
            }
            _ => {}
        }
    }
}

pub(super) fn mirror(
    state: &mut ToolState,
    sketch: &mut Sketch,
    cursor: Vec2D,
    snap_tol: f32,
    selected: &HashSet<Uuid>,
    params: &super::ToolParams,
) -> ToolEffect {
    if params.mirror_center {
        // One click: the centre, an existing point where there is one.
        let point = match snap::snap_to_point(sketch, cursor, snap_tol, &[]) {
            snap::SnapTarget::Existing(id) => Some(id),
            _ => None,
        };
        let at = point
            .and_then(|id| sketch.point_position(id))
            .unwrap_or_else(|| snapped_pos(sketch, cursor, snap_tol));
        return mirror_now(
            state,
            sketch,
            selected,
            MirrorAbout::Point { at, point },
            params,
        );
    }
    match *state {
        ToolState::MirrorAxisFrom { a } => {
            let b = snapped_pos(sketch, cursor, snap_tol);
            let about = MirrorAbout::Line {
                a,
                b,
                element: None,
            };
            mirror_now(state, sketch, selected, about, params)
        }
        _ => {
            // A clicked point starts a two-point axis; otherwise a clicked
            // line element is the axis; empty space starts a two-point axis.
            if let snap::SnapTarget::Existing(pid) =
                snap::snap_to_point(sketch, cursor, snap_tol, &[])
                && let Some(pos) = sketch.point_position(pid)
            {
                *state = ToolState::MirrorAxisFrom { a: pos };
                return ToolEffect::none();
            }
            if let Some((a, b, element)) = line_under_cursor(sketch, cursor, snap_tol) {
                let about = MirrorAbout::Line {
                    a,
                    b,
                    element: Some(element),
                };
                return mirror_now(state, sketch, selected, about, params);
            }
            *state = ToolState::MirrorAxisFrom { a: cursor };
            ToolEffect::none()
        }
    }
}
