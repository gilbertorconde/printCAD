//! Section to sketch: a body's solid cut by a plane, the outline kept as
//! a new sketch on that plane (its outer boundary and every hole, closed
//! where the cut closes), or several parallel cuts a spacing apart, each
//! a sketch of its own. The sketches are taken once, as plain geometry to
//! edit or pad; they do not follow the solid.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, Document, FeatureId,
    WorkbenchRuntimeContext,
};
use kernel_api::{ProjectedEdge, SectionCurve, SectionRun};
use serde_json::Value;
use uuid::Uuid;
use wb_sketch::SketchFeature;
use wb_sketch::sketch::{
    Arc, BSpline, Circle, Ellipse, GeometryElement, Line, Point, Sketch, SketchPlane, Vec2D,
};

use crate::DesignWorkbench;
use crate::feature::MirrorPlane;

/// Ends closer than this (mm) are one point, so the outline closes
/// wherever the kernel chained the cut's curves into a closed run.
const JOIN: f64 = kernel_api::JOIN_MM;

/// Where the cuts stand: `count` planes `spacing` apart along the normal
/// of `plane`, the first moved `offset` along it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cuts {
    pub plane: MirrorPlane,
    pub offset: f32,
    pub count: u32,
    pub spacing: f32,
}

impl Cuts {
    /// One cut square to `axis` (0 for X) through the middle of `body`'s
    /// solid.
    pub(crate) fn middle(document: &Document, body: BodyId, axis: usize) -> Cuts {
        let plane = [MirrorPlane::YZ, MirrorPlane::XZ, MirrorPlane::XY][axis];
        let offset = document
            .local_geometry(body)
            .and_then(|(mesh, bounds)| bounds.or_else(|| mesh.bounds()))
            .map(|(lo, hi)| (lo[axis] + hi[axis]) / 2.0)
            .unwrap_or(0.0);
        Cuts {
            plane,
            offset,
            count: 1,
            spacing: 0.0,
        }
    }
}

/// The sketch plane of a cut: a base plane keeps its own axes, any other
/// takes axes from its normal; moved `offset` along the normal.
fn sketch_plane(
    document: &Document,
    plane: &MirrorPlane,
    offset: f64,
) -> Result<SketchPlane, String> {
    let mut sketch_plane = match plane {
        MirrorPlane::XY => SketchPlane::xy(),
        MirrorPlane::XZ => SketchPlane::xz(),
        MirrorPlane::YZ => SketchPlane::yz(),
        other => {
            let (origin, normal) = crate::build::mirror_plane(document, other)?;
            SketchPlane::from_face(origin, normal)
        }
    };
    let n = sketch_plane.normal;
    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if length < 1e-12 {
        return Err("the plane has no normal".into());
    }
    for (o, d) in sketch_plane.origin.iter_mut().zip(n) {
        *o += d / length * offset;
    }
    Ok(sketch_plane)
}

/// The sketch's points, found again by position so curves that meet
/// share their end.
struct Points(Vec<(Vec2D, Uuid)>);

impl Points {
    fn at(&mut self, sketch: &mut Sketch, p: [f64; 2]) -> Uuid {
        let at = Vec2D::new(p[0], p[1]);
        if let Some((_, id)) = self
            .0
            .iter()
            .find(|(q, _)| (q.x - at.x).hypot(q.y - at.y) < JOIN)
        {
            return *id;
        }
        let id = sketch.add_geometry(GeometryElement::Point(Point::new(at)));
        self.0.push((at, id));
        id
    }

    /// A centre or a control point: never an end another curve meets.
    fn own(sketch: &mut Sketch, p: [f64; 2]) -> Uuid {
        sketch.add_geometry(GeometryElement::Point(Point::new(Vec2D::new(p[0], p[1]))))
    }
}

fn on_circle(centre: [f64; 2], radius: f64, t: f64) -> [f64; 2] {
    [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()]
}

fn on_ellipse(centre: [f64; 2], major: [f64; 2], ratio: f64, t: f64) -> [f64; 2] {
    let minor = [-major[1] * ratio, major[0] * ratio];
    [
        centre[0] + major[0] * t.cos() + minor[0] * t.sin(),
        centre[1] + major[1] * t.cos() + minor[1] * t.sin(),
    ]
}

/// `point` (in the shape's frame) in `plane`'s own coordinates.
fn in_plane(plane: &SketchPlane, point: [f64; 3]) -> [f64; 2] {
    let d = [0, 1, 2].map(|i| point[i] - plane.origin[i]);
    let dot = |a: [f64; 3]| d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
    [dot(plane.x_axis), dot(plane.y_axis)]
}

/// Add one curve of the cut to `sketch`.
fn add_curve(sketch: &mut Sketch, points: &mut Points, plane: &SketchPlane, curve: &SectionCurve) {
    let full = |range: (f64, f64)| range.1 - range.0 >= std::f64::consts::TAU - 1e-9;
    match &curve.curve {
        ProjectedEdge::Point(_) => {}
        ProjectedEdge::Line { start, end } => {
            let (a, b) = (points.at(sketch, *start), points.at(sketch, *end));
            if a != b {
                sketch.add_geometry(GeometryElement::Line(Line::new(a, b)));
            }
        }
        ProjectedEdge::Circle {
            centre,
            radius,
            range,
        } => {
            let c = Points::own(sketch, *centre);
            let element = if full(*range) {
                GeometryElement::Circle(Circle::new(c, *radius))
            } else {
                let s = points.at(sketch, on_circle(*centre, *radius, range.0));
                let e = points.at(sketch, on_circle(*centre, *radius, range.1));
                GeometryElement::Arc(Arc::new(c, s, e, *radius))
            };
            sketch.add_geometry(element);
        }
        ProjectedEdge::Ellipse {
            centre,
            major,
            ratio,
            range,
        } => {
            let c = Points::own(sketch, *centre);
            let axis = Vec2D::new(major[0], major[1]);
            let element = if full(*range) {
                GeometryElement::Ellipse(Ellipse::new(c, axis, *ratio))
            } else {
                let s = points.at(sketch, on_ellipse(*centre, *major, *ratio, range.0));
                let e = points.at(sketch, on_ellipse(*centre, *major, *ratio, range.1));
                GeometryElement::Ellipse(Ellipse::new_arc(c, axis, *ratio, s, e))
            };
            sketch.add_geometry(element);
        }
        ProjectedEdge::Spline {
            degree,
            knots,
            control_points,
            weights,
        } if control_points.len() >= 2 => {
            let last = control_points.len() - 1;
            let ids: Vec<Uuid> = control_points
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    if i == 0 || i == last {
                        points.at(sketch, *p)
                    } else {
                        Points::own(sketch, *p)
                    }
                })
                .collect();
            sketch.add_geometry(GeometryElement::BSpline(BSpline {
                degree: *degree,
                knots: knots.clone(),
                weights: weights.clone(),
                ..BSpline::new(ids, false)
            }));
        }
        // A conic arc or anything else: the points along it, as lines.
        _ => {
            let along: Vec<[f64; 2]> = match &curve.curve {
                ProjectedEdge::Polyline(along) => along.clone(),
                _ => curve.points.iter().map(|p| in_plane(plane, *p)).collect(),
            };
            let ids: Vec<Uuid> = along.iter().map(|p| points.at(sketch, *p)).collect();
            for pair in ids.windows(2).filter(|pair| pair[0] != pair[1]) {
                sketch.add_geometry(GeometryElement::Line(Line::new(pair[0], pair[1])));
            }
        }
    }
}

/// The sketch on `plane` of a cut's `runs`: every curve, the ends that
/// meet as one point.
fn outline(plane: SketchPlane, runs: &[SectionRun]) -> Sketch {
    let mut sketch = Sketch::new("section");
    sketch.plane = plane;
    let mut points = Points(Vec::new());
    for run in runs {
        for curve in &run.curves {
            add_curve(&mut sketch, &mut points, &plane, curve);
        }
    }
    sketch
}

/// Cut `source`'s solid as `cuts` says and add each cut's outline as a
/// sketch on its plane: in `into`, or in a new body placed where the
/// source stands. Each sketch's id, in order along the normal.
pub(crate) fn section_body(
    ctx: &mut WorkbenchRuntimeContext,
    source: BodyId,
    cuts: Cuts,
    into: Option<BodyId>,
    name: Option<&str>,
) -> Result<Vec<FeatureId>, String> {
    let Some(entry) = ctx.document.body(source) else {
        return Err("no such body".into());
    };
    let body_name = entry.name.clone();
    if !(1..=100).contains(&cuts.count) {
        return Err("make from 1 to 100 cuts".into());
    }
    if cuts.count > 1 && !(cuts.spacing.is_finite() && cuts.spacing.abs() > 1e-6) {
        return Err("several cuts need a spacing other than zero".into());
    }
    let blob = ctx
        .document
        .imported_brep_blob(source)
        .ok_or("the body has no solid to cut yet; build it first")?
        .to_vec();
    let kernel = ctx.kernel.ok_or("no kernel to cut the solid with")?;
    let mut sketches = Vec::new();
    for index in 0..cuts.count {
        let offset = f64::from(cuts.offset) + f64::from(cuts.spacing) * f64::from(index);
        let plane = sketch_plane(ctx.document, &cuts.plane, offset)?;
        let runs = kernel
            .section(&blob, &wb_sketch::profile::plane_of(&plane), None)
            .map_err(|e| e.to_string())?;
        sketches.push((offset, outline(plane, &runs)));
    }
    let curves = |s: &Sketch| {
        s.geometry
            .iter()
            .filter(|g| !matches!(g, GeometryElement::Point(_)))
            .count()
    };
    if sketches.iter().all(|(_, s)| curves(s) == 0) {
        return Err("the plane misses the solid: nothing to keep".into());
    }
    let body = match into {
        Some(body) => body,
        None => {
            let body = ctx
                .document
                .create_body(Some(format!("{body_name} section")));
            let placement = ctx.document.body_placement(source);
            ctx.document.set_body_placement(body, placement);
            body
        }
    };
    let base = name.unwrap_or("Section");
    let mut made = Vec::new();
    for (_, sketch) in sketches {
        let plane = sketch.plane;
        let name = DesignWorkbench::next_feature_name(ctx, base);
        let id = ctx
            .document
            .add_feature_in_body(SketchFeature::new(sketch, plane), name, Some(body))
            .map_err(|e| e.to_string())?;
        made.push(id);
    }
    Ok(made)
}

/// Run `design.section`.
pub(crate) fn command(
    a: &Args,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let source = match a.opt_id("body")? {
        Some(id) => BodyId(id),
        None => ctx
            .selected_body_id
            .map(BodyId)
            .ok_or_else(|| CommandError::bad("body", "is required when no body is selected"))?,
    };
    let mut cuts = Cuts::middle(ctx.document, source, 2);
    if let Some(plane) = args.get("plane") {
        cuts.plane = serde_json::from_value(plane.clone()).map_err(|_| {
            CommandError::bad(
                "plane",
                "is not a plane: give \"XY\", \"XZ\", \"YZ\" or {Datum = {datum = id}}",
            )
        })?;
        cuts.offset = 0.0;
    }
    if let Some(offset) = a.opt_number("offset")? {
        cuts.offset = offset as f32;
    }
    if let Some(count) = a.opt_number("count")? {
        cuts.count = count.max(0.0) as u32;
    }
    if let Some(spacing) = a.opt_number("spacing")? {
        cuts.spacing = spacing as f32;
    }
    let into = a.opt_id("into")?.map(BodyId);
    let made = section_body(ctx, source, cuts, into, a.opt_string("name")?)
        .map_err(CommandError::failed)?;
    Ok(Value::from(
        made.iter().map(|f| f.0.to_string()).collect::<Vec<_>>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square cut whose corners the kernel's curves miss by `gap` on
    /// each side, chained by the kernel's join: the sketch closes exactly
    /// where the run is closed, from a hair inside the old point join to
    /// just inside the kernel's, and is open past it.
    #[test]
    fn the_sketch_closes_what_the_kernel_chained() {
        let join = kernel_api::JOIN_MM;
        for gap in [0.0, 5e-5, 5e-4, 0.9 * join, 2.0 * join] {
            let corners = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
            let curves = (0..4)
                .map(|i| {
                    let (a, b): ([f64; 2], [f64; 2]) = (corners[i], corners[(i + 1) % 4]);
                    let start = [a[0] + gap, a[1]];
                    SectionCurve {
                        curve: ProjectedEdge::Line { start, end: b },
                        points: vec![[start[0], start[1], 0.0], [b[0], b[1], 0.0]],
                        reversed: false,
                    }
                })
                .collect();
            let run = SectionRun {
                curves,
                closed: gap < join,
            };
            let sketch = outline(SketchPlane::xy(), std::slice::from_ref(&run));
            assert_eq!(
                wb_sketch::profile::extract_wires(&sketch).is_ok(),
                run.closed,
                "gap {gap}"
            );
        }
    }
}
