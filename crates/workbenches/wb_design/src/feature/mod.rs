//! Design feature payloads stored in the document feature tree.

use core_document::describe::{Describe, Shape};
use core_document::walk::Walk;
use core_document::{BodyId, FeatureId, WorkbenchFeature, WorkbenchId};
use serde::{Deserialize, Serialize};

use crate::hole_tables::{ThreadSize, ThreadStandard};

pub use borrow::{BorrowOptions, BorrowSource, BorrowedRef, FrozenBorrow, FrozenEdge, FrozenFace};
use dressup::half;
pub use dressup::{ChamferMode, FilletMode, FilletProfile, RadiusPoint};
pub use extrude::{
    BaseAxis, ExtrudeDirection, ExtrudeExtras, ExtrudeMode, RevolveAxis, RevolveMode,
};
pub use holes::{
    DrillPoint, HoleCut, HoleFit, HoleShape, HoleStart, NutSide, NutTrap, ThreadLength, ThreadSpec,
};
use holes::{
    default_thread_depth, default_thread_length, default_thread_pitch, thread_or_custom, z_axis,
};
pub(crate) use holes::{hole_shape_or_name, nut_trap_or_flag};
pub use labels::{primitive_icon, primitive_preset};
pub use refs::{Attached, EdgePick, EdgeSel, FacePick, PlaneTarget, PullRef};
use sweeps::deserialize_pipe_orientation;
pub use sweeps::{HelixMode, LoftSection, PipeCorner, PipeOrientation};
pub use transforms::{
    FillArea, FillKind, MirrorPlane, PathSpacing, PatternAxis, PatternPath, ScaleCentre,
    SketchAxis, TransformStep,
};
use transforms::{default_fill_rows, unit_factors};

/// `$then` over the refine switch and boolean tolerance of a feature that
/// fuses or cuts, `None` for the rest: the one list of those features.
macro_rules! fuse_settings {
    ($feature:expr, |$refine:ident, $tolerance:ident| $then:expr) => {
        match $feature {
            DesignFeature::Pad {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Pocket {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Revolution {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Groove {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Loft {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Pipe {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Helix {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Primitive {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Hole {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Mirrored {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::LinearPattern {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::PolarPattern {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::CircularPattern {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::PointPattern {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::PathPattern {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::FillPattern {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::MultiTransform {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::BodyBoolean {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Thread {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            }
            | DesignFeature::Rib {
                refine: $refine,
                boolean_tolerance: $tolerance,
                ..
            } => Some($then),
            _ => None,
        }
    };
}

mod access;
mod borrow;
mod dressup;
mod extrude;
mod holes;
mod labels;
mod refs;
mod sweeps;
mod transforms;

/// A solid-modeling feature in a body's linear history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Walk)]
pub enum DesignFeature {
    /// Start the body from its base solid: the shape an import, a mesh
    /// conversion or a repair made of it, kept by the document.
    Base {},
    /// Start the body from a copy of another body's solid, as that body
    /// is built now.
    Clone {
        #[walk(setting)]
        source: BodyId,
    },
    /// One piece of a split body: the solid before it (a Clone of the body
    /// split) cut along a plane, or by a sketch's profile carried through
    /// the whole solid, keeping one side.
    Split {
        /// The plane it cuts along, when no sketch cuts.
        #[serde(default)]
        plane: MirrorPlane,
        /// mm the plane moves along its normal.
        #[serde(default)]
        #[walk(length("offset", "Offset"))]
        offset: f32,
        /// A sketch whose profile cuts, through the whole solid along its
        /// normal, in place of the plane.
        #[serde(default)]
        sketch: Option<FeatureId>,
        /// Keep the other side: behind the plane, or outside the sketch's
        /// profile.
        #[serde(default)]
        #[walk(setting)]
        other_side: bool,
        /// The split it is a piece of: the pieces of one split share it,
        /// and a change to one piece's cut is made to them all.
        #[serde(default)]
        #[walk(setting)]
        split: u64,
    },
    /// Keep one piece of a solid of several: the one nearest `near`, in
    /// the body's own frame. A body separated from another starts with a
    /// Clone of it and this.
    Piece {
        #[walk(setting)]
        near: [f32; 3],
    },
    /// Extrude the sketch profile (or a flat face of the solid), adding
    /// material.
    Pad {
        /// The sketch whose profile it extrudes; `None` when the profile is
        /// `profile_face`.
        #[serde(default)]
        sketch: Option<FeatureId>,
        #[walk(length("length", "Length"))]
        length: f32,
        /// Extrude along -normal instead of +normal.
        #[walk(setting)]
        reversed: bool,
        /// Extrude half the length to each side of the sketch plane.
        #[serde(default)]
        #[walk(setting)]
        symmetric: bool,
        #[serde(default)]
        mode: ExtrudeMode,
        #[serde(default)]
        #[walk(length("length2", "Second length"))]
        length2: f32,
        #[serde(default)]
        #[walk(angle("taper", "Taper angle"))]
        taper_deg: f32,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        #[serde(default)]
        #[walk(length("offset", "Offset"))]
        up_to_offset: f32,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// A flat face of the solid extruded in place of a sketch: its
        /// boundaries are the profile and its outward normal the sketch's.
        #[serde(default)]
        profile_face: Option<FacePick>,
        /// A flat face another body lends this one, extruded in place of a
        /// sketch as `profile_face` is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_borrowed: Option<BorrowedRef>,
        #[serde(default)]
        direction: ExtrudeDirection,
        /// The faces an up-to-shape extrusion stops on.
        #[serde(default)]
        up_to_shape: Vec<FacePick>,
        /// How the second side ends, when the pad runs both ways.
        #[serde(default)]
        mode2: Option<ExtrudeMode>,
        #[serde(default)]
        up_to_face2: Option<FacePick>,
        #[serde(default)]
        #[walk(length("offset2", "Second offset"))]
        up_to_offset2: f32,
        #[serde(default)]
        up_to_shape2: Vec<FacePick>,
        /// The sketch's regions it takes, by name (`sketch_model::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        #[walk(setting)]
        regions: Vec<String>,
        #[serde(default)]
        extras: ExtrudeExtras,
    },
    /// Extrude the sketch profile (or a flat face of the solid) and
    /// subtract it (cuts against the sketch normal by default: a face
    /// sketch's normal points out of the material).
    Pocket {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        #[walk(setting)]
        keep_inside: bool,
        /// The sketch whose profile it cuts; `None` when the profile is
        /// `profile_face`.
        #[serde(default)]
        sketch: Option<FeatureId>,
        #[walk(length("depth", "Depth"))]
        depth: f32,
        #[walk(setting)]
        reversed: bool,
        /// Cut half the depth to each side of the sketch plane.
        #[serde(default)]
        #[walk(setting)]
        symmetric: bool,
        /// The same setting as `mode` ThroughAll, kept in step with it by
        /// commands and the panel; a file with only this set cuts through
        /// all.
        #[serde(default)]
        #[walk(setting)]
        through_all: bool,
        #[serde(default)]
        mode: ExtrudeMode,
        #[serde(default)]
        #[walk(length("depth2", "Second depth"))]
        depth2: f32,
        #[serde(default)]
        #[walk(angle("taper", "Taper angle"))]
        taper_deg: f32,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        #[serde(default)]
        #[walk(length("offset", "Offset"))]
        up_to_offset: f32,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// A flat face of the solid cut in place of a sketch's profile.
        #[serde(default)]
        profile_face: Option<FacePick>,
        /// A flat face another body lends this one, cut in place of a
        /// sketch's profile.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        profile_borrowed: Option<BorrowedRef>,
        /// A direction set here is the way the cut runs; along the normal
        /// it runs against it.
        #[serde(default)]
        direction: ExtrudeDirection,
        #[serde(default)]
        up_to_shape: Vec<FacePick>,
        #[serde(default)]
        mode2: Option<ExtrudeMode>,
        #[serde(default)]
        up_to_face2: Option<FacePick>,
        #[serde(default)]
        #[walk(length("offset2", "Second offset"))]
        up_to_offset2: f32,
        #[serde(default)]
        up_to_shape2: Vec<FacePick>,
        /// The sketch's regions it takes, by name (`sketch_model::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        #[walk(setting)]
        regions: Vec<String>,
        #[serde(default)]
        extras: ExtrudeExtras,
    },
    /// Revolve the sketch profile about an in-plane axis, adding material.
    Revolution {
        sketch: FeatureId,
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        /// An axis that does not run along the sketch plane is taken as its
        /// shadow on it, rather than refused.
        #[serde(default)]
        #[walk(setting)]
        project_axis: bool,
        #[serde(default)]
        #[walk(setting)]
        reversed: bool,
        #[serde(default)]
        #[walk(setting)]
        midplane: bool,
        /// Degrees the profile is turned about the axis, the way the
        /// turn runs, before it starts.
        #[serde(default)]
        #[walk(angle("start_angle", "Start angle"))]
        start_angle_deg: f32,
        #[serde(default)]
        #[walk(angle("angle2", "Second angle"))]
        second_angle_deg: Option<f32>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        #[serde(default)]
        mode: RevolveMode,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        /// The sketch's regions it takes, by name (`sketch_model::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        #[walk(setting)]
        regions: Vec<String>,
    },
    /// Revolve the sketch profile and subtract it.
    Groove {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        #[walk(setting)]
        keep_inside: bool,
        sketch: FeatureId,
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        /// An axis that does not run along the sketch plane is taken as its
        /// shadow on it, rather than refused.
        #[serde(default)]
        #[walk(setting)]
        project_axis: bool,
        #[serde(default)]
        #[walk(setting)]
        reversed: bool,
        #[serde(default)]
        #[walk(setting)]
        midplane: bool,
        /// Degrees the profile is turned about the axis, the way the
        /// turn runs, before it starts.
        #[serde(default)]
        #[walk(angle("start_angle", "Start angle"))]
        start_angle_deg: f32,
        #[serde(default)]
        #[walk(angle("angle2", "Second angle"))]
        second_angle_deg: Option<f32>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        #[serde(default)]
        mode: RevolveMode,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        /// The sketch's regions it takes, by name (`sketch_model::profile::regions`);
        /// empty takes what the sketch's profile is.
        #[serde(default)]
        #[walk(setting)]
        regions: Vec<String>,
    },
    /// Skin through two or more sections: sketches, datum points (or
    /// sketches of a single point) at either end, and flat faces of the
    /// solid.
    Loft {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        #[walk(setting)]
        keep_inside: bool,
        sections: Vec<LoftSection>,
        #[walk(setting)]
        ruled: bool,
        #[walk(setting)]
        closed: bool,
        #[walk(setting)]
        subtractive: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
    },
    /// Sweep a profile sketch along a spine sketch's path.
    Pipe {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        #[walk(setting)]
        keep_inside: bool,
        profile: FeatureId,
        spine: FeatureId,
        /// How the section turns down the path; files that hold only the
        /// `frenet` flag read it as the frame it named.
        #[serde(
            default,
            alias = "frenet",
            deserialize_with = "deserialize_pipe_orientation"
        )]
        orientation: PipeOrientation,
        /// How the section turns the path's sharp corners.
        #[serde(default)]
        #[walk(setting)]
        corner: PipeCorner,
        /// Further section sketches the pipe passes through down its path,
        /// in order, after the profile; the last may be a datum point or a
        /// sketch of a single point, where the pipe closes to it.
        #[serde(default)]
        sections: Vec<FeatureId>,
        #[walk(setting)]
        subtractive: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// A flat face of the solid as the profile, in place of `profile`.
        #[serde(default)]
        profile_face: Option<FacePick>,
        /// Edges of the solid, picked, as the path in place of `spine`.
        #[serde(default)]
        path_edges: Vec<EdgePick>,
        /// Edges another body lends, as the path in place of `spine`.
        #[serde(default)]
        path_borrowed: Vec<BorrowedRef>,
        /// The curves of `spine`, by id, that make the path; empty takes
        /// every curve it draws.
        #[serde(default)]
        #[walk(setting)]
        path_curves: Vec<uuid::Uuid>,
    },
    /// Sweep the sketch profile along a helix about an in-plane axis.
    Helix {
        sketch: FeatureId,
        axis: RevolveAxis,
        /// An axis that does not run along the sketch plane is taken as its
        /// shadow on it, rather than refused.
        #[serde(default)]
        #[walk(setting)]
        project_axis: bool,
        #[walk(setting)]
        mode: HelixMode,
        #[walk(length("pitch", "Pitch"))]
        pitch: f32,
        #[walk(length("height", "Height"))]
        height: f32,
        #[walk(number("turns", "Turns"))]
        turns: f32,
        #[walk(setting)]
        left_handed: bool,
        #[walk(angle("cone_angle", "Cone angle"))]
        cone_angle_deg: f32,
        #[walk(setting)]
        reversed: bool,
        #[walk(setting)]
        subtractive: bool,
        /// How far every point moves away from the axis per turn, in
        /// [`HelixMode::HeightTurnsGrowth`].
        #[serde(default)]
        #[walk(length("growth", "Growth per turn"))]
        growth: f32,
        /// A subtractive helix keeps what it shares with the body instead
        /// of cutting it away.
        #[serde(default)]
        #[walk(setting)]
        keep_inside: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
    },
    /// Parametric primitive fused into (or cut from) the body.
    Primitive {
        /// A cutting feature keeps only what it shares with the body
        /// instead of cutting it away.
        #[serde(default)]
        #[walk(setting)]
        keep_inside: bool,
        #[walk(with = crate::params::primitive_kind)]
        kind: kernel_api::PrimitiveKind,
        #[walk(with = crate::params::primitive_placement)]
        placement: kernel_api::Placement,
        #[walk(setting)]
        subtractive: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Placed as a datum is, by a mode on what was picked, in place of
        /// `placement`, and following what it stands on.
        #[serde(default)]
        attached: Option<Box<Attached>>,
    },
    /// Standards-aware drilled cuts at every circle center of a sketch.
    Hole {
        sketch: FeatureId,
        #[walk(length("diameter", "Diameter"))]
        diameter: f32,
        #[walk(length("depth", "Depth"))]
        depth: f32,
        #[walk(setting)]
        through_all: bool,
        #[serde(default)]
        cut: HoleCut,
        /// The thread the hole is sized from when it is standards-driven;
        /// the diameter then derives from the thread and the fit.
        #[serde(default, alias = "metric_index")]
        #[walk(setting)]
        thread: Option<ThreadSpec>,
        #[serde(default)]
        #[walk(setting)]
        threaded: bool,
        /// A threaded hole's thread cut into its wall, not only its tap
        /// drill: for printing threads rather than tapping them.
        #[serde(default)]
        #[walk(setting)]
        modeled_thread: bool,
        /// How far down a modeled thread runs, mm.
        #[serde(default)]
        #[walk(length("thread_depth", "Thread depth"))]
        thread_depth: f32,
        /// A threaded hole drilled to its tap size whose thread is shown,
        /// not cut: a stripe down the bore and its designation in the
        /// tree, the parts list and STEP's callouts; no export carries
        /// the thread's shape.
        #[serde(default)]
        #[walk(setting)]
        cosmetic_thread: bool,
        #[serde(default)]
        #[walk(setting)]
        fit: HoleFit,
        /// A clearance diameter of one's own, in place of the fit's, mm.
        #[serde(default)]
        #[walk(setting)]
        clearance: Option<f32>,
        /// How long a modeled thread runs.
        #[serde(default)]
        #[walk(setting)]
        thread_length: ThreadLength,
        /// The bottom of a blind hole.
        #[serde(default)]
        drill_point: DrillPoint,
        /// The drill point lies within the depth rather than below it.
        #[serde(default)]
        #[walk(setting)]
        point_in_depth: bool,
        /// The wall leans in toward the bottom by this angle, degrees;
        /// a tapered thread standard's taper stands in for it.
        #[serde(default)]
        #[walk(angle("taper", "Taper angle"))]
        taper_deg: f32,
        /// A hexagonal pocket for a captive nut at one end.
        #[serde(default, deserialize_with = "nut_trap_or_flag")]
        nut_trap: Option<NutTrap>,
        /// Where it starts drilling, when not on its sketch's plane.
        #[serde(default)]
        start: HoleStart,
        /// The bore's outline across its axis: round, or a teardrop or
        /// flat top for a hole printed lying down.
        #[serde(default, deserialize_with = "hole_shape_or_name")]
        shape: HoleShape,
        /// A sacrificial layer this many mm thick left across the hole at
        /// the floor of its cut, so the cut's floor prints as a bridge;
        /// drilled out after printing. `None` leaves the hole open.
        #[serde(default)]
        #[walk(length("bridge", "Bridge layer"))]
        bridge: Option<f32>,
        #[serde(default)]
        #[walk(setting)]
        reversed: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
    },
    /// A thread cut into a cylindrical face of the solid: a shaft's or a
    /// boss's outside, a bore's inside, whichever way the face faces.
    /// Cosmetic, it is shown and called out, not cut.
    Thread {
        /// The cylindrical face it goes on; the thread starts at the end
        /// of it nearer where it was picked.
        face: FacePick,
        /// The standard size it is cut to: its pitch, and its depth from
        /// the major to the minor diameter. `None` (read from "Custom"
        /// too) cuts `pitch` and `depth`.
        #[serde(default, deserialize_with = "thread_or_custom")]
        #[walk(setting)]
        thread: Option<ThreadSpec>,
        /// Its own pitch, mm, with no standard.
        #[serde(default = "default_thread_pitch")]
        #[walk(length("pitch", "Pitch"))]
        pitch: f32,
        /// Its own depth into the material from the face, mm, with no
        /// standard.
        #[serde(default = "default_thread_depth")]
        #[walk(length("depth", "Depth"))]
        depth: f32,
        /// How far along the face from its starting end, mm, unless it
        /// runs the face's whole length.
        #[serde(default = "default_thread_length")]
        #[walk(length("length", "Length"))]
        length: f32,
        #[serde(default = "default_true")]
        #[walk(setting)]
        full_length: bool,
        /// Starts at the end of the face farther from where it was picked.
        #[serde(default)]
        #[walk(setting)]
        from_far_end: bool,
        /// Turns left; a standard size's own left hand counts too.
        #[serde(default)]
        #[walk(setting)]
        left_handed: bool,
        /// A 45° chamfer at the starting end, down to the thread's root.
        #[serde(default)]
        #[walk(setting)]
        chamfer: bool,
        /// The groove grows shallower over its last two pitches rather
        /// than ending square.
        #[serde(default)]
        #[walk(setting)]
        run_out: bool,
        /// Shown on the face and called out, not cut.
        #[serde(default)]
        #[walk(setting)]
        cosmetic: bool,
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
    },
    /// A web grown from an open line or chain of a sketch's lines and
    /// arcs, `thickness` thick, until it meets the solid: across the
    /// sketch's plane, its ends carried on along the chain, or along the
    /// plane's normal.
    Rib {
        sketch: FeatureId,
        #[walk(length("thickness", "Thickness"))]
        thickness: f32,
        /// Where the thickness lies about the chain.
        #[serde(default)]
        #[walk(setting)]
        side: kernel_api::RibSide,
        /// The web runs along the sketch plane's normal, not across it.
        #[serde(default)]
        #[walk(setting)]
        along_normal: bool,
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
    },
    Fillet {
        #[walk(length("radius", "Radius"))]
        radius: f32,
        #[serde(default)]
        edges: EdgeSel,
        /// Every edge meeting a selected one tangentially is taken too, on
        /// along the chain.
        #[serde(default)]
        #[walk(setting)]
        follow_tangent: bool,
        /// Merge the faces the round or bevel leaves split on one surface.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// How the rounds close a vertex where they meet.
        #[serde(default)]
        #[walk(setting)]
        corner: kernel_api::FilletCorner,
        /// One radius; one size each face takes as far as it goes (Auto);
        /// `radius` on one face and `radius2` on the other; or a radius
        /// running from `radius` to `radius2` along the edges (Variable).
        #[serde(default)]
        #[walk(setting)]
        mode: FilletMode,
        /// With two distances, the distance on the face across the edge;
        /// variable, the radius at the run's end.
        #[serde(default)]
        #[walk(length("radius2", "Second distance"))]
        radius2: f32,
        /// With two distances, measure `radius` on the edge's other face;
        /// variable, start the run at its other end.
        #[serde(default)]
        #[walk(setting)]
        flip: bool,
        /// Variable, the radii between the run's ends.
        #[serde(default)]
        #[walk(steps("point", "Point"))]
        points: Vec<RadiusPoint>,
        /// With two distances, the round's section.
        #[serde(default)]
        #[walk(setting)]
        profile: FilletProfile,
        /// A conic section's fullness, strictly between 0 and 1: near 0 a
        /// bevel, 0.5 a parabola, near 1 the sharp edge.
        #[serde(default = "half")]
        #[walk(number("rho", "Rho"))]
        rho: f32,
    },
    Chamfer {
        #[walk(length("size", "Size"))]
        size: f32,
        #[serde(default)]
        #[walk(setting)]
        mode: ChamferMode,
        #[serde(default)]
        #[walk(length("size2", "Second size"))]
        size2: f32,
        #[serde(default)]
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        #[serde(default)]
        #[walk(setting)]
        flip: bool,
        #[serde(default)]
        edges: EdgeSel,
        /// Every edge meeting a selected one tangentially is taken too, on
        /// along the chain.
        #[serde(default)]
        #[walk(setting)]
        follow_tangent: bool,
        /// Merge the faces the round or bevel leaves split on one surface.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// How the bevels close a vertex where three or more meet.
        #[serde(default)]
        #[walk(setting)]
        corner: kernel_api::ChamferCorner,
    },
    Draft {
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        neutral: FacePick,
        faces: Vec<FacePick>,
        #[serde(default)]
        #[walk(setting)]
        reversed: bool,
        /// One of the body's planes or a datum plane as the neutral plane,
        /// in place of `neutral`.
        #[serde(default)]
        neutral_plane: Option<PlaneTarget>,
        /// The way the draft pulls, in place of the neutral plane's normal.
        #[serde(default)]
        pull: Option<PullRef>,
    },
    /// Delete faces of the solid and close the openings from their
    /// neighbours: a hole, a boss or a round taken away.
    DeleteFaces { faces: Vec<FacePick> },
    /// Push or pull faces of the solid along their outward normals, the
    /// faces around them following: a wall thicker, a bore wider.
    OffsetFaces {
        faces: Vec<FacePick>,
        /// mm along the outward normals; negative moves them in.
        #[walk(length("distance", "Distance"))]
        distance: f32,
    },
    /// Grow the whole solid by `distance` along its faces' outward
    /// normals, or shrink it with a negative one: a cavity with clearance,
    /// a socket with play.
    OffsetSolid {
        /// mm along the outward normals; negative shrinks.
        #[walk(length("distance", "Distance"))]
        distance: f32,
    },
    /// Scale the whole solid about a point: by one factor, or by one per
    /// axis of the body's frame.
    Scale {
        #[walk(number("factor", "Factor"))]
        factor: f32,
        /// Scale by `factors`, one per axis, in place of `factor`.
        #[serde(default)]
        #[walk(setting)]
        per_axis: bool,
        #[serde(default = "unit_factors")]
        #[walk(axes("factor_", "Factor ", number))]
        factors: [f32; 3],
        #[serde(default)]
        about: ScaleCentre,
    },
    /// Move faces of the solid, the faces around them following: shifted
    /// by `translation`, and turned `angle_deg` about the axis through
    /// `axis_point` along `axis_dir`.
    MoveFaces {
        faces: Vec<FacePick>,
        #[walk(axes("", "", length))]
        translation: [f32; 3],
        #[serde(default)]
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        #[serde(default)]
        #[walk(setting)]
        axis_point: [f32; 3],
        #[serde(default = "z_axis")]
        #[walk(setting)]
        axis_dir: [f32; 3],
    },
    /// A sketch carried onto faces of the solid along its plane's normal,
    /// or with `wrap` wrapped onto the curved surface they lie on: the
    /// faces split along its curves, and as `mode` says the regions it
    /// encloses raised out of them or sunk into them by `height`.
    Project {
        sketch: FeatureId,
        faces: Vec<FacePick>,
        #[serde(default)]
        #[walk(setting)]
        mode: kernel_api::ProjectMode,
        /// mm the regions stand out or sink in; unused for split lines.
        #[serde(default = "default_project_height")]
        #[walk(length("height", "Height"))]
        height: f32,
        /// Wrap the sketch onto the faces' curved surface rather than carry
        /// it straight along its normal.
        #[serde(default)]
        #[walk(setting)]
        wrap: bool,
    },
    /// Text on faces of the solid, written in a sketch of its own (hidden,
    /// standing off the face it was put on) that the feature keeps in step
    /// with these fields, and carried onto the faces as `Project` carries
    /// a sketch: raised, sunk or imprinted, wrapped onto a curved face.
    /// Its size, spacing and angle take formulas as other features'
    /// numbers do, the build writing what they come to into the sketch;
    /// its other numbers are plain settings written into the sketch.
    Text {
        sketch: FeatureId,
        faces: Vec<FacePick>,
        #[serde(default)]
        #[walk(setting)]
        mode: kernel_api::ProjectMode,
        #[serde(default = "default_project_height")]
        #[walk(length("height", "Height"))]
        height: f32,
        #[serde(default)]
        #[walk(setting)]
        wrap: bool,
        /// What it says; `{formula}` shows a formula's value.
        #[walk(setting)]
        text: String,
        /// A font the application carries, an installed family or a font
        /// file's path.
        #[serde(default = "default_text_font")]
        #[walk(setting)]
        font: String,
        /// The font's em, mm.
        #[serde(default = "default_text_size")]
        #[walk(length("size", "Size"))]
        size: f32,
        /// Added between letters, mm.
        #[serde(default)]
        #[walk(length("spacing", "Spacing"))]
        spacing: f32,
        /// Turned on the face, degrees.
        #[serde(default)]
        #[walk(angle("angle", "Angle"))]
        angle: f32,
        /// Where its middle stands from where the face was picked, mm
        /// across and up the face as the text reads.
        #[serde(default)]
        #[walk(length("across", "Across"))]
        across: f32,
        #[serde(default)]
        #[walk(length("up", "Up"))]
        up: f32,
        /// On a cylinder or cone, the text runs along its axis rather than
        /// round it.
        #[serde(default)]
        #[walk(setting)]
        along: bool,
        /// Round a cylinder, the letters spread evenly over the whole turn.
        #[serde(default)]
        #[walk(setting)]
        full_turn: bool,
        /// The radius of the cylinder a whole turn goes round: where the
        /// text was put on one, then as each build finds the face; zero on
        /// any other face.
        #[serde(default)]
        #[walk(setting)]
        radius: f32,
    },
    Thickness {
        #[walk(length("thickness", "Thickness"))]
        value: f32,
        faces: Vec<FacePick>,
        #[serde(default = "default_true")]
        #[walk(setting)]
        inward: bool,
        /// How the walls meet where the solid's faces meet.
        #[serde(default)]
        #[walk(setting)]
        join: kernel_api::ThicknessJoin,
        /// Walls on both sides of the faces, the thickness each way.
        #[serde(default)]
        #[walk(setting)]
        both_sides: bool,
    },
    Mirrored {
        originals: Vec<FeatureId>,
        plane: MirrorPlane,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    LinearPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        #[walk(length("length", "Length"))]
        length: f32,
        #[walk(count("occurrences", "Occurrences"))]
        occurrences: u32,
        /// `length` is the spacing between occurrences instead of the total.
        #[serde(default)]
        #[walk(setting)]
        spacing_mode: bool,
        #[serde(default)]
        #[walk(setting)]
        reversed: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Uneven spacing: the gap from each occurrence to the next, first
        /// to last. A gap past the list's end is the even spacing `length`
        /// gives, or, with `length` the overall span, a share of what the
        /// listed gaps leave of it; listed in full, they must add up to it.
        /// An empty list spaces every occurrence evenly.
        #[serde(default)]
        #[walk(lengths("spacing", "Spacing"))]
        spacings: Vec<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    PolarPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        #[walk(count("occurrences", "Occurrences"))]
        occurrences: u32,
        #[serde(default)]
        #[walk(setting)]
        reversed: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// `angle_deg` is the angle between occurrences instead of the
        /// overall one.
        #[serde(default)]
        #[walk(setting)]
        step_mode: bool,
        /// Uneven steps: the angle from each occurrence to the next, first
        /// to last, degrees. A step past the list's end is the even one
        /// `angle_deg` gives, or, with `angle_deg` the overall angle, a
        /// share of what the listed steps leave of it (round a whole turn
        /// the step back to the original among them); with no such step
        /// left they must add up to it. An empty list turns every
        /// occurrence evenly.
        #[serde(default)]
        #[walk(angles("step_angle", "Step angle"))]
        angles: Vec<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    /// Concentric rings about `axis`: ring `i` moved `i × spacing` along
    /// `outward`, then spread evenly round the axis, `counts[i]`
    /// occurrences in it, the original first in the first ring.
    CircularPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        outward: PatternAxis,
        #[walk(length("ring_spacing", "Ring spacing"))]
        spacing: f32,
        #[walk(counts("ring", "Ring"))]
        counts: Vec<u32>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    /// An occurrence at every point of `sketch` after its first, moved by
    /// how far that point is from the first.
    PointPattern {
        originals: Vec<FeatureId>,
        sketch: FeatureId,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    /// Occurrences along a sketch's chain of curves or an edge of the
    /// solid, by arc length, the original standing at the path's start.
    PathPattern {
        originals: Vec<FeatureId>,
        path: PatternPath,
        #[serde(flatten)]
        layout: PathSpacing,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    /// Copies over a flat region, a sketch's area or a face of the solid,
    /// at the places of a grid or a honeycomb through where the original
    /// stands.
    FillPattern {
        originals: Vec<FeatureId>,
        region: FillArea,
        #[serde(default)]
        #[walk(setting)]
        lattice: FillKind,
        /// A grid's spacing along its rows, a honeycomb's from each place
        /// to its neighbours, mm.
        #[walk(length("spacing", "Spacing"))]
        spacing: f32,
        /// A grid's spacing from row to row, mm.
        #[serde(default = "default_fill_rows")]
        #[walk(length("row_spacing", "Row spacing"))]
        row_spacing: f32,
        /// The lattice turned about the region's normal, degrees.
        #[serde(default)]
        #[walk(angle("angle", "Angle"))]
        angle_deg: f32,
        /// How far every copy keeps inside the region's edge, mm.
        #[serde(default)]
        #[walk(length("margin", "Margin"))]
        margin: f32,
        /// Copies the edge passes through are trimmed to the region
        /// rather than left out.
        #[serde(default)]
        #[walk(setting)]
        clip: bool,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn, row by row.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    MultiTransform {
        originals: Vec<FeatureId>,
        #[walk(steps("step", "Step"))]
        steps: Vec<TransformStep>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
        /// Occurrences left out, by index: 0 is the original, then each
        /// copy in turn.
        #[serde(default)]
        #[walk(setting)]
        skipped: Vec<u32>,
    },
    /// Boolean against another body's built solid.
    BodyBoolean {
        #[walk(setting)]
        tool_body: BodyId,
        #[walk(setting)]
        kind: kernel_api::BoolKind,
        /// Further tool bodies, each taken the same way after the first.
        #[serde(default)]
        #[walk(setting)]
        more_tools: Vec<BodyId>,
        /// Merge the coplanar faces the fuse or cut leaves behind.
        #[serde(default)]
        #[walk(setting)]
        refine: bool,
        /// The tolerance its fuse or cut runs at, in mm: faces within it
        /// of touching are joined. `None` or zero leaves it to the kernel.
        #[serde(default)]
        #[walk(with = crate::params::boolean_tolerance)]
        boolean_tolerance: Option<f32>,
    },
    /// Geometry of another body lent to this one, where the two bodies
    /// sit: a sketch whose profile features here take, faces they stop on
    /// or sketch on, edges they turn about or run along. It builds nothing
    /// itself.
    Borrow {
        source: BorrowSource,
        /// The geometry as it was frozen; `None` follows the source.
        #[serde(default)]
        #[walk(setting)]
        frozen: Option<FrozenBorrow>,
        #[serde(default)]
        #[walk(setting)]
        options: BorrowOptions,
    },
}

fn default_true() -> bool {
    true
}

fn default_project_height() -> f32 {
    1.0
}

fn default_text_font() -> String {
    sketch_model::text::DEFAULT_FONT.to_string()
}

fn default_text_size() -> f32 {
    5.0
}

impl WorkbenchFeature for DesignFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from(crate::KIND)
    }

    /// Every feature its data names (`#[derive(Walk)]`), but the sketch
    /// a frozen borrow was taken from: it keeps what it took.
    fn dependencies(&self) -> Vec<FeatureId> {
        let mut deps = core_document::walk::Walker::of(self).references;
        if let DesignFeature::Borrow {
            source: BorrowSource::Sketch(sketch),
            frozen: Some(_),
            ..
        } = self
        {
            deps.retain(|d| d != sketch);
        }
        deps
    }

    fn name(&self) -> &str {
        self.kind_label()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A countersink or counterdrill given no angle takes the usual one,
    /// as an angled drill point does, rather than refusing the hole.
    #[test]
    fn hole_cuts_given_no_angle_take_the_usual_one() {
        let read = |json| serde_json::from_value::<HoleCut>(json).unwrap();
        let sink = read(serde_json::json!({"Countersink": {"diameter": 6.0}}));
        assert_eq!(
            sink,
            HoleCut::Countersink {
                diameter: 6.0,
                angle_deg: 90.0
            }
        );
        let drill = read(serde_json::json!({"Counterdrill": {"diameter": 8.0, "depth": 2.0}}));
        assert!(matches!(drill, HoleCut::Counterdrill { angle_deg, .. } if angle_deg == 118.0));
    }

    /// A projection saved with only its sketch and faces reads as split
    /// lines at the default height, its sketch its one dependency, and
    /// comes back as it was written.
    #[test]
    fn a_projection_reads_its_defaults_and_round_trips() {
        let sketch = FeatureId::new();
        let value = serde_json::json!({"Project": {
            "sketch": sketch.0.to_string(),
            "faces": [{"point": [0.0, -10.0, 10.0], "normal": [0.0, -1.0, 0.0]}],
        }});
        let read = DesignFeature::from_json(&value).unwrap();
        let DesignFeature::Project {
            faces,
            mode,
            height,
            wrap,
            ..
        } = &read
        else {
            panic!("{read:?}");
        };
        assert_eq!(faces.len(), 1);
        assert_eq!(*mode, kernel_api::ProjectMode::Split);
        assert_eq!(*height, 1.0);
        assert!(!wrap, "a projection runs straight unless asked to wrap");
        assert_eq!(read.dependencies(), vec![sketch]);
        assert!(read.is_modifier());
        let embossed = DesignFeature::Project {
            sketch,
            faces: faces.clone(),
            mode: kernel_api::ProjectMode::Emboss,
            height: 0.6,
            wrap: true,
        };
        assert_eq!(
            DesignFeature::from_json(&embossed.to_json()).unwrap(),
            embossed
        );
    }

    /// A feature saved before its newer fields reads them as their
    /// defaults.
    #[test]
    fn features_saved_before_their_newer_fields_read_them_as_defaults() {
        let sketch = FeatureId::new();
        type Reads = fn(&DesignFeature) -> bool;
        let rows: [(&str, serde_json::Value, Reads); 6] = [
            (
                "a pad without its termination modes",
                serde_json::json!({"Pad": {"sketch": sketch, "length": 5.0, "reversed": false}}),
                |f| {
                    matches!(f, DesignFeature::Pad {
                        symmetric: false,
                        mode: ExtrudeMode::Dimension,
                        taper_deg,
                        up_to_face: None,
                        ..
                    } if *taper_deg == 0.0)
                },
            ),
            (
                "a pocket through all",
                serde_json::json!({"Pocket": {
                    "sketch": sketch, "depth": 5.0, "reversed": false, "through_all": true
                }}),
                |f| {
                    matches!(
                        f,
                        DesignFeature::Pocket {
                            symmetric: false,
                            through_all: true,
                            mode: ExtrudeMode::Dimension,
                            ..
                        }
                    )
                },
            ),
            (
                "a revolution without its axis",
                serde_json::json!({"Revolution": {"sketch": sketch, "angle_deg": 180.0}}),
                |f| {
                    matches!(
                        f,
                        DesignFeature::Revolution {
                            axis: RevolveAxis::SketchY,
                            midplane: false,
                            second_angle_deg: None,
                            ..
                        }
                    )
                },
            ),
            (
                "a helix that neither grows nor keeps inside",
                serde_json::json!({"Helix": {
                    "sketch": sketch, "axis": "SketchY", "mode": "PitchHeight",
                    "pitch": 2.0, "height": 10.0, "turns": 5.0, "left_handed": false,
                    "cone_angle_deg": 0.0, "reversed": false, "subtractive": true
                }}),
                |f| {
                    matches!(f, DesignFeature::Helix {
                        keep_inside: false,
                        growth,
                        ..
                    } if *growth == 0.0)
                },
            ),
            (
                "a fillet with its edges alone",
                serde_json::json!({"Fillet": {"radius": 1.0}}),
                |f| {
                    matches!(
                        f,
                        DesignFeature::Fillet {
                            follow_tangent: false,
                            ..
                        }
                    )
                },
            ),
            (
                "a chamfer with its edges alone",
                serde_json::json!({"Chamfer": {"size": 1.0}}),
                |f| {
                    matches!(
                        f,
                        DesignFeature::Chamfer {
                            follow_tangent: false,
                            ..
                        }
                    )
                },
            ),
        ];
        for (what, old, reads) in rows {
            let feature = DesignFeature::from_json(&old).unwrap_or_else(|e| panic!("{what}: {e}"));
            assert!(reads(&feature), "{what}: {feature:?}");
        }
    }

    /// A feature saved before it had a boolean tolerance reads as leaving
    /// it automatic, and a stated one goes through a save and a load.
    #[test]
    fn a_boolean_tolerance_reads_automatic_when_absent_and_keeps_when_stated() {
        let old = serde_json::json!({
            "Pocket": { "sketch": FeatureId::new(), "depth": 5.0, "reversed": false }
        });
        let mut feature = DesignFeature::from_json(&old).unwrap();
        assert_eq!(feature.boolean_tolerance(), None);
        feature.set_boolean_tolerance(Some(0.02));
        let again = DesignFeature::from_json(&feature.to_json()).unwrap();
        assert_eq!(again.boolean_tolerance(), Some(0.02));
        // Zero is automatic too, and a feature with no fuse or cut has none.
        feature.set_boolean_tolerance(Some(0.0));
        assert_eq!(feature.boolean_tolerance(), None);
        let mut fillet = DesignFeature::from_json(&serde_json::json!({
            "Fillet": { "radius": 1.0, "edges": "All" }
        }))
        .unwrap();
        fillet.set_boolean_tolerance(Some(0.02));
        assert_eq!(fillet.boolean_tolerance(), None);
    }

    #[test]
    fn an_old_pad_reads_as_one_sided_along_the_normal_from_its_sketch() {
        let sketch = FeatureId::new();
        let old = serde_json::json!({
            "Pad": { "sketch": sketch, "length": 5.0, "reversed": false, "mode": "TwoLengths" }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert_eq!(feature.sketch(), Some(sketch));
        let DesignFeature::Pad {
            profile_face,
            direction,
            up_to_shape,
            mode,
            mode2,
            ..
        } = feature
        else {
            panic!("a pad");
        };
        assert_eq!(profile_face, None);
        assert_eq!(direction, ExtrudeDirection::Normal);
        assert!(up_to_shape.is_empty());
        // Two lengths is a dimension each way.
        assert_eq!(
            mode.sides(mode2),
            (ExtrudeMode::Dimension, Some(ExtrudeMode::Dimension))
        );
        assert_eq!(
            ExtrudeMode::UpToFace.sides(Some(ExtrudeMode::ThroughAll)),
            (ExtrudeMode::UpToFace, Some(ExtrudeMode::ThroughAll))
        );
        assert_eq!(
            ExtrudeMode::ToFirst.sides(None),
            (ExtrudeMode::ToFirst, None)
        );
    }

    #[test]
    fn a_datum_axis_is_a_dependency() {
        let (sketch, datum) = (FeatureId::new(), FeatureId::new());
        let old = serde_json::json!({
            "Revolution": { "sketch": sketch, "angle_deg": 180.0, "axis": {"Datum": datum} }
        });
        let feature = DesignFeature::from_json(&old).unwrap();
        assert_eq!(feature.dependencies(), vec![sketch, datum]);
    }

    #[test]
    fn an_old_pipe_reads_its_frenet_flag_as_its_orientation() {
        let old = |frenet: bool| {
            serde_json::json!({
                "Pipe": {
                    "profile": FeatureId::new(),
                    "spine": FeatureId::new(),
                    "frenet": frenet,
                    "subtractive": false
                }
            })
        };
        for (frenet, orientation) in [
            (true, PipeOrientation::Frenet),
            (false, PipeOrientation::Standard),
        ] {
            let feature = DesignFeature::from_json(&old(frenet)).unwrap();
            let DesignFeature::Pipe {
                orientation: read,
                corner,
                sections,
                ..
            } = &feature
            else {
                panic!("not a pipe: {feature:?}");
            };
            assert_eq!(*read, orientation);
            assert_eq!(*corner, PipeCorner::Transformed);
            assert!(sections.is_empty());
            // Written again, it carries the orientation and reads back.
            let again = DesignFeature::from_json(&feature.to_json()).unwrap();
            assert_eq!(again, feature);
        }
    }

    #[test]
    fn a_pipe_orientation_round_trips() {
        let feature = DesignFeature::Pipe {
            keep_inside: false,
            path_borrowed: Vec::new(),
            path_curves: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            boolean_tolerance: None,
            profile: FeatureId::new(),
            spine: FeatureId::new(),
            orientation: PipeOrientation::Binormal {
                x: 0.0,
                y: 0.0,
                z: 1.0,
            },
            corner: PipeCorner::Round,
            sections: vec![FeatureId::new()],
            subtractive: true,
        };
        assert_eq!(
            DesignFeature::from_json(&feature.to_json()).unwrap(),
            feature
        );
    }

    #[test]
    fn a_pipe_depends_on_its_sketches_guide_and_sections() {
        let (a, b, c, d) = (
            FeatureId::new(),
            FeatureId::new(),
            FeatureId::new(),
            FeatureId::new(),
        );
        let pipe = |orientation, sections| DesignFeature::Pipe {
            keep_inside: false,
            path_borrowed: Vec::new(),
            path_curves: Vec::new(),
            path_edges: Vec::new(),
            profile_face: None,
            refine: false,
            boolean_tolerance: None,
            profile: a,
            spine: b,
            orientation,
            corner: PipeCorner::Transformed,
            sections,
            subtractive: false,
        };
        let plain = pipe(PipeOrientation::Standard, Vec::new());
        assert_eq!(plain.dependencies(), vec![a, b]);
        let guided = pipe(PipeOrientation::Auxiliary { path: c }, vec![d]);
        assert_eq!(guided.dependencies(), vec![a, b, c, d]);
    }

    #[test]
    fn a_pattern_saved_before_skipping_skips_nothing_and_a_toggle_flips_one() {
        let original = FeatureId::new();
        let saved = serde_json::json!({
            "LinearPattern": {
                "originals": [original], "axis": "X", "length": 10.0, "occurrences": 3
            }
        });
        let mut pattern = DesignFeature::from_json(&saved).unwrap();
        assert_eq!(pattern.skipped(), Some(&[][..]));
        assert!(pattern.toggle_skipped(2));
        assert!(pattern.toggle_skipped(0));
        assert_eq!(pattern.skipped(), Some(&[0, 2][..]));
        assert!(pattern.toggle_skipped(2));
        assert_eq!(pattern.skipped(), Some(&[0][..]));
        let read = DesignFeature::from_json(&pattern.to_json()).unwrap();
        assert_eq!(read.skipped(), Some(&[0][..]));
        let mut fillet =
            DesignFeature::from_json(&serde_json::json!({ "Fillet": { "radius": 1.0 } })).unwrap();
        assert!(!fillet.toggle_skipped(0));
        assert_eq!(fillet.skipped(), None);
    }

    #[test]
    fn a_pattern_follows_the_datum_or_sketch_its_axis_comes_from() {
        let (original, datum, sketch) = (FeatureId::new(), FeatureId::new(), FeatureId::new());
        let linear = serde_json::json!({
            "LinearPattern": {
                "originals": [original], "axis": {"Datum": datum},
                "length": 10.0, "occurrences": 3
            }
        });
        let linear = DesignFeature::from_json(&linear).unwrap();
        assert_eq!(linear.dependencies(), vec![original, datum]);
        let polar = serde_json::json!({
            "PolarPattern": {
                "originals": [original],
                "axis": {"Sketch": {"sketch": sketch, "axis": "Normal"}},
                "angle_deg": 90.0, "occurrences": 3
            }
        });
        let polar = DesignFeature::from_json(&polar).unwrap();
        assert_eq!(polar.dependencies(), vec![original, sketch]);
        assert!(matches!(
            polar,
            DesignFeature::PolarPattern { step_mode: false, ref angles, .. } if angles.is_empty()
        ));
    }
}
