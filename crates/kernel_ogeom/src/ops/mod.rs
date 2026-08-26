//! Per-op translators from `kernel_api::SolidOp` to ogeom calls.

pub mod dressup;
pub mod loft_pipe;
pub mod pattern;
pub mod primitive;
pub mod sweep;

use ogeom::core::Tolerances;
use ogeom::topo::{Model, Shape};

use crate::tess;

pub fn tol() -> Tolerances {
    tess::tolerances()
}

/// Whether two shapes' bounding boxes come anywhere near each other.
pub fn bounds_overlap(model: &Model, a: &Shape, b: &Shape) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) =
        (tess::robust_bounds(model, a), tess::robust_bounds(model, b))
    else {
        return true;
    };
    const M: f64 = 1e-6;
    alo.x <= bhi.x + M
        && blo.x <= ahi.x + M
        && alo.y <= bhi.y + M
        && blo.y <= ahi.y + M
        && alo.z <= bhi.z + M
        && blo.z <= ahi.z + M
}

/// Fuse two solids, or compound them when they are clearly disjoint (the
/// kernel's disjoint-fuse path drops swept solids — building the compound
/// directly sidesteps it and costs nothing).
pub fn fuse_or_compound(model: &mut Model, a: &Shape, b: &Shape) -> Result<Shape, String> {
    if bounds_overlap(model, a, b) {
        ogeom::boolean::fuse(model, a, b, tol())
            .map(|built| built.shape)
            .map_err(|e| format!("fuse failed: {e}"))
    } else {
        model
            .add_compound(&[a.clone(), b.clone()])
            .map_err(|e| format!("compounding disjoint solids failed: {e}"))
    }
}

/// Fuse a list of region solids into one (multi-region profiles).
pub fn fuse_all(model: &mut Model, mut parts: Vec<Shape>) -> Result<Shape, String> {
    let first = parts
        .drain(..1)
        .next()
        .ok_or_else(|| "operation produced no solid".to_string())?;
    let mut acc = first;
    for part in parts {
        acc = fuse_or_compound(model, &acc, &part)
            .map_err(|e| format!("fusing profile regions failed: {e}"))?;
    }
    Ok(acc)
}

/// Unwrap a compound holding exactly one solid — boolean results sometimes
/// come back wrapped, and downstream booleans insist on solid operands.
pub fn normalized(model: &Model, shape: Shape) -> Shape {
    use ogeom::topo::ShapeType;
    if model.kind_of(&shape) != Ok(ShapeType::Compound) {
        return shape;
    }
    let Ok(children) = model.children_of(&shape) else {
        return shape;
    };
    let solids: Vec<_> = children
        .iter()
        .filter(|c| model.kind_of(c) == Ok(ShapeType::Solid))
        .collect();
    if solids.len() == 1 && children.len() == 1 {
        solids[0].clone()
    } else {
        shape
    }
}

/// The solid pieces of a shape: itself when it is a solid, its solid
/// children when it is a compound.
pub fn solids_of(model: &Model, shape: &Shape) -> Vec<Shape> {
    use ogeom::topo::{explore, Filter, ShapeType};
    match model.kind_of(shape) {
        Ok(ShapeType::Solid) => vec![shape.clone()],
        Ok(ShapeType::Compound | ShapeType::CompSolid) => {
            explore(model, shape, Filter::OfType(ShapeType::Solid)).unwrap_or_default()
        }
        _ => vec![shape.clone()],
    }
}

fn wrap_pieces(model: &mut Model, mut pieces: Vec<Shape>) -> Result<Shape, String> {
    match pieces.len() {
        0 => Err("boolean removed all material".into()),
        1 => Ok(pieces.remove(0)),
        _ => model
            .add_compound(&pieces)
            .map_err(|e| format!("compounding boolean pieces failed: {e}")),
    }
}

/// One solid-vs-solid boolean with a fuzzy retry when the kernel says a
/// looser tolerance could resolve a near-coincidence.
pub fn bool_once(
    model: &mut Model,
    a: &Shape,
    b: &Shape,
    kind: kernel_api::BoolKind,
) -> Result<Shape, String> {
    use kernel_api::BoolKind;
    let t = tol();
    let run = |model: &mut Model| match kind {
        BoolKind::Fuse => ogeom::boolean::fuse(model, a, b, t),
        BoolKind::Cut => ogeom::boolean::cut(model, a, b, t),
        BoolKind::Common => ogeom::boolean::common(model, a, b, t),
    };
    let verb = match kind {
        BoolKind::Fuse => "fuse",
        BoolKind::Cut => "cut",
        BoolKind::Common => "common",
    };
    match run(model) {
        Ok(built) => Ok(normalized(model, built.shape)),
        Err(e) if e.is_tolerance_sensitive() => {
            let fuzz = t.confusion() * 100.0;
            let retried = match kind {
                BoolKind::Fuse => ogeom::boolean::fuse_fuzzy(model, a, b, fuzz, t),
                BoolKind::Cut => ogeom::boolean::cut_fuzzy(model, a, b, fuzz, t),
                BoolKind::Common => Err(e),
            };
            retried
                .map(|built| normalized(model, built.shape))
                .map_err(|e| format!("boolean {verb} failed: {e}"))
        }
        Err(e) => Err(format!("boolean {verb} failed: {e}")),
    }
}

/// Compound-aware boolean: both operands may be compounds of disjoint
/// solids (the previous kernel took those in stride, ogeom's booleans insist
/// on solids). Pieces pair up by bounding-box overlap; disjoint pieces skip
/// the boolean entirely.
pub fn combine_solids(
    model: &mut Model,
    base: &Shape,
    tool: &Shape,
    kind: kernel_api::BoolKind,
) -> Result<Shape, String> {
    use kernel_api::BoolKind;
    let bases = solids_of(model, base);
    let tools = solids_of(model, tool);
    if bases.is_empty() {
        return Err("the running solid holds no material".into());
    }
    match kind {
        BoolKind::Fuse => {
            let mut pieces = bases;
            for t in tools {
                if let Some(i) = pieces.iter().position(|p| bounds_overlap(model, p, &t)) {
                    let fused = bool_once(model, &pieces[i], &t, BoolKind::Fuse)?;
                    // The fuse may itself come back as pieces.
                    let mut fused_pieces = solids_of(model, &fused);
                    pieces.remove(i);
                    pieces.append(&mut fused_pieces);
                } else {
                    pieces.push(t);
                }
            }
            wrap_pieces(model, pieces)
        }
        BoolKind::Cut => {
            let mut out = Vec::new();
            for b in bases {
                let mut acc = b;
                let mut gone = false;
                for t in &tools {
                    if !bounds_overlap(model, &acc, t) {
                        continue;
                    }
                    match bool_once(model, &acc, t, BoolKind::Cut) {
                        Ok(next) => {
                            let pieces = solids_of(model, &next);
                            if pieces.is_empty() {
                                gone = true;
                                break;
                            }
                            acc = next;
                        }
                        Err(e) => return Err(e),
                    }
                }
                if !gone {
                    out.push(acc);
                }
            }
            wrap_pieces(model, out)
        }
        BoolKind::Common => {
            let mut out = Vec::new();
            for b in &bases {
                for t in &tools {
                    if !bounds_overlap(model, b, t) {
                        continue;
                    }
                    let piece = bool_once(model, b, t, BoolKind::Common)?;
                    out.extend(solids_of(model, &piece));
                }
            }
            wrap_pieces(model, out)
        }
    }
}
