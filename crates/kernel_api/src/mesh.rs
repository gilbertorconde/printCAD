//! Meshes the kernel draws: tessellation settings and triangle meshes with
//! their faces' names and surfaces.

use serde::{Deserialize, Serialize};

use crate::*;

/// How linear (chord) deflection is chosen for kernel meshing.
///
/// The default is **bbox-scaled** deflection: absolute chord height is derived
/// from the sum of the shape's bounding-box extents and a dimensionless
/// multiplier ([`TessellationSettings::mesh_deviation`], typically `0.2`).
/// This produces visually consistent tessellation across small and large
/// parts without per-model tuning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LinearDeflectionMode {
    /// Bounding-box scaled: linear deflection = `(dx + dy + dz) / 300 ×`
    /// [`TessellationSettings::mesh_deviation`].
    #[default]
    BboxScaled,
    /// Fixed absolute linear deflection in model units (millimetres for STEP geometry).
    AbsoluteMm,
}

/// Parameters controlling tessellation quality for viewport rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TessellationSettings {
    #[serde(default)]
    pub linear_deflection_mode: LinearDeflectionMode,
    /// Dimensionless multiplier applied to the bbox-derived chord height when
    /// [`Self::linear_deflection_mode`] is [`LinearDeflectionMode::BboxScaled`].
    /// Typical range `0.05..=1.0`; smaller values yield finer triangulation.
    #[serde(default = "default_mesh_deviation")]
    pub mesh_deviation: f32,
    /// Absolute chord height in model units when using [`LinearDeflectionMode::AbsoluteMm`].
    pub chord_tolerance: f32,
    pub angular_tolerance_deg: f32,
    /// When true, the kernel collapses vertices that share a position across
    /// multiple faces *and* whose face normals are within
    /// [`Self::weld_angle_threshold_deg`] of each other. This typically
    /// shrinks vertex counts by 4–6× on dense CAD models without flattening
    /// genuine sharp edges, since dissimilar face normals stay separate.
    #[serde(default = "default_weld_cross_face")]
    pub weld_cross_face: bool,
    /// Maximum angle between two face normals at a shared position for them
    /// to be merged into a single welded vertex. Above this angle the kernel
    /// keeps them separate so hard CAD edges stay crisp under shading.
    /// Defaults to 30°.
    #[serde(default = "default_weld_angle_threshold_deg")]
    pub weld_angle_threshold_deg: f32,
    /// When true, import keeps each body's shape snapshot in `brep_blob`
    /// beside its mesh. Meshing happens during the import either way;
    /// serializing the snapshots is most of the work of reading a large
    /// assembly, and turning this off skips it. The STEP asset remains
    /// available in the document regardless.
    #[serde(default = "default_persist_brep_snapshot")]
    pub persist_brep_snapshot: bool,
    /// When true, compute mesh outline / edge segments for the viewport (face
    /// boundaries). Skipping saves CPU on huge imports when outlines are not needed.
    #[serde(default = "default_generate_boundary_edges")]
    pub generate_boundary_edges: bool,
}

fn default_weld_cross_face() -> bool {
    false
}

fn default_weld_angle_threshold_deg() -> f32 {
    30.0
}

fn default_mesh_deviation() -> f32 {
    0.2
}

fn default_persist_brep_snapshot() -> bool {
    true
}

fn default_generate_boundary_edges() -> bool {
    true
}

impl Default for TessellationSettings {
    fn default() -> Self {
        Self {
            linear_deflection_mode: LinearDeflectionMode::default(),
            mesh_deviation: default_mesh_deviation(),
            chord_tolerance: 0.1,
            angular_tolerance_deg: 28.65,
            weld_cross_face: default_weld_cross_face(),
            weld_angle_threshold_deg: default_weld_angle_threshold_deg(),
            persist_brep_snapshot: default_persist_brep_snapshot(),
            generate_boundary_edges: default_generate_boundary_edges(),
        }
    }
}

/// Triangular mesh generated from kernel bodies for viewports and export.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TriMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Optional line-list of feature edges that should be drawn as outlines on
    /// top of the shaded surface (e.g. face boundaries from a STEP import).
    /// Stored as flat pairs of indices into [`Self::positions`]; the slice
    /// length is therefore always a multiple of two. Empty when the source has
    /// no edge information available.
    #[serde(default)]
    pub edges: Vec<u32>,
    /// Optional per-vertex linear RGB albedo in 0..1 (same length as [`Self::positions`] when set).
    /// Empty means the renderer uses the body tint from push constants only.
    #[serde(default)]
    pub colors: Vec<[f32; 3]>,
    /// Which kernel face each triangle was cut from: one entry per triangle
    /// (`indices.len() / 3`), an index into the body's faces in the kernel's
    /// own order. A curved face is many triangles with many normals, and
    /// this is what lets a click on one of them select the whole face.
    /// Empty when the source has no faces (a sketch, a datum, a mesh saved
    /// before faces were recorded), and a consumer falls back to geometry.
    #[serde(default)]
    pub faces: Vec<u32>,
    /// Which kernel edge each outline segment belongs to: one entry per
    /// pair in [`Self::edges`], an index into the body's edges in the
    /// kernel's own order. A curved edge is many segments, and this is what
    /// lets a click on one of them select the whole edge. Empty when the
    /// outline came from triangle boundaries rather than kernel edges.
    #[serde(default)]
    pub edge_ids: Vec<u32>,
    /// The exact surface of each kernel face, indexed like [`Self::faces`]:
    /// what a picked face is, beyond the triangles drawn for it. Empty when
    /// the source has no faces.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_surfaces: Vec<FaceSurface>,
    /// The name of each kernel face, indexed like [`Self::faces`]: what a
    /// pick keeps so a rebuild finds the face again. Zero where a face has
    /// none; empty when the source names none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_names: Vec<TopoName>,
    /// The tag of the op that made each kernel face, indexed like
    /// [`Self::faces`]: the name of the feature a body's chain built it
    /// under (`naming::name_of_id` of its id), kept by a face a later op
    /// splits or trims. Zero where unknown; empty when the source names
    /// none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face_makers: Vec<TopoName>,
    /// The names of the two faces each kernel edge runs between, indexed
    /// like [`Self::edge_ids`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edge_faces: Vec<[TopoName; 2]>,
}

/// The kind and placement of a kernel face's surface, in the mesh's frame.
#[cfg_attr(not(feature = "exhaustive"), non_exhaustive)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum FaceSurface {
    /// A plane through `origin`, with the face's outward `normal`.
    Plane { origin: [f32; 3], normal: [f32; 3] },
    /// A circular cylinder about the line through `origin` along `axis`.
    Cylinder {
        origin: [f32; 3],
        axis: [f32; 3],
        radius: f32,
    },
    /// A circular cone about the line through `apex` along `axis`, its
    /// wall `half_angle` (radians, positive) off the axis.
    Cone {
        apex: [f32; 3],
        axis: [f32; 3],
        #[serde(default)]
        half_angle: f32,
    },
    /// A sphere.
    Sphere { center: [f32; 3], radius: f32 },
    /// A torus about the line through `center` along `axis`: its tube's
    /// centre `major_radius` from the axis, the tube `minor_radius` round.
    Torus {
        center: [f32; 3],
        axis: [f32; 3],
        #[serde(default)]
        major_radius: f32,
        #[serde(default)]
        minor_radius: f32,
    },
    /// Any other surface: a spline, a sweep.
    #[default]
    Other,
}

impl FaceSurface {
    /// The axis a turned face turns about, as a point on it and its
    /// direction: a cylinder's, a cone's or a torus's.
    pub fn axis(&self) -> Option<([f32; 3], [f32; 3])> {
        match *self {
            FaceSurface::Cylinder { origin, axis, .. } => Some((origin, axis)),
            FaceSurface::Cone { apex, axis, .. } => Some((apex, axis)),
            FaceSurface::Torus { center, axis, .. } => Some((center, axis)),
            _ => None,
        }
    }

    /// The same surface moved: `point` maps points, `direction` maps
    /// directions (a rigid motion, so lengths stay).
    pub fn moved(
        &self,
        point: impl Fn([f32; 3]) -> [f32; 3],
        direction: impl Fn([f32; 3]) -> [f32; 3],
    ) -> Self {
        match *self {
            FaceSurface::Plane { origin, normal } => FaceSurface::Plane {
                origin: point(origin),
                normal: direction(normal),
            },
            FaceSurface::Cylinder {
                origin,
                axis,
                radius,
            } => FaceSurface::Cylinder {
                origin: point(origin),
                axis: direction(axis),
                radius,
            },
            FaceSurface::Cone {
                apex,
                axis,
                half_angle,
            } => FaceSurface::Cone {
                apex: point(apex),
                axis: direction(axis),
                half_angle,
            },
            FaceSurface::Sphere { center, radius } => FaceSurface::Sphere {
                center: point(center),
                radius,
            },
            FaceSurface::Torus {
                center,
                axis,
                major_radius,
                minor_radius,
            } => FaceSurface::Torus {
                center: point(center),
                axis: direction(axis),
                major_radius,
                minor_radius,
            },
            FaceSurface::Other => FaceSurface::Other,
        }
    }
}

impl TriMesh {
    /// Compute axis-aligned bounding box `(min, max)` of the mesh, if non-empty.
    pub fn bounds(&self) -> Option<([f32; 3], [f32; 3])> {
        self.aabb().map(Into::into)
    }

    /// The box around the mesh's points; `None` for a mesh with none.
    pub fn aabb(&self) -> Option<crate::Aabb> {
        crate::Aabb::around(self.positions.iter().copied())
    }
}
