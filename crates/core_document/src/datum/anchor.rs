//! What a datum is attached to: frames, faces, edges, points, lines and
//! planes, as last seen.

use super::*;

/// A frame as it was last seen, which a datum follows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FrameAnchor {
    pub origin: [f32; 3],
    pub normal: [f32; 3],
    pub x_axis: [f32; 3],
}

impl From<DatumFrame> for FrameAnchor {
    fn from(f: DatumFrame) -> Self {
        Self {
            origin: f.origin,
            normal: f.normal,
            x_axis: f.x_axis,
        }
    }
}

/// A line a datum is made from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum LineAnchor {
    /// A straight edge, or the tangent of a curved one where picked.
    Edge { edge: EdgeAnchor },
    /// A datum line, as it was last seen.
    Datum {
        datum: FeatureId,
        origin: [f32; 3],
        direction: [f32; 3],
    },
    /// A sketch's line, its ends where they were last seen.
    Sketch {
        sketch: FeatureId,
        element: uuid::Uuid,
        start: [f32; 3],
        end: [f32; 3],
    },
}

impl LineAnchor {
    /// A point of the line and the way it runs.
    pub fn line(&self) -> ([f32; 3], [f32; 3]) {
        match *self {
            LineAnchor::Edge { edge } => match edge.ends {
                Some([a, b]) if edge.circle.is_none() && length(sub(b, a)) > 1e-6 => {
                    (a, normalize(sub(b, a)))
                }
                _ => (edge.point, normalize(edge.direction)),
            },
            LineAnchor::Datum {
                origin, direction, ..
            } => (origin, normalize(direction)),
            LineAnchor::Sketch { start, end, .. } => (start, normalize(sub(end, start))),
        }
    }

    /// The feature it is made from, when it is another feature.
    pub(super) fn feature(&self) -> Option<FeatureId> {
        match self {
            LineAnchor::Datum { datum, .. } => Some(*datum),
            LineAnchor::Sketch { sketch, .. } => Some(*sketch),
            LineAnchor::Edge { .. } => None,
        }
    }
}

pub(super) fn is_zero(name: &kernel_api::TopoName) -> bool {
    *name == 0
}

pub(super) fn are_zero(names: &[kernel_api::TopoName; 2]) -> bool {
    names.iter().all(|n| *n == 0)
}

/// A face of a solid: a point on it and its outward normal there, and its
/// surface when known.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FaceAnchor {
    pub point: [f32; 3],
    pub normal: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface: Option<kernel_api::FaceSurface>,
    /// The face is on the datum's own body, whose rebuilds find it again,
    /// so the datum follows it.
    #[serde(default)]
    pub follows: bool,
    /// The face's name, which a rebuild finds it by (`kernel_api::naming`);
    /// zero when it has none, and the point finds it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub name: kernel_api::TopoName,
}

/// An edge of a solid: a point on it and the way it runs there, with what
/// else is known of it (its ends, its middle, the circle it runs round).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EdgeAnchor {
    pub point: [f32; 3],
    pub direction: [f32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends: Option<[[f32; 3]; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middle: Option<[f32; 3]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub circle: Option<AnchorCircle>,
    /// The edge is on the datum's own body, whose rebuilds find it again.
    #[serde(default)]
    pub follows: bool,
    /// The names of the two faces the edge runs between, which a rebuild
    /// finds it by; zeros when it has none, and the point finds it.
    #[serde(default, skip_serializing_if = "are_zero")]
    pub faces: [kernel_api::TopoName; 2],
    /// Where the edge is a share along it from its start, and the way it
    /// runs there, as the solid last answered: the share, the point and the
    /// tangent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub along: Option<(f32, [f32; 3], [f32; 3])>,
}

/// The circle a circular edge runs round.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AnchorCircle {
    pub center: [f32; 3],
    pub normal: [f32; 3],
    pub radius: f32,
}

/// Which point of an edge a reference takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum EdgeSpot {
    /// Where it was picked.
    #[default]
    Picked,
    Start,
    End,
    Middle,
    /// The centre of the circle it runs round.
    Centre,
}

impl EdgeSpot {
    pub const ALL: [EdgeSpot; 5] = [
        EdgeSpot::Picked,
        EdgeSpot::Start,
        EdgeSpot::End,
        EdgeSpot::Middle,
        EdgeSpot::Centre,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            EdgeSpot::Picked => "Where picked",
            EdgeSpot::Start => "Start",
            EdgeSpot::End => "End",
            EdgeSpot::Middle => "Middle",
            EdgeSpot::Centre => "Centre",
        }
    }

    /// The name a command takes it by.
    pub fn key(&self) -> &'static str {
        match self {
            EdgeSpot::Picked => "picked",
            EdgeSpot::Start => "start",
            EdgeSpot::End => "end",
            EdgeSpot::Middle => "middle",
            EdgeSpot::Centre => "centre",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == key)
    }
}

/// A point a datum passes through.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PointAnchor {
    /// A point given by its coordinates.
    At { point: [f32; 3] },
    /// A point picked on a face.
    Face { face: FaceAnchor },
    /// A point of an edge.
    Edge {
        edge: EdgeAnchor,
        #[serde(default)]
        spot: EdgeSpot,
    },
    /// A datum's origin, as it was last seen.
    Datum { datum: FeatureId, point: [f32; 3] },
    /// A sketch's point, as it was last seen.
    Sketch {
        sketch: FeatureId,
        element: uuid::Uuid,
        point: [f32; 3],
    },
}

impl PointAnchor {
    pub fn point(&self) -> [f32; 3] {
        match self {
            PointAnchor::At { point } => *point,
            PointAnchor::Face { face } => face.point,
            PointAnchor::Edge { edge, spot } => edge.spot_point(*spot),
            PointAnchor::Datum { point, .. } | PointAnchor::Sketch { point, .. } => *point,
        }
    }

    /// The feature it is made from, when it is another feature.
    pub(super) fn feature(&self) -> Option<FeatureId> {
        match self {
            PointAnchor::Datum { datum, .. } => Some(*datum),
            PointAnchor::Sketch { sketch, .. } => Some(*sketch),
            _ => None,
        }
    }
}

/// A plane a datum is made from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PlaneAnchor {
    Base(BasePlane),
    /// Another datum's plane (a coordinate system's XY plane), as it was
    /// last seen; the datum follows it.
    Datum {
        datum: FeatureId,
        origin: [f32; 3],
        normal: [f32; 3],
    },
    /// A face's plane, or the plane tangent to it at the point.
    Face {
        face: FaceAnchor,
    },
}

impl PlaneAnchor {
    /// (origin, normal) of the plane.
    pub fn plane(&self) -> ([f32; 3], [f32; 3]) {
        match self {
            PlaneAnchor::Base(plane) => {
                let (origin, normal, _) = plane.frame();
                (origin, normal)
            }
            PlaneAnchor::Datum { origin, normal, .. } => (*origin, normalize(*normal)),
            PlaneAnchor::Face { face } => (face.point, normalize(face.normal)),
        }
    }
}

impl EdgeAnchor {
    /// The point `share` of the way along the edge from its start (0 to 1)
    /// and the way it runs there: as the solid answered for that share, else
    /// worked out for a straight edge or an arc of a circle, else where it
    /// was picked.
    pub fn at_share(&self, share: f32) -> ([f32; 3], [f32; 3]) {
        if let Some((s, point, tangent)) = self.along
            && (s - share).abs() < 1e-6
        {
            return (point, normalize(tangent));
        }
        let Some([a, b]) = self.ends else {
            return (self.point, normalize(self.direction));
        };
        match self.circle {
            Some(circle) => {
                let n = normalize(circle.normal);
                let (ra, rb) = (sub(a, circle.center), sub(b, circle.center));
                // The sweep from start to end, the way the edge runs.
                let mut sweep = dot(cross(ra, rb), n).atan2(dot(ra, rb));
                let turn = dot(cross(ra, self.tangent_at(a)), n);
                if turn < 0.0 {
                    sweep = if sweep > 0.0 {
                        sweep - std::f32::consts::TAU
                    } else {
                        sweep
                    };
                } else if sweep <= 0.0 {
                    sweep += std::f32::consts::TAU;
                }
                let angle = sweep * share;
                let (sin, cos) = angle.sin_cos();
                let rotated = add(
                    add(scale(ra, cos), scale(cross(n, ra), sin)),
                    scale(n, dot(n, ra) * (1.0 - cos)),
                );
                let point = add(circle.center, rotated);
                (point, self.tangent_at(point))
            }
            None => (add(a, scale(sub(b, a), share)), normalize(sub(b, a))),
        }
    }

    /// The point of the edge `spot` names; where it was picked when the
    /// spot is not known (an edge picked without its ends).
    pub fn spot_point(&self, spot: EdgeSpot) -> [f32; 3] {
        match spot {
            EdgeSpot::Picked => Some(self.point),
            EdgeSpot::Start => self.ends.map(|[a, _]| a),
            EdgeSpot::End => self.ends.map(|[_, b]| b),
            EdgeSpot::Middle => self.middle,
            EdgeSpot::Centre => self.circle.map(|c| c.center),
        }
        .unwrap_or(self.point)
    }

    /// The way the edge runs at `at`: square to the radius on a circle,
    /// turned the way it was picked; along the picked direction otherwise.
    pub(super) fn tangent_at(&self, at: [f32; 3]) -> [f32; 3] {
        let picked = normalize(self.direction);
        if let Some(circle) = self.circle {
            let tangent = cross(circle.normal, sub(at, circle.center));
            if length(tangent) > 1e-6 {
                let tangent = normalize(tangent);
                return if dot(tangent, picked) < 0.0 {
                    scale(tangent, -1.0)
                } else {
                    tangent
                };
            }
        }
        picked
    }
}
