//! Per-op translators from `kernel_api::SolidOp` to ogeom calls.

pub mod dressup;
pub mod loft_pipe;
pub mod pattern;
pub mod pattern_steps;
pub mod primitive;
pub mod project;
pub mod surface;
pub mod sweep;
mod wrap;

use ogeom::core::Tolerances;
use ogeom::topo::{Model, Shape};

use crate::tess;

pub fn tol() -> Tolerances {
    tess::tolerances()
}

thread_local! {
    /// The tolerance the op being built states for its booleans.
    static STATED: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
}

/// The tolerance the op being built on this thread states for its
/// booleans, if any.
pub fn stated_tolerance() -> Option<f64> {
    STATED.with(|s| s.get())
}

/// While held, the booleans on this thread run at `tolerance` (none: the
/// kernel's own, with its fuzzy retry); dropping it puts back the one
/// before.
pub struct ToleranceScope(Option<f64>);

impl ToleranceScope {
    pub fn state(tolerance: Option<f64>) -> Self {
        ToleranceScope(STATED.with(|s| s.replace(tolerance)))
    }
}

impl Drop for ToleranceScope {
    fn drop(&mut self) {
        STATED.with(|s| s.set(self.0));
    }
}

/// Whether two shapes' bounding boxes come anywhere near each other.
pub fn bounds_overlap(model: &Model, a: &Shape, b: &Shape) -> bool {
    let (Some((alo, ahi)), Some((blo, bhi))) =
        (tess::robust_bounds(model, a), tess::robust_bounds(model, b))
    else {
        return true;
    };
    // Shapes a stated tolerance would join count as meeting.
    let m = stated_tolerance().unwrap_or(0.0).max(1e-6);
    alo.x <= bhi.x + m
        && blo.x <= ahi.x + m
        && alo.y <= bhi.y + m
        && blo.y <= ahi.y + m
        && alo.z <= bhi.z + m
        && blo.z <= ahi.z + m
}

/// Fuse two solids, or compound them when they are clearly disjoint (the
/// kernel's disjoint-fuse path drops swept solids; building the compound
/// directly sidesteps it and costs nothing).
pub fn fuse_or_compound(model: &mut Model, a: &Shape, b: &Shape) -> Result<Shape, String> {
    if bounds_overlap(model, a, b) {
        ogeom::boolean::fuse(model, a, b, tol())
            .map(|built| {
                crate::naming::record(&built.history);
                built.shape
            })
            .map_err(|e| format!("fuse failed: {e}"))
    } else {
        model
            .add_compound(&[a.clone(), b.clone()])
            .map_err(|e| format!("compounding disjoint solids failed: {e}"))
    }
}

/// Fuse a list of region solids into one (multi-region profiles): each
/// region fuses with the solids its bounds meet, one at a time, and what
/// stays apart is compounded at the end, since a boolean takes solids and
/// never a compound.
pub fn fuse_all(model: &mut Model, parts: Vec<Shape>) -> Result<Shape, String> {
    if parts.is_empty() {
        return Err("operation produced no solid".to_string());
    }
    let mut solids: Vec<Shape> = Vec::new();
    for part in parts {
        let mut pending = vec![part];
        while let Some(piece) = pending.pop() {
            let Some(i) = solids.iter().position(|s| bounds_overlap(model, s, &piece)) else {
                solids.push(piece);
                continue;
            };
            let other = solids.remove(i);
            let fused = ogeom::boolean::fuse(model, &other, &piece, tol())
                .map(|built| {
                    crate::naming::record(&built.history);
                    built.shape
                })
                .map_err(|e| format!("fusing profile regions failed: fuse failed: {e}"))?;
            match solids_of(model, &fused).as_slice() {
                // One solid: it may meet others yet.
                [one] => pending.push(one.clone()),
                // Their boxes met but they did not: both stay as they are.
                several => solids.extend(several.iter().cloned()),
            }
        }
    }
    if solids.len() == 1 {
        return Ok(solids.remove(0));
    }
    model
        .add_compound(&solids)
        .map_err(|e| format!("compounding disjoint solids failed: {e}"))
}

/// Unwrap a compound holding exactly one solid: boolean results sometimes
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
    use ogeom::topo::{Filter, ShapeType, explore};
    match model.kind_of(shape) {
        Ok(ShapeType::Solid) => vec![shape.clone()],
        Ok(ShapeType::Compound | ShapeType::CompSolid) => {
            explore(model, shape, Filter::OfType(ShapeType::Solid)).unwrap_or_else(|e| {
                tracing::warn!(target: "printcad.kernel", "a compound's solids do not read: {e}");
                Vec::new()
            })
        }
        _ => vec![shape.clone()],
    }
}

pub(crate) fn wrap_pieces(model: &mut Model, mut pieces: Vec<Shape>) -> Result<Shape, String> {
    match pieces.len() {
        0 => Err("boolean removed all material".into()),
        1 => Ok(pieces.remove(0)),
        _ => model
            .add_compound(&pieces)
            .map_err(|e| format!("compounding boolean pieces failed: {e}")),
    }
}

/// One solid-vs-solid boolean. At a stated tolerance (see
/// [`ToleranceScope`]) it runs fuzzy at that tolerance from the start,
/// since an exact boolean that succeeds can still leave the sliver the
/// user stated it to close; otherwise it runs exact, with a fuzzy retry at
/// a hundred times the confusion when the kernel says a looser tolerance
/// could resolve a near-coincidence.
pub fn bool_once(
    model: &mut Model,
    a: &Shape,
    b: &Shape,
    kind: kernel_api::BoolKind,
) -> Result<Shape, String> {
    use kernel_api::BoolKind;
    let t = tol();
    let fuzzy = |model: &mut Model, fuzz: f64| match kind {
        BoolKind::Fuse => ogeom::boolean::fuse_fuzzy(model, a, b, fuzz, t),
        BoolKind::Cut => ogeom::boolean::cut_fuzzy(model, a, b, fuzz, t),
        BoolKind::Common => ogeom::boolean::common_fuzzy(model, a, b, fuzz, t),
    };
    let verb = match kind {
        BoolKind::Fuse => "fuse",
        BoolKind::Cut => "cut",
        BoolKind::Common => "common",
    };
    let done = |model: &mut Model, built: ogeom::algo::Built| {
        crate::naming::record(&built.history);
        normalized(model, built.shape)
    };
    if let Some(stated) = stated_tolerance() {
        return match fuzzy(model, stated) {
            Ok(built) => Ok(done(model, built)),
            Err(e) => Err(format!(
                "boolean {verb} at a tolerance of {stated} mm failed: {e}"
            )),
        };
    }
    let exact = match kind {
        BoolKind::Fuse => ogeom::boolean::fuse(model, a, b, t),
        BoolKind::Cut => ogeom::boolean::cut(model, a, b, t),
        BoolKind::Common => ogeom::boolean::common(model, a, b, t),
    };
    match exact {
        Ok(built) => Ok(done(model, built)),
        Err(e) if e.is_tolerance_sensitive() => fuzzy(model, t.confusion() * 100.0)
            .map(|built| done(model, built))
            .map_err(|e| format!("boolean {verb} failed: {e}")),
        Err(e) => Err(format!("boolean {verb} failed: {e}")),
    }
}

/// Compound-aware boolean: both operands may be compounds of disjoint
/// solids, where ogeom's booleans insist on solids. Pieces pair up by
/// bounding-box overlap; disjoint pieces skip the boolean entirely.
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
                    let next = bool_once(model, &acc, t, BoolKind::Cut)?;
                    let pieces = solids_of(model, &next);
                    if pieces.is_empty() {
                        gone = true;
                        break;
                    }
                    acc = next;
                }
                if !gone {
                    out.push(acc);
                }
            }
            wrap_pieces(model, out)
        }
        BoolKind::Common => {
            let out = common_pieces(model, base, tool)?;
            wrap_pieces(model, out)
        }
    }
}

/// The solids `a` and `b` share, piece by piece: none when they are apart.
pub fn common_pieces(model: &mut Model, a: &Shape, b: &Shape) -> Result<Vec<Shape>, String> {
    let tools = solids_of(model, b);
    let mut out = Vec::new();
    for piece in solids_of(model, a) {
        for t in &tools {
            if !bounds_overlap(model, &piece, t) {
                continue;
            }
            let shared = bool_once(model, &piece, t, kernel_api::BoolKind::Common)?;
            out.extend(solids_of(model, &shared));
        }
    }
    Ok(out)
}
