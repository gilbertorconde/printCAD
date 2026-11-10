//! Split a body into pieces: each piece a body of its own that starts
//! with a Clone of the body split and keeps one side of a cut (a plane,
//! or a sketch's profile carried through the solid), so editing the body
//! split rebuilds every piece. The body split is hidden; the pieces stand
//! where it stood.
//!
//! The pieces of one split share its `split` number, and a change to one
//! piece's cut is made to all of them (`sync`), each keeping its own side.

use core_document::{
    Args, BodyId, CommandArgs, CommandError, CommandResult, Document, FeatureId, WorkbenchFeature,
    WorkbenchRuntimeContext,
};
use serde_json::{Value, json};

use crate::DesignWorkbench;
use crate::feature::{DesignFeature, MirrorPlane};

/// Where a split cuts: a plane moved `offset` along its normal, or a
/// sketch's profile in its place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cut {
    pub plane: MirrorPlane,
    pub offset: f32,
    pub sketch: Option<FeatureId>,
}

impl Cut {
    /// The cut a split piece makes.
    fn of(feature: &DesignFeature) -> Option<Cut> {
        match feature {
            DesignFeature::Split {
                plane,
                offset,
                sketch,
                ..
            } => Some(Cut {
                plane: *plane,
                offset: *offset,
                sketch: *sketch,
            }),
            _ => None,
        }
    }

    /// Across the middle of `body`'s solid, square to its longest side:
    /// what a part too big for the bed is cut along first.
    pub(crate) fn across_middle(document: &Document, body: BodyId) -> Cut {
        let bounds = document
            .local_geometry(body)
            .and_then(|(mesh, bounds)| bounds.or_else(|| mesh.bounds()));
        let Some((lo, hi)) = bounds else {
            return Cut {
                plane: MirrorPlane::XY,
                offset: 0.0,
                sketch: None,
            };
        };
        let axis = (0..3)
            .max_by(|&a, &b| (hi[a] - lo[a]).total_cmp(&(hi[b] - lo[b])))
            .unwrap_or(2);
        Cut {
            plane: [MirrorPlane::YZ, MirrorPlane::XZ, MirrorPlane::XY][axis],
            offset: (lo[axis] + hi[axis]) / 2.0,
            sketch: None,
        }
    }
}

/// The body a split piece was cut from: the source of its Clone.
pub(crate) fn source_of(document: &Document, body: BodyId) -> Option<BodyId> {
    match crate::build::design_features_of_body(document, body).first() {
        Some((_, DesignFeature::Clone { source })) => Some(*source),
        _ => None,
    }
}

/// The other pieces of the split `feature` is a piece of.
fn siblings(
    document: &Document,
    feature: FeatureId,
    split: u64,
) -> Vec<(FeatureId, DesignFeature)> {
    document
        .feature_tree()
        .all_nodes()
        .filter(|(id, n)| **id != feature && n.workbench_id.as_str() == crate::KIND)
        .filter_map(|(id, n)| Some((*id, DesignFeature::from_json(&n.data).ok()?)))
        .filter(|(_, f)| f.split_group() == Some(split))
        .collect()
}

/// After `feature`, a split piece, changed from `before` to `after`: make
/// the same cut in every other piece of its split, and when it changed
/// the side it keeps, change theirs too, so the pieces stay complementary.
pub(crate) fn sync(
    document: &mut Document,
    feature: FeatureId,
    before: &DesignFeature,
    after: &DesignFeature,
) {
    let (Some(split), Some(cut)) = (after.split_group(), Cut::of(after)) else {
        return;
    };
    let flipped = |f: &DesignFeature| match f {
        DesignFeature::Split { other_side, .. } => *other_side,
        _ => false,
    };
    let flip = flipped(before) != flipped(after);
    if Cut::of(before) == Some(cut) && !flip {
        return;
    }
    for (id, mut piece) in siblings(document, feature, split) {
        if let DesignFeature::Split {
            plane,
            offset,
            sketch,
            other_side,
            ..
        } = &mut piece
        {
            *plane = cut.plane;
            *offset = cut.offset;
            *sketch = cut.sketch;
            *other_side ^= flip;
        }
        let deps = piece.dependencies();
        if document.update_feature_data(id, piece.to_json()).is_ok() {
            document.set_feature_dependencies(id, deps);
            document.mark_feature_dirty(id);
        }
    }
}

/// Split `source` along `cut` into two pieces, each a new body placed
/// where the source stands: a Clone of it and a Split keeping one side.
/// The source is hidden. Each piece's body and its Split feature, in
/// order: the side the plane's normal points to (inside the sketch's
/// profile) first.
pub(crate) fn split_body(
    ctx: &mut WorkbenchRuntimeContext,
    source: BodyId,
    cut: Cut,
    name: Option<&str>,
) -> Result<Vec<(BodyId, FeatureId)>, String> {
    let Some(entry) = ctx.document.body(source) else {
        return Err("no such body".into());
    };
    let base_name = name
        .map(str::to_string)
        .unwrap_or_else(|| entry.name.clone());
    if ctx.document.is_mesh_body(source) {
        return Err("a mesh body has no solid to split; convert it to a solid first".into());
    }
    if ctx.document.imported_brep_blob(source).is_none() {
        return Err("the body has no solid to split yet; build it first".into());
    }
    if let Some(sketch) = cut.sketch
        && !ctx
            .document
            .get_feature_meta(sketch)
            .is_some_and(|n| n.workbench_id.as_str() == wb_sketch::KIND)
    {
        return Err("the sketch to cut by is not a sketch".into());
    }
    let split = uuid::Uuid::new_v4().as_u64_pair().0;
    let placement = ctx.document.body_placement(source);
    let mut pieces = Vec::new();
    for (index, other_side) in [false, true].into_iter().enumerate() {
        let body = ctx
            .document
            .create_body(Some(format!("{base_name} piece {}", index + 1)));
        ctx.document.set_body_placement(body, placement);
        let clone = DesignWorkbench::next_feature_name(ctx, "Clone");
        ctx.document
            .add_feature_in_body(DesignFeature::Clone { source }, clone, Some(body))
            .map_err(|e| e.to_string())?;
        let piece = DesignFeature::Split {
            plane: cut.plane,
            offset: cut.offset,
            sketch: cut.sketch,
            other_side,
            split,
        };
        let name = DesignWorkbench::next_feature_name(ctx, "Split");
        let id = ctx
            .document
            .add_feature_in_body(piece, name, Some(body))
            .map_err(|e| e.to_string())?;
        pieces.push((body, id));
    }
    ctx.document.set_body_visible(source, false);
    Ok(pieces)
}

/// Undo a split the task made and was cancelled: its pieces go and the
/// body split shows again.
pub(crate) fn discard(document: &mut Document, pieces: &[BodyId], source: Option<BodyId>) {
    for body in pieces {
        document.remove_body(*body);
    }
    if let Some(source) = source {
        document.set_body_visible(source, true);
    }
}

/// The arguments of a `design.split` call that makes the pieces of a
/// split as `feature` (one of its pieces, in `body`) cuts.
pub(crate) fn recorded_args(
    document: &Document,
    body: BodyId,
    feature: &DesignFeature,
) -> Option<serde_json::Map<String, Value>> {
    let cut = Cut::of(feature)?;
    let source = source_of(document, body)?;
    let mut args = serde_json::Map::new();
    args.insert("body".into(), json!(source.0.to_string()));
    match cut.sketch {
        Some(sketch) => {
            args.insert("sketch".into(), json!(sketch.0.to_string()));
        }
        None => {
            args.insert("plane".into(), serde_json::to_value(cut.plane).ok()?);
            args.insert("offset".into(), json!(cut.offset));
        }
    }
    Some(args)
}

/// Run `design.split`.
pub(crate) fn command(
    a: &Args,
    args: &CommandArgs,
    ctx: &mut WorkbenchRuntimeContext,
) -> CommandResult {
    let source = match a.opt_id("body")? {
        Some(id) => BodyId(id),
        None => ctx
            .selected_body_id
            .map(BodyId)
            .ok_or_else(|| CommandError::bad("body", "is required when no body is selected"))?,
    };
    let mut cut = Cut::across_middle(ctx.document, source);
    if let Some(sketch) = a.opt_id("sketch")? {
        cut.sketch = Some(FeatureId(sketch));
    }
    if let Some(plane) = args.get("plane") {
        cut.plane = serde_json::from_value(plane.clone()).map_err(|_| {
            CommandError::bad(
                "plane",
                "is not a plane: give \"XY\", \"XZ\", \"YZ\" or {Datum = {datum = id}}",
            )
        })?;
        cut.offset = 0.0;
    }
    if let Some(offset) = a.opt_number("offset")? {
        cut.offset = offset as f32;
    }
    let pieces =
        split_body(ctx, source, cut, a.opt_string("name")?).map_err(CommandError::failed)?;
    Ok(Value::from(
        pieces
            .iter()
            .map(|(body, _)| body.0.to_string())
            .collect::<Vec<_>>(),
    ))
}
