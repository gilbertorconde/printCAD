//! Separate a body whose solid is several pieces into one body per piece.
//!
//! Each piece is a body of its own that starts with a Clone of the body
//! separated and a Piece keeping the solid nearest a point on that piece,
//! so editing the body separated rebuilds the pieces, as a split's do. The
//! body separated is hidden; the pieces stand where it stood.

use core_document::{Args, BodyId, CommandError, CommandResult, Document, WorkbenchRuntimeContext};
use serde_json::Value;

use crate::DesignWorkbench;
use crate::feature::DesignFeature;

/// Corners of the drawn mesh a ten-thousandth of a millimetre apart are
/// one corner: coarser than the mesh's own weld, so corners of one solid's
/// faces, split at their creases for shading, join even where rounding
/// left them a hair apart, and far finer than any gap a body's separate
/// solids leave between them.
const PIECE_CORNER: kernel_api::PointMatch = kernel_api::PointMatch::Grid { per_mm: 1.0e4 };

/// How many pieces `body`'s drawn mesh falls into: its triangles joined
/// where they share a corner. A body whose mesh is one piece has one
/// solid; the kernel says exactly when the body is separated.
pub(crate) fn drawn_pieces(document: &Document, body: BodyId) -> usize {
    let Some((mesh, _)) = document.local_geometry(body) else {
        return 0;
    };
    let mut corner: std::collections::HashMap<[i64; 3], usize> = Default::default();
    let ids: Vec<usize> = mesh
        .positions
        .iter()
        .map(|p| {
            let key = PIECE_CORNER.key(*p);
            let next = corner.len();
            *corner.entry(key).or_insert(next)
        })
        .collect();
    let mut parent: Vec<usize> = (0..corner.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for triangle in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = [0, 1, 2].map(|k| ids[triangle[k] as usize]);
        for other in [b, c] {
            let (ra, ro) = (root(&mut parent, a), root(&mut parent, other));
            parent[ra] = ro;
        }
    }
    let used: std::collections::HashSet<usize> =
        mesh.indices.iter().map(|&i| ids[i as usize]).collect();
    let roots: std::collections::HashSet<usize> =
        used.into_iter().map(|i| root(&mut parent, i)).collect();
    roots.len()
}

/// Separate `source` into one body per piece of its solid, each placed
/// where the source stands. The source is hidden. The pieces' bodies, in
/// the order the solid holds them.
pub(crate) fn separate_body(
    ctx: &mut WorkbenchRuntimeContext,
    source: BodyId,
) -> Result<Vec<BodyId>, String> {
    let Some(entry) = ctx.document.body(source) else {
        return Err("no such body".into());
    };
    let name = entry.name.clone();
    let blob = ctx
        .document
        .imported_brep_blob(source)
        .ok_or("the body has no solid to separate yet; build it first")?;
    let kernel = ctx
        .kernel
        .ok_or("no kernel to read the solid's pieces with")?;
    let points = kernel.solid_pieces(blob).map_err(|e| e.to_string())?;
    if points.len() < 2 {
        return Err("the body's solid is one piece: there is nothing to separate".into());
    }
    let placement = ctx.document.body_placement(source);
    let mut pieces = Vec::new();
    for (index, near) in points.into_iter().enumerate() {
        let body = ctx
            .document
            .create_body(Some(format!("{name} piece {}", index + 1)));
        ctx.document.set_body_placement(body, placement);
        let clone = DesignWorkbench::next_feature_name(ctx, "Clone");
        ctx.document
            .add_feature_in_body(DesignFeature::Clone { source }, clone, Some(body))
            .map_err(|e| e.to_string())?;
        let piece = DesignWorkbench::next_feature_name(ctx, "Piece");
        let near = near.map(|c| c as f32);
        ctx.document
            .add_feature_in_body(DesignFeature::Piece { near }, piece, Some(body))
            .map_err(|e| e.to_string())?;
        pieces.push(body);
    }
    ctx.document.set_body_visible(source, false);
    Ok(pieces)
}

/// Run `design.separate`.
pub(crate) fn command(a: &Args, ctx: &mut WorkbenchRuntimeContext) -> CommandResult {
    let source = match a.opt_id("body")? {
        Some(id) => BodyId(id),
        None => ctx
            .selected_body_id
            .map(BodyId)
            .ok_or_else(|| CommandError::bad("body", "is required when no body is selected"))?,
    };
    let pieces = separate_body(ctx, source).map_err(CommandError::failed)?;
    Ok(Value::from(
        pieces.iter().map(|b| b.0.to_string()).collect::<Vec<_>>(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::bare_geometry;

    /// Two triangles sharing an edge are one piece, though their corners
    /// are listed twice as a face boundary splits them; a third triangle
    /// apart is another.
    #[test]
    fn the_drawn_mesh_counts_its_pieces() {
        let mut doc = Document::new("t");
        let body = doc.create_body(None);
        let p = |x: f32, y: f32| [x, y, 0.0];
        let positions = vec![
            p(0.0, 0.0),
            p(1.0, 0.0),
            p(0.0, 1.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
            p(5.0, 0.0),
            p(6.0, 0.0),
            p(5.0, 1.0),
        ];
        let counted = |indices: Vec<u32>, doc: &mut Document| {
            let mesh = kernel_api::TriMesh {
                positions: positions.clone(),
                normals: vec![[0.0, 0.0, 1.0]; positions.len()],
                indices,
                ..Default::default()
            };
            doc.set_imported_geometry(
                body,
                core_document::ImportedGeometry {
                    mesh: std::sync::Arc::new(mesh),
                    ..bare_geometry(None)
                },
            );
            drawn_pieces(doc, body)
        };
        assert_eq!(counted(vec![0, 1, 2, 3, 4, 5], &mut doc), 1);
        assert_eq!(counted(vec![0, 1, 2, 3, 4, 5, 6, 7, 8], &mut doc), 2);
    }
}
