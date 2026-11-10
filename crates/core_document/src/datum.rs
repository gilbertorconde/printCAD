//! Core datum features: reference planes, lines, points and coordinate
//! systems shared across workbenches. A datum is a feature node (`workbench_id = "core.datum"`)
//! whose placement comes from an attachment plus a local offset, so
//! downstream sketches stay decoupled from generated-face topology churn.

use kernel_api::{ProbeAnswer, ShapeProbe};
use serde::{Deserialize, Serialize};

use crate::{FeatureId, WorkbenchFeature, WorkbenchId};

mod anchor;
mod follow;
mod frame;

pub use anchor::*;
pub use follow::*;
pub use frame::*;

/// The feature kind datums are stored under.
pub const DATUM_KIND: &str = "core.datum";

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
    /// A local coordinate system: the frame's origin and its three axes
    /// (x, y and the normal as z), drawn `size` long. Its XY, XZ and YZ
    /// planes carry sketches the way the base planes do.
    CoordinateSystem { size: f32 },
}

impl DatumShape {
    pub fn label(&self) -> &'static str {
        match self {
            DatumShape::Plane { .. } => "Datum Plane",
            DatumShape::Line { .. } => "Datum Line",
            DatumShape::Point => "Datum Point",
            DatumShape::CoordinateSystem { .. } => "Local Coordinate System",
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

/// What the datum is anchored to. The references to a solid are kept
/// geometrically, in the body's own frame: a point and a direction, never a
/// face or edge number, which a rebuilt solid gives out afresh.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DatumAttachment {
    /// One of the document base planes.
    BasePlane(BasePlane),
    /// A planar face, as a point and normal kept as they were picked.
    FlatFace { point: [f32; 3], normal: [f32; 3] },
    /// On a face at a point: its plane when the face is flat, the plane
    /// tangent to it there when it is curved.
    Face { face: FaceAnchor },
    /// The plane through three points; its x-axis runs from the first
    /// towards the second.
    ThreePoints { points: [PointAnchor; 3] },
    /// The plane square to an edge, where `spot` falls on it, or a share
    /// `along` the edge from its start (0 to 1): the planes along a curve.
    NormalToEdge {
        edge: EdgeAnchor,
        #[serde(default)]
        spot: EdgeSpot,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        along: Option<f32>,
    },
    /// Along a straight edge, centred on it; round a circular one, its
    /// axis through its centre.
    AlongEdge { edge: EdgeAnchor },
    /// The line through two points, centred between them.
    TwoPoints { points: [PointAnchor; 2] },
    /// The line where two planes meet, centred nearest their origins.
    PlaneIntersection { planes: [PlaneAnchor; 2] },
    /// The centre of a circular edge, square to it.
    CurveCentre { edge: EdgeAnchor },
    /// The body's centre of mass, on its principal axes of inertia: x the
    /// axis it turns about most easily, the normal the one it resists most.
    Inertia {
        centre: [f32; 3],
        axes: [[f32; 3]; 3],
    },
    /// On another datum: a datum plane, or one of a coordinate system's
    /// three planes (`plane`), as it was last seen; the offset moves and
    /// turns it from there, and it follows the datum.
    OnDatum {
        datum: FeatureId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plane: Option<BasePlane>,
        frame: FrameAnchor,
    },
    /// One of another body's origin planes, where it stands in this body's
    /// frame as last seen; it follows the other body as it is placed.
    OtherBody {
        body: crate::BodyId,
        plane: BasePlane,
        frame: FrameAnchor,
    },
    /// The line square to a face where it was picked.
    FaceNormal { face: FaceAnchor },
    /// The line tangent to an edge at `spot`.
    TangentToEdge {
        edge: EdgeAnchor,
        #[serde(default)]
        spot: EdgeSpot,
    },
    /// The plane through a line and a point off it; its x-axis along the
    /// line, its origin the point's foot on it.
    LineAndPoint {
        line: LineAnchor,
        point: PointAnchor,
    },
    /// The point where a line meets a plane.
    LineMeetsPlane {
        line: LineAnchor,
        plane: PlaneAnchor,
    },
    /// Where two lines cross: the middle of where they pass nearest, the
    /// x-axis along the first, the normal square to both.
    TwoLines { lines: [LineAnchor; 2] },
    /// Midway between two planes: halfway between parallel ones, else the
    /// bisector of their angle that passes between where each was taken;
    /// its origin the middle of the two planes' origins, brought onto it,
    /// its normal to the first plane's side.
    Midplane { planes: [PlaneAnchor; 2] },
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
    /// Degrees it tilts about the local x-axis, then about the local
    /// y-axis, after the turn about the normal.
    #[serde(default)]
    pub tilt: [f32; 2],
}

/// A datum feature payload.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DatumFeature {
    pub shape: DatumShape,
    pub attachment: DatumAttachment,
    #[serde(default)]
    pub offset: AttachmentOffset,
}

impl WorkbenchFeature for DatumFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(DATUM_KIND)
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        self.attachment.datums()
    }

    fn name(&self) -> &str {
        self.shape.label()
    }
}

/// A datum with where its body puts it: its values as evaluated and its
/// body's placement (the world's own for a datum of no body).
#[derive(Debug, Clone)]
pub struct PlacedDatum {
    pub name: String,
    pub feature: DatumFeature,
    pub placement: crate::BodyPlacement,
}

impl PlacedDatum {
    /// Its origin in the world.
    pub fn origin(&self) -> [f32; 3] {
        self.placement.point(self.feature.frame().origin)
    }
}

impl crate::Document {
    /// Datum `id` placed in the world. A suppressed datum stands nowhere,
    /// as it builds nothing: `None`, like a feature that is not a datum.
    pub fn placed_datum(&self, id: FeatureId) -> Option<PlacedDatum> {
        self.place_datum(id, self.feature_as::<DatumFeature>(id)?)
    }

    /// Every datum [`Self::placed_datum`] places, in build order.
    pub fn placed_datums(&self) -> Vec<(FeatureId, PlacedDatum)> {
        self.features_of::<DatumFeature>()
            .into_iter()
            .filter_map(|(id, feature)| Some((id, self.place_datum(id, feature)?)))
            .collect()
    }

    fn place_datum(&self, id: FeatureId, feature: DatumFeature) -> Option<PlacedDatum> {
        let node = self.get_feature_meta(id).filter(|n| !n.suppressed)?;
        let placement = node
            .body
            .map(|body| self.body_placement(body))
            .unwrap_or_default();
        Some(PlacedDatum {
            name: node.name.clone(),
            feature,
            placement,
        })
    }
}

/// All datum features of a body with their names, in history order.
pub fn datums_of_body(
    document: &crate::Document,
    body: crate::BodyId,
) -> Vec<(FeatureId, String, DatumFeature)> {
    let mut datums: Vec<(u64, FeatureId, String, DatumFeature)> = document
        .feature_tree()
        .all_nodes()
        .filter(|(_, n)| n.workbench_id.as_str() == "core.datum" && n.body == Some(body))
        .filter_map(|(id, n)| {
            document
                .feature_as::<DatumFeature>(*id)
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

    /// A plane on XY, moved by `offset`.
    fn offset_xy(offset: AttachmentOffset) -> DatumFeature {
        DatumFeature {
            shape: DatumShape::Plane { size: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset,
        }
    }

    /// An offset moves a plane along its normal, flips it, spins its x
    /// axis in the plane and leans it about its axes.
    #[test]
    fn an_offset_moves_flips_spins_and_tilts_a_plane() {
        let none = AttachmentOffset::default();
        type Want = Option<[f32; 3]>;
        let rows: [(&str, AttachmentOffset, Want, Want, Want); 5] = [
            (
                "translated along the normal",
                AttachmentOffset {
                    translation: [0.0, 0.0, 7.5],
                    ..none
                },
                Some([0.0, 0.0, 7.5]),
                Some([0.0, 0.0, 1.0]),
                None,
            ),
            (
                "flipped",
                AttachmentOffset { flip: true, ..none },
                None,
                Some([0.0, 0.0, -1.0]),
                None,
            ),
            (
                "rotated a quarter turn",
                AttachmentOffset {
                    rotation_deg: 90.0,
                    ..none
                },
                None,
                None,
                Some([0.0, 1.0, 0.0]),
            ),
            // A quarter turn about x takes the normal from z to -y.
            (
                "tilted about x",
                AttachmentOffset {
                    tilt: [90.0, 0.0],
                    ..none
                },
                None,
                Some([0.0, -1.0, 0.0]),
                Some([1.0, 0.0, 0.0]),
            ),
            (
                "tilted about y",
                AttachmentOffset {
                    tilt: [0.0, 90.0],
                    ..none
                },
                None,
                Some([1.0, 0.0, 0.0]),
                None,
            ),
        ];
        for (what, offset, origin, normal, x_axis) in rows {
            let frame = offset_xy(offset).frame();
            orthonormal(&frame);
            for (got, want) in [
                (frame.origin, origin),
                (frame.normal, normal),
                (frame.x_axis, x_axis),
            ] {
                if let Some(want) = want {
                    assert!(close(got, want), "{what}: {frame:?}");
                }
            }
        }
    }

    #[test]
    fn flat_face_attachment_derives_orthonormal_frame() {
        orthonormal(&plane(DatumAttachment::FlatFace {
            point: [3.0, 4.0, 5.0],
            normal: [0.0, 3.0, 4.0],
        }));
    }

    #[test]
    fn a_coordinate_system_lays_out_its_planes_as_the_base_planes_are() {
        let datum = DatumFeature {
            shape: DatumShape::CoordinateSystem { size: 20.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: AttachmentOffset {
                translation: [1.0, 2.0, 3.0],
                ..AttachmentOffset::default()
            },
        };
        let frame = datum.frame();
        for ((label, plane), base) in frame.planes().into_iter().zip(BasePlane::ALL) {
            let (_, base_normal, base_x) = base.frame();
            assert!(close(plane.origin, [1.0, 2.0, 3.0]), "{label}");
            assert!(close(plane.normal, base_normal), "{label}");
            assert!(close(plane.x_axis, base_x), "{label}");
        }
    }

    fn datum(shape: DatumShape, attachment: DatumAttachment) -> DatumFeature {
        DatumFeature {
            shape,
            attachment,
            offset: AttachmentOffset::default(),
        }
    }

    fn plane(attachment: DatumAttachment) -> DatumFrame {
        datum(DatumShape::Plane { size: 20.0 }, attachment).frame()
    }

    fn line(attachment: DatumAttachment) -> DatumFrame {
        datum(DatumShape::Line { length: 20.0 }, attachment).frame()
    }

    fn parallel(a: [f32; 3], b: [f32; 3]) -> bool {
        (dot(normalize(a), normalize(b)).abs() - 1.0).abs() < 1e-5
    }

    fn orthonormal(frame: &DatumFrame) {
        assert!((length(frame.normal) - 1.0).abs() < 1e-5);
        assert!((length(frame.x_axis) - 1.0).abs() < 1e-5);
        assert!(dot(frame.normal, frame.x_axis).abs() < 1e-5);
    }

    fn at(point: [f32; 3]) -> PointAnchor {
        PointAnchor::At { point }
    }

    fn straight_edge() -> EdgeAnchor {
        EdgeAnchor {
            along: None,
            faces: [0, 0],
            point: [2.0, 0.0, 0.0],
            direction: [1.0, 0.0, 0.0],
            ends: Some([[0.0, 0.0, 0.0], [10.0, 0.0, 0.0]]),
            middle: Some([5.0, 0.0, 0.0]),
            circle: None,
            follows: true,
        }
    }

    fn rim() -> EdgeAnchor {
        EdgeAnchor {
            along: None,
            faces: [0, 0],
            point: [5.0, 0.0, 12.0],
            direction: [0.0, 1.0, 0.0],
            ends: None,
            middle: None,
            circle: Some(AnchorCircle {
                center: [0.0, 0.0, 12.0],
                normal: [0.0, 0.0, 1.0],
                radius: 5.0,
            }),
            follows: true,
        }
    }

    fn cylinder_face() -> FaceAnchor {
        FaceAnchor {
            name: 0,
            point: [0.0, 5.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            surface: Some(kernel_api::FaceSurface::Cylinder {
                origin: [0.0; 3],
                axis: [0.0, 0.0, 1.0],
                radius: 5.0,
            }),
            follows: true,
        }
    }

    #[test]
    fn a_plane_on_a_cylinder_is_tangent_where_picked_its_x_along_the_axis() {
        let frame = plane(DatumAttachment::Face {
            face: cylinder_face(),
        });
        orthonormal(&frame);
        assert!(close(frame.origin, [0.0, 5.0, 3.0]));
        assert!(close(frame.normal, [0.0, 1.0, 0.0]));
        assert!(parallel(frame.x_axis, [0.0, 0.0, 1.0]));
    }

    #[test]
    fn three_points_make_the_plane_through_them() {
        let frame = plane(DatumAttachment::ThreePoints {
            points: [
                at([1.0, 1.0, 2.0]),
                at([4.0, 1.0, 2.0]),
                at([1.0, 5.0, 2.0]),
            ],
        });
        orthonormal(&frame);
        assert!(close(frame.origin, [1.0, 1.0, 2.0]));
        assert!(close(frame.normal, [0.0, 0.0, 1.0]));
        assert!(close(frame.x_axis, [1.0, 0.0, 0.0]));
    }

    #[test]
    fn points_in_a_line_or_on_top_of_each_other_are_refused() {
        let collinear = DatumAttachment::ThreePoints {
            points: [at([0.0; 3]), at([1.0, 0.0, 0.0]), at([2.0, 0.0, 0.0])],
        };
        assert!(collinear.problem().is_some());
        let same = DatumAttachment::TwoPoints {
            points: [at([1.0; 3]), at([1.0; 3])],
        };
        assert!(same.problem().is_some());
        let fine = DatumAttachment::TwoPoints {
            points: [at([0.0; 3]), at([1.0; 3])],
        };
        assert!(fine.problem().is_none());
    }

    #[test]
    fn a_plane_square_to_an_edge_stands_where_the_spot_is() {
        let frame = plane(DatumAttachment::NormalToEdge {
            along: None,
            edge: straight_edge(),
            spot: EdgeSpot::End,
        });
        orthonormal(&frame);
        assert!(close(frame.origin, [10.0, 0.0, 0.0]));
        assert!(parallel(frame.normal, [1.0, 0.0, 0.0]));

        // On a circle: square to it where picked, x out from the centre.
        let frame = plane(DatumAttachment::NormalToEdge {
            along: None,
            edge: rim(),
            spot: EdgeSpot::Picked,
        });
        orthonormal(&frame);
        assert!(close(frame.origin, [5.0, 0.0, 12.0]));
        assert!(close(frame.normal, [0.0, 1.0, 0.0]));
        assert!(close(frame.x_axis, [1.0, 0.0, 0.0]));
    }

    #[test]
    fn a_line_along_an_edge_runs_along_it_or_round_a_circle_on_its_axis() {
        let frame = line(DatumAttachment::AlongEdge {
            edge: straight_edge(),
        });
        orthonormal(&frame);
        assert!(close(frame.origin, [5.0, 0.0, 0.0]));
        assert!(parallel(frame.x_axis, [1.0, 0.0, 0.0]));

        let frame = line(DatumAttachment::AlongEdge { edge: rim() });
        assert!(close(frame.origin, [0.0, 0.0, 12.0]));
        assert!(parallel(frame.x_axis, [0.0, 0.0, 1.0]));
    }

    #[test]
    fn a_line_through_two_points_is_centred_between_them() {
        let frame = line(DatumAttachment::TwoPoints {
            points: [
                at([0.0, 0.0, 0.0]),
                PointAnchor::Edge {
                    edge: straight_edge(),
                    spot: EdgeSpot::End,
                },
            ],
        });
        orthonormal(&frame);
        assert!(close(frame.origin, [5.0, 0.0, 0.0]));
        assert!(parallel(frame.x_axis, [1.0, 0.0, 0.0]));
    }

    #[test]
    fn two_planes_meet_in_a_line_on_both() {
        let attachment = DatumAttachment::PlaneIntersection {
            planes: [
                PlaneAnchor::Datum {
                    datum: FeatureId(uuid::Uuid::nil()),
                    origin: [0.0, 0.0, 4.0],
                    normal: [0.0, 0.0, 1.0],
                },
                PlaneAnchor::Base(BasePlane::YZ),
            ],
        };
        let frame = line(attachment);
        orthonormal(&frame);
        assert!(parallel(frame.x_axis, [0.0, 1.0, 0.0]));
        assert!(frame.origin[0].abs() < 1e-5 && (frame.origin[2] - 4.0).abs() < 1e-5);
        assert_eq!(attachment.datums(), vec![FeatureId(uuid::Uuid::nil())]);

        let parallel_planes = DatumAttachment::PlaneIntersection {
            planes: [
                PlaneAnchor::Base(BasePlane::XY),
                PlaneAnchor::Face {
                    face: FaceAnchor {
                        name: 0,
                        point: [0.0, 0.0, 3.0],
                        normal: [0.0, 0.0, -1.0],
                        surface: None,
                        follows: false,
                    },
                },
            ],
        };
        assert!(parallel_planes.problem().is_some());
    }

    fn flat(point: [f32; 3], normal: [f32; 3]) -> PlaneAnchor {
        PlaneAnchor::Face {
            face: FaceAnchor {
                name: 0,
                point,
                normal,
                surface: None,
                follows: true,
            },
        }
    }

    #[test]
    fn a_midplane_of_parallel_faces_sits_halfway_whichever_way_they_face() {
        // A slab's two sides, facing away from each other.
        let slab = DatumAttachment::Midplane {
            planes: [
                flat([1.0, 2.0, 10.0], [0.0, 0.0, 1.0]),
                flat([5.0, 6.0, 0.0], [0.0, 0.0, -1.0]),
            ],
        };
        assert!(slab.problem().is_none());
        let frame = plane(slab);
        orthonormal(&frame);
        assert!(close(frame.origin, [3.0, 4.0, 5.0]), "{frame:?}");
        assert!(close(frame.normal, [0.0, 0.0, 1.0]), "{frame:?}");

        // Two steps of a staircase, facing the same way.
        let steps = DatumAttachment::Midplane {
            planes: [
                flat([0.0, 0.0, 2.0], [0.0, 0.0, 1.0]),
                flat([4.0, 0.0, 8.0], [0.0, 0.0, 1.0]),
            ],
        };
        let frame = plane(steps);
        orthonormal(&frame);
        assert!(close(frame.origin, [2.0, 0.0, 5.0]), "{frame:?}");
        assert!(close(frame.normal, [0.0, 0.0, 1.0]), "{frame:?}");

        // A gap's two walls, facing each other, and a datum plane.
        let gap = DatumAttachment::Midplane {
            planes: [
                PlaneAnchor::Base(BasePlane::YZ),
                flat([6.0, 1.0, 1.0], [-1.0, 0.0, 0.0]),
            ],
        };
        let frame = plane(gap);
        assert!((frame.origin[0] - 3.0).abs() < 1e-5, "{frame:?}");
        assert!(close(frame.normal, [1.0, 0.0, 0.0]), "{frame:?}");
    }

    #[test]
    fn a_midplane_of_faces_at_an_angle_bisects_the_angle_between_them() {
        // A box's top and its side at x = 10, both facing out: the plane
        // through their common edge, leaning into the box.
        let corner = DatumAttachment::Midplane {
            planes: [
                flat([5.0, 5.0, 10.0], [0.0, 0.0, 1.0]),
                flat([10.0, 5.0, 5.0], [1.0, 0.0, 0.0]),
            ],
        };
        assert!(corner.problem().is_none());
        let frame = plane(corner);
        orthonormal(&frame);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(frame.normal, [-h, 0.0, h]), "{frame:?}");
        assert!(parallel(frame.x_axis, [0.0, 1.0, 0.0]), "along the edge");
        assert!(close(frame.origin, [7.5, 5.0, 7.5]), "{frame:?}");
        // Every point of it stands as far from one face as from the other.
        let p = scale_add(frame.origin, frame.y_axis(), 3.0);
        assert!(((10.0 - p[2]) - (10.0 - p[0])).abs() < 1e-4, "{p:?}");

        // A ridge's two faces, each 60° from level and picked below the
        // ridge: the upright plane through the material between them, not
        // the level one outside it.
        let (s, c) = 60f32.to_radians().sin_cos();
        let ridge = DatumAttachment::Midplane {
            planes: [
                flat([4.0 * c, 0.0, -4.0 * s], [s, 0.0, c]),
                flat([-4.0 * c, 2.0, -4.0 * s], [-s, 0.0, c]),
            ],
        };
        let frame = plane(ridge);
        assert!(close(frame.normal, [1.0, 0.0, 0.0]), "{frame:?}");
        assert!(close(frame.origin, [0.0, 1.0, -4.0 * s]), "{frame:?}");
    }

    #[test]
    fn a_midplane_of_one_plane_twice_is_refused() {
        let same = DatumAttachment::Midplane {
            planes: [
                PlaneAnchor::Base(BasePlane::XY),
                flat([3.0, 4.0, 0.0], [0.0, 0.0, -1.0]),
            ],
        };
        assert!(same.problem().is_some());
        let frame = plane(same);
        assert!(close(frame.normal, [0.0, 0.0, 1.0]), "{frame:?}");
    }

    #[test]
    fn a_midplane_follows_both_faces_and_the_datums_it_is_made_from() {
        let other = FeatureId(uuid::Uuid::new_v4());
        let mut middle = datum(
            DatumShape::Plane { size: 20.0 },
            DatumAttachment::Midplane {
                planes: [
                    flat([0.0, 0.0, 10.0], [0.0, 0.0, 1.0]),
                    PlaneAnchor::Datum {
                        datum: other,
                        origin: [0.0; 3],
                        normal: [0.0, 0.0, 1.0],
                    },
                ],
            },
        );
        assert_eq!(middle.attachment.datums(), vec![other]);
        assert_eq!(middle.probes().len(), 1);
        middle.take_answers(&[Ok(ProbeAnswer::Face {
            point: [0.0, 0.0, 20.0],
            normal: [0.0, 0.0, 1.0],
            surface: kernel_api::FaceSurface::Other,
        })]);
        let moved = offset_xy(AttachmentOffset {
            translation: [0.0, 0.0, 4.0],
            ..AttachmentOffset::default()
        });
        let values_of = |id: FeatureId| (id == other).then(|| moved.to_json());
        let mut values = middle.to_json();
        assert!(derive(&mut values, None, &values_of, &|_, _| None));
        let frame = DatumFeature::from_json(&values).unwrap().frame();
        assert!((frame.origin[2] - 12.0).abs() < 1e-5, "{frame:?}");
    }

    #[test]
    fn a_point_at_a_curve_s_centre_sits_on_the_circle_s_centre() {
        let frame = datum(
            DatumShape::Point,
            DatumAttachment::CurveCentre { edge: rim() },
        )
        .frame();
        assert!(close(frame.origin, [0.0, 0.0, 12.0]));
        assert!(parallel(frame.normal, [0.0, 0.0, 1.0]));
        let mut straight = straight_edge();
        straight.circle = None;
        assert!(
            DatumAttachment::CurveCentre { edge: straight }
                .problem()
                .is_some()
        );
    }

    #[test]
    fn inertia_axes_make_the_coordinate_system() {
        let frame = datum(
            DatumShape::CoordinateSystem { size: 10.0 },
            DatumAttachment::Inertia {
                centre: [1.0, 2.0, 3.0],
                axes: [[0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            },
        )
        .frame();
        orthonormal(&frame);
        assert!(close(frame.origin, [1.0, 2.0, 3.0]));
        assert!(close(frame.x_axis, [0.0, 0.0, 1.0]));
        assert!(close(frame.y_axis(), [1.0, 0.0, 0.0]));
        assert!(close(frame.normal, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn only_references_on_the_body_are_probed_and_answers_land_in_order() {
        let mut other = cylinder_face();
        other.follows = false;
        let mut datum = datum(
            DatumShape::Plane { size: 20.0 },
            DatumAttachment::ThreePoints {
                points: [
                    PointAnchor::Face {
                        face: cylinder_face(),
                    },
                    PointAnchor::Face { face: other },
                    PointAnchor::Edge {
                        edge: straight_edge(),
                        spot: EdgeSpot::Start,
                    },
                ],
            },
        );
        let probes = datum.probes();
        assert_eq!(probes.len(), 2);
        assert!(matches!(probes[0], ShapeProbe::Face { .. }));
        assert!(matches!(probes[1], ShapeProbe::Edge { .. }));

        datum.take_answers(&[
            Ok(ProbeAnswer::Face {
                point: [0.0, 8.0, 3.0],
                normal: [0.0, 1.0, 0.0],
                surface: kernel_api::FaceSurface::Other,
            }),
            Ok(ProbeAnswer::Edge {
                along: None,
                point: [2.0, 0.0, 0.0],
                direction: [1.0, 0.0, 0.0],
                start: [-1.0, 0.0, 0.0],
                end: [11.0, 0.0, 0.0],
                middle: [5.0, 0.0, 0.0],
                circle: None,
            }),
        ]);
        let DatumAttachment::ThreePoints { points } = datum.attachment else {
            unreachable!()
        };
        assert!(close(points[0].point(), [0.0, 8.0, 3.0]));
        assert!(close(points[1].point(), [0.0, 5.0, 3.0]), "not probed");
        assert!(close(points[2].point(), [-1.0, 0.0, 0.0]));
    }

    #[test]
    fn derive_takes_answers_only_for_the_references_they_were_asked_of() {
        let datum = datum(
            DatumShape::Plane { size: 20.0 },
            DatumAttachment::Face {
                face: cylinder_face(),
            },
        );
        let answer = Ok(ProbeAnswer::Face {
            point: [0.0, 8.0, 3.0],
            normal: [0.0, 1.0, 0.0],
            surface: kernel_api::FaceSurface::Other,
        });
        let none = |_: FeatureId| None;
        let mut values = datum.to_json();
        let stale = crate::rebuild::ProbedReferences {
            probes: vec![ShapeProbe::Mass],
            answers: vec![answer.clone()],
        };
        assert!(!derive(&mut values, Some(&stale), &none, &|_, _| None));
        let fresh = crate::rebuild::ProbedReferences {
            probes: datum.probes(),
            answers: vec![answer],
        };
        assert!(derive(&mut values, Some(&fresh), &none, &|_, _| None));
        let followed = DatumFeature::from_json(&values).unwrap().frame();
        assert!(close(followed.origin, [0.0, 8.0, 3.0]));
    }

    #[test]
    fn derive_follows_the_datums_a_line_is_made_from() {
        let other = FeatureId(uuid::Uuid::new_v4());
        let line = datum(
            DatumShape::Line { length: 20.0 },
            DatumAttachment::PlaneIntersection {
                planes: [
                    PlaneAnchor::Datum {
                        datum: other,
                        origin: [0.0; 3],
                        normal: [0.0, 0.0, 1.0],
                    },
                    PlaneAnchor::Base(BasePlane::YZ),
                ],
            },
        );
        let moved = offset_xy(AttachmentOffset {
            translation: [0.0, 0.0, 6.0],
            ..AttachmentOffset::default()
        });
        let values_of = |id: FeatureId| (id == other).then(|| moved.to_json());
        let mut values = line.to_json();
        assert!(derive(&mut values, None, &values_of, &|_, _| None));
        let frame = DatumFeature::from_json(&values).unwrap().frame();
        assert!((frame.origin[2] - 6.0).abs() < 1e-5, "{:?}", frame.origin);
    }

    #[test]
    fn every_mode_round_trips_through_json() {
        let modes = [
            DatumAttachment::FlatFace {
                point: [1.0, 2.0, 3.0],
                normal: [0.0, 0.0, 1.0],
            },
            DatumAttachment::Face {
                face: cylinder_face(),
            },
            DatumAttachment::ThreePoints {
                points: [
                    at([0.0; 3]),
                    PointAnchor::Face {
                        face: cylinder_face(),
                    },
                    PointAnchor::Edge {
                        edge: rim(),
                        spot: EdgeSpot::Centre,
                    },
                ],
            },
            DatumAttachment::NormalToEdge {
                along: None,
                edge: straight_edge(),
                spot: EdgeSpot::Middle,
            },
            DatumAttachment::AlongEdge { edge: rim() },
            DatumAttachment::TwoPoints {
                points: [at([0.0; 3]), at([1.0; 3])],
            },
            DatumAttachment::PlaneIntersection {
                planes: [
                    PlaneAnchor::Base(BasePlane::XZ),
                    PlaneAnchor::Face {
                        face: cylinder_face(),
                    },
                ],
            },
            DatumAttachment::CurveCentre { edge: rim() },
            DatumAttachment::Midplane {
                planes: [
                    PlaneAnchor::Base(BasePlane::XY),
                    PlaneAnchor::Face {
                        face: cylinder_face(),
                    },
                ],
            },
            DatumAttachment::Inertia {
                centre: [1.0; 3],
                axes: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            },
        ];
        for attachment in modes {
            let datum = DatumFeature {
                shape: DatumShape::Line { length: 30.0 },
                attachment,
                offset: AttachmentOffset {
                    translation: [1.0, 2.0, 3.0],
                    rotation_deg: 15.0,
                    flip: true,
                    ..AttachmentOffset::default()
                },
            };
            assert_eq!(DatumFeature::from_json(&datum.to_json()).unwrap(), datum);
        }
    }

    #[test]
    fn a_datum_saved_before_the_modes_still_loads() {
        let old = serde_json::json!({
            "shape": { "Plane": { "size": 30.0 } },
            "attachment": { "FlatFace": { "point": [1.0, 2.0, 3.0], "normal": [0.0, 0.0, 1.0] } },
        });
        let datum = DatumFeature::from_json(&old).unwrap();
        assert!(close(datum.frame().origin, [1.0, 2.0, 3.0]));
        assert!(datum.probes().is_empty());
    }

    #[test]
    fn a_line_square_to_a_face_and_one_tangent_to_an_edge() {
        let face = line(DatumAttachment::FaceNormal {
            face: cylinder_face(),
        });
        assert!(close(face.origin, [0.0, 5.0, 3.0]));
        assert!(parallel(face.x_axis, [0.0, 1.0, 0.0]));
        let tangent = line(DatumAttachment::TangentToEdge {
            edge: rim(),
            spot: EdgeSpot::Picked,
        });
        assert!(close(tangent.origin, [5.0, 0.0, 12.0]));
        assert!(parallel(tangent.x_axis, [0.0, 1.0, 0.0]));
        orthonormal(&tangent);
    }

    #[test]
    fn planes_along_an_edge_stand_that_share_of_the_way() {
        let straight = plane(DatumAttachment::NormalToEdge {
            edge: straight_edge(),
            spot: EdgeSpot::Picked,
            along: Some(0.3),
        });
        assert!(
            close(straight.origin, [3.0, 0.0, 0.0]),
            "{:?}",
            straight.origin
        );
        assert!(parallel(straight.normal, [1.0, 0.0, 0.0]));
        // A half circle from (5, 0) round to (-5, 0) counter-clockwise: a
        // quarter of the way is a quarter turn round.
        let arc = EdgeAnchor {
            ends: Some([[5.0, 0.0, 12.0], [-5.0, 0.0, 12.0]]),
            ..rim()
        };
        let quarter = plane(DatumAttachment::NormalToEdge {
            edge: arc,
            spot: EdgeSpot::Picked,
            along: Some(0.5),
        });
        assert!(
            close(quarter.origin, [0.0, 5.0, 12.0]),
            "{:?}",
            quarter.origin
        );
        assert!(parallel(quarter.normal, [1.0, 0.0, 0.0]));
    }

    #[test]
    fn a_plane_through_a_line_and_a_point_and_where_lines_and_planes_meet() {
        let x_line = LineAnchor::Edge {
            edge: straight_edge(),
        };
        let through = plane(DatumAttachment::LineAndPoint {
            line: x_line,
            point: at([4.0, 3.0, 0.0]),
        });
        assert!(close(through.origin, [4.0, 0.0, 0.0]));
        assert!(parallel(through.normal, [0.0, 0.0, 1.0]));
        assert!(parallel(through.x_axis, [1.0, 0.0, 0.0]));
        let upright = LineAnchor::Datum {
            datum: FeatureId(uuid::Uuid::nil()),
            origin: [2.0, 3.0, -5.0],
            direction: [0.0, 0.0, 1.0],
        };
        let meet = plane(DatumAttachment::LineMeetsPlane {
            line: upright,
            plane: PlaneAnchor::Base(BasePlane::XY),
        });
        assert!(close(meet.origin, [2.0, 3.0, 0.0]));
        // Two skew lines: the x-axis, and one along y 4 above (2, 0).
        let cross_line = LineAnchor::Sketch {
            sketch: FeatureId(uuid::Uuid::nil()),
            element: uuid::Uuid::nil(),
            start: [2.0, -1.0, 4.0],
            end: [2.0, 1.0, 4.0],
        };
        let crossing = plane(DatumAttachment::TwoLines {
            lines: [x_line, cross_line],
        });
        assert!(
            close(crossing.origin, [2.0, 0.0, 2.0]),
            "{:?}",
            crossing.origin
        );
        assert!(parallel(crossing.normal, [0.0, 0.0, 1.0]));
        assert_eq!(
            DatumAttachment::TwoLines {
                lines: [x_line, x_line]
            }
            .problem(),
            Some("the two lines are parallel")
        );
    }

    #[test]
    fn a_datum_on_a_datum_and_on_another_body_follow_them() {
        let leader = DatumFeature {
            shape: DatumShape::CoordinateSystem { size: 10.0 },
            attachment: DatumAttachment::BasePlane(BasePlane::XY),
            offset: AttachmentOffset {
                translation: [0.0, 0.0, 7.0],
                ..AttachmentOffset::default()
            },
        };
        let leader_id = FeatureId(uuid::Uuid::new_v4());
        let follower = datum(
            DatumShape::Plane { size: 10.0 },
            DatumAttachment::OnDatum {
                datum: leader_id,
                plane: Some(BasePlane::YZ),
                frame: FrameAnchor {
                    origin: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                    x_axis: [1.0, 0.0, 0.0],
                },
            },
        );
        let values_of = |id: FeatureId| (id == leader_id).then(|| leader.to_json());
        let mut values = follower.to_json();
        assert!(derive(&mut values, None, &values_of, &|_, _| None));
        let frame = DatumFeature::from_json(&values).unwrap().frame();
        assert!(close(frame.origin, [0.0, 0.0, 7.0]));
        assert!(parallel(frame.normal, [1.0, 0.0, 0.0]), "its YZ plane");

        let mut doc = crate::Document::new("t");
        let own = doc.create_body(None);
        let other = doc.create_body(None);
        doc.set_body_placement(
            other,
            crate::BodyPlacement::new(glam::Quat::IDENTITY, glam::Vec3::new(0.0, 0.0, 20.0)),
        );
        let on_other = datum(
            DatumShape::Plane { size: 10.0 },
            DatumAttachment::OtherBody {
                body: other,
                plane: BasePlane::XY,
                frame: FrameAnchor {
                    origin: [0.0; 3],
                    normal: [0.0, 0.0, 1.0],
                    x_axis: [1.0, 0.0, 0.0],
                },
            },
        );
        let mut values = on_other.to_json();
        assert!(derive(&mut values, None, &|_| None, &|b, p| {
            body_plane_in(&doc, own, b, p)
        }));
        let frame = DatumFeature::from_json(&values).unwrap().frame();
        assert!(close(frame.origin, [0.0, 0.0, 20.0]), "{:?}", frame.origin);
    }
}
