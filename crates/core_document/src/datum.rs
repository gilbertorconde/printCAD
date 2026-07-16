//! Core datum features: reference planes, lines, and points shared across
//! workbenches. A datum is a feature node (`workbench_id = "core.datum"`)
//! whose placement comes from an attachment plus a local offset, so
//! downstream sketches stay decoupled from generated-face topology churn.

use serde::{Deserialize, Serialize};

use crate::{DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId};

/// What geometry the datum represents.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DatumShape {
    /// Reference plane, drawn as a `size`-sided square.
    Plane { size: f32 },
    /// Reference line along the attachment frame's normal-perpendicular
    /// x-axis, drawn `length` long.
    Line { length: f32 },
    /// Reference point at the attachment origin.
    Point,
}

impl DatumShape {
    pub fn label(&self) -> &'static str {
        match self {
            DatumShape::Plane { .. } => "Datum Plane",
            DatumShape::Line { .. } => "Datum Line",
            DatumShape::Point => "Datum Point",
        }
    }
}

/// One of the document's three base planes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BasePlane {
    #[default]
    XY,
    XZ,
    YZ,
}

impl BasePlane {
    pub const ALL: [BasePlane; 3] = [BasePlane::XY, BasePlane::XZ, BasePlane::YZ];

    pub fn label(&self) -> &'static str {
        match self {
            BasePlane::XY => "XY (Top)",
            BasePlane::XZ => "XZ (Front)",
            BasePlane::YZ => "YZ (Side)",
        }
    }

    /// (origin, normal, x_axis) of the base plane.
    pub fn frame(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        match self {
            BasePlane::XY => ([0.0; 3], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]),
            BasePlane::XZ => ([0.0; 3], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0]),
            BasePlane::YZ => ([0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        }
    }
}

/// What the datum is anchored to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DatumAttachment {
    /// One of the document base planes.
    BasePlane(BasePlane),
    /// A planar face picked in the viewport, identified geometrically.
    FlatFace { point: [f32; 3], normal: [f32; 3] },
}

impl DatumAttachment {
    pub fn label(&self) -> &'static str {
        match self {
            DatumAttachment::BasePlane(plane) => plane.label(),
            DatumAttachment::FlatFace { .. } => "Picked face",
        }
    }
}

/// Extra placement applied IN the attachment coordinate system.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct AttachmentOffset {
    /// Translation along the attachment x/y/normal axes (millimetres).
    pub translation: [f32; 3],
    /// In-plane rotation about the attachment normal (degrees).
    pub rotation_deg: f32,
    /// Flip to the other side (180° about the local x-axis).
    pub flip: bool,
}

/// A datum feature payload.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DatumFeature {
    pub shape: DatumShape,
    pub attachment: DatumAttachment,
    #[serde(default)]
    pub offset: AttachmentOffset,
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len < 1e-9 {
        return [0.0, 0.0, 1.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn scale_add(p: [f32; 3], v: [f32; 3], s: f32) -> [f32; 3] {
    [p[0] + v[0] * s, p[1] + v[1] * s, p[2] + v[2] * s]
}

/// Stable in-plane basis for an arbitrary normal: pick the world axis least
/// aligned with the normal and orthogonalize it.
fn stable_x_axis(normal: [f32; 3]) -> [f32; 3] {
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

/// A resolved datum placement in world coordinates.
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
}

impl DatumFeature {
    /// Resolve the attachment + offset into a world placement.
    pub fn frame(&self) -> DatumFrame {
        let (origin, normal, x_axis) = match self.attachment {
            DatumAttachment::BasePlane(plane) => plane.frame(),
            DatumAttachment::FlatFace { point, normal } => {
                let n = normalize(normal);
                (point, n, stable_x_axis(n))
            }
        };
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

impl WorkbenchFeature for DatumFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("core.datum")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            crate::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        Vec::new()
    }

    fn name(&self) -> &str {
        self.shape.label()
    }
}

/// All datum features of a body, resolved and named.
pub fn datums_of_body(
    document: &crate::Document,
    body: crate::BodyId,
) -> Vec<(FeatureId, String, DatumFeature)> {
    let mut datums: Vec<(u64, FeatureId, String, DatumFeature)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "core.datum" && n.body == Some(body))
        .filter_map(|(id, n)| {
            DatumFeature::from_json(&n.data)
                .ok()
                .map(|d| (n.seq, *id, n.name.clone(), d))
        })
        .collect();
    datums.sort_by_key(|(seq, ..)| *seq);
    datums
        .into_iter()
        .map(|(_, id, name, d)| (id, name, d))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|i| (a[i] - b[i]).abs() < 1e-5)
    }

    #[test]
    fn base_plane_with_offset_translates_along_normal() {
        let datum = DatumFeature {
            shape: DatumShape::Plane { size: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: AttachmentOffset {
                translation: [0.0, 0.0, 7.5],
                rotation_deg: 0.0,
                flip: false,
            },
        };
        let frame = datum.frame();
        assert!(close(frame.origin, [0.0, 0.0, 7.5]));
        assert!(close(frame.normal, [0.0, 0.0, 1.0]));
    }

    #[test]
    fn flat_face_attachment_derives_orthonormal_frame() {
        let datum = DatumFeature {
            shape: DatumShape::Plane { size: 20.0 },
            attachment: DatumAttachment::FlatFace {
                point: [3.0, 4.0, 5.0],
                normal: [0.0, 3.0, 4.0],
            },
            offset: AttachmentOffset::default(),
        };
        let frame = datum.frame();
        let n = frame.normal;
        let x = frame.x_axis;
        let dot = n[0] * x[0] + n[1] * x[1] + n[2] * x[2];
        assert!(dot.abs() < 1e-5, "x-axis orthogonal to normal");
        assert!((n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn flip_reverses_the_normal() {
        let datum = DatumFeature {
            shape: DatumShape::Plane { size: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: AttachmentOffset {
                translation: [0.0; 3],
                rotation_deg: 0.0,
                flip: true,
            },
        };
        assert!(close(datum.frame().normal, [0.0, 0.0, -1.0]));
    }

    #[test]
    fn rotation_spins_the_x_axis_in_plane() {
        let datum = DatumFeature {
            shape: DatumShape::Plane { size: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: AttachmentOffset {
                translation: [0.0; 3],
                rotation_deg: 90.0,
                flip: false,
            },
        };
        assert!(close(datum.frame().x_axis, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn json_round_trip() {
        let datum = DatumFeature {
            shape: DatumShape::Line { length: 30.0 },
            attachment: DatumAttachment::FlatFace {
                point: [1.0, 2.0, 3.0],
                normal: [0.0, 0.0, 1.0],
            },
            offset: AttachmentOffset {
                translation: [1.0, 2.0, 3.0],
                rotation_deg: 15.0,
                flip: true,
            },
        };
        let json = datum.to_json();
        assert_eq!(DatumFeature::from_json(&json).unwrap(), datum);
    }
}
