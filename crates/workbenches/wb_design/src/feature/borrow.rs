//! Geometry borrowed from another body or file, kept as it was frozen
//! or followed as it changes.

use super::*;

/// What a body borrows from another: one of its sketches, or faces and
/// edges of its solid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Walk)]
pub enum BorrowSource {
    /// A sketch of another body: its profile, where that body sits.
    Sketch(FeatureId),
    /// Faces and edges of another body's solid, each where it was picked,
    /// in that body's own frame, and re-found on the solid as it is built.
    Solid {
        #[walk(setting)]
        body: BodyId,
        #[serde(default)]
        faces: Vec<FacePick>,
        #[serde(default)]
        edges: Vec<EdgePick>,
    },
}

/// Borrowed geometry as it was when it was frozen, in the borrowing body's
/// frame: what the body keeps once it stops following the source.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, Describe)]
pub struct FrozenBorrow {
    /// The sketch, with its plane in the borrowing body's frame, as a
    /// sketch feature's data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sketch: Option<serde_json::Value>,
    #[serde(default)]
    pub faces: Vec<FrozenFace>,
    #[serde(default)]
    pub edges: Vec<FrozenEdge>,
}

/// A borrowed face as it was frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Describe)]
pub struct FrozenFace {
    /// Where it was picked, in the borrowing body's frame.
    pub pick: FacePick,
    /// The face alone, as a native-format snapshot in its own body's frame.
    pub shape: String,
    /// What moves `shape` into the borrowing body's frame, a rigid
    /// row-major 4×4 matrix.
    pub transform: [[f64; 4]; 4],
    /// Its surface, in the borrowing body's frame, when the mesh knew it.
    #[serde(default)]
    pub surface: Option<kernel_api::FaceSurface>,
    /// Its outline, pairs of points in the borrowing body's frame.
    #[serde(default)]
    pub outline: Vec<[f32; 3]>,
}

/// A borrowed edge as it was frozen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Describe)]
pub struct FrozenEdge {
    /// A point of it and its direction there, in the borrowing body's
    /// frame.
    pub pick: EdgePick,
    /// Its outline, pairs of points in the borrowing body's frame.
    #[serde(default)]
    pub outline: Vec<[f32; 3]>,
}

/// One face or edge of a borrow: the borrow feature and its place in the
/// borrow's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Walk)]
pub struct BorrowedRef {
    pub borrow: FeatureId,
    #[walk(setting)]
    pub index: usize,
}

/// How a borrow lends what it borrows.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Describe)]
#[serde(default)]
pub struct BorrowOptions {
    /// Moved and turned from where the bodies put it: along and about the
    /// borrowing body's own axes (the turn about z, then the tilt about x
    /// and y).
    pub offset: core_document::AttachmentOffset,
    /// Borrowed edges that close into a loop in one plane lend the face
    /// they bound, as a profile features take.
    pub fill: bool,
    /// All of the other body's solid, drawn as reference: its edges lent,
    /// its faces to stop on.
    pub whole: bool,
}

impl BorrowOptions {
    pub fn is_plain(&self) -> bool {
        *self == Self::default()
    }

    /// What the offset does to the borrowed geometry, in the borrowing
    /// body's frame.
    pub fn placement(&self) -> core_document::BodyPlacement {
        let o = &self.offset;
        let turn = glam::Quat::from_rotation_z(o.rotation_deg.to_radians())
            * glam::Quat::from_rotation_x(o.tilt[0].to_radians())
            * glam::Quat::from_rotation_y(o.tilt[1].to_radians())
            * if o.flip {
                glam::Quat::from_rotation_x(std::f32::consts::PI)
            } else {
                glam::Quat::IDENTITY
            };
        core_document::BodyPlacement::new(turn, glam::Vec3::from_array(o.translation))
    }
}
