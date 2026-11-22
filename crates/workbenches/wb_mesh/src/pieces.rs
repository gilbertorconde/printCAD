//! A mesh body of several loose pieces split into one mesh body per
//! piece: each takes its piece's triangles as its own mesh, the body
//! split's placement and look, and stands where the body stood, which is
//! hidden. One undoable change.

use super::*;

/// The pieces of `mesh`, in the order of their first triangles, each its
/// triangles by their numbers in the mesh: triangles joined where they
/// share a corner, corners at one point taken as one. A piece is what
/// holds together whichever way its triangles wind: the kernel's own
/// pieces part across an edge two triangles run the same way along, which
/// would split off every patch of a scan's faces turned the wrong way.
pub fn piece_triangles(mesh: &TriMesh) -> Vec<Vec<u32>> {
    let mut first: std::collections::HashMap<[u32; 3], usize> = Default::default();
    let welded: Vec<usize> = mesh
        .positions
        .iter()
        .enumerate()
        .map(|(i, p)| *first.entry(p.map(f32::to_bits)).or_insert(i))
        .collect();
    let mut parent: Vec<usize> = (0..mesh.positions.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let triangles = mesh.indices.as_chunks::<3>().0;
    for t in triangles {
        let [a, b, c] = t.map(|v| welded[v as usize]);
        for other in [b, c] {
            let (ra, ro) = (root(&mut parent, a), root(&mut parent, other));
            parent[ra] = ro;
        }
    }
    let mut piece_of_root: std::collections::HashMap<usize, usize> = Default::default();
    let mut out: Vec<Vec<u32>> = Vec::new();
    for (k, t) in triangles.iter().enumerate() {
        let r = root(&mut parent, welded[t[0] as usize]);
        let piece = *piece_of_root.entry(r).or_insert_with(|| {
            out.push(Vec::new());
            out.len() - 1
        });
        out[piece].push(k as u32);
    }
    out
}

/// The pieces of `mesh` ([`piece_triangles`]), each a mesh of its own.
/// Normals and colours go with their vertices.
pub fn pieces(mesh: &TriMesh) -> Vec<TriMesh> {
    let normals = mesh.normals.len() == mesh.positions.len();
    let colors = mesh.colors.len() == mesh.positions.len();
    let triangles = mesh.indices.as_chunks::<3>().0;
    piece_triangles(mesh)
        .into_iter()
        .map(|members| {
            let mut piece = TriMesh::default();
            let mut renumbered: std::collections::HashMap<u32, u32> = Default::default();
            for t in members {
                for v in triangles[t as usize] {
                    let at = *renumbered.entry(v).or_insert_with(|| {
                        piece.positions.push(mesh.positions[v as usize]);
                        if normals {
                            piece.normals.push(mesh.normals[v as usize]);
                        }
                        if colors {
                            piece.colors.push(mesh.colors[v as usize]);
                        }
                        (piece.positions.len() - 1) as u32
                    });
                    piece.indices.push(at);
                }
            }
            piece
        })
        .collect()
}

/// Split `body`'s mesh as it is drawn now, its history applied, into one
/// new mesh body per piece, named after it and numbered from 1, each
/// placed and drawn as it is; `body` is hidden. The new bodies, in the
/// order of `pieces`.
pub(crate) fn split(
    ctx: &mut WorkbenchRuntimeContext,
    body: BodyId,
) -> Result<Vec<BodyId>, CommandError> {
    let (drawn, _) = ctx
        .document
        .derived()
        .local_geometry(body)
        .ok_or_else(|| CommandError::failed("the scan has no mesh"))?;
    let pieces = pieces(&drawn);
    if pieces.len() < 2 {
        return Err(CommandError::failed(
            "the mesh is one piece: there is nothing to split",
        ));
    }
    let source = ctx
        .document
        .body(body)
        .cloned()
        .ok_or_else(|| CommandError::bad("scan", "names no body"))?;
    let mut made = Vec::with_capacity(pieces.len());
    for (k, piece) in pieces.iter().enumerate() {
        let new = ctx
            .document
            .create_mesh_body(format!("{} {}", source.name, k + 1), piece)
            .ok_or_else(|| CommandError::failed("a piece did not make a body"))?;
        ctx.document.set_body_placement(new, source.placement);
        ctx.document.set_body_display(new, source.display);
        ctx.document.set_body_shading(new, source.shading);
        ctx.document.set_body_material(new, source.material.clone());
        ctx.document.set_body_scan(new, source.scan);
        if source.component.is_some() {
            ctx.document
                .set_body_component(new, source.component)
                .map_err(|e| CommandError::failed(e.to_string()))?;
        }
        made.push(new);
    }
    ctx.document.set_body_visible(body, false);
    Ok(made)
}

/// What `mesh.split` answers: the new bodies, by id.
pub(crate) fn answer(made: &[BodyId]) -> Value {
    json!({"bodies": made.iter().map(|b| b.0.to_string()).collect::<Vec<_>>()})
}
