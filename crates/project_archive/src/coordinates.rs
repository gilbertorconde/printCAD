//! Source coordinates in millimetres. Arrays at the application boundary keep
//! the reader's public contract independent of the consumer's math library.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2 {
    pub x: f64,
    pub y: f64,
}

impl Point2 {
    pub const ZERO: Self = Self::new(0.0, 0.0);
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub const fn to_array(self) -> [f64; 2] {
        [self.x, self.y]
    }
    pub const fn from_array([x, y]: [f64; 2]) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
    pub const fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
    pub const fn from_array([x, y, z]: [f64; 3]) -> Self {
        Self { x, y, z }
    }
}

/// Quaternion components in x, y, z, w order. Decoding validates unit length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation(pub [f64; 4]);

impl Rotation {
    pub const IDENTITY: Self = Self([0.0, 0.0, 0.0, 1.0]);
    pub const fn to_array(self) -> [f64; 4] {
        self.0
    }
    pub fn transform_vector(self, vector: [f64; 3]) -> [f64; 3] {
        (glam::DQuat::from_array(self.0) * glam::DVec3::from_array(vector)).to_array()
    }
}
