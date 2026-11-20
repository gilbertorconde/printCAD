//! The settings of lofts, helices and pipes.

use super::*;

/// One section of a loft: a feature (a sketch's profile; a datum point or a
/// sketch holding a single point, where the loft closes to it) or a flat
/// face of the solid, picked. A file listing sections by id reads each as
/// a feature.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Walk)]
#[serde(untagged)]
pub enum LoftSection {
    Feature(FeatureId),
    Face(FacePick),
}

impl LoftSection {
    pub fn feature(&self) -> Option<FeatureId> {
        match self {
            LoftSection::Feature(id) => Some(*id),
            LoftSection::Face(_) => None,
        }
    }
}

/// How a helix's extent is specified; the missing quantity is derived.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Describe)]
pub enum HelixMode {
    #[default]
    PitchHeight,
    PitchTurns,
    HeightTurns,
    /// Height, turns and a growth per turn in place of the cone angle; a
    /// height of 0 is a flat spiral.
    HeightTurnsGrowth,
}

impl HelixMode {
    pub const ALL: [HelixMode; 4] = [
        HelixMode::PitchHeight,
        HelixMode::PitchTurns,
        HelixMode::HeightTurns,
        HelixMode::HeightTurnsGrowth,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            HelixMode::PitchHeight => "Pitch + height",
            HelixMode::PitchTurns => "Pitch + turns",
            HelixMode::HeightTurns => "Height + turns",
            HelixMode::HeightTurnsGrowth => "Height + turns + growth",
        }
    }
}

/// How a pipe's section turns as it runs down its path.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, Walk)]
pub enum PipeOrientation {
    /// The section neither twists nor kinks where the path bends.
    #[default]
    Standard,
    /// The section turns with the path's own curvature frame.
    Frenet,
    /// One direction of the section keeps pointing at a second path sketch
    /// running beside the first.
    Auxiliary { path: FeatureId },
    /// The section keeps a fixed binormal, a direction in the body's frame.
    Binormal {
        #[walk(number("binormal_x", "Binormal X"))]
        x: f32,
        #[walk(number("binormal_y", "Binormal Y"))]
        y: f32,
        #[walk(number("binormal_z", "Binormal Z"))]
        z: f32,
    },
    /// The section keeps its orientation in space, as it is drawn.
    Fixed,
}

impl PipeOrientation {
    /// The second path sketch this orientation follows, if any.
    pub fn path(&self) -> Option<FeatureId> {
        match self {
            PipeOrientation::Auxiliary { path } => Some(*path),
            _ => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            PipeOrientation::Standard => "Standard",
            PipeOrientation::Frenet => "Frenet",
            PipeOrientation::Auxiliary { .. } => "Auxiliary path",
            PipeOrientation::Binormal { .. } => "Binormal",
            PipeOrientation::Fixed => "Fixed",
        }
    }
}

/// Reads a [`PipeOrientation`] or the `frenet` flag it stands in for.
pub(super) fn deserialize_pipe_orientation<'de, D>(
    deserializer: D,
) -> Result<PipeOrientation, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Stored {
        Flag(bool),
        Orientation(PipeOrientation),
    }
    Ok(match Stored::deserialize(deserializer)? {
        Stored::Flag(true) => PipeOrientation::Frenet,
        Stored::Flag(false) => PipeOrientation::Standard,
        Stored::Orientation(orientation) => orientation,
    })
}

/// How a pipe's section turns a sharp corner of its path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Describe)]
pub enum PipeCorner {
    /// Carried into the corner and sheared onto the plane bisecting it.
    #[default]
    Transformed,
    /// Each leg runs on straight past the corner, the outside left square.
    Right,
    /// Turned about the corner, the outside rounded.
    Round,
}

impl PipeCorner {
    pub const ALL: [PipeCorner; 3] = [
        PipeCorner::Transformed,
        PipeCorner::Right,
        PipeCorner::Round,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            PipeCorner::Transformed => "Transformed",
            PipeCorner::Right => "Right corner",
            PipeCorner::Round => "Round corner",
        }
    }
}
