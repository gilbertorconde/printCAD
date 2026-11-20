//! A surface texture as a body keeps it: which pattern, how it lies on
//! the faces and how deep it goes. Pressing it into a mesh is the
//! `surface_texture` crate's work.

use serde::{Deserialize, Serialize};

/// An axis of the body's own frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Axis; 3] = [Axis::X, Axis::Y, Axis::Z];

    pub fn label(self) -> &'static str {
        match self {
            Axis::X => "X",
            Axis::Y => "Y",
            Axis::Z => "Z",
        }
    }
}

/// What a texture presses into its faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    /// Pyramids in a diamond grid, as a knurled grip has.
    Knurl,
    /// Straight ridges.
    Ribs,
    /// Round bumps in a square grid.
    Dots,
    /// Hexagonal cells parted by grooves.
    Hex,
    /// Bricks in offset rows, parted by grooves.
    Bricks,
    /// Smooth waves across the tile.
    Waves,
    /// Rough, even noise.
    Noise,
    /// Criss-cross grooves, as a file's face.
    Crosshatch,
    /// A greyscale picture, white high: a document asset.
    Image { asset: uuid::Uuid },
}

impl Pattern {
    /// Every pattern made by code, in the order a gallery shows them.
    pub const BUILT_IN: [Pattern; 8] = [
        Pattern::Knurl,
        Pattern::Ribs,
        Pattern::Dots,
        Pattern::Hex,
        Pattern::Bricks,
        Pattern::Waves,
        Pattern::Noise,
        Pattern::Crosshatch,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pattern::Knurl => "Knurl",
            Pattern::Ribs => "Ribs",
            Pattern::Dots => "Dots",
            Pattern::Hex => "Hexagons",
            Pattern::Bricks => "Bricks",
            Pattern::Waves => "Waves",
            Pattern::Noise => "Noise",
            Pattern::Crosshatch => "Crosshatch",
            Pattern::Image { .. } => "Picture",
        }
    }
}

/// How the pattern's flat tile is laid on the faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Projection {
    /// Three flat projections, each weighted by how squarely a face looks
    /// along its axis: good on any shape.
    Triplanar,
    /// One flat projection along an axis.
    Planar(Axis),
    /// Wrapped round an axis, a whole number of tiles round.
    Cylindrical(Axis),
    /// Wrapped round a centre, a whole number of tiles round its equator.
    Spherical,
}

impl Projection {
    pub fn label(self) -> String {
        match self {
            Projection::Triplanar => "Triplanar".to_string(),
            Projection::Planar(axis) => format!("Flat along {}", axis.label()),
            Projection::Cylindrical(axis) => format!("Round {}", axis.label()),
            Projection::Spherical => "Spherical".to_string(),
        }
    }

    /// Every projection, in the order a list shows them.
    pub fn all() -> Vec<Projection> {
        let mut out = vec![Projection::Triplanar];
        out.extend(Axis::ALL.map(Projection::Planar));
        out.extend(Axis::ALL.map(Projection::Cylindrical));
        out.push(Projection::Spherical);
        out
    }
}

/// A texture as it is set: its pattern, how it lies and how deep it goes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Texture {
    pub pattern: Pattern,
    pub projection: Projection,
    /// One tile's size, mm.
    pub tile_mm: f32,
    /// The tile's turn on the surface, degrees.
    #[serde(default)]
    pub rotation_deg: f32,
    /// How far the pattern's high points stand from the surface, mm.
    pub depth_mm: f32,
    /// Into the surface rather than out of it.
    #[serde(default)]
    pub inward: bool,
    /// Faces within this many degrees of facing straight up or down stay
    /// smooth (a print's top and the side it stands on); 0 textures them
    /// too.
    #[serde(default)]
    pub keep_flat_deg: f32,
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            pattern: Pattern::Knurl,
            projection: Projection::Triplanar,
            tile_mm: 3.0,
            rotation_deg: 0.0,
            depth_mm: 0.4,
            inward: false,
            keep_flat_deg: 0.0,
        }
    }
}
