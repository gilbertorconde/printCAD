//! What the tree and the menus show for a feature: its kind's label and
//! icon, and the presets of the primitives.

use super::*;

impl DesignFeature {
    pub fn kind_label(&self) -> &'static str {
        match self {
            DesignFeature::Pad { .. } => "Pad",
            DesignFeature::Pocket { .. } => "Pocket",
            DesignFeature::Revolution { .. } => "Revolution",
            DesignFeature::Groove { .. } => "Groove",
            DesignFeature::Loft { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Loft"
                } else {
                    "Additive Loft"
                }
            }
            DesignFeature::Pipe { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Pipe"
                } else {
                    "Additive Pipe"
                }
            }
            DesignFeature::Helix { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Helix"
                } else {
                    "Additive Helix"
                }
            }
            DesignFeature::Primitive { subtractive, .. } => {
                if *subtractive {
                    "Subtractive Primitive"
                } else {
                    "Primitive"
                }
            }
            DesignFeature::Hole { .. } => "Hole",
            DesignFeature::Thread { .. } => "Thread",
            DesignFeature::Rib { .. } => "Rib",
            DesignFeature::Fillet { .. } => "Fillet",
            DesignFeature::Chamfer { .. } => "Chamfer",
            DesignFeature::Draft { .. } => "Draft",
            DesignFeature::Thickness { .. } => "Thickness",
            DesignFeature::DeleteFaces { .. } => "Delete Faces",
            DesignFeature::OffsetFaces { .. } => "Offset Faces",
            DesignFeature::OffsetSolid { .. } => "Offset Solid",
            DesignFeature::Scale { .. } => "Scale",
            DesignFeature::MoveFaces { .. } => "Move Faces",
            DesignFeature::Project { .. } => "Project",
            DesignFeature::Text { .. } => "Text",
            DesignFeature::Mirrored { .. } => "Mirrored",
            DesignFeature::LinearPattern { .. } => "Linear Pattern",
            DesignFeature::PolarPattern { .. } => "Polar Pattern",
            DesignFeature::CircularPattern { .. } => "Circular Pattern",
            DesignFeature::PointPattern { .. } => "Point Pattern",
            DesignFeature::PathPattern { .. } => "Path Pattern",
            DesignFeature::FillPattern { .. } => "Fill Pattern",
            DesignFeature::MultiTransform { .. } => "Multi Transform",
            DesignFeature::BodyBoolean { .. } => "Boolean",
            DesignFeature::Borrow { .. } => "Borrowed geometry",
            DesignFeature::Base { .. } => "Base shape",
            DesignFeature::Clone { .. } => "Clone",
            DesignFeature::Split { .. } => "Split",
            DesignFeature::Piece { .. } => "Piece",
        }
    }

    /// The design set's icon for this feature.
    pub fn icon(&self) -> &'static str {
        use kernel_api::PrimitiveKind as P;
        match self {
            DesignFeature::Pad { .. } => "pad",
            DesignFeature::Pocket { .. } => "pocket",
            DesignFeature::Revolution { .. } => "revolution",
            DesignFeature::Groove { .. } => "groove",
            DesignFeature::Loft { subtractive, .. } => {
                if *subtractive {
                    "subtractive-loft"
                } else {
                    "additive-loft"
                }
            }
            DesignFeature::Pipe { subtractive, .. } => {
                if *subtractive {
                    "subtractive-pipe"
                } else {
                    "additive-pipe"
                }
            }
            DesignFeature::Helix { subtractive, .. } => {
                if *subtractive {
                    "subtractive-helix"
                } else {
                    "additive-helix"
                }
            }
            DesignFeature::Primitive {
                kind, subtractive, ..
            } => {
                let shape = match kind {
                    P::Box { .. } => "box",
                    P::Cylinder { .. } => "cylinder",
                    P::Sphere { .. } => "sphere",
                    P::Cone { .. } => "cone",
                    P::Torus { .. } => "torus",
                    P::Ellipsoid { .. } => "ellipsoid",
                    P::Prism { .. } => "prism",
                    P::Wedge { .. } => "wedge",
                    P::Tube { .. } => "tube",
                };
                primitive_icon(shape, *subtractive)
            }
            DesignFeature::Hole { .. } => "hole",
            DesignFeature::Thread { .. } => "thread",
            DesignFeature::Rib { .. } => "rib",
            DesignFeature::Fillet { .. } => "fillet",
            DesignFeature::Chamfer { .. } => "chamfer",
            DesignFeature::Draft { .. } => "draft",
            DesignFeature::Thickness { .. } => "thickness",
            DesignFeature::DeleteFaces { .. } => "delete",
            DesignFeature::OffsetFaces { .. } => "offset-geometry",
            DesignFeature::OffsetSolid { .. } => "offset-solid",
            DesignFeature::Scale { .. } => "scale-geometry",
            DesignFeature::MoveFaces { .. } => "move-geometry",
            DesignFeature::Project { .. } => "sketch-map",
            DesignFeature::Text { .. } => "sketch-text",
            DesignFeature::Mirrored { .. } => "mirrored",
            DesignFeature::LinearPattern { .. } => "linear-pattern",
            DesignFeature::PolarPattern { .. } => "polar-pattern",
            DesignFeature::CircularPattern { .. } => "circular-pattern",
            DesignFeature::PointPattern { .. } => "point-pattern",
            DesignFeature::PathPattern { .. } => "path-pattern",
            DesignFeature::FillPattern { .. } => "fill-pattern",
            DesignFeature::MultiTransform { .. } => "multi-transform",
            DesignFeature::BodyBoolean { .. } => "boolean",
            DesignFeature::Borrow { .. } => "clone-geometry",
            DesignFeature::Base { .. } => "file-document",
            DesignFeature::Clone { .. } => "clone",
            DesignFeature::Split { .. } => "split",
            DesignFeature::Piece { .. } => "separate-pieces",
        }
    }
}

/// The icon of a primitive shape, additive or subtractive.
pub fn primitive_icon(shape: &str, subtractive: bool) -> &'static str {
    match (shape, subtractive) {
        ("box", false) => "additive-box",
        ("box", true) => "subtractive-box",
        ("cylinder", false) => "additive-cylinder",
        ("cylinder", true) => "subtractive-cylinder",
        ("sphere", false) => "additive-sphere",
        ("sphere", true) => "subtractive-sphere",
        ("cone", false) => "additive-cone",
        ("cone", true) => "subtractive-cone",
        ("torus", false) => "additive-torus",
        ("torus", true) => "subtractive-torus",
        ("ellipsoid", false) => "additive-ellipsoid",
        ("ellipsoid", true) => "subtractive-ellipsoid",
        ("prism", false) => "additive-prism",
        ("prism", true) => "subtractive-prism",
        ("wedge", false) => "additive-wedge",
        ("wedge", true) => "subtractive-wedge",
        ("tube", false) => "additive-tube",
        ("tube", true) => "subtractive-tube",
        (_, false) => "additive-box",
        (_, true) => "subtractive-box",
    }
}

/// A primitive of the named shape with default dimensions.
pub fn primitive_preset(shape: &str) -> Option<kernel_api::PrimitiveKind> {
    use kernel_api::PrimitiveKind as P;
    Some(match shape {
        "box" => P::Box {
            length: 10.0,
            width: 10.0,
            height: 10.0,
        },
        "cylinder" => P::Cylinder {
            radius: 5.0,
            height: 10.0,
            angle_deg: 360.0,
        },
        "sphere" => P::Sphere {
            radius: 5.0,
            angle1_deg: -90.0,
            angle2_deg: 90.0,
            angle3_deg: 360.0,
        },
        "cone" => P::Cone {
            radius1: 5.0,
            radius2: 2.0,
            height: 10.0,
            angle_deg: 360.0,
        },
        "torus" => P::Torus {
            radius1: 10.0,
            radius2: 2.0,
            angle1_deg: -180.0,
            angle2_deg: 180.0,
            angle3_deg: 360.0,
        },
        "ellipsoid" => P::Ellipsoid {
            radius1: 8.0,
            radius2: 5.0,
            radius3: 3.0,
            angle1_deg: -90.0,
            angle2_deg: 90.0,
            angle3_deg: 360.0,
        },
        "prism" => P::Prism {
            sides: 6,
            circumradius: 5.0,
            height: 10.0,
            skew_x_deg: 0.0,
            skew_y_deg: 0.0,
        },
        "wedge" => P::Wedge {
            xmin: 0.0,
            xmax: 10.0,
            ymin: 0.0,
            ymax: 10.0,
            zmin: 0.0,
            zmax: 10.0,
            x2min: 2.0,
            x2max: 8.0,
            z2min: 2.0,
            z2max: 8.0,
        },
        "tube" => P::Tube {
            outer: 5.0,
            inner: 3.0,
            height: 10.0,
        },
        _ => return None,
    })
}
