//! Revolutions, helices, lofts and pipes: their axes, ends, sections and
//! spines.

use super::{cross3, dot3, face_probe, load_sketch, normalize, profile_of};
use core_document::{Document, FeatureId, WorkbenchFeature};
use kernel_api::{Profile, ProfileSegment, ProfileWire, RevolveTermination, SweepKind};
use wb_sketch::SketchFeature;
use wb_sketch::profile;
use wb_sketch::sketch::{GeometryElement, Sketch};

use crate::feature::{HelixMode, PipeOrientation, RevolveAxis};

/// The way a datum runs: a line along itself, a plane or coordinate
/// system square to it.
pub(super) fn datum_direction(document: &Document, datum: FeatureId) -> Result<[f64; 3], String> {
    let data = document
        .feature_values(datum)
        .ok_or("the datum it runs along is gone")?;
    let made = core_document::DatumFeature::from_json(data)
        .map_err(|_| "what it runs along is not a datum".to_string())?;
    let frame = made.frame();
    Ok(match made.shape {
        core_document::DatumShape::Line { .. } => frame.x_axis,
        _ => frame.normal,
    }
    .map(f64::from))
}

/// The way a sketch's line runs, in the body's frame.
pub(super) fn sketch_line_direction(
    document: &Document,
    sketch: FeatureId,
    element: uuid::Uuid,
) -> Result<[f64; 3], String> {
    let points = crate::datum_refs::sketch_points(document, sketch, element)
        .ok_or("the sketch line it runs along is gone")?;
    let [a, b] = <[[f32; 3]; 2]>::try_from(points)
        .map_err(|_| "what it runs along is not a line".to_string())?;
    Ok([b[0] - a[0], b[1] - a[1], b[2] - a[2]].map(f64::from))
}

/// Where a revolution's (or helix's) axis lies in its sketch: a point and a
/// direction in the sketch's own coordinates. A picked edge or a datum line
/// must run along the sketch plane, and is taken where it falls on it; with
/// `project`, one tilted to the plane is taken as its shadow on it, and
/// only one square to it is refused.
pub(crate) fn axis_in_sketch(
    document: &Document,
    sketch: &SketchFeature,
    axis: &RevolveAxis,
    project: bool,
) -> Result<([f64; 2], [f64; 2]), String> {
    let plane = profile::plane_of(&sketch.plane);
    let onto_plane = |point: [f64; 3], dir: [f64; 3], what: &str| {
        let d = normalize(dir).map_err(|_| format!("the {what} has no direction"))?;
        let n = normalize(plane.normal).map_err(|_| "the sketch has no normal".to_string())?;
        let across = dot3(d, n);
        if across.abs() > 1.0 - 1e-6 {
            return Err(format!(
                "the {what} stands square to the sketch plane, so it cannot be the axis"
            ));
        }
        if across.abs() > 1e-3 && !project {
            return Err(format!(
                "the {what} does not run along the sketch plane, so it cannot be the axis; \
                 project the axis to take its shadow on the plane"
            ));
        }
        let rel = [
            point[0] - plane.origin[0],
            point[1] - plane.origin[1],
            point[2] - plane.origin[2],
        ];
        let (dx, dy) = (dot3(d, plane.x_axis), dot3(d, plane.y_axis));
        let length = dx.hypot(dy);
        Ok((
            [dot3(rel, plane.x_axis), dot3(rel, plane.y_axis)],
            [dx / length, dy / length],
        ))
    };
    let (origin, dir) = match axis {
        RevolveAxis::SketchY => ([0.0, 0.0], [0.0, 1.0]),
        RevolveAxis::SketchX => ([0.0, 0.0], [1.0, 0.0]),
        RevolveAxis::Base(base) => onto_plane([0.0; 3], base.vector(), "body's axis")?,
        RevolveAxis::SketchNormal => {
            return Err(
                "the sketch's normal stands square to its plane; only a helix turns about it"
                    .into(),
            );
        }
        RevolveAxis::Custom { origin, dir } => (origin.map(f64::from), dir.map(f64::from)),
        RevolveAxis::Edge(edge) => onto_plane(
            edge.point.map(f64::from),
            edge.direction.map(f64::from),
            "picked edge",
        )?,
        RevolveAxis::Datum(id) => {
            let data = document
                .feature_values(*id)
                .ok_or("the axis's datum line is missing")?;
            let datum = core_document::DatumFeature::from_json(data)
                .map_err(|_| "the axis is not a datum".to_string())?;
            if !matches!(datum.shape, core_document::DatumShape::Line { .. }) {
                return Err("the axis's datum is not a line".into());
            }
            let frame = datum.frame();
            onto_plane(
                frame.origin.map(f64::from),
                frame.x_axis.map(f64::from),
                "datum line",
            )?
        }
        RevolveAxis::Borrowed(r) => {
            let edge = crate::borrow::edge(document, r)?;
            onto_plane(
                edge.point.map(f64::from),
                edge.direction.map(f64::from),
                "borrowed edge",
            )?
        }
        RevolveAxis::SketchLine(id) => {
            let Some(GeometryElement::Line(line)) = sketch.sketch.get_geometry(*id) else {
                return Err("the axis line is not a line of the sketch".into());
            };
            let at = |point| {
                sketch
                    .sketch
                    .point_position(point)
                    .map(|p| [p.x, p.y])
                    .ok_or_else(|| "the axis line has lost an end".to_string())
            };
            let (a, b) = (at(line.start)?, at(line.end)?);
            (a, [b[0] - a[0], b[1] - a[1]])
        }
    };
    if dir[0].hypot(dir[1]) < 1e-12 {
        return Err("revolution axis direction is zero".into());
    }
    Ok((origin, dir))
}

/// Build the revolve sweep for a feature. `reversed` flips the axis so the
/// sweep runs the other way around.
/// `plane` turned `angle_deg` about the axis it holds (a point and a
/// direction in its own coordinates), right-handed about the direction:
/// the axis stays where it is in the plane's coordinates.
pub(super) fn turn_plane(
    plane: &mut kernel_api::ProfilePlane,
    (axis_origin, axis_dir): ([f64; 2], [f64; 2]),
    angle_deg: f64,
) {
    let at = |p: [f64; 2]| -> [f64; 3] {
        std::array::from_fn(|k| plane.origin[k] + plane.x_axis[k] * p[0] + plane.y_axis[k] * p[1])
    };
    let pivot = at(axis_origin);
    let way: [f64; 3] =
        std::array::from_fn(|k| plane.x_axis[k] * axis_dir[0] + plane.y_axis[k] * axis_dir[1]);
    let Ok(k) = normalize(way) else {
        return;
    };
    let (sin, cos) = angle_deg.to_radians().sin_cos();
    // Rodrigues: v cos + (k × v) sin + k (k · v)(1 − cos).
    let turn = |v: [f64; 3]| -> [f64; 3] {
        let kv = cross3(k, v);
        let along = dot3(k, v) * (1.0 - cos);
        std::array::from_fn(|i| v[i] * cos + kv[i] * sin + k[i] * along)
    };
    let from_pivot: [f64; 3] = std::array::from_fn(|i| plane.origin[i] - pivot[i]);
    let moved = turn(from_pivot);
    plane.origin = std::array::from_fn(|i| pivot[i] + moved[i]);
    plane.x_axis = turn(plane.x_axis);
    plane.y_axis = turn(plane.y_axis);
    plane.normal = turn(plane.normal);
}

pub(super) fn revolve_kind(
    (axis_origin, axis_dir): ([f64; 2], [f64; 2]),
    angle_deg: f32,
    reversed: bool,
    midplane: bool,
    second_angle_deg: Option<f32>,
    termination: RevolveTermination,
) -> Result<SweepKind, String> {
    if termination == RevolveTermination::Angle && (angle_deg <= 0.0 || angle_deg > 360.0) {
        return Err(format!(
            "revolution angle must be in (0, 360], got {angle_deg}"
        ));
    }
    Ok(SweepKind::Revolve {
        axis_origin,
        axis_dir,
        angle_deg: f64::from(angle_deg),
        second_angle_deg: second_angle_deg.map(f64::from),
        midplane,
        reversed,
        termination,
    })
}

/// A helix's extent as the kernel reads it: pitch and height, and in the
/// growth mode the turn count and growth per turn as given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct HelixExtent {
    pub(super) pitch: f64,
    pub(super) height: f64,
    pub(super) turns: Option<f64>,
    pub(super) growth: Option<f64>,
}

pub(super) fn helix_extent(
    mode: HelixMode,
    pitch: f32,
    height: f32,
    turns: f32,
    growth: f32,
) -> Result<HelixExtent, String> {
    let (pitch, height) = match mode {
        HelixMode::PitchHeight => (pitch, height),
        HelixMode::PitchTurns => (pitch, pitch * turns),
        HelixMode::HeightTurns => {
            if turns <= 0.0 {
                return Err("helix turns must be positive".into());
            }
            (height / turns, height)
        }
        HelixMode::HeightTurnsGrowth => {
            if turns <= 0.0 {
                return Err("helix turns must be positive".into());
            }
            if height < 0.0 {
                return Err("helix height must not be negative".into());
            }
            if height == 0.0 && growth == 0.0 {
                return Err("a flat spiral needs a growth per turn".into());
            }
            return Ok(HelixExtent {
                pitch: f64::from(height) / f64::from(turns),
                height: f64::from(height),
                turns: Some(f64::from(turns)),
                growth: Some(f64::from(growth)),
            });
        }
    };
    if pitch <= 0.0 || height <= 0.0 {
        return Err("helix pitch and height must be positive".into());
    }
    Ok(HelixExtent {
        pitch: f64::from(pitch),
        height: f64::from(height),
        turns: None,
        growth: None,
    })
}

/// A loft's section as the kernel takes it: a sketch's profile, a point
/// (a datum point, or a sketch holding only one point), or a face.
pub(super) fn loft_section(
    document: &Document,
    section: &crate::feature::LoftSection,
) -> Result<kernel_api::LoftSection, String> {
    use crate::feature::LoftSection;
    let id = match section {
        LoftSection::Face(pick) => return Ok(kernel_api::LoftSection::Face(face_probe(pick))),
        LoftSection::Feature(id) => *id,
    };
    let node = document
        .get_feature_meta(id)
        .ok_or("a loft section is missing")?;
    if node.workbench_id.as_str() == core_document::DATUM_KIND {
        let data = document
            .feature_values(id)
            .ok_or("a loft section is missing")?;
        let datum = core_document::DatumFeature::from_json(data)
            .map_err(|_| "a loft section is not a datum".to_string())?;
        if !matches!(datum.shape, core_document::DatumShape::Point) {
            return Err("a datum section of a loft must be a point".into());
        }
        return Ok(kernel_api::LoftSection::Point(
            datum.frame().origin.map(f64::from),
        ));
    }
    let sketch = load_sketch(document, id)?;
    let curves = sketch
        .sketch
        .geometry
        .iter()
        .filter(|g| !matches!(g, wb_sketch::sketch::GeometryElement::Point(_)))
        .count();
    if curves == 0 {
        let points: Vec<_> = sketch
            .sketch
            .geometry
            .iter()
            .filter_map(|g| match g {
                wb_sketch::sketch::GeometryElement::Point(p) => Some(p.position),
                _ => None,
            })
            .collect();
        if let [p] = points.as_slice() {
            let plane = profile::plane_of(&sketch.plane);
            let at = |i: usize| plane.origin[i] + plane.x_axis[i] * p.x + plane.y_axis[i] * p.y;
            return Ok(kernel_api::LoftSection::Point([at(0), at(1), at(2)]));
        }
    }
    Ok(kernel_api::LoftSection::Profile(profile_of(&sketch, &[])?))
}

/// The kernel's frame for a pipe's orientation: an auxiliary path read as a
/// spine of its own, a binormal as the direction it names in the body's
/// frame, where every profile of the build sits.
pub(super) fn pipe_frame(
    document: &Document,
    orientation: &PipeOrientation,
) -> Result<kernel_api::PipeFrame, String> {
    use kernel_api::PipeFrame;
    Ok(match orientation {
        PipeOrientation::Standard => PipeFrame::RotationMinimizing,
        PipeOrientation::Frenet => PipeFrame::Frenet,
        PipeOrientation::Auxiliary { path } => PipeFrame::Auxiliary {
            path: sketch_spine(document, *path, None, &[])
                .map_err(|e| format!("auxiliary path: {e}"))?,
        },
        PipeOrientation::Binormal { x, y, z } => {
            let direction = [f64::from(*x), f64::from(*y), f64::from(*z)];
            if direction.iter().all(|c| *c == 0.0) {
                return Err("the binormal direction is zero".into());
            }
            PipeFrame::Binormal { direction }
        }
        PipeOrientation::Fixed => PipeFrame::Fixed,
    })
}

/// Where a profile sits, near enough to tell which end of a path it is at:
/// its first curve's centre, or where that curve starts when it has none.
pub(super) fn profile_anchor(profile: &Profile) -> Option<[f64; 3]> {
    let [u, v] = match profile.wires.first()?.segments.first()? {
        ProfileSegment::Line { start, .. } | ProfileSegment::Arc { start, .. } => *start,
        ProfileSegment::Circle { center, .. }
        | ProfileSegment::Ellipse { center, .. }
        | ProfileSegment::EllipseArc { center, .. } => *center,
        ProfileSegment::BSpline { control_points, .. }
        | ProfileSegment::Nurbs { control_points, .. } => *control_points.first()?,
    };
    let plane = &profile.plane;
    Some([0, 1, 2].map(|k| plane.origin[k] + plane.x_axis[k] * u + plane.y_axis[k] * v))
}

/// A sketch's curves as one path, in order: those named in `only`, or every
/// one it draws when `only` is empty. An open path runs from the end nearer
/// `near` (where the profile swept along it sits).
pub(super) fn sketch_spine(
    document: &Document,
    sketch_id: FeatureId,
    near: Option<[f64; 3]>,
    only: &[uuid::Uuid],
) -> Result<Profile, String> {
    let sketch_feature = load_sketch(document, sketch_id)?;
    let sketch = &sketch_feature.sketch;
    let plane = profile::plane_of(&sketch_feature.plane);

    let curves: Vec<&GeometryElement> = sketch
        .geometry
        .iter()
        .filter(|e| !sketch.is_construction(e.id()))
        .filter(|e| !matches!(e, GeometryElement::Point(_)))
        .filter(|e| only.is_empty() || only.contains(&e.id()))
        .collect();
    if curves.is_empty() {
        return Err(if only.is_empty() {
            "the spine sketch has no curves".into()
        } else {
            "none of the path's chosen curves is in its sketch".into()
        });
    }

    // A single circle is a closed spine by itself.
    if curves.len() == 1
        && let GeometryElement::Circle(circle) = curves[0]
    {
        let center = sketch
            .point_position(circle.center)
            .ok_or("spine circle has no center point")?;
        return Ok(Profile {
            plane,
            wires: vec![ProfileWire {
                names: Vec::new(),
                segments: vec![ProfileSegment::Circle {
                    center: [center.x, center.y],
                    radius: circle.radius,
                }],
            }],
        });
    }

    // Order curves into one chain by shared endpoints.
    let mut endpoints: Vec<(uuid::Uuid, uuid::Uuid, usize)> = Vec::new();
    for (i, element) in curves.iter().enumerate() {
        let ids = Sketch::curve_point_ids(element);
        match element {
            GeometryElement::Line(line) => endpoints.push((line.start, line.end, i)),
            GeometryElement::Arc(arc) => endpoints.push((arc.start, arc.end, i)),
            _ => {
                // Other curves with ends (splines) give them as the first
                // and last points they reference.
                if ids.len() >= 2 {
                    endpoints.push((ids[0], *ids.last().unwrap(), i));
                } else {
                    return Err("the spine may only contain connected lines and arcs".into());
                }
            }
        }
    }
    let mut degree: std::collections::HashMap<uuid::Uuid, u32> = std::collections::HashMap::new();
    for (a, b, _) in &endpoints {
        *degree.entry(*a).or_default() += 1;
        *degree.entry(*b).or_default() += 1;
    }
    if degree.values().any(|d| *d > 2) {
        return Err("the spine path branches; it must be a single chain".into());
    }
    // The open ends, in the order the sketch holds its curves.
    let mut odd: Vec<uuid::Uuid> = Vec::new();
    for (a, b, _) in &endpoints {
        for end in [*a, *b] {
            if degree[&end] == 1 && !odd.contains(&end) {
                odd.push(end);
            }
        }
    }
    if odd.len() != 2 && !odd.is_empty() {
        return Err("the spine must be one connected chain".into());
    }

    // An open path starts at the end nearer `near`, else at its first.
    let world = |id: uuid::Uuid| {
        sketch.point_position(id).map(|p| {
            let (u, v) = (p.x, p.y);
            [0, 1, 2].map(|k| plane.origin[k] + plane.x_axis[k] * u + plane.y_axis[k] * v)
        })
    };
    let distance = |id: uuid::Uuid, to: [f64; 3]| {
        world(id).map_or(f64::INFINITY, |p| {
            (0..3).map(|k| (p[k] - to[k]).powi(2)).sum::<f64>()
        })
    };
    let start = match (odd.as_slice(), near) {
        ([first, second], Some(to)) if distance(*second, to) < distance(*first, to) => *second,
        ([first, ..], _) => *first,
        _ => endpoints[0].0,
    };
    let mut remaining: Vec<(uuid::Uuid, uuid::Uuid, usize)> = endpoints.clone();
    let mut segments = Vec::with_capacity(curves.len());
    let mut cursor = start;
    while !remaining.is_empty() {
        let position = remaining
            .iter()
            .position(|(a, b, _)| *a == cursor || *b == cursor)
            .ok_or("the spine is disconnected")?;
        let (a, b, index) = remaining.swap_remove(position);
        let forward = a == cursor;
        segments.push(spine_segment(sketch, curves[index], forward)?);
        cursor = if forward { b } else { a };
    }

    Ok(Profile {
        plane,
        wires: vec![ProfileWire::new(segments)],
    })
}

fn spine_segment(
    sketch: &Sketch,
    element: &GeometryElement,
    forward: bool,
) -> Result<ProfileSegment, String> {
    let pos = |id: uuid::Uuid| -> Result<[f64; 2], String> {
        sketch
            .point_position(id)
            .map(|p| [p.x, p.y])
            .ok_or_else(|| "spine references a missing point".into())
    };
    match element {
        GeometryElement::Line(line) => {
            let (s, e) = if forward {
                (line.start, line.end)
            } else {
                (line.end, line.start)
            };
            Ok(ProfileSegment::Line {
                start: pos(s)?,
                end: pos(e)?,
            })
        }
        GeometryElement::Arc(arc) => {
            let center = pos(arc.center)?;
            let start = pos(arc.start)?;
            let end = pos(arc.end)?;
            let start_vec =
                wb_sketch::sketch::Vec2D::new(start[0] - center[0], start[1] - center[1]);
            let end_vec = wb_sketch::sketch::Vec2D::new(end[0] - center[0], end[1] - center[1]);
            let (start_angle, sweep) =
                wb_sketch::snap::arc_angles(start_vec.to_glam(), end_vec.to_glam());
            let mid_angle = start_angle + sweep * 0.5;
            let radius = ((start[0] - center[0]).powi(2) + (start[1] - center[1]).powi(2)).sqrt();
            let mid = [
                center[0] + radius * mid_angle.cos(),
                center[1] + radius * mid_angle.sin(),
            ];
            let (s, e) = if forward { (start, end) } else { (end, start) };
            Ok(ProfileSegment::Arc {
                start: s,
                mid,
                end: e,
            })
        }
        _ => Err("the spine may only contain connected lines and arcs".into()),
    }
}
