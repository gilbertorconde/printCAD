use glam::{Mat3, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisDirection {
    X,
    Y,
    Z,
}

impl AxisDirection {
    pub const fn label(self) -> &'static str {
        match self {
            AxisDirection::X => "X",
            AxisDirection::Y => "Y",
            AxisDirection::Z => "Z",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisSign {
    Positive,
    Negative,
}

impl AxisSign {
    pub const fn scalar(self) -> f32 {
        match self {
            AxisSign::Positive => 1.0,
            AxisSign::Negative => -1.0,
        }
    }

    pub const fn invert(self) -> Self {
        match self {
            AxisSign::Positive => AxisSign::Negative,
            AxisSign::Negative => AxisSign::Positive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Axis {
    direction: AxisDirection,
    sign: AxisSign,
}

impl Axis {
    pub const fn new(direction: AxisDirection, sign: AxisSign) -> Self {
        Self { direction, sign }
    }

    pub const fn positive(direction: AxisDirection) -> Self {
        Self {
            direction,
            sign: AxisSign::Positive,
        }
    }

    pub const fn negative(direction: AxisDirection) -> Self {
        Self {
            direction,
            sign: AxisSign::Negative,
        }
    }

    pub fn vector(self) -> Vec3 {
        let base = match self.direction {
            AxisDirection::X => Vec3::X,
            AxisDirection::Y => Vec3::Y,
            AxisDirection::Z => Vec3::Z,
        };
        base * self.sign.scalar()
    }

    pub const fn direction(&self) -> AxisDirection {
        self.direction
    }

    pub const fn sign(&self) -> AxisSign {
        self.sign
    }

    pub const fn inverted(self) -> Self {
        Self {
            direction: self.direction,
            sign: self.sign.invert(),
        }
    }

    pub const fn signed_label(&self) -> &'static str {
        match (self.direction, self.sign) {
            (AxisDirection::X, AxisSign::Positive) => "+X",
            (AxisDirection::X, AxisSign::Negative) => "-X",
            (AxisDirection::Y, AxisSign::Positive) => "+Y",
            (AxisDirection::Y, AxisSign::Negative) => "-Y",
            (AxisDirection::Z, AxisSign::Positive) => "+Z",
            (AxisDirection::Z, AxisSign::Negative) => "-Z",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisSystem {
    horizontal: Axis,
    vertical: Axis,
    depth: Axis,
}

impl AxisSystem {
    pub const fn from_preset(preset: AxisPreset) -> Self {
        preset.axis_system()
    }

    pub const fn new(horizontal: Axis, vertical: Axis, depth: Axis) -> Self {
        Self {
            horizontal,
            vertical,
            depth,
        }
    }

    pub const fn horizontal(&self) -> Axis {
        self.horizontal
    }

    pub const fn vertical(&self) -> Axis {
        self.vertical
    }

    pub const fn depth(&self) -> Axis {
        self.depth
    }

    pub fn right_vec(&self) -> Vec3 {
        self.horizontal.vector()
    }

    pub fn left_vec(&self) -> Vec3 {
        -self.right_vec()
    }

    pub fn up_vec(&self) -> Vec3 {
        self.vertical.vector()
    }

    pub fn down_vec(&self) -> Vec3 {
        -self.up_vec()
    }

    pub fn forward_vec(&self) -> Vec3 {
        self.depth.vector()
    }

    pub fn back_vec(&self) -> Vec3 {
        -self.forward_vec()
    }

    pub fn canonical_basis(&self) -> Mat3 {
        Mat3::from_cols(self.right_vec(), self.up_vec(), self.forward_vec())
    }

    pub fn canonical_to_world(&self, canonical: Vec3) -> Vec3 {
        self.canonical_basis() * canonical
    }

    pub fn world_to_canonical(&self, world: Vec3) -> Vec3 {
        self.canonical_basis().transpose() * world
    }
}

impl Default for AxisSystem {
    fn default() -> Self {
        AxisPreset::default().axis_system()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AxisPreset {
    /// X right, Y up, Z forward (right-handed, default CAD layout)
    RightHandedZForward,
    /// X right, Y up, Z backward (right-handed, OpenGL-style forward)
    RightHandedZBackward,
    /// X right, Z up, -Y forward (right-handed, Z-up workflows)
    ZUpRightHanded,
}

impl AxisPreset {
    pub const ALL: [AxisPreset; 3] = [
        AxisPreset::RightHandedZForward,
        AxisPreset::RightHandedZBackward,
        AxisPreset::ZUpRightHanded,
    ];

    pub const fn label(&self) -> &'static str {
        match self {
            AxisPreset::RightHandedZForward => "X right / Y up / Z forward",
            AxisPreset::RightHandedZBackward => "X right / Y up / -Z forward",
            AxisPreset::ZUpRightHanded => "X right / Z up / -Y forward",
        }
    }

    pub const fn description(&self) -> &'static str {
        match self {
            AxisPreset::RightHandedZForward => {
                "Typical CAD layout (match FreeCAD / conventional engineering axes)"
            }
            AxisPreset::RightHandedZBackward => {
                "OpenGL-style view with camera forward along -Z (legacy DCC tooling)"
            }
            AxisPreset::ZUpRightHanded => {
                "Z points up (architectural/animation workflows), depth runs along -Y"
            }
        }
    }

    pub const fn axis_system(self) -> AxisSystem {
        match self {
            AxisPreset::RightHandedZForward => AxisSystem::new(
                Axis::positive(AxisDirection::X),
                Axis::positive(AxisDirection::Y),
                Axis::positive(AxisDirection::Z),
            ),
            AxisPreset::RightHandedZBackward => AxisSystem::new(
                Axis::positive(AxisDirection::X),
                Axis::positive(AxisDirection::Y),
                Axis::negative(AxisDirection::Z),
            ),
            AxisPreset::ZUpRightHanded => AxisSystem::new(
                Axis::positive(AxisDirection::X),
                Axis::positive(AxisDirection::Z),
                Axis::negative(AxisDirection::Y),
            ),
        }
    }
}

impl Default for AxisPreset {
    fn default() -> Self {
        AxisPreset::RightHandedZForward
    }
}

impl From<AxisPreset> for AxisSystem {
    fn from(value: AxisPreset) -> Self {
        value.axis_system()
    }
}
