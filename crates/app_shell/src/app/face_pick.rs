//! Faces picked on a body's mesh: the pure part is `doc_render::face_pick`;
//! here is what reads the GPU pick.

pub(crate) use doc_render::face_pick::*;

/// The picked triangle as one of the document's: `drawn` is the body's
/// entry in `DocumentSession::drawn.meshes`, the revision it was last
/// submitted with and its geometry's. A pick drawn with another revision,
/// or a body drawn with a mesh of its own, names no triangle of the
/// document's.
pub(crate) fn document_triangle(
    picked: render_wgpu::PickedTriangle,
    drawn: Option<(u64, u64)>,
) -> Option<HoveredTriangle> {
    let (submitted, geometry) = drawn?;
    (submitted == picked.revision).then_some(HoveredTriangle {
        index: picked.index,
        revision: geometry,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pick_counts_in_the_document_mesh_only_as_it_was_drawn() {
        use super::{HoveredTriangle, document_triangle};
        let picked = render_wgpu::PickedTriangle {
            index: 3,
            revision: 9,
        };
        assert_eq!(
            document_triangle(picked, Some((9, 2))),
            Some(HoveredTriangle {
                index: 3,
                revision: 2
            }),
            "drawn as submitted: the geometry's revision"
        );
        assert_eq!(
            document_triangle(picked, Some((10, 2))),
            None,
            "re-meshed since"
        );
        assert_eq!(
            document_triangle(picked, None),
            None,
            "drawn with its own mesh"
        );
    }
}
