//! The settings of fillets and chamfers.

use super::*;

/// Fillet sizing style.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Describe)]
pub enum FilletMode {
    /// One size, each face's contact stopping where the face runs out
    /// while the other's goes on (on one edge).
    Auto,
    #[default]
    Radius,
    /// One distance on each face from the edge.
    TwoDistances,
    /// A radius running along the edges, taken as one run: `radius` at its
    /// start, `radius2` at its end, `points` between.
    Variable,
}

impl FilletMode {
    pub const ALL: [FilletMode; 4] = [
        FilletMode::Auto,
        FilletMode::Radius,
        FilletMode::TwoDistances,
        FilletMode::Variable,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            FilletMode::Auto => "Auto",
            FilletMode::Radius => "Radius",
            FilletMode::TwoDistances => "Two distances",
            FilletMode::Variable => "Variable",
        }
    }
}

/// A variable fillet's radius at a place along its run of edges.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
pub struct RadiusPoint {
    /// The share of the run's length from its start, strictly between 0
    /// and 1.
    #[walk(number("at", "At"))]
    pub at: f32,
    #[walk(length("radius", "Radius"))]
    pub radius: f32,
}

/// The section of a fillet of two distances.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Describe)]
pub enum FilletProfile {
    /// The ellipse tangent to each face at its distance.
    #[default]
    Elliptical,
    /// A conic whose fullness `rho` sets.
    Conic,
}

impl FilletProfile {
    pub const ALL: [FilletProfile; 2] = [FilletProfile::Elliptical, FilletProfile::Conic];

    pub fn label(&self) -> &'static str {
        match self {
            FilletProfile::Elliptical => "Elliptical",
            FilletProfile::Conic => "Conic",
        }
    }
}

pub(super) fn half() -> f32 {
    0.5
}

/// Chamfer sizing style.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Describe)]
pub enum ChamferMode {
    /// One size, each face's leg stopping where the face runs out while
    /// the other's goes on.
    Auto,
    #[default]
    EqualDistance,
    TwoDistances,
    DistanceAngle,
}

impl ChamferMode {
    pub const ALL: [ChamferMode; 4] = [
        ChamferMode::Auto,
        ChamferMode::EqualDistance,
        ChamferMode::TwoDistances,
        ChamferMode::DistanceAngle,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ChamferMode::Auto => "Auto",
            ChamferMode::EqualDistance => "Equal distance",
            ChamferMode::TwoDistances => "Two distances",
            ChamferMode::DistanceAngle => "Distance + angle",
        }
    }
}
