//! External geometry: a solid's edges projected into the sketch, and the
//! curves where its faces cross the sketch plane (intersection references).
//!
//! The kernel gives each as the exact curve it is on the sketch plane (a
//! line, a circle or arc, an ellipse or arc of one, an arc of a parabola or
//! of a hyperbola's branch, a spline, a point); a curve with no closed form
//! comes as points along it and is kept as a chain of lines. Each element remembers its edge or face (`ExternalSource`), so the
//! sketch brings it up to the solid again when it is edited and the solid
//! has changed: in place where the shapes are the same kinds, keeping the
//! constraints that refer to them, and afresh where they are not.

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use core_document::{BodyId, FeatureId, InputResult, WorkbenchRuntimeContext};
use kernel_api::ProjectedEdge;
use uuid::Uuid;

use crate::boundary::widen3;
use crate::external_follow::{external_stamp, lost_external_elements, project_source};
use crate::sketch::{
    Arc, BSpline, Circle, Conic, ConicKind, Ellipse, ExternalSource, GeometryElement, Line, Point,
    Sketch, Vec2D,
};
use crate::{SketchWorkbench, commands, sketch};

fn v(p: [f64; 2]) -> Vec2D {
    Vec2D::new(p[0], p[1])
}

/// Whether a range of angles closes the turn.
fn full_turn(range: (f64, f64)) -> bool {
    range.1 - range.0 >= TAU - 1e-9
}

fn circle_point(centre: [f64; 2], radius: f64, t: f64) -> [f64; 2] {
    [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
}

fn ellipse_point(centre: [f64; 2], major: [f64; 2], ratio: f64, t: f64) -> [f64; 2] {
    let minor = [-major[1] * ratio, major[0] * ratio];
    [
        centre[0] + major[0] * t.cos() + minor[0] * t.sin(),
        centre[1] + major[1] * t.cos() + minor[1] * t.sin(),
    ]
}

/// A projected parabola or hyperbola in the sketch's terms.
struct ProjectedConic {
    kind: ConicKind,
    /// The parabola's vertex, the hyperbola's centre.
    origin: [f64; 2],
    /// Along the axis, as long as the focal distance or the semi-major axis.
    axis: Vec2D,
    /// The hyperbola's semi-minor axis.
    minor: f64,
    start: [f64; 2],
    end: [f64; 2],
}

fn conic_of(projected: &ProjectedEdge) -> Option<ProjectedConic> {
    match projected {
        ProjectedEdge::Parabola {
            vertex,
            axis,
            focal,
            range,
        } => {
            let side = [-axis[1], axis[0]];
            let at = |s: f64| {
                let along = s * s / (4.0 * focal);
                [
                    vertex[0] + axis[0] * along + side[0] * s,
                    vertex[1] + axis[1] * along + side[1] * s,
                ]
            };
            Some(ProjectedConic {
                kind: ConicKind::Parabola,
                origin: *vertex,
                axis: v([axis[0] * focal, axis[1] * focal]),
                minor: 0.0,
                start: at(range.0),
                end: at(range.1),
            })
        }
        ProjectedEdge::Hyperbola {
            centre,
            major,
            minor,
            range,
        } => {
            let at = |s: f64| {
                [
                    centre[0] + major[0] * s.cosh() + minor[0] * s.sinh(),
                    centre[1] + major[1] * s.cosh() + minor[1] * s.sinh(),
                ]
            };
            Some(ProjectedConic {
                kind: ConicKind::Hyperbola,
                origin: *centre,
                axis: v(*major),
                minor: minor[0].hypot(minor[1]),
                start: at(range.0),
                end: at(range.1),
            })
        }
        _ => None,
    }
}

/// Add a projected edge to the sketch as external geometry from `source`.
/// Returns how many elements it became.
pub fn add(sketch: &mut Sketch, projected: &ProjectedEdge, source: ExternalSource) -> usize {
    let point = |sketch: &mut Sketch, p: [f64; 2]| {
        sketch.add_geometry(GeometryElement::Point(Point::new(v(p))))
    };
    let mut made: Vec<Uuid> = Vec::new();
    match projected {
        ProjectedEdge::Point(p) => made.push(point(sketch, *p)),
        ProjectedEdge::Line { start, end } => {
            let (a, b) = (point(sketch, *start), point(sketch, *end));
            made.push(sketch.add_geometry(GeometryElement::Line(Line::new(a, b))));
        }
        ProjectedEdge::Circle {
            centre,
            radius,
            range,
        } => {
            let c = point(sketch, *centre);
            let element = if full_turn(*range) {
                GeometryElement::Circle(Circle::new(c, *radius))
            } else {
                let s = point(sketch, circle_point(*centre, *radius, range.0));
                let e = point(sketch, circle_point(*centre, *radius, range.1));
                GeometryElement::Arc(Arc::new(c, s, e, *radius))
            };
            made.push(sketch.add_geometry(element));
        }
        ProjectedEdge::Ellipse {
            centre,
            major,
            ratio,
            range,
        } => {
            let c = point(sketch, *centre);
            let element = if full_turn(*range) {
                GeometryElement::Ellipse(Ellipse::new(c, v(*major), *ratio))
            } else {
                let s = point(sketch, ellipse_point(*centre, *major, *ratio, range.0));
                let e = point(sketch, ellipse_point(*centre, *major, *ratio, range.1));
                GeometryElement::Ellipse(Ellipse::new_arc(c, v(*major), *ratio, s, e))
            };
            made.push(sketch.add_geometry(element));
        }
        ProjectedEdge::Parabola { .. } | ProjectedEdge::Hyperbola { .. } => {
            if let Some(conic) = conic_of(projected) {
                let c = point(sketch, conic.origin);
                let (s, e) = (point(sketch, conic.start), point(sketch, conic.end));
                made.push(sketch.add_geometry(GeometryElement::Conic(Conic::new(
                    conic.kind,
                    c,
                    conic.axis,
                    conic.minor,
                    s,
                    e,
                ))));
            }
        }
        ProjectedEdge::Spline {
            degree,
            knots,
            control_points,
            weights,
        } => {
            let ids: Vec<Uuid> = control_points.iter().map(|p| point(sketch, *p)).collect();
            made.push(sketch.add_geometry(GeometryElement::BSpline(BSpline {
                degree: *degree,
                knots: knots.clone(),
                weights: weights.clone(),
                ..BSpline::new(ids, false)
            })));
        }
        ProjectedEdge::Polyline(points) => {
            let ids: Vec<Uuid> = points.iter().map(|p| point(sketch, *p)).collect();
            for pair in ids.windows(2) {
                made.push(sketch.add_geometry(GeometryElement::Line(Line::new(pair[0], pair[1]))));
            }
        }
    }
    let count = made.len();
    for id in made {
        sketch.external.insert(id, source);
    }
    count
}

/// How many sketch elements `add` makes of a projected curve.
fn element_count(projected: &ProjectedEdge) -> usize {
    match projected {
        ProjectedEdge::Polyline(points) => points.len().saturating_sub(1),
        _ => 1,
    }
}

/// Move the points of `group` (one curve's elements, in the order they
/// were made) to `projected`, when it is the same kind of shape. Returns
/// whether it was.
fn update(sketch: &mut Sketch, group: &[Uuid], projected: &ProjectedEdge) -> bool {
    let place = |sketch: &mut Sketch, id: Uuid, p: [f64; 2]| {
        if let Some(GeometryElement::Point(point)) = sketch.get_geometry_mut(id) {
            point.position = v(p);
        }
    };
    let elements: Vec<GeometryElement> = group
        .iter()
        .filter_map(|id| sketch.get_geometry(*id).cloned())
        .collect();
    match (projected, elements.as_slice()) {
        (ProjectedEdge::Point(p), [GeometryElement::Point(point)]) => {
            place(sketch, point.id, *p);
        }
        (ProjectedEdge::Line { start, end }, [GeometryElement::Line(line)]) => {
            place(sketch, line.start, *start);
            place(sketch, line.end, *end);
        }
        (
            ProjectedEdge::Circle {
                centre,
                radius,
                range,
            },
            [GeometryElement::Circle(circle)],
        ) if full_turn(*range) => {
            place(sketch, circle.center, *centre);
            if let Some(GeometryElement::Circle(c)) = sketch.get_geometry_mut(circle.id) {
                c.radius = *radius;
            }
        }
        (
            ProjectedEdge::Circle {
                centre,
                radius,
                range,
            },
            [GeometryElement::Arc(arc)],
        ) if !full_turn(*range) => {
            place(sketch, arc.center, *centre);
            place(sketch, arc.start, circle_point(*centre, *radius, range.0));
            place(sketch, arc.end, circle_point(*centre, *radius, range.1));
            if let Some(GeometryElement::Arc(a)) = sketch.get_geometry_mut(arc.id) {
                a.radius = *radius;
            }
        }
        (
            ProjectedEdge::Ellipse {
                centre,
                major,
                ratio,
                range,
            },
            [GeometryElement::Ellipse(ellipse)],
        ) if full_turn(*range) == ellipse.arc.is_none() => {
            place(sketch, ellipse.center, *centre);
            if let Some(ends) = ellipse.arc {
                place(
                    sketch,
                    ends.start,
                    ellipse_point(*centre, *major, *ratio, range.0),
                );
                place(
                    sketch,
                    ends.end,
                    ellipse_point(*centre, *major, *ratio, range.1),
                );
            }
            if let Some(GeometryElement::Ellipse(e)) = sketch.get_geometry_mut(ellipse.id) {
                e.major = v(*major);
                e.ratio = *ratio;
            }
        }
        (
            ProjectedEdge::Parabola { .. } | ProjectedEdge::Hyperbola { .. },
            [GeometryElement::Conic(conic)],
        ) => {
            let Some(now) = conic_of(projected).filter(|now| now.kind == conic.kind) else {
                return false;
            };
            place(sketch, conic.center, now.origin);
            place(sketch, conic.start, now.start);
            place(sketch, conic.end, now.end);
            if let Some(GeometryElement::Conic(c)) = sketch.get_geometry_mut(conic.id) {
                c.axis = now.axis;
                c.minor = now.minor;
            }
        }
        (
            ProjectedEdge::Spline {
                degree,
                knots,
                control_points,
                weights,
            },
            [GeometryElement::BSpline(spline)],
        ) if !spline.periodic && spline.control_points.len() == control_points.len() => {
            for (id, p) in spline.control_points.iter().zip(control_points) {
                place(sketch, *id, *p);
            }
            if let Some(GeometryElement::BSpline(b)) = sketch.get_geometry_mut(spline.id) {
                b.degree = *degree;
                b.knots = knots.clone();
                b.weights = weights.clone();
            }
        }
        (ProjectedEdge::Polyline(points), lines)
            if lines.len() + 1 == points.len()
                && lines.iter().all(|l| matches!(l, GeometryElement::Line(_))) =>
        {
            for (line, pair) in lines.iter().zip(points.windows(2)) {
                if let GeometryElement::Line(line) = line {
                    place(sketch, line.start, pair[0]);
                    place(sketch, line.end, pair[1]);
                }
            }
        }
        _ => return false,
    }
    true
}

/// The external elements grouped by the edge they came from, each group
/// in the order its elements were made.
pub fn groups(sketch: &Sketch) -> Vec<(ExternalSource, Vec<Uuid>)> {
    let key = |s: &ExternalSource| {
        let mut k = s.body.as_u128().to_le_bytes().to_vec();
        for c in s.point.iter().chain(&s.direction) {
            k.extend(c.to_bits().to_le_bytes());
        }
        k.push(u8::from(s.section));
        k.push(u8::from(s.defining));
        match s.reference {
            Some(crate::sketch::ExternalReference::SketchElement { sketch, element }) => {
                k.extend(sketch.as_u128().to_le_bytes());
                k.extend(element.as_u128().to_le_bytes());
            }
            Some(crate::sketch::ExternalReference::Datum { datum }) => {
                k.extend(datum.as_u128().to_le_bytes());
            }
            None => {}
        }
        k
    };
    let mut by_source: BTreeMap<Vec<u8>, (ExternalSource, Vec<Uuid>)> = BTreeMap::new();
    for element in &sketch.geometry {
        if let Some(source) = sketch.external.get(&element.id()) {
            by_source
                .entry(key(source))
                .or_insert_with(|| (*source, Vec::new()))
                .1
                .push(element.id());
        }
    }
    by_source.into_values().collect()
}

/// Move one source's elements to `projected` where they are the same kinds
/// of shape, keeping their ids. Returns whether they were; when not, the
/// sketch may be partly moved.
pub fn refresh_in_place(sketch: &mut Sketch, group: &[Uuid], projected: &[ProjectedEdge]) -> bool {
    let counts: Vec<usize> = projected.iter().map(element_count).collect();
    counts.iter().sum::<usize>() == group.len() && {
        let mut at = 0;
        projected.iter().zip(&counts).all(|(curve, n)| {
            let fits = update(sketch, &group[at..at + n], curve);
            at += n;
            fits
        })
    }
}

/// Whether `projected` is what one source's elements already are: the same
/// kinds of shape, every point within `REPEAT_MM` of where it is.
pub fn already_holds(sketch: &Sketch, group: &[Uuid], projected: &[ProjectedEdge]) -> bool {
    let mut moved = sketch.clone();
    if !refresh_in_place(&mut moved, group, projected) {
        return false;
    }
    let near = |a: Option<Vec2D>, b: Option<Vec2D>| match (a, b) {
        (Some(a), Some(b)) => (a.to_glam() - b.to_glam()).length() <= REPEAT_MM,
        _ => false,
    };
    group.iter().all(|id| {
        let (Some(was), Some(now)) = (sketch.get_geometry(*id), moved.get_geometry(*id)) else {
            return false;
        };
        let points: Vec<Uuid> = match was {
            GeometryElement::Point(p) => vec![p.id],
            other => Sketch::curve_point_ids(other),
        };
        let size = |g: &GeometryElement| match g {
            GeometryElement::Circle(c) => c.radius,
            GeometryElement::Arc(a) => a.radius,
            GeometryElement::Ellipse(e) => e.major.to_glam().length() * (1.0 + e.ratio),
            GeometryElement::Conic(c) => c.axis.to_glam().length() + c.minor,
            _ => 0.0,
        };
        (size(was) - size(now)).abs() <= REPEAT_MM
            && points
                .iter()
                .all(|p| near(sketch.point_position(*p), moved.point_position(*p)))
    })
}

/// How close a projection must come to external geometry already in the
/// sketch to be the same curve again, mm.
const REPEAT_MM: f64 = 1e-3;

/// Bring one source's elements up to `projected`, the curves its edge or
/// face comes to now: moved in place where they are the same kinds of
/// shape as before, else made again (constraints on the old ones go with
/// them).
pub fn refresh_group(
    sketch: &mut Sketch,
    source: ExternalSource,
    group: &[Uuid],
    projected: &[ProjectedEdge],
) {
    if !refresh_in_place(sketch, group, projected) {
        sketch.remove_geometry_cascade(group);
        for curve in projected {
            add(sketch, curve, source);
        }
    }
}

impl SketchWorkbench {
    /// Sketch `id`'s external elements whose source is gone, worked out
    /// again only when what they were worked out from changed.
    pub(crate) fn lost_external(
        &self,
        document: &core_document::Document,
        kernel: Option<&dyn kernel_api::KernelQueries>,
        id: FeatureId,
    ) -> Vec<Uuid> {
        let Some(stamp) = external_stamp(document, id) else {
            return Vec::new();
        };
        let Ok(mut cache) = self.lost_external.lock() else {
            return Vec::new();
        };
        if let Some((at, lost)) = cache.get(&id)
            && *at == stamp
        {
            return lost.clone();
        }
        let lost = lost_external_elements(document, kernel, id);
        cache.insert(id, (stamp, lost.clone()));
        lost
    }

    /// Selected external geometry counts in profiles, or stops counting:
    /// all of it on when any was off.
    pub(crate) fn toggle_external_defining(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
    ) -> InputResult {
        let (Some(mut feature), Some(id)) = (self.get_active_sketch(ctx), self.active_sketch_id)
        else {
            return InputResult::ignored();
        };
        let items: Vec<Uuid> = self
            .selected
            .iter()
            .copied()
            .filter(|e| feature.sketch.external.contains_key(e))
            .collect();
        if items.is_empty() {
            ctx.log_warn("Select external geometry to count in the profile");
            return InputResult::consumed();
        }
        let on = items.iter().any(|e| !feature.sketch.external[e].defining);
        commands::set_external_defining(&mut feature.sketch, &items, on);
        self.store_sketch(ctx, feature);
        ctx.record(
            "sketch.external_defining",
            commands::args(serde_json::json!({
                "sketch": id.0.to_string(),
                "items": items.iter().map(|i| i.to_string()).collect::<Vec<_>>(),
                "on": on,
            })),
            serde_json::Value::Null,
        );
        ctx.log_info(if on {
            "External geometry counts in the profile"
        } else {
            "External geometry only guides the sketch"
        });
        InputResult::consumed()
    }

    /// Project every edge picked since the external geometry tool was
    /// armed into the edited sketch; a face picked brings every edge
    /// around it.
    pub(crate) fn take_external_picks(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let fresh: Vec<core_document::EdgeRef> = ctx
            .selected_edges
            .iter()
            .filter(|e| {
                self.external_seen
                    .insert((e.body, e.point.map(f32::to_bits)))
            })
            .copied()
            .collect();
        // As the drawing tools do: a guide in construction mode, counting
        // in the profile out of it.
        let defining = !self.construction_mode();
        let mut sources: Vec<sketch::ExternalSource> = fresh
            .iter()
            .map(|edge| {
                let local = edge.moved(&ctx.document.body_placement(BodyId(edge.body)).inverse());
                sketch::ExternalSource {
                    body: edge.body,
                    point: widen3(local.point),
                    direction: widen3(local.direction),
                    section: false,
                    defining,
                    reference: None,
                    faces: [0; 2],
                }
            })
            .collect();
        if ctx.selected_edges.is_empty() {
            sources.extend(self.face_outline_sources(ctx, defining));
        }
        if sources.is_empty() {
            return;
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let before = (
            feature
                .sketch
                .geometry
                .iter()
                .map(GeometryElement::id)
                .collect(),
            feature.sketch.constraints.iter().map(|c| c.id).collect(),
        );
        let added = match commands::add_external(ctx, &feature.plane, &mut feature.sketch, &sources)
        {
            Ok(added) => added,
            Err(why) => {
                ctx.log_warn(format!("Could not bring that edge in: {why}"));
                0
            }
        };
        if added > 0
            && let Some(id) = self.active_sketch_id
        {
            let edges: Vec<serde_json::Value> = sources
                .iter()
                .map(|s| {
                    serde_json::json!({
                        "body": s.body.to_string(),
                        "point": s.point,
                        "direction": s.direction,
                    })
                })
                .collect();
            ctx.record(
                "sketch.external",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "edges": edges,
                    "counts": !self.construction_mode(),
                })),
                commands::made_since(&feature.sketch, &before),
            );
        }
        if added > 0 {
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
            ctx.log_info(match added {
                1 => "Added 1 external element".to_string(),
                n => format!("Added {n} external elements"),
            });
        }
    }

    /// The edges around the face picked on a solid, as projection sources
    /// in its body's frame, once per face picked.
    fn face_outline_sources(
        &mut self,
        ctx: &mut WorkbenchRuntimeContext,
        defining: bool,
    ) -> Vec<sketch::ExternalSource> {
        let (Some(face), Some(body)) = (ctx.selected_face, ctx.selected_body_id) else {
            return Vec::new();
        };
        if !self
            .external_seen
            .insert((body, face.point.map(f32::to_bits)))
        {
            return Vec::new();
        }
        let (Some(kernel), Some(brep)) =
            (ctx.kernel, ctx.document.imported_brep_blob(BodyId(body)))
        else {
            return Vec::new();
        };
        let local = face.moved(&ctx.document.body_placement(BodyId(body)).inverse());
        match kernel.face_edges(brep, local.point.map(f64::from)) {
            Ok(edges) => edges
                .into_iter()
                .map(|(point, direction)| sketch::ExternalSource {
                    body,
                    point,
                    direction,
                    section: false,
                    defining,
                    reference: None,
                    faces: [0; 2],
                })
                .collect(),
            Err(why) => {
                ctx.log_warn(format!("The face's edges could not be read: {why}"));
                Vec::new()
            }
        }
    }

    /// Add where every face picked since the intersection tool was armed
    /// crosses the edited sketch's plane.
    pub(crate) fn take_intersection_picks(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let (Some(face), Some(body)) = (ctx.selected_face, ctx.selected_body_id) else {
            return;
        };
        if !self
            .external_seen
            .insert((body, face.point.map(f32::to_bits)))
        {
            return;
        }
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let local = face.moved(&ctx.document.body_placement(BodyId(body)).inverse());
        let source = sketch::ExternalSource {
            body,
            point: widen3(local.point),
            direction: widen3(local.normal),
            section: true,
            defining: !self.construction_mode(),
            reference: None,
            faces: [0; 2],
        };
        let before = commands::ids_of(&feature.sketch);
        let added =
            match commands::add_external(ctx, &feature.plane, &mut feature.sketch, &[source]) {
                Ok(added) => added,
                Err(why) => {
                    ctx.log_warn(format!("No intersection with that face: {why}"));
                    return;
                }
            };
        if let Some(id) = self.active_sketch_id {
            ctx.record(
                "sketch.intersection",
                commands::args(serde_json::json!({
                    "sketch": id.0.to_string(),
                    "faces": [{
                        "body": body.to_string(),
                        "point": source.point,
                        "normal": source.direction,
                    }],
                    "counts": !self.construction_mode(),
                })),
                commands::made_since(&feature.sketch, &before),
            );
        }
        self.solve(ctx, &mut feature);
        self.store_sketch(ctx, feature);
        ctx.log_info(match added {
            1 => "Added 1 intersection element".to_string(),
            n => format!("Added {n} intersection elements"),
        });
    }

    /// Bring the edited sketch's external geometry up to the solids it came
    /// from: each edge projected again and each face cut again, moved in
    /// place where the curves are the same kinds. An edge or face that no
    /// longer exists is left as it was.
    pub(crate) fn refresh_external(&mut self, ctx: &mut WorkbenchRuntimeContext) {
        let Some(mut feature) = self.get_active_sketch(ctx) else {
            return;
        };
        let groups = groups(&feature.sketch);
        if groups.is_empty() {
            return;
        }
        let before = feature.sketch.clone();
        let mut lost = 0;
        for (source, group) in groups {
            // A solid's edge needs the kernel; another sketch or a datum
            // does not.
            if source.reference.is_none() && ctx.kernel.is_none() {
                continue;
            }
            match project_source(ctx, &feature.plane, &source) {
                Ok(projected) => {
                    refresh_group(&mut feature.sketch, source, &group, &projected);
                }
                Err(_) => lost += 1,
            }
        }
        if lost > 0 {
            ctx.log_warn(format!(
                "{lost} external element(s) kept where they were: their edges or faces could not be reached"
            ));
        }
        // Nothing moved: no edit to record.
        if serde_json::to_value(&before).ok() != serde_json::to_value(&feature.sketch).ok() {
            self.solve(ctx, &mut feature);
            self.store_sketch(ctx, feature);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_kinds::arc_of;

    fn source() -> ExternalSource {
        ExternalSource {
            body: Uuid::nil(),
            point: [1.0, 2.0, 3.0],
            direction: [0.0, 0.0, 1.0],
            section: false,
            defining: false,
            reference: None,
            faces: [0; 2],
        }
    }

    /// A projected spline is one spline of the sketch, with the kernel's
    /// degree, knots and weights, which the profile hands on exactly and a
    /// fresh projection moves in place.
    /// An edge projected again, from any point along it, is the curve the
    /// sketch holds; a neighbouring one is not.
    #[test]
    fn a_projection_already_held_is_known_again() {
        let mut sketch = Sketch::new("t");
        let edge = |y: f64| ProjectedEdge::Line {
            start: [0.0, y],
            end: [10.0, y],
        };
        add(&mut sketch, &edge(0.0), source());
        let (_, group) = groups(&sketch).remove(0);
        assert!(already_holds(&sketch, &group, &[edge(0.0)]));
        assert!(!already_holds(&sketch, &group, &[edge(1.0)]));
        let circle = ProjectedEdge::Circle {
            centre: [0.0, 0.0],
            radius: 2.0,
            range: (0.0, TAU),
        };
        assert!(!already_holds(&sketch, &group, &[circle]));
    }

    #[test]
    fn a_projected_spline_stays_a_spline() {
        let mut sketch = Sketch::new("t");
        let spline = |lift: f64| ProjectedEdge::Spline {
            degree: 2,
            knots: vec![0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            control_points: vec![[0.0, 0.0], [5.0, 5.0 + lift], [10.0, 0.0]],
            weights: vec![1.0, 0.5, 1.0],
        };
        let counting = ExternalSource {
            defining: true,
            ..source()
        };
        assert_eq!(add(&mut sketch, &spline(0.0), counting), 1);
        let a = sketch.geometry[0].id();
        let c = sketch.geometry[2].id();
        let line = sketch.add_geometry(GeometryElement::Line(Line::new(c, a)));
        assert!(sketch.get_geometry(line).is_some());
        let wires = crate::profile::extract_wires(&sketch).expect("the spline and line close");
        let nurbs = wires[0]
            .segments
            .iter()
            .find_map(|s| match s {
                kernel_api::ProfileSegment::Nurbs {
                    degree, weights, ..
                } => Some((*degree, weights.clone())),
                _ => None,
            })
            .expect("the spline goes across as a NURBS segment");
        assert_eq!(nurbs, (2, vec![1.0, 0.5, 1.0]));

        let group = groups(&sketch)[0].1.clone();
        assert!(update(&mut sketch, &group, &spline(1.0)), "in place");
        let GeometryElement::BSpline(b) = sketch.get_geometry(group[0]).unwrap() else {
            panic!("still a spline");
        };
        let middle = sketch.point_position(b.control_points[1]).unwrap();
        assert!((middle.y - 6.0).abs() < 1e-6, "{middle:?}");
    }

    #[test]
    fn each_projection_becomes_the_sketch_curve_it_is() {
        let mut sketch = Sketch::new("t");
        let full = ProjectedEdge::Circle {
            centre: [1.0, 2.0],
            radius: 3.0,
            range: (0.0, TAU),
        };
        assert_eq!(add(&mut sketch, &full, source()), 1);
        let arc = ProjectedEdge::Circle {
            centre: [0.0, 0.0],
            radius: 2.0,
            range: (0.0, std::f64::consts::FRAC_PI_2),
        };
        add(&mut sketch, &arc, source());
        let chain = ProjectedEdge::Polyline(vec![[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]]);
        assert_eq!(add(&mut sketch, &chain, source()), 2);
        let kinds: Vec<&str> = sketch
            .geometry
            .iter()
            .filter(|g| sketch.external.contains_key(&g.id()))
            .map(|g| match g {
                GeometryElement::Circle(_) => "circle",
                GeometryElement::Arc(_) => "arc",
                GeometryElement::Line(_) => "line",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, ["circle", "arc", "line", "line"]);
        // The arc's ends sit on the circle where its angles say.
        let arc = arc_of(&sketch);
        let end = sketch.point_position(arc.end).unwrap();
        assert!(end.x.abs() < 1e-6 && (end.y - 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_refresh_moves_the_same_kind_in_place_and_remakes_another() {
        let mut sketch = Sketch::new("t");
        add(
            &mut sketch,
            &ProjectedEdge::Line {
                start: [0.0, 0.0],
                end: [5.0, 0.0],
            },
            source(),
        );
        let (src, group) = groups(&sketch).remove(0);
        let line_id = group[0];
        refresh_group(
            &mut sketch,
            src,
            &group,
            &[ProjectedEdge::Line {
                start: [0.0, 1.0],
                end: [8.0, 1.0],
            }],
        );
        assert!(sketch.get_geometry(line_id).is_some(), "moved in place");
        let GeometryElement::Line(line) = sketch.get_geometry(line_id).unwrap() else {
            panic!()
        };
        assert_eq!(sketch.point_position(line.end), Some(Vec2D::new(8.0, 1.0)));
        // The edge is now round: the line gives way to a circle.
        refresh_group(
            &mut sketch,
            src,
            &group,
            &[ProjectedEdge::Circle {
                centre: [0.0, 0.0],
                radius: 1.0,
                range: (0.0, TAU),
            }],
        );
        assert!(sketch.get_geometry(line_id).is_none());
        assert_eq!(groups(&sketch).len(), 1);
        assert!(matches!(
            sketch.get_geometry(groups(&sketch)[0].1[0]),
            Some(GeometryElement::Circle(_))
        ));
    }

    #[test]
    fn a_face_cut_in_two_lines_refreshes_both_and_remakes_them_when_they_change() {
        let face = ExternalSource {
            section: true,
            defining: false,
            reference: None,
            faces: [0; 2],
            ..source()
        };
        let mut sketch = Sketch::new("t");
        let cut = [
            ProjectedEdge::Line {
                start: [-5.0, 0.0],
                end: [-5.0, 12.0],
            },
            ProjectedEdge::Line {
                start: [5.0, 0.0],
                end: [5.0, 12.0],
            },
        ];
        for curve in &cut {
            add(&mut sketch, curve, face);
        }
        // An edge from the same point is a source of its own.
        add(
            &mut sketch,
            &ProjectedEdge::Point([1.0, 1.0]),
            ExternalSource {
                section: false,
                defining: false,
                reference: None,
                faces: [0; 2],
                ..face
            },
        );
        let all = groups(&sketch);
        assert_eq!(all.len(), 2);
        let (src, group) = all.into_iter().find(|(s, _)| s.section).unwrap();
        assert_eq!(group.len(), 2);
        // Taller now: both move in place.
        let taller = [
            ProjectedEdge::Line {
                start: [-5.0, 0.0],
                end: [-5.0, 20.0],
            },
            ProjectedEdge::Line {
                start: [5.0, 0.0],
                end: [5.0, 20.0],
            },
        ];
        refresh_group(&mut sketch, src, &group, &taller);
        assert!(group.iter().all(|id| sketch.get_geometry(*id).is_some()));
        let GeometryElement::Line(l) = sketch.get_geometry(group[1]).unwrap() else {
            panic!()
        };
        assert_eq!(sketch.point_position(l.end), Some(Vec2D::new(5.0, 20.0)));
        // Cut across instead: one circle takes the two lines' place.
        refresh_group(
            &mut sketch,
            src,
            &group,
            &[ProjectedEdge::Circle {
                centre: [0.0, 0.0],
                radius: 5.0,
                range: (0.0, TAU),
            }],
        );
        assert!(group.iter().all(|id| sketch.get_geometry(*id).is_none()));
        let (_, now) = groups(&sketch)
            .into_iter()
            .find(|(s, _)| s.section)
            .unwrap();
        assert!(matches!(
            sketch.get_geometry(now[0]),
            Some(GeometryElement::Circle(_))
        ));
    }

    /// A projected parabola or hyperbola is one conic arc of the sketch, of
    /// that kind, its ends on the curve where the projection's range puts
    /// them; a fresh projection of the same kind moves it in place.
    #[test]
    fn a_projected_conic_stays_a_conic() {
        let r = 2f64.sqrt() / 2.0;
        let parabola = |lift: f64| ProjectedEdge::Parabola {
            vertex: [1.0, lift],
            axis: [r, r],
            focal: 2.0,
            range: (-3.0, 1.5),
        };
        let hyperbola = |lift: f64| ProjectedEdge::Hyperbola {
            centre: [0.0, lift],
            major: [0.0, -3.0],
            minor: [-2.0, 0.0],
            range: (-1.0, 0.5),
        };
        type Lifted<'a> = &'a dyn Fn(f64) -> ProjectedEdge;
        let cases: [(Lifted, ConicKind, [f64; 2]); 2] = [
            (
                &parabola,
                ConicKind::Parabola,
                [1.0 + 9.0 / 8.0 * r + 3.0 * r, 9.0 / 8.0 * r - 3.0 * r],
            ),
            (
                &hyperbola,
                ConicKind::Hyperbola,
                [-2.0 * (-1f64).sinh(), -3.0 * 1f64.cosh()],
            ),
        ];
        for (edge, kind, start) in cases {
            let mut sketch = Sketch::new("t");
            assert_eq!(add(&mut sketch, &edge(0.0), source()), 1);
            let (_, group) = groups(&sketch).remove(0);
            let Some(GeometryElement::Conic(conic)) = sketch.get_geometry(group[0]).cloned() else {
                panic!("{:?}", sketch.get_geometry(group[0]));
            };
            assert_eq!(conic.kind, kind);
            let shape = crate::conic::Shape::of(&conic, &sketch).expect("a whole conic");
            for end in [conic.start, conic.end] {
                let p = sketch.point_position(end).expect("an end");
                let off = shape.distance([p.x, p.y]);
                assert!(off.abs() < 1e-5, "{kind:?} end {off} off");
            }
            let at = sketch.point_position(conic.start).expect("the start");
            assert!(
                (at.x - start[0]).abs() < 1e-5 && (at.y - start[1]).abs() < 1e-5,
                "{kind:?} starts at {at:?}, not {start:?}"
            );
            assert!(already_holds(&sketch, &group, &[edge(0.0)]));
            assert!(refresh_in_place(&mut sketch, &group, &[edge(2.0)]));
            assert!(already_holds(&sketch, &group, &[edge(2.0)]));
        }
        // A parabola's arc is not a hyperbola's, so it is made again.
        let mut sketch = Sketch::new("t");
        add(&mut sketch, &parabola(0.0), source());
        let (_, group) = groups(&sketch).remove(0);
        assert!(!refresh_in_place(&mut sketch, &group, &[hyperbola(0.0)]));
    }
}

#[cfg(test)]
mod external_geometry {
    use super::*;
    use crate::test_kinds::all_points;
    use crate::*;
    use core_document::KeyCode;
    use core_document::{BodyPlacement, Document, EdgeRef};
    use kernel_api::{KernelQueries, KernelResult, ProfilePlane, ProjectedEdge};

    /// A kernel whose every edge is a segment across the plane's X axis,
    /// from where the probe lands in the plane to 10 mm beyond it.
    struct FlatKernel;

    impl KernelQueries for FlatKernel {
        fn project_edge(
            &self,
            _brep: &[u8],
            near: [f64; 3],
            plane: &ProfilePlane,
        ) -> KernelResult<ProjectedEdge> {
            let d: Vec<f64> = (0..3).map(|k| near[k] - plane.origin[k]).collect();
            let x: f64 = (0..3).map(|k| d[k] * plane.x_axis[k]).sum();
            let y: f64 = (0..3).map(|k| d[k] * plane.y_axis[k]).sum();
            Ok(ProjectedEdge::Line {
                start: [x, y],
                end: [x + 10.0, y],
            })
        }

        /// Every face crosses the plane in two lines up the plane's Y axis,
        /// either side of where the probe lands.
        fn section_face(
            &self,
            _brep: &[u8],
            near: [f64; 3],
            plane: &ProfilePlane,
        ) -> KernelResult<Vec<ProjectedEdge>> {
            let d: Vec<f64> = (0..3).map(|k| near[k] - plane.origin[k]).collect();
            let x: f64 = (0..3).map(|k| d[k] * plane.x_axis[k]).sum();
            Ok([x - 1.0, x + 1.0]
                .map(|x| ProjectedEdge::Line {
                    start: [x, 0.0],
                    end: [x, 5.0],
                })
                .to_vec())
        }
    }

    static FLAT: FlatKernel = FlatKernel;

    #[test]
    fn a_picked_face_brings_in_where_it_crosses_the_plane_once_and_records_it() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        doc.set_imported_brep_data(body, b"ogeom shape".to_vec(), Vec::new());
        let sketch = add_sketch(
            &mut doc,
            Sketch::new("s"),
            SketchPlane::xy(),
            "s",
            Some(body),
        );
        let mut wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            external_refreshed: Some(sketch),
            ..SketchWorkbench::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(sketch);
        ctx.kernel = Some(&FLAT);
        // The tool's intersection variant is armed; the host picks a face.
        let armed = WorkbenchInputEvent::KeyPress { key: KeyCode::A };
        wb.on_input(&armed, Some("sketch.external:intersection"), &mut ctx);
        ctx.selected_body_id = Some(body.0);
        ctx.selected_face = Some(core_document::FaceRef {
            name: 0,
            point: [4.0, 2.0, -3.0],
            normal: [1.0, 0.0, 0.0],
            surface: None,
        });
        wb.on_frame(0.016, &mut ctx);
        wb.on_frame(0.016, &mut ctx);
        let recorded = core_document::HookOutcome::take(&mut ctx).recorded;
        assert_eq!(recorded.len(), 1, "taken once: {recorded:?}");
        assert_eq!(recorded[0].id, "sketch.intersection");

        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        assert_eq!(stored.external.len(), 2, "both lines came in");
        assert!(stored.external.values().all(|s| s.section));
        let xs: Vec<f64> = all_points(&stored)
            .into_iter()
            .map(|p| p.position.x)
            .collect();
        assert!(
            xs.iter()
                .all(|x| (x - 3.0).abs() < 1e-5 || (x - 5.0).abs() < 1e-5)
        );
        // The edge variant again takes edges, not the face.
        wb.on_input(&armed, Some("sketch.external:edges"), &mut ctx);
        wb.on_frame(0.016, &mut ctx);
        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        assert_eq!(stored.external.len(), 2);
    }

    #[test]
    fn a_picked_edge_comes_in_fixed_and_out_of_profiles() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        doc.set_imported_brep_data(body, b"ogeom shape".to_vec(), Vec::new());
        // The body sits 3 up: the pick is in the scene, the edge in its body.
        doc.set_body_placement(
            body,
            BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 3.0, 0.0)),
        );
        let sketch = add_sketch(
            &mut doc,
            Sketch::new("s"),
            SketchPlane::xy(),
            "s",
            Some(body),
        );
        let mut wb = SketchWorkbench {
            active_sketch_id: Some(sketch),
            last_tool: Some("sketch.external".to_string()),
            external_refreshed: Some(sketch),
            ..SketchWorkbench::default()
        };
        let mut ctx =
            WorkbenchRuntimeContext::new(&mut doc, [0.0, 0.0, 50.0], [0.0; 3], (0, 0, 800, 600));
        ctx.active_document_object = Some(sketch);
        ctx.kernel = Some(&FLAT);
        ctx.selected_edges = vec![EdgeRef {
            faces: [0, 0],
            point: [2.0, 7.0, 0.0],
            direction: [1.0, 0.0, 0.0],
            length_mm: 10.0,
            body: body.0,
            circle: None,
        }];
        wb.on_frame(0.016, &mut ctx);
        // Taken once, however many frames the pick stays selected.
        wb.on_frame(0.016, &mut ctx);

        let stored = stored_sketch(ctx.document, sketch).unwrap().sketch;
        assert_eq!(stored.external.len(), 1);
        let (line_id, source) = stored.external.iter().next().unwrap();
        assert_eq!(source.point, [2.0, 4.0, 0.0], "kept in the body's frame");
        let Some(GeometryElement::Line(line)) = stored.get_geometry(*line_id) else {
            panic!("a line came in")
        };
        // The scene has the sketch where the body sits; in the sketch's own
        // coordinates the edge lies where it was picked.
        assert_eq!(
            stored.point_position(line.start),
            Some(Vec2D::new(2.0, 4.0))
        );
        assert!(stored.is_external(line.start), "its points are held too");
        assert!(
            profile::extract_wires(&stored).is_err(),
            "no profile from it"
        );
    }
}
