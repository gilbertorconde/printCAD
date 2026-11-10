//! Following a datum's references to the body's solid and to other
//! datums and bodies.

use super::*;

/// A reference of a datum a build finds again on the body's solid.
pub(super) enum Followed<'a> {
    Face(&'a mut FaceAnchor),
    /// An edge, and the share along it the datum takes, when it takes one.
    Edge(&'a mut EdgeAnchor, Option<f32>),
    Mass(&'a mut [f32; 3], &'a mut [[f32; 3]; 3]),
}

pub(super) fn f64s(v: [f32; 3]) -> [f64; 3] {
    v.map(f64::from)
}

pub(super) fn f32s(v: [f64; 3]) -> [f32; 3] {
    v.map(|c| c as f32)
}

impl DatumFeature {
    /// The references that follow the body's solid, in a fixed order.
    fn followed(&mut self) -> Vec<Followed<'_>> {
        let mut followed = Vec::new();
        match &mut self.attachment {
            DatumAttachment::BasePlane(_) | DatumAttachment::FlatFace { .. } => {}
            DatumAttachment::Face { face } => followed.push(Followed::Face(face)),
            DatumAttachment::ThreePoints { points } => {
                for anchor in points.iter_mut() {
                    push_point(anchor, &mut followed);
                }
            }
            DatumAttachment::TwoPoints { points } => {
                for anchor in points.iter_mut() {
                    push_point(anchor, &mut followed);
                }
            }
            DatumAttachment::NormalToEdge { edge, along, .. } => {
                followed.push(Followed::Edge(edge, *along));
            }
            DatumAttachment::AlongEdge { edge }
            | DatumAttachment::CurveCentre { edge }
            | DatumAttachment::TangentToEdge { edge, .. } => {
                followed.push(Followed::Edge(edge, None));
            }
            DatumAttachment::PlaneIntersection { planes }
            | DatumAttachment::Midplane { planes } => {
                for plane in planes.iter_mut() {
                    push_plane(plane, &mut followed);
                }
            }
            DatumAttachment::Inertia { centre, axes } => {
                followed.push(Followed::Mass(centre, axes));
            }
            DatumAttachment::FaceNormal { face } => followed.push(Followed::Face(face)),
            DatumAttachment::LineAndPoint { line, point } => {
                push_line(line, &mut followed);
                push_point(point, &mut followed);
            }
            DatumAttachment::LineMeetsPlane { line, plane } => {
                push_line(line, &mut followed);
                push_plane(plane, &mut followed);
            }
            DatumAttachment::TwoLines { lines } => {
                for line in lines.iter_mut() {
                    push_line(line, &mut followed);
                }
            }
            DatumAttachment::OnDatum { .. } | DatumAttachment::OtherBody { .. } => {}
        }
        followed.retain(|f| match f {
            Followed::Face(face) => face.follows,
            Followed::Edge(edge, _) => edge.follows,
            Followed::Mass(..) => true,
        });
        followed
    }

    /// What a build asks of the body's solid to find this datum's
    /// references again, in a fixed order.
    pub fn probes(&self) -> Vec<ShapeProbe> {
        let mut copy = *self;
        copy.followed()
            .into_iter()
            .map(|f| match f {
                Followed::Face(face) => ShapeProbe::Face {
                    point: f64s(face.point),
                    normal: f64s(face.normal),
                    name: face.name,
                },
                Followed::Edge(edge, along) => ShapeProbe::Edge {
                    point: f64s(edge.point),
                    direction: f64s(edge.direction),
                    faces: edge.faces,
                    along: along.map(f64::from),
                },
                Followed::Mass(..) => ShapeProbe::Mass,
            })
            .collect()
    }

    /// Take what the body's solid answered to [`Self::probes`], in the
    /// same order; a reference whose answer failed or does not fit keeps
    /// what it had.
    pub fn take_answers(&mut self, answers: &[Result<ProbeAnswer, String>]) {
        for (reference, answer) in self.followed().into_iter().zip(answers) {
            match (reference, answer) {
                (
                    Followed::Face(face),
                    Ok(ProbeAnswer::Face {
                        point,
                        normal,
                        surface,
                    }),
                ) => {
                    face.point = f32s(*point);
                    face.normal = f32s(*normal);
                    face.surface = Some(*surface);
                }
                (
                    Followed::Edge(edge, share),
                    Ok(ProbeAnswer::Edge {
                        point,
                        direction,
                        start,
                        end,
                        middle,
                        circle,
                        along,
                    }),
                ) => {
                    edge.along = share
                        .zip(*along)
                        .map(|(s, (at, tangent))| (s, f32s(at), f32s(tangent)));
                    edge.point = f32s(*point);
                    edge.direction = f32s(*direction);
                    edge.ends = Some([f32s(*start), f32s(*end)]);
                    edge.middle = Some(f32s(*middle));
                    edge.circle = circle.map(|c| AnchorCircle {
                        center: f32s(c.centre),
                        normal: f32s(c.normal),
                        radius: c.radius as f32,
                    });
                }
                (Followed::Mass(centre, axes), Ok(ProbeAnswer::Mass { centre: c, axes: a })) => {
                    *centre = f32s(*c);
                    *axes = a.map(f32s);
                }
                _ => {}
            }
        }
    }

    /// Take the frames of the other datums this one is made from, as
    /// `frame_of` gives them now (a datum's frame, and for a coordinate
    /// system the plane named).
    pub fn follow_datums(&mut self, frame_of: &dyn Fn(FeatureId) -> Option<DatumFrame>) {
        let follow_plane = |plane: &mut PlaneAnchor| {
            if let PlaneAnchor::Datum {
                datum,
                origin,
                normal,
            } = plane
                && let Some(frame) = frame_of(*datum)
            {
                *origin = frame.origin;
                *normal = frame.normal;
            }
        };
        let follow_line = |line: &mut LineAnchor| {
            if let LineAnchor::Datum {
                datum,
                origin,
                direction,
            } = line
                && let Some(frame) = frame_of(*datum)
            {
                *origin = frame.origin;
                *direction = frame.x_axis;
            }
        };
        let follow_point = |point: &mut PointAnchor| {
            if let PointAnchor::Datum { datum, point } = point
                && let Some(frame) = frame_of(*datum)
            {
                *point = frame.origin;
            }
        };
        match &mut self.attachment {
            DatumAttachment::PlaneIntersection { planes }
            | DatumAttachment::Midplane { planes } => planes.iter_mut().for_each(follow_plane),
            DatumAttachment::ThreePoints { points } => points.iter_mut().for_each(follow_point),
            DatumAttachment::TwoPoints { points } => points.iter_mut().for_each(follow_point),
            DatumAttachment::OnDatum {
                datum,
                plane,
                frame,
            } => {
                if let Some(found) = frame_of(*datum) {
                    let found = match plane {
                        Some(which) => found
                            .planes()
                            .into_iter()
                            .zip(BasePlane::ALL)
                            .find(|(_, p)| p == which)
                            .map(|((_, f), _)| f)
                            .unwrap_or(found),
                        None => found,
                    };
                    *frame = found.into();
                }
            }
            DatumAttachment::LineAndPoint { line, point } => {
                follow_line(line);
                follow_point(point);
            }
            DatumAttachment::LineMeetsPlane { line, plane } => {
                follow_line(line);
                follow_plane(plane);
            }
            DatumAttachment::TwoLines { lines } => lines.iter_mut().for_each(follow_line),
            _ => {}
        }
    }

    /// Take where another body's origin plane stands now in this body's
    /// frame, as `frame_of` gives it.
    pub fn follow_bodies(
        &mut self,
        frame_of: &dyn Fn(crate::BodyId, BasePlane) -> Option<DatumFrame>,
    ) {
        if let DatumAttachment::OtherBody { body, plane, frame } = &mut self.attachment
            && let Some(found) = frame_of(*body, *plane)
        {
            *frame = found.into();
        }
    }

    /// The sketch references this datum takes, to be brought up to date by
    /// the sketches' owner.
    pub fn sketch_points_mut(&mut self) -> Vec<&mut PointAnchor> {
        match &mut self.attachment {
            DatumAttachment::ThreePoints { points } => points.iter_mut().collect(),
            DatumAttachment::TwoPoints { points } => points.iter_mut().collect(),
            DatumAttachment::LineAndPoint { point, .. } => vec![point],
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|p| matches!(p, PointAnchor::Sketch { .. }))
        .collect()
    }

    /// The sketch lines this datum takes, to be brought up to date by the
    /// sketches' owner.
    pub fn sketch_lines_mut(&mut self) -> Vec<&mut LineAnchor> {
        match &mut self.attachment {
            DatumAttachment::LineAndPoint { line, .. }
            | DatumAttachment::LineMeetsPlane { line, .. } => vec![line],
            DatumAttachment::TwoLines { lines } => lines.iter_mut().collect(),
            _ => Vec::new(),
        }
        .into_iter()
        .filter(|l| matches!(l, LineAnchor::Sketch { .. }))
        .collect()
    }
}

/// Where one of `other`'s origin planes stands in `own`'s frame, the
/// bodies placed as the document has them.
pub fn body_plane_in(
    document: &crate::Document,
    own: crate::BodyId,
    other: crate::BodyId,
    plane: BasePlane,
) -> Option<DatumFrame> {
    document.body(other)?;
    let (origin, normal, x_axis) = plane.frame();
    let there = document.body_placement(other);
    let back = document.body_placement(own).inverse();
    let to_own = back.after(&there);
    Some(DatumFrame {
        origin: to_own.point(origin),
        normal: to_own.direction(normal),
        x_axis: to_own.direction(x_axis),
    })
}

pub(super) fn push_plane<'a>(plane: &'a mut PlaneAnchor, into: &mut Vec<Followed<'a>>) {
    if let PlaneAnchor::Face { face } = plane {
        into.push(Followed::Face(face));
    }
}

pub(super) fn push_line<'a>(line: &'a mut LineAnchor, into: &mut Vec<Followed<'a>>) {
    if let LineAnchor::Edge { edge } = line {
        into.push(Followed::Edge(edge, None));
    }
}

pub(super) fn push_point<'a>(anchor: &'a mut PointAnchor, into: &mut Vec<Followed<'a>>) {
    match anchor {
        PointAnchor::At { .. } | PointAnchor::Datum { .. } | PointAnchor::Sketch { .. } => {}
        PointAnchor::Face { face } => into.push(Followed::Face(face)),
        PointAnchor::Edge { edge, .. } => into.push(Followed::Edge(edge, None)),
    }
}

/// Bring a datum's working data up to date with what it stands on: the
/// answers the last build found of its references on the body's solid
/// (when they answer the references it has now), and the planes of the
/// datums it is made from. Answers whether anything changed.
pub fn derive(
    values: &mut serde_json::Value,
    probed: Option<&crate::rebuild::ProbedReferences>,
    values_of: &dyn Fn(FeatureId) -> Option<serde_json::Value>,
    body_plane: &dyn Fn(crate::BodyId, BasePlane) -> Option<DatumFrame>,
) -> bool {
    let Ok(mut datum) = DatumFeature::from_json(values) else {
        return false;
    };
    let before = datum;
    if let Some(probed) = probed
        && probed.probes == datum.probes()
    {
        datum.take_answers(&probed.answers);
    }
    datum.follow_datums(&|id| {
        let data = values_of(id)?;
        DatumFeature::from_json(&data).ok().map(|d| d.frame())
    });
    datum.follow_bodies(body_plane);
    if datum == before {
        return false;
    }
    *values = datum.to_json();
    true
}
