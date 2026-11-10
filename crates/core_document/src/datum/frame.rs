//! Resolving an attachment and its offset into a datum's frame.

use super::*;

impl DatumAttachment {
    pub fn label(&self) -> &'static str {
        match self {
            DatumAttachment::BasePlane(plane) => plane.label(),
            DatumAttachment::FlatFace { .. } => "Picked face",
            DatumAttachment::Face { .. } => "On a face",
            DatumAttachment::ThreePoints { .. } => "Through three points",
            DatumAttachment::NormalToEdge { .. } => "Square to an edge",
            DatumAttachment::AlongEdge { .. } => "Along an edge",
            DatumAttachment::TwoPoints { .. } => "Through two points",
            DatumAttachment::PlaneIntersection { .. } => "Where two planes meet",
            DatumAttachment::CurveCentre { .. } => "Centre of a curve",
            DatumAttachment::Inertia { .. } => "Centre of mass",
            DatumAttachment::OnDatum { .. } => "On a datum",
            DatumAttachment::OtherBody { .. } => "Another body's origin",
            DatumAttachment::FaceNormal { .. } => "Square to a face",
            DatumAttachment::TangentToEdge { .. } => "Tangent to an edge",
            DatumAttachment::LineAndPoint { .. } => "Through a line and a point",
            DatumAttachment::LineMeetsPlane { .. } => "Where a line meets a plane",
            DatumAttachment::TwoLines { .. } => "Where two lines cross",
            DatumAttachment::Midplane { .. } => "Midway between two planes",
        }
    }

    /// The name a command takes the mode by.
    pub fn mode(&self) -> &'static str {
        match self {
            DatumAttachment::BasePlane(_) => "base_plane",
            DatumAttachment::FlatFace { .. } => "flat_face",
            DatumAttachment::Face { .. } => "face",
            DatumAttachment::ThreePoints { .. } => "three_points",
            DatumAttachment::NormalToEdge { .. } => "normal_to_edge",
            DatumAttachment::AlongEdge { .. } => "along_edge",
            DatumAttachment::TwoPoints { .. } => "two_points",
            DatumAttachment::PlaneIntersection { .. } => "plane_intersection",
            DatumAttachment::CurveCentre { .. } => "curve_centre",
            DatumAttachment::Inertia { .. } => "inertia",
            DatumAttachment::OnDatum { .. } => "on_datum",
            DatumAttachment::OtherBody { .. } => "other_body",
            DatumAttachment::FaceNormal { .. } => "face_normal",
            DatumAttachment::TangentToEdge { .. } => "tangent_to_edge",
            DatumAttachment::LineAndPoint { .. } => "line_and_point",
            DatumAttachment::LineMeetsPlane { .. } => "line_meets_plane",
            DatumAttachment::TwoLines { .. } => "two_lines",
            DatumAttachment::Midplane { .. } => "midplane",
        }
    }

    /// The other features it is made from: datums and sketches.
    pub fn datums(&self) -> Vec<FeatureId> {
        let plane = |p: &PlaneAnchor| match p {
            PlaneAnchor::Datum { datum, .. } => Some(*datum),
            _ => None,
        };
        match self {
            DatumAttachment::PlaneIntersection { planes }
            | DatumAttachment::Midplane { planes } => planes.iter().filter_map(plane).collect(),
            DatumAttachment::ThreePoints { points } => {
                points.iter().filter_map(PointAnchor::feature).collect()
            }
            DatumAttachment::TwoPoints { points } => {
                points.iter().filter_map(PointAnchor::feature).collect()
            }
            DatumAttachment::OnDatum { datum, .. } => vec![*datum],
            DatumAttachment::LineAndPoint { line, point } => {
                line.feature().into_iter().chain(point.feature()).collect()
            }
            DatumAttachment::LineMeetsPlane { line, plane: p } => {
                line.feature().into_iter().chain(plane(p)).collect()
            }
            DatumAttachment::TwoLines { lines } => {
                lines.iter().filter_map(LineAnchor::feature).collect()
            }
            _ => Vec::new(),
        }
    }

    /// Why the references make no frame, when they make none: points on
    /// top of each other or in a line, parallel planes, an edge with no
    /// circle for a centre.
    pub fn problem(&self) -> Option<&'static str> {
        const LEAST: f32 = 1e-4;
        let not_round = "the edge is not a circle or an arc of one";
        match self {
            DatumAttachment::ThreePoints { points } => {
                let [a, b, c] = points.map(|p| p.point());
                if length(sub(b, a)) < LEAST || length(sub(c, a)) < LEAST {
                    Some("two of the points are the same point")
                } else if length(cross(normalize(sub(b, a)), normalize(sub(c, a)))) < LEAST {
                    Some("the three points lie on one line")
                } else {
                    None
                }
            }
            DatumAttachment::TwoPoints { points } => {
                let [a, b] = points.map(|p| p.point());
                (length(sub(b, a)) < LEAST).then_some("the two points are the same point")
            }
            DatumAttachment::PlaneIntersection { planes } => {
                let [(_, n1), (_, n2)] = planes.map(|p| p.plane());
                (length(cross(n1, n2)) < LEAST).then_some("the two planes are parallel")
            }
            DatumAttachment::Midplane { planes } => midplane(planes)
                .is_none()
                .then_some("the two planes are one plane"),
            DatumAttachment::CurveCentre { edge } => edge.circle.is_none().then_some(not_round),
            DatumAttachment::NormalToEdge {
                edge,
                spot: EdgeSpot::Centre,
                along: None,
            }
            | DatumAttachment::TangentToEdge {
                edge,
                spot: EdgeSpot::Centre,
            } => edge.circle.is_none().then_some(not_round),
            DatumAttachment::LineAndPoint { line, point } => {
                let (p, d) = line.line();
                let off = reject(sub(point.point(), p), d);
                (length(off) < LEAST).then_some("the point lies on the line")
            }
            DatumAttachment::LineMeetsPlane { line, plane } => {
                let (_, d) = line.line();
                let (_, n) = plane.plane();
                (dot(d, n).abs() < LEAST).then_some("the line runs along the plane")
            }
            DatumAttachment::TwoLines { lines } => {
                let [(_, d1), (_, d2)] = lines.map(|l| l.line());
                (length(cross(d1, d2)) < LEAST).then_some("the two lines are parallel")
            }
            _ => None,
        }
    }

    /// (origin, normal, x_axis) the references make, before the offset.
    fn base_frame(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        match *self {
            DatumAttachment::BasePlane(plane) => plane.frame(),
            DatumAttachment::FlatFace { point, normal } => {
                let n = normalize(normal);
                (point, n, stable_x_axis(n))
            }
            DatumAttachment::Face { face } => {
                let n = normalize(face.normal);
                // Along a turned face's axis, so a line on a cylinder runs
                // along it.
                let along = face
                    .surface
                    .and_then(|s| s.axis())
                    .map(|(_, axis)| reject(axis, n))
                    .filter(|x| length(*x) > 1e-4);
                (
                    face.point,
                    n,
                    along.map(normalize).unwrap_or_else(|| stable_x_axis(n)),
                )
            }
            DatumAttachment::ThreePoints { points } => {
                let [a, b, c] = points.map(|p| p.point());
                let x = sub(b, a);
                let n = cross(x, sub(c, a));
                if length(x) < 1e-6 || length(n) < 1e-9 {
                    return along_x(a, x);
                }
                let n = normalize(n);
                (a, n, normalize(reject(x, n)))
            }
            DatumAttachment::NormalToEdge {
                edge,
                along: Some(share),
                ..
            } => {
                let (at, tangent) = edge.at_share(share);
                let radial = edge
                    .circle
                    .map(|c| reject(sub(at, c.center), tangent))
                    .filter(|r| length(*r) > 1e-4);
                (
                    at,
                    tangent,
                    radial
                        .map(normalize)
                        .unwrap_or_else(|| stable_x_axis(tangent)),
                )
            }
            DatumAttachment::NormalToEdge { edge, spot, .. } => {
                let at = edge.spot_point(spot);
                // A circle's centre is off the edge: the plane there is
                // square to the edge where it was picked.
                let on_edge = if spot == EdgeSpot::Centre {
                    edge.point
                } else {
                    at
                };
                let tangent = edge.tangent_at(on_edge);
                // On a circle the x-axis points out from its centre.
                let radial = edge
                    .circle
                    .map(|c| reject(sub(on_edge, c.center), tangent))
                    .filter(|r| length(*r) > 1e-4);
                (
                    at,
                    tangent,
                    radial
                        .map(normalize)
                        .unwrap_or_else(|| stable_x_axis(tangent)),
                )
            }
            DatumAttachment::AlongEdge { edge } => match edge.circle {
                Some(circle) => along_x(circle.center, circle.normal),
                None => {
                    let middle = match edge.ends {
                        Some([a, b]) => scale(add(a, b), 0.5),
                        None => edge.middle.unwrap_or(edge.point),
                    };
                    along_x(middle, edge.direction)
                }
            },
            DatumAttachment::TwoPoints { points } => {
                let [a, b] = points.map(|p| p.point());
                along_x(scale(add(a, b), 0.5), sub(b, a))
            }
            DatumAttachment::PlaneIntersection { planes } => {
                let [(o1, n1), (o2, n2)] = planes.map(|p| p.plane());
                let u = cross(n1, n2);
                let uu = dot(u, u);
                if uu < 1e-12 {
                    return (o1, n1, stable_x_axis(n1));
                }
                let (d1, d2) = (dot(n1, o1), dot(n2, o2));
                let on_both = scale(
                    add(scale(cross(n2, u), d1), scale(cross(u, n1), d2)),
                    1.0 / uu,
                );
                // The point of the line nearest the planes' own origins.
                let x = normalize(u);
                let middle = scale(add(o1, o2), 0.5);
                let origin = scale_add(on_both, x, dot(x, sub(middle, on_both)));
                (origin, normalize(reject(n1, x)), x)
            }
            DatumAttachment::CurveCentre { edge } => match edge.circle {
                Some(circle) => {
                    let n = normalize(circle.normal);
                    let radial = reject(sub(edge.point, circle.center), n);
                    let x = if length(radial) > 1e-4 {
                        normalize(radial)
                    } else {
                        stable_x_axis(n)
                    };
                    (circle.center, n, x)
                }
                None => along_x(edge.point, edge.direction),
            },
            DatumAttachment::Inertia { centre, axes } => {
                let x = normalize(axes[0]);
                let n = normalize(reject(axes[2], x));
                (centre, n, x)
            }
            DatumAttachment::OnDatum { frame, .. } | DatumAttachment::OtherBody { frame, .. } => {
                (frame.origin, frame.normal, frame.x_axis)
            }
            DatumAttachment::FaceNormal { face } => along_x(face.point, face.normal),
            DatumAttachment::TangentToEdge { edge, spot } => {
                let at = edge.spot_point(spot);
                let on_edge = if spot == EdgeSpot::Centre {
                    edge.point
                } else {
                    at
                };
                along_x(at, edge.tangent_at(on_edge))
            }
            DatumAttachment::LineAndPoint { line, point } => {
                let (p, d) = line.line();
                let q = point.point();
                let foot = scale_add(p, d, dot(sub(q, p), d));
                let n = cross(d, sub(q, p));
                if length(n) < 1e-9 {
                    return along_x(p, d);
                }
                (foot, normalize(n), d)
            }
            DatumAttachment::LineMeetsPlane { line, plane } => {
                let (p, d) = line.line();
                let (o, n) = plane.plane();
                let along = dot(d, n);
                if along.abs() < 1e-9 {
                    return (p, n, stable_x_axis(n));
                }
                let at = scale_add(p, d, dot(sub(o, p), n) / along);
                let x = reject(d, n);
                let x = if length(x) > 1e-4 {
                    normalize(x)
                } else {
                    stable_x_axis(n)
                };
                (at, n, x)
            }
            DatumAttachment::TwoLines { lines } => {
                let [(p1, d1), (p2, d2)] = lines.map(|l| l.line());
                let n = cross(d1, d2);
                if length(n) < 1e-9 {
                    return along_x(p1, d1);
                }
                // The nearest points of the two lines, and the middle.
                let w = sub(p1, p2);
                let (b, d, e) = (dot(d1, d2), dot(d1, w), dot(d2, w));
                let denom = 1.0 - b * b;
                let (s, t) = ((b * e - d) / denom, (e - b * d) / denom);
                let (a, c) = (scale_add(p1, d1, s), scale_add(p2, d2, t));
                (scale(add(a, c), 0.5), normalize(n), d1)
            }
            DatumAttachment::Midplane { planes } => midplane(&planes).unwrap_or_else(|| {
                let (o, n) = planes[0].plane();
                (o, n, stable_x_axis(n))
            }),
        }
    }
}

/// (origin, normal, x_axis) of the plane midway between two planes, `None`
/// when they are one plane. Parallel planes give the plane halfway between
/// them; others meet in a line, about which they have two bisectors, and
/// the one taken leaves each plane's origin on its own side of it, so it
/// passes between the faces the planes were taken from. Its x-axis runs
/// along the line the planes meet in.
pub(super) fn midplane(planes: &[PlaneAnchor; 2]) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
    const LEAST: f32 = 1e-4;
    let [(o1, n1), (o2, n2)] = planes.map(|p| p.plane());
    let (d1, d2) = (dot(n1, o1), dot(n2, o2));
    // How far each origin stands off the other plane.
    let (off1, off2) = (dot(n2, o1) - d2, dot(n1, o2) - d1);
    let meet = cross(n1, n2);
    if length(meet) < LEAST && off1.abs() < LEAST && off2.abs() < LEAST {
        return None;
    }
    // Points as far off both planes on the same side lie on n1 - n2, on
    // opposite sides on n1 + n2; the first separates the two origins when
    // each stands on the same side of the other plane.
    let apart = (sub(n1, n2), d1 - d2);
    let together = (add(n1, n2), d1 + d2);
    let same_side = if (off1 * off2).abs() > LEAST * LEAST {
        off1 * off2 > 0.0
    } else {
        dot(n1, n2) < 0.0
    };
    let (mut chosen, mut other) = if same_side {
        (apart, together)
    } else {
        (together, apart)
    };
    if length(chosen.0) < LEAST {
        std::mem::swap(&mut chosen, &mut other);
    }
    let (normal, height) = chosen;
    let size = length(normal);
    let (normal, height) = (scale(normal, 1.0 / size), height / size);
    let middle = scale(add(o1, o2), 0.5);
    let origin = scale_add(middle, normal, height - dot(normal, middle));
    let x = if length(meet) < LEAST {
        stable_x_axis(normal)
    } else {
        normalize(reject(meet, normal))
    };
    Some((origin, normal, x))
}

/// A frame whose x-axis runs along `x` through `origin`, the normal square
/// to it.
pub(super) fn along_x(origin: [f32; 3], x: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    let x = normalize(x);
    (origin, stable_x_axis(x), x)
}

/// `v` turned by `angle` radians about the unit axis `k` (Rodrigues).
pub(super) fn turned(v: [f32; 3], k: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    add(
        add(scale(v, c), scale(cross(k, v), s)),
        scale(k, dot(k, v) * (1.0 - c)),
    )
}

pub(super) fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-9 {
        return [0.0, 0.0, 1.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

pub(super) fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) fn scale_add(p: [f32; 3], v: [f32; 3], s: f32) -> [f32; 3] {
    [p[0] + v[0] * s, p[1] + v[1] * s, p[2] + v[2] * s]
}

pub(super) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(super) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn scale(v: [f32; 3], s: f32) -> [f32; 3] {
    [v[0] * s, v[1] * s, v[2] * s]
}

pub(super) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn length(v: [f32; 3]) -> f32 {
    dot(v, v).sqrt()
}

/// `v` less its part along the unit vector `n`.
pub(super) fn reject(v: [f32; 3], n: [f32; 3]) -> [f32; 3] {
    scale_add(v, n, -dot(v, n))
}

/// Stable in-plane basis for an arbitrary normal: pick the world axis least
/// aligned with the normal and orthogonalize it.
pub(super) fn stable_x_axis(normal: [f32; 3]) -> [f32; 3] {
    let candidates = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut best = candidates[0];
    let mut best_dot = f32::MAX;
    for candidate in candidates {
        let dot =
            (candidate[0] * normal[0] + candidate[1] * normal[1] + candidate[2] * normal[2]).abs();
        if dot < best_dot {
            best_dot = dot;
            best = candidate;
        }
    }
    let dot = best[0] * normal[0] + best[1] * normal[1] + best[2] * normal[2];
    normalize([
        best[0] - normal[0] * dot,
        best[1] - normal[1] * dot,
        best[2] - normal[2] * dot,
    ])
}

/// A resolved datum placement, in its body's own frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DatumFrame {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub x_axis: [f32; 3],
}

impl DatumFrame {
    pub fn y_axis(&self) -> [f32; 3] {
        cross(self.normal, self.x_axis)
    }

    /// The frame's own XY, XZ and YZ planes, labelled, laid out as the
    /// document's base planes are on the world axes.
    pub fn planes(&self) -> [(&'static str, DatumFrame); 3] {
        let (x, y, z) = (self.x_axis, self.y_axis(), self.normal);
        let plane = |normal, x_axis| DatumFrame {
            origin: self.origin,
            normal,
            x_axis,
        };
        [
            ("XY", plane(z, x)),
            ("XZ", plane([-y[0], -y[1], -y[2]], x)),
            ("YZ", plane(x, y)),
        ]
    }
}

impl DatumFeature {
    /// Resolve the attachment + offset into a placement in the body's frame.
    pub fn frame(&self) -> DatumFrame {
        let (origin, normal, x_axis) = self.attachment.base_frame();
        let mut normal = normalize(normal);
        let mut x_axis = normalize(x_axis);
        let mut y_axis = cross(normal, x_axis);

        // In-plane rotation about the normal.
        let rot = self.offset.rotation_deg.to_radians();
        if rot.abs() > 1e-9 {
            let (s, c) = rot.sin_cos();
            let rotated = [
                x_axis[0] * c + y_axis[0] * s,
                x_axis[1] * c + y_axis[1] * s,
                x_axis[2] * c + y_axis[2] * s,
            ];
            x_axis = normalize(rotated);
            y_axis = cross(normal, x_axis);
        }
        // Tilted about x, then about the y it leaves.
        let [about_x, about_y] = self.offset.tilt.map(f32::to_radians);
        if about_x.abs() > 1e-9 {
            normal = normalize(turned(normal, x_axis, about_x));
            y_axis = cross(normal, x_axis);
        }
        if about_y.abs() > 1e-9 {
            normal = normalize(turned(normal, y_axis, about_y));
            x_axis = normalize(turned(x_axis, y_axis, about_y));
        }

        let mut origin = origin;
        origin = scale_add(origin, x_axis, self.offset.translation[0]);
        origin = scale_add(origin, y_axis, self.offset.translation[1]);
        origin = scale_add(origin, normal, self.offset.translation[2]);

        if self.offset.flip {
            // 180° about the local x-axis: the normal reverses, x stays.
            normal = [-normal[0], -normal[1], -normal[2]];
        }
        DatumFrame {
            origin,
            normal,
            x_axis,
        }
    }
}
