//! Part Design feature payloads stored in the document feature tree.

use core_document::{
    BodyId, DocumentResult, FeatureError, FeatureId, WorkbenchFeature, WorkbenchId,
};
use serde::{Deserialize, Serialize};

/// Which in-plane sketch axis a revolution/helix spins about.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum RevolveAxis {
    /// The sketch's vertical (y) axis through the origin (the default).
    #[default]
    SketchY,
    /// The sketch's horizontal (x) axis through the origin.
    SketchX,
    /// An arbitrary in-plane axis (point + direction in sketch coordinates).
    Custom { origin: [f32; 2], dir: [f32; 2] },
}

impl RevolveAxis {
    pub fn label(&self) -> &'static str {
        match self {
            RevolveAxis::SketchY => "Sketch Y axis",
            RevolveAxis::SketchX => "Sketch X axis",
            RevolveAxis::Custom { .. } => "Custom axis",
        }
    }

    /// Axis origin in sketch 2D coordinates.
    pub fn origin_2d(&self) -> [f64; 2] {
        match self {
            RevolveAxis::SketchY | RevolveAxis::SketchX => [0.0, 0.0],
            RevolveAxis::Custom { origin, .. } => [origin[0] as f64, origin[1] as f64],
        }
    }

    /// Direction in sketch 2D coordinates.
    pub fn dir_2d(&self) -> [f64; 2] {
        match self {
            RevolveAxis::SketchY => [0.0, 1.0],
            RevolveAxis::SketchX => [1.0, 0.0],
            RevolveAxis::Custom { dir, .. } => [dir[0] as f64, dir[1] as f64],
        }
    }
}

/// A planar face picked in the viewport, identified geometrically.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FacePick {
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

/// Where a pad/pocket stops along the sweep direction.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ExtrudeMode {
    /// Fixed length/depth.
    #[default]
    Dimension,
    /// Independent lengths to each side of the sketch plane.
    TwoLengths,
    /// Through every face of the existing material.
    ThroughAll,
    /// Stop at the first face hit along the direction.
    ToFirst,
    /// Stop at the last face hit along the direction.
    ToLast,
    /// Stop on a picked planar face (plus offset).
    UpToFace,
}

impl ExtrudeMode {
    pub const ALL: [ExtrudeMode; 6] = [
        ExtrudeMode::Dimension,
        ExtrudeMode::TwoLengths,
        ExtrudeMode::ThroughAll,
        ExtrudeMode::ToFirst,
        ExtrudeMode::ToLast,
        ExtrudeMode::UpToFace,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ExtrudeMode::Dimension => "Dimension",
            ExtrudeMode::TwoLengths => "Two lengths",
            ExtrudeMode::ThroughAll => "Through all",
            ExtrudeMode::ToFirst => "To first",
            ExtrudeMode::ToLast => "To last",
            ExtrudeMode::UpToFace => "Up to face",
        }
    }
}

/// How a helix's extent is specified; the missing quantity is derived.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum HelixMode {
    #[default]
    PitchHeight,
    PitchTurns,
    HeightTurns,
}

impl HelixMode {
    pub const ALL: [HelixMode; 3] = [
        HelixMode::PitchHeight,
        HelixMode::PitchTurns,
        HelixMode::HeightTurns,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            HelixMode::PitchHeight => "Pitch + height",
            HelixMode::PitchTurns => "Pitch + turns",
            HelixMode::HeightTurns => "Height + turns",
        }
    }
}

/// Chamfer sizing style.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum ChamferMode {
    #[default]
    EqualDistance,
    TwoDistances,
    DistanceAngle,
}

impl ChamferMode {
    pub const ALL: [ChamferMode; 3] = [
        ChamferMode::EqualDistance,
        ChamferMode::TwoDistances,
        ChamferMode::DistanceAngle,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ChamferMode::EqualDistance => "Equal distance",
            ChamferMode::TwoDistances => "Two distances",
            ChamferMode::DistanceAngle => "Distance + angle",
        }
    }
}

/// Which edges a dress-up applies to.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub enum EdgeSel {
    /// Every edge of the solid (immune to topology churn).
    #[default]
    All,
    /// The edges bordering the picked faces.
    Faces(Vec<FacePick>),
}

/// A mirror/pattern reference plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum MirrorPlane {
    #[default]
    XY,
    XZ,
    YZ,
    Face(FacePick),
}

impl MirrorPlane {
    pub const BASE: [MirrorPlane; 3] = [MirrorPlane::XY, MirrorPlane::XZ, MirrorPlane::YZ];

    pub fn label(&self) -> &'static str {
        match self {
            MirrorPlane::XY => "XY plane",
            MirrorPlane::XZ => "XZ plane",
            MirrorPlane::YZ => "YZ plane",
            MirrorPlane::Face(_) => "Picked face",
        }
    }

    /// World-space (point, normal).
    pub fn plane(&self) -> ([f64; 3], [f64; 3]) {
        match self {
            MirrorPlane::XY => ([0.0; 3], [0.0, 0.0, 1.0]),
            MirrorPlane::XZ => ([0.0; 3], [0.0, 1.0, 0.0]),
            MirrorPlane::YZ => ([0.0; 3], [1.0, 0.0, 0.0]),
            MirrorPlane::Face(pick) => (
                [
                    pick.point[0] as f64,
                    pick.point[1] as f64,
                    pick.point[2] as f64,
                ],
                [
                    pick.normal[0] as f64,
                    pick.normal[1] as f64,
                    pick.normal[2] as f64,
                ],
            ),
        }
    }
}

/// A pattern direction/axis in world space.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum PatternAxis {
    X,
    #[default]
    Y,
    Z,
    Custom {
        origin: [f32; 3],
        dir: [f32; 3],
    },
}

impl PatternAxis {
    pub const BASE: [PatternAxis; 3] = [PatternAxis::X, PatternAxis::Y, PatternAxis::Z];

    pub fn label(&self) -> &'static str {
        match self {
            PatternAxis::X => "X axis",
            PatternAxis::Y => "Y axis",
            PatternAxis::Z => "Z axis",
            PatternAxis::Custom { .. } => "Custom axis",
        }
    }

    pub fn origin(&self) -> [f64; 3] {
        match self {
            PatternAxis::Custom { origin, .. } => {
                [origin[0] as f64, origin[1] as f64, origin[2] as f64]
            }
            _ => [0.0; 3],
        }
    }

    pub fn dir(&self) -> [f64; 3] {
        match self {
            PatternAxis::X => [1.0, 0.0, 0.0],
            PatternAxis::Y => [0.0, 1.0, 0.0],
            PatternAxis::Z => [0.0, 0.0, 1.0],
            PatternAxis::Custom { dir, .. } => [dir[0] as f64, dir[1] as f64, dir[2] as f64],
        }
    }
}

/// One step of a multi-transform (each applies to every result of the
/// previous step).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransformStep {
    Linear {
        axis: PatternAxis,
        length: f32,
        occurrences: u32,
    },
    Polar {
        axis: PatternAxis,
        angle_deg: f32,
        occurrences: u32,
    },
    Mirror {
        plane: MirrorPlane,
    },
    Scale {
        factor: f32,
        center: [f32; 3],
        occurrences: u32,
    },
}

/// Counterbore/countersink options for a hole.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub enum HoleCut {
    #[default]
    None,
    Counterbore {
        diameter: f32,
        depth: f32,
    },
    Countersink {
        diameter: f32,
        angle_deg: f32,
    },
}

impl HoleCut {
    pub fn label(&self) -> &'static str {
        match self {
            HoleCut::None => "None",
            HoleCut::Counterbore { .. } => "Counterbore",
            HoleCut::Countersink { .. } => "Countersink",
        }
    }
}

/// ISO 273 metric clearance style for a threaded hole size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum HoleFit {
    Close,
    #[default]
    Normal,
    Loose,
}

impl HoleFit {
    pub const ALL: [HoleFit; 3] = [HoleFit::Close, HoleFit::Normal, HoleFit::Loose];

    pub fn label(&self) -> &'static str {
        match self {
            HoleFit::Close => "Close",
            HoleFit::Normal => "Normal",
            HoleFit::Loose => "Loose",
        }
    }
}

/// ISO metric coarse sizes: (designation, thread pitch, tap drill Ø,
/// clearance Ø close/normal/loose per ISO 273).
pub const METRIC_SIZES: [(&str, f32, f32, [f32; 3]); 10] = [
    ("M2", 0.4, 1.6, [2.2, 2.4, 2.6]),
    ("M2.5", 0.45, 2.05, [2.7, 2.9, 3.1]),
    ("M3", 0.5, 2.5, [3.2, 3.4, 3.6]),
    ("M4", 0.7, 3.3, [4.3, 4.5, 4.8]),
    ("M5", 0.8, 4.2, [5.3, 5.5, 5.8]),
    ("M6", 1.0, 5.0, [6.4, 6.6, 7.0]),
    ("M8", 1.25, 6.8, [8.4, 9.0, 10.0]),
    ("M10", 1.5, 8.5, [10.5, 11.0, 12.0]),
    ("M12", 1.75, 10.2, [13.0, 13.5, 14.5]),
    ("M16", 2.0, 14.0, [17.0, 17.5, 18.5]),
];

/// A solid-modeling feature in a body's linear history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PartFeature {
    /// Extrude the sketch profile, adding material.
    Pad {
        sketch: FeatureId,
        length: f32,
        /// Extrude along -normal instead of +normal.
        reversed: bool,
        /// Extrude half the length to each side of the sketch plane.
        #[serde(default)]
        symmetric: bool,
        #[serde(default)]
        mode: ExtrudeMode,
        #[serde(default)]
        length2: f32,
        #[serde(default)]
        taper_deg: f32,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        #[serde(default)]
        up_to_offset: f32,
    },
    /// Extrude the sketch profile and subtract it (cuts against the sketch
    /// normal by default: a face sketch's normal points out of the material).
    Pocket {
        sketch: FeatureId,
        depth: f32,
        reversed: bool,
        #[serde(default)]
        through_all: bool,
        #[serde(default)]
        mode: ExtrudeMode,
        #[serde(default)]
        depth2: f32,
        #[serde(default)]
        taper_deg: f32,
        #[serde(default)]
        up_to_face: Option<FacePick>,
        #[serde(default)]
        up_to_offset: f32,
    },
    /// Revolve the sketch profile about an in-plane axis, adding material.
    Revolution {
        sketch: FeatureId,
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        #[serde(default)]
        reversed: bool,
        #[serde(default)]
        midplane: bool,
        #[serde(default)]
        second_angle_deg: Option<f32>,
    },
    /// Revolve the sketch profile and subtract it.
    Groove {
        sketch: FeatureId,
        angle_deg: f32,
        #[serde(default)]
        axis: RevolveAxis,
        #[serde(default)]
        reversed: bool,
        #[serde(default)]
        midplane: bool,
        #[serde(default)]
        second_angle_deg: Option<f32>,
    },
    /// Skin through two or more section sketches.
    Loft {
        sections: Vec<FeatureId>,
        ruled: bool,
        closed: bool,
        subtractive: bool,
    },
    /// Sweep a profile sketch along a spine sketch's path.
    Pipe {
        profile: FeatureId,
        spine: FeatureId,
        frenet: bool,
        subtractive: bool,
    },
    /// Sweep the sketch profile along a helix about an in-plane axis.
    Helix {
        sketch: FeatureId,
        axis: RevolveAxis,
        mode: HelixMode,
        pitch: f32,
        height: f32,
        turns: f32,
        left_handed: bool,
        cone_angle_deg: f32,
        reversed: bool,
        subtractive: bool,
    },
    /// Parametric primitive fused into (or cut from) the body.
    Primitive {
        kind: kernel_api::PrimitiveKind,
        placement: kernel_api::Placement,
        subtractive: bool,
    },
    /// Standards-aware cylindrical cuts at every circle center of a sketch.
    Hole {
        sketch: FeatureId,
        diameter: f32,
        depth: f32,
        through_all: bool,
        #[serde(default)]
        cut: HoleCut,
        /// ISO metric designation index into [`METRIC_SIZES`] when the hole
        /// is standards-driven; the diameter then derives from thread/fit.
        #[serde(default)]
        metric_index: Option<usize>,
        #[serde(default)]
        threaded: bool,
        #[serde(default)]
        fit: HoleFit,
        #[serde(default)]
        reversed: bool,
    },
    Fillet {
        radius: f32,
        #[serde(default)]
        edges: EdgeSel,
    },
    Chamfer {
        size: f32,
        #[serde(default)]
        mode: ChamferMode,
        #[serde(default)]
        size2: f32,
        #[serde(default)]
        angle_deg: f32,
        #[serde(default)]
        flip: bool,
        #[serde(default)]
        edges: EdgeSel,
    },
    Draft {
        angle_deg: f32,
        neutral: FacePick,
        faces: Vec<FacePick>,
        #[serde(default)]
        reversed: bool,
    },
    Thickness {
        value: f32,
        faces: Vec<FacePick>,
        #[serde(default = "default_true")]
        inward: bool,
    },
    Mirrored {
        originals: Vec<FeatureId>,
        plane: MirrorPlane,
    },
    LinearPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        length: f32,
        occurrences: u32,
        /// `length` is the spacing between occurrences instead of the total.
        #[serde(default)]
        spacing_mode: bool,
        #[serde(default)]
        reversed: bool,
    },
    PolarPattern {
        originals: Vec<FeatureId>,
        axis: PatternAxis,
        angle_deg: f32,
        occurrences: u32,
        #[serde(default)]
        reversed: bool,
    },
    MultiTransform {
        originals: Vec<FeatureId>,
        steps: Vec<TransformStep>,
    },
    /// Boolean against another body's built solid.
    BodyBoolean {
        tool_body: BodyId,
        kind: kernel_api::BoolKind,
    },
}

fn default_true() -> bool {
    true
}

impl PartFeature {
    /// The sketch this feature consumes, when it is sketch-based.
    pub fn sketch(&self) -> Option<FeatureId> {
        match self {
            PartFeature::Pad { sketch, .. }
            | PartFeature::Pocket { sketch, .. }
            | PartFeature::Revolution { sketch, .. }
            | PartFeature::Groove { sketch, .. }
            | PartFeature::Helix { sketch, .. }
            | PartFeature::Hole { sketch, .. } => Some(*sketch),
            PartFeature::Pipe { profile, .. } => Some(*profile),
            _ => None,
        }
    }

    /// Every sketch referenced by this feature.
    pub fn sketches(&self) -> Vec<FeatureId> {
        match self {
            PartFeature::Loft { sections, .. } => sections.clone(),
            PartFeature::Pipe { profile, spine, .. } => vec![*profile, *spine],
            _ => self.sketch().into_iter().collect(),
        }
    }

    /// Earlier part features this feature re-applies (patterns/mirror).
    pub fn originals(&self) -> Option<&[FeatureId]> {
        match self {
            PartFeature::Mirrored { originals, .. }
            | PartFeature::LinearPattern { originals, .. }
            | PartFeature::PolarPattern { originals, .. }
            | PartFeature::MultiTransform { originals, .. } => Some(originals),
            _ => None,
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            PartFeature::Pad { .. } => "Pad",
            PartFeature::Pocket { .. } => "Pocket",
            PartFeature::Revolution { .. } => "Revolution",
            PartFeature::Groove { .. } => "Groove",
            PartFeature::Loft { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Loft"
                } else {
                    "Additive Loft"
                }
            }
            PartFeature::Pipe { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Pipe"
                } else {
                    "Additive Pipe"
                }
            }
            PartFeature::Helix { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Helix"
                } else {
                    "Additive Helix"
                }
            }
            PartFeature::Primitive { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Primitive"
                } else {
                    "Primitive"
                }
            }
            PartFeature::Hole { .. } => "Hole",
            PartFeature::Fillet { .. } => "Fillet",
            PartFeature::Chamfer { .. } => "Chamfer",
            PartFeature::Draft { .. } => "Draft",
            PartFeature::Thickness { .. } => "Thickness",
            PartFeature::Mirrored { .. } => "Mirrored",
            PartFeature::LinearPattern { .. } => "Linear Pattern",
            PartFeature::PolarPattern { .. } => "Polar Pattern",
            PartFeature::MultiTransform { .. } => "Multi Transform",
            PartFeature::BodyBoolean { .. } => "Boolean",
        }
    }

    /// True when this feature removes material (must not be a body's first).
    pub fn is_subtractive(&self) -> bool {
        match self {
            PartFeature::Pocket { .. } | PartFeature::Groove { .. } | PartFeature::Hole { .. } => {
                true
            }
            PartFeature::Loft { subtractive, .. }
            | PartFeature::Pipe { subtractive, .. }
            | PartFeature::Helix { subtractive, .. }
            | PartFeature::Primitive { subtractive, .. } => *subtractive,
            _ => false,
        }
    }

    /// True for features that modify the running solid instead of sweeping a
    /// new tool (dress-ups, patterns, booleans). These need existing material.
    pub fn is_modifier(&self) -> bool {
        matches!(
            self,
            PartFeature::Fillet { .. }
                | PartFeature::Chamfer { .. }
                | PartFeature::Draft { .. }
                | PartFeature::Thickness { .. }
                | PartFeature::Mirrored { .. }
                | PartFeature::LinearPattern { .. }
                | PartFeature::PolarPattern { .. }
                | PartFeature::MultiTransform { .. }
                | PartFeature::BodyBoolean { .. }
        )
    }
}

impl WorkbenchFeature for PartFeature {
    fn workbench_id() -> WorkbenchId {
        WorkbenchId::from("wb.part")
    }

    fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or(serde_json::Value::Null)
    }

    fn from_json(value: &serde_json::Value) -> DocumentResult<Self> {
        serde_json::from_value(value.clone()).map_err(|e| {
            core_document::DocumentError::Feature(FeatureError::Deserialization(e.to_string()))
        })
    }

    fn dependencies(&self) -> Vec<FeatureId> {
        let mut deps = self.sketches();
        if let Some(originals) = self.originals() {
            deps.extend_from_slice(originals);
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

    #[test]
    fn old_pad_json_without_new_fields_still_deserializes() {
        // A Pad serialized before the termination-mode fields existed.
        let old = serde_json::json!({
            "Pad": { "sketch": FeatureId::new(), "length": 5.0, "reversed": false }
        });
        let feature = PartFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            PartFeature::Pad {
                symmetric: false,
                mode: ExtrudeMode::Dimension,
                taper_deg: t,
                up_to_face: None,
                ..
            } if t == 0.0
        ));
    }

    #[test]
    fn old_pocket_json_still_deserializes() {
        let old = serde_json::json!({
            "Pocket": {
                "sketch": FeatureId::new(),
                "depth": 5.0,
                "reversed": false,
                "through_all": true
            }
        });
        let feature = PartFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            PartFeature::Pocket {
                through_all: true,
                mode: ExtrudeMode::Dimension,
                ..
            }
        ));
    }

    #[test]
    fn old_revolution_json_still_deserializes() {
        let old = serde_json::json!({
            "Revolution": { "sketch": FeatureId::new(), "angle_deg": 180.0 }
        });
        let feature = PartFeature::from_json(&old).unwrap();
        assert!(matches!(
            feature,
            PartFeature::Revolution {
                axis: RevolveAxis::SketchY,
                midplane: false,
                second_angle_deg: None,
                ..
            }
        ));
    }

    #[test]
    fn dependencies_cover_sketches_and_originals() {
        let a = FeatureId::new();
        let b = FeatureId::new();
        let pipe = PartFeature::Pipe {
            profile: a,
            spine: b,
            frenet: false,
            subtractive: false,
        };
        assert_eq!(pipe.dependencies(), vec![a, b]);

        let pattern = PartFeature::LinearPattern {
            originals: vec![a],
            axis: PatternAxis::X,
            length: 10.0,
            occurrences: 3,
            spacing_mode: false,
            reversed: false,
        };
        assert_eq!(pattern.dependencies(), vec![a]);
    }
}
