//! Editing tools that operate on existing geometry: fillet and chamfer,
//! trim, extend, split and offset.

use std::collections::{HashMap, HashSet};

use glam::DVec2;
use uuid::Uuid;

use super::{ToolEffect, ToolState};
use crate::curves::{Curve, END_SLACK};
use crate::geom2d::arc_angles;
use crate::geom2d::{self, Prim, prim_of, raw_hits, within};
use crate::sketch::{
    Arc, ConstraintKind, GeometryElement, Line, Point, Sketch, Vec2D, X_AXIS_ID, Y_AXIS_ID,
};
use crate::snap;

mod corner;
mod general;
mod offset;
mod trim;

pub(super) use corner::{CornerCut, corner};
use corner::{continues, curve_resized};
pub(crate) use corner::{corner_operands, round_corner};
use general::{
    extend_general, general_points, general_preview, general_under_cursor, split_general,
    trim_general,
};
use offset::chain_ends;
pub(super) use offset::{OffsetOptions, offset};
use trim::{drop_if_orphan, ends_on};
pub(super) use trim::{extend, split, trim};
pub use trim::{next_stroke_crossing, trim_preview};

/// Relative slack in curve parameter space: intersections this close to a
/// span end don't count as cut points (shared endpoints intersect exactly
/// at the ends).
const SPAN_EPS: f64 = 1e-3;
