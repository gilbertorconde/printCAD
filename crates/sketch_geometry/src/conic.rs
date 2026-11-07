//! Resolved conic spans shared by geometry queries and host adapters.

use crate::curves::{
    ellipse_angle, ellipse_semi_minor, hyperbola_branch, hyperbola_param, hyperbola_semi_minor,
    parabola_param,
};
use crate::types::{Angle, HyperbolaBranch, HyperbolaParam, ParabolaParam};
use crate::world::{WorldLength, WorldPoint};

/// The carrier and directed parameter interval of a conic arc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ConicArcParams {
    Ellipse {
        center: WorldPoint,
        focus1: WorldPoint,
        semi_minor: WorldLength,
        t_start: Angle,
        t_end: Angle,
        clockwise: bool,
    },
    Hyperbola {
        center: WorldPoint,
        focus1: WorldPoint,
        semi_minor: WorldLength,
        t_start: HyperbolaParam,
        t_end: HyperbolaParam,
        /// The parameter alone does not distinguish the two branches.
        branch: HyperbolaBranch,
    },
    Parabola {
        vertex: WorldPoint,
        focus: WorldPoint,
        u_start: ParabolaParam,
        u_end: ParabolaParam,
    },
}

impl ConicArcParams {
    pub fn ellipse(
        center: WorldPoint,
        focus1: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
        clockwise: bool,
    ) -> Self {
        let semi_minor = ellipse_semi_minor(center, focus1, start);
        Self::Ellipse {
            center,
            focus1,
            semi_minor,
            t_start: ellipse_angle(center, focus1, semi_minor, start),
            t_end: ellipse_angle(center, focus1, semi_minor, end),
            clockwise,
        }
    }

    /// The start names the branch; a valid span ends on that same branch.
    pub fn hyperbola(
        center: WorldPoint,
        focus1: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
    ) -> Option<Self> {
        let semi_minor = hyperbola_semi_minor(center, focus1, start);
        let branch = hyperbola_branch(center, focus1, semi_minor, start)?;
        Some(Self::Hyperbola {
            center,
            focus1,
            semi_minor,
            t_start: hyperbola_param(center, focus1, semi_minor, start),
            t_end: hyperbola_param(center, focus1, semi_minor, end),
            branch,
        })
    }

    pub fn parabola(
        vertex: WorldPoint,
        focus: WorldPoint,
        start: WorldPoint,
        end: WorldPoint,
    ) -> Self {
        Self::Parabola {
            vertex,
            focus,
            u_start: parabola_param(vertex, focus, start),
            u_end: parabola_param(vertex, focus, end),
        }
    }
}
