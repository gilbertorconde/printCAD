//! Whether a feature that adds or removes material left the solid as it
//! was: a pocket pointing away from the material, a pad buried in it, a
//! copy landing past it. The chain builds such a feature and warns of it.

use kernel_api::{BoolKind, BooleanOp, SolidOp};
use ogeom::math::Point;
use ogeom::topo::{Model, Shape, ShapeType, explore_unique};

use crate::convert::array;

/// What an op that adds or removes material says when its feature changes
/// nothing: what it did not do, then why that may be, for the feature's
/// kind to be put before it. `None` for an op that neither adds nor
/// removes, or whose material is withheld (an original only a pattern's
/// copies stand for). `subtractive` says of an earlier op, by its index,
/// whether its tool cuts.
pub(crate) fn said_of(
    op: &SolidOp,
    withheld: bool,
    subtractive: impl Fn(usize) -> Option<bool>,
) -> Option<&'static str> {
    if withheld {
        return None;
    }
    const ADDS: &str = "adds nothing: it may lie inside the solid";
    const REMOVES: &str = "removes nothing: it may point away from the material";
    match op {
        SolidOp::Boolean { kind, .. } => match kind {
            BoolKind::Fuse => Some("adds nothing: its tool body may lie inside the solid"),
            BoolKind::Cut => Some("removes nothing: its tool body may not meet the solid"),
            _ => None,
        },
        SolidOp::Transform { originals, .. }
        | SolidOp::TransformCopies { originals, .. }
        | SolidOp::Pattern { originals, .. }
            if !originals.is_empty() =>
        {
            Some(if originals.iter().any(|&o| subtractive(o) == Some(true)) {
                "removes nothing: its copies may land past the material"
            } else {
                "adds nothing: its copies may land inside the solid"
            })
        }
        SolidOp::Transform { .. } => Some("adds nothing: its copies may land inside the solid"),
        _ => match op.boolean_op()? {
            BooleanOp::Fuse => Some(ADDS),
            BooleanOp::Cut => Some(REMOVES),
            BooleanOp::Common | BooleanOp::NewSolid => None,
        },
    }
}

/// Whether `after` holds the material `before` does: every face of it
/// one of `before`'s, or a piece of one (a face split where a tool only
/// touched it), and the same volume enclosed. The faces are compared
/// first, each by its surface's kind and its box to the micrometre, as
/// a face comes out of no face of `before` wherever material came or
/// went; the volume is measured only when they agree.
pub(crate) fn left_alone(model: &Model, before: &Shape, after: &Shape) -> bool {
    if before == after {
        return true;
    }
    let (Some(was), Some(is)) = (described(model, before), described(model, after)) else {
        return false;
    };
    let mut kept: std::collections::HashMap<&Face, usize> = std::collections::HashMap::new();
    for face in &was {
        *kept.entry(face).or_default() += 1;
    }
    let mut pieces = Vec::new();
    for face in &is {
        match kept.get_mut(face) {
            Some(n) if *n > 0 => *n -= 1,
            _ => pieces.push(face),
        }
    }
    let within = |piece: &Face, whole: &Face| {
        piece.0 == whole.0
            && (0..3).all(|i| piece.1[i] >= whole.1[i] - 1 && piece.2[i] <= whole.2[i] + 1)
    };
    if !pieces
        .iter()
        .all(|piece| was.iter().any(|whole| within(piece, whole)))
    {
        return false;
    }
    match (volume(model, before), volume(model, after)) {
        (Some(was), Some(is)) => (was - is).abs() <= 1e-6 * was.abs().max(1.0),
        _ => false,
    }
}

/// A face as [`left_alone`] compares it: its surface's kind and the low
/// and high corners of its box, in micrometres.
type Face = (u8, [i64; 3], [i64; 3]);

/// `shape`'s faces as [`left_alone`] compares them.
fn described(model: &Model, shape: &Shape) -> Option<Vec<Face>> {
    let micrometres = |p: Point| array(p).map(|c| (c * 1e3).round() as i64);
    explore_unique(model, shape, ShapeType::Face)
        .ok()?
        .iter()
        .map(|face| {
            let bounds = ogeom::algo::face_bounds(model, face).ok()?;
            Some((
                crate::naming::surface_kind(model, face),
                micrometres(bounds.low()?),
                micrometres(bounds.high()?),
            ))
        })
        .collect()
}

/// The volume `shape`'s solids enclose, when the kernel measures it.
fn volume(model: &Model, shape: &Shape) -> Option<f64> {
    let tol = crate::tess::tolerances();
    let deflection = ogeom::mesh::Deflection::default();
    explore_unique(model, shape, ShapeType::Solid)
        .ok()?
        .iter()
        .map(|solid| {
            ogeom::algo::volume_properties(model, solid, deflection, tol)
                .ok()
                .map(|v| v.mass)
        })
        .sum()
}
